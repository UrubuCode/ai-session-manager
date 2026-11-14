//! Skills globais compartilhadas entre ferramentas.
//!
//! A fonte de cada skill global é `~/.ai-skills/<skill>/SKILL.md`. Ligar uma
//! skill numa ferramenta cria um link (junction) na pasta de skills dela:
//!   Claude  → ~/.claude/skills/<skill>
//!   Codex   → ~/.codex/skills/<skill>
//!   agy     → ~/.gemini/config/skills/<skill>
//! Os perfis de cada ferramenta já compartilham essa pasta, então a skill vale
//! para todas as contas.

use crate::links::{is_link, link_target, make_dir_link, remove_link, same_path};
use crate::tools::Tool;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LinkState {
    /// Ligada: a pasta da ferramenta aponta para a skill global.
    On,
    /// Desligada.
    Off,
    /// A ferramenta tem uma skill própria com o mesmo nome.
    Conflict,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ToolLink {
    pub tool: Tool,
    pub state: LinkState,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSkill {
    pub name: String,
    pub description: Option<String>,
    pub path: String,
    pub links: Vec<ToolLink>,
}

/// Skill que existe só numa ferramenta (candidata a "tornar global").
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LocalSkill {
    pub tool: Tool,
    pub name: String,
    pub description: Option<String>,
    pub path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsState {
    pub root: String,
    pub global: Vec<GlobalSkill>,
    pub local: Vec<LocalSkill>,
}

pub struct Skills {
    home: PathBuf,
    root: PathBuf,
}

/// Lê `name`/`description` do frontmatter YAML simples do SKILL.md.
pub fn frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let (mut name, mut desc) = (None, None);
    for l in lines {
        if l.trim() == "---" {
            break;
        }
        let unq = |v: &str| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
        if let Some(v) = l.strip_prefix("name:") {
            name = Some(unq(v)).filter(|s| !s.is_empty());
        } else if let Some(v) = l.strip_prefix("description:") {
            desc = Some(unq(v)).filter(|s| !s.is_empty());
        }
    }
    (name, desc)
}

fn read_desc(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join("SKILL.md")).ok().and_then(|t| frontmatter(&t).1)
}

pub fn validate_skill_name(name: &str) -> Result<String, String> {
    let n = name.trim().to_lowercase().replace(' ', "-");
    if n.is_empty() || n.len() > 64 {
        return Err("O nome da skill deve ter de 1 a 64 caracteres.".into());
    }
    if !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') || n.starts_with('-') {
        return Err("Use letras minúsculas, números, '-' ou '_' no nome da skill.".into());
    }
    Ok(n)
}

fn subdirs(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(p)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    v
}

impl Skills {
    pub fn new(home: PathBuf) -> Self {
        let root = home.join(".ai-skills");
        Self { home, root }
    }

    pub fn from_env() -> Self {
        Self::new(dirs::home_dir().unwrap_or_default())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn tool_dir(&self, tool: Tool) -> PathBuf {
        tool.skills_dir(&self.home)
    }

    fn link_state(&self, tool: Tool, name: &str) -> LinkState {
        let dst = self.tool_dir(tool).join(name);
        if fs::symlink_metadata(&dst).is_err() {
            return LinkState::Off;
        }
        match link_target(&dst) {
            Some(t) if same_path(&t, &self.root.join(name)) => LinkState::On,
            _ => LinkState::Conflict,
        }
    }

    pub fn state(&self) -> SkillsState {
        let global: Vec<GlobalSkill> = subdirs(&self.root)
            .into_iter()
            .filter(|d| d.join("SKILL.md").is_file())
            .map(|d| {
                let name = d.file_name().unwrap().to_string_lossy().into_owned();
                GlobalSkill {
                    description: read_desc(&d),
                    path: d.display().to_string(),
                    links: Tool::ALL.into_iter().map(|t| ToolLink { tool: t, state: self.link_state(t, &name) }).collect(),
                    name,
                }
            })
            .collect();
        let mut local = vec![];
        for tool in Tool::ALL {
            for d in subdirs(&self.tool_dir(tool)) {
                if !d.join("SKILL.md").is_file() {
                    continue;
                }
                // links para a pasta global (ou outros links) não são "locais"
                if is_link(&d) {
                    continue;
                }
                let name = d.file_name().unwrap().to_string_lossy().into_owned();
                local.push(LocalSkill { tool, description: read_desc(&d), path: d.display().to_string(), name });
            }
        }
        SkillsState { root: self.root.display().to_string(), global, local }
    }

    /// Liga ou desliga uma skill global numa ferramenta.
    pub fn set_link(&self, name: &str, tool: Tool, on: bool) -> Result<(), String> {
        let src = self.root.join(name);
        if !src.join("SKILL.md").is_file() {
            return Err(format!("A skill global '{name}' não existe."));
        }
        let dst = self.tool_dir(tool).join(name);
        match (on, self.link_state(tool, name)) {
            (true, LinkState::On) | (false, LinkState::Off) => Ok(()),
            (_, LinkState::Conflict) => Err(format!(
                "{} já tem uma skill própria chamada '{name}'. Torne-a global ou renomeie antes.",
                tool.label()
            )),
            (true, LinkState::Off) => {
                fs::create_dir_all(self.tool_dir(tool)).map_err(|e| e.to_string())?;
                make_dir_link(&src, &dst).map_err(|e| format!("Não foi possível ligar em {}: {e}", tool.label()))
            }
            (false, LinkState::On) => remove_link(&dst).map_err(|e| e.to_string()),
        }
    }

    /// Move uma skill própria de uma ferramenta para a pasta global e deixa um
    /// link no lugar (para a ferramenta nada muda).
    pub fn make_global(&self, tool: Tool, name: &str) -> Result<(), String> {
        let src = self.tool_dir(tool).join(name);
        if is_link(&src) || !src.join("SKILL.md").is_file() {
            return Err(format!("'{name}' não é uma skill própria de {}.", tool.label()));
        }
        let dst = self.root.join(name);
        if dst.exists() {
            return Err(format!("Já existe uma skill global chamada '{name}'."));
        }
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        fs::rename(&src, &dst).map_err(|e| format!("Não foi possível mover a skill (feche editores que a usam): {e}"))?;
        if let Err(e) = make_dir_link(&dst, &src) {
            // desfaz para não sumir com a skill da ferramenta
            let _ = fs::rename(&dst, &src);
            return Err(format!("Não foi possível criar o link de volta: {e}"));
        }
        Ok(())
    }

    /// Cria uma skill global nova a partir de um modelo.
    pub fn create(&self, name: &str, description: &str) -> Result<String, String> {
        let n = validate_skill_name(name)?;
        let dir = self.root.join(&n);
        if dir.exists() {
            return Err(format!("Já existe uma skill global chamada '{n}'."));
        }
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let desc = if description.trim().is_empty() { "Descreva quando o agente deve usar esta skill." } else { description.trim() };
        let body = format!(
            "---\nname: {n}\ndescription: {}\n---\n\n# {n}\n\nInstruções para o agente.\n",
            desc.replace('\n', " ")
        );
        fs::write(dir.join("SKILL.md"), body).map_err(|e| e.to_string())?;
        Ok(n)
    }


    pub fn dir_of(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter() {
        let (n, d) = frontmatter("---\nname: x\ndescription: \"Faz y\"\n---\ncorpo");
        assert_eq!((n.as_deref(), d.as_deref()), (Some("x"), Some("Faz y")));
        assert_eq!(frontmatter("sem frontmatter"), (None, None));
    }

    #[test]
    fn link_make_global_and_conflict() {
        let d = tempfile::tempdir().unwrap();
        let s = Skills::new(d.path().to_path_buf());
        let n = s.create("Minha Skill", "teste").unwrap();
        assert_eq!(n, "minha-skill");
        s.set_link(&n, Tool::Codex, true).unwrap();
        let st = s.state();
        let g = &st.global[0];
        assert_eq!(g.description.as_deref(), Some("teste"));
        let state = |t| g.links.iter().find(|l| l.tool == t).unwrap().state;
        assert_eq!(state(Tool::Codex), LinkState::On);
        assert_eq!(state(Tool::Claude), LinkState::Off);
        assert!(d.path().join(".codex/skills/minha-skill/SKILL.md").exists());
        assert!(st.local.is_empty(), "link não é skill local");

        // skill própria do Claude → global, com link de volta
        let own = d.path().join(".claude").join("skills").join("propria");
        fs::create_dir_all(&own).unwrap();
        fs::write(own.join("SKILL.md"), "---\nname: propria\n---\n").unwrap();
        assert_eq!(s.state().local.len(), 1);
        s.make_global(Tool::Claude, "propria").unwrap();
        assert!(own.join("SKILL.md").exists(), "Claude continua vendo a skill");
        assert!(d.path().join(".ai-skills/propria/SKILL.md").exists());
        assert!(s.state().local.is_empty());

        // conflito: Codex tem uma skill própria com o mesmo nome
        let c = d.path().join(".codex").join("skills").join("propria");
        fs::create_dir_all(&c).unwrap();
        fs::write(c.join("SKILL.md"), "x").unwrap();
        assert!(s.set_link("propria", Tool::Codex, true).is_err());

        // desligar remove só o link
        s.set_link(&n, Tool::Codex, false).unwrap();
        assert!(!d.path().join(".codex/skills/minha-skill").exists());
        assert!(d.path().join(".ai-skills/minha-skill/SKILL.md").exists());
    }
}
