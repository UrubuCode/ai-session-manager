//! Onde está o executável de cada ferramenta.
//!
//! Ordem de busca:
//! 1. caminho registrado pelo usuário no app (`~/.ai-sessions/executables.json`);
//! 2. `PATH` do processo e o `PATH` atual do registro (usuário + máquina), que
//!    pega instalações feitas depois do app abrir;
//! 3. locais conhecidos de cada forma de instalação (instalador nativo, npm,
//!    pnpm, bun, scoop, volta, extensões do VS Code/Cursor/Windsurf...).
//!
//! Ex.: o Codex instalado só pela extensão do VS Code fica em
//! `~/.codex/.sandbox-bin/codex.exe`, fora do PATH.

use crate::tools::Tool;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Custom,
    Path,
    Known,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub path: String,
    pub source: Source,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExeInfo {
    pub tool: Tool,
    pub label: String,
    pub command: String,
    /// Caminho registrado no app (mesmo que não exista mais).
    pub custom: Option<String>,
    /// O caminho registrado não aponta para um arquivo.
    pub custom_missing: bool,
    /// Executável que será usado.
    pub resolved: Option<Candidate>,
    /// Tudo o que foi encontrado (sem o registrado).
    pub candidates: Vec<Candidate>,
}

fn file() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".ai-sessions").join("executables.json")
}

fn load() -> BTreeMap<String, String> {
    fs::read(file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(map: &BTreeMap<String, String>) -> Result<(), String> {
    let f = file();
    if let Some(d) = f.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = f.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(map).unwrap()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &f).map_err(|e| e.to_string())
}

pub fn custom(tool: Tool) -> Option<String> {
    load().remove(tool.id()).filter(|p| !p.trim().is_empty())
}

/// Registra (ou remove, com `None`/vazio) o caminho do executável.
pub fn set_custom(tool: Tool, path: Option<&str>) -> Result<(), String> {
    let mut map = load();
    match path.map(clean).filter(|p| !p.is_empty()) {
        None => {
            map.remove(tool.id());
        }
        Some(p) => {
            let pb = PathBuf::from(&p);
            let pb = if pb.is_dir() { find_in_dir(&pb, tool.command()).ok_or_else(|| format!("Nenhum '{}' dentro de {p}.", tool.command()))? } else { pb };
            if !pb.is_file() {
                return Err(format!("Arquivo não encontrado: {}", pb.display()));
            }
            map.insert(tool.id().into(), pb.display().to_string());
        }
    }
    save(&map)
}

/// Tira aspas e espaços de um caminho colado.
fn clean(p: &str) -> String {
    p.trim().trim_matches('"').trim().to_string()
}

fn exts() -> Vec<String> {
    if cfg!(windows) {
        let mut v: Vec<String> = std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_lowercase())
            .collect();
        if !v.iter().any(|e| e == ".ps1") {
            v.push(".ps1".into());
        }
        v
    } else {
        vec![String::new()]
    }
}

fn find_in_dir(dir: &Path, cmd: &str) -> Option<PathBuf> {
    exts().iter().map(|e| dir.join(format!("{cmd}{e}"))).find(|p| p.is_file())
}

/// Pastas do PATH: do processo e, no Windows, as atuais do registro.
fn path_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        use winreg::RegKey;
        let read = |root, sub: &str| -> Option<String> { RegKey::predef(root).open_subkey(sub).ok()?.get_value("Path").ok() };
        for v in [
            read(HKEY_CURRENT_USER, "Environment"),
            read(HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"),
        ]
        .into_iter()
        .flatten()
        {
            for part in v.split(';').filter(|s| !s.trim().is_empty()) {
                out.push(PathBuf::from(expand(part.trim())));
            }
        }
    }
    out
}

/// Expande `%VAR%` (valores REG_EXPAND_SZ do PATH).
fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) => {
                let name = &after[..j];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

/// Pastas de gerenciadores de pacote que costumam ficar fora do PATH do app.
fn package_manager_dirs(home: &Path) -> Vec<PathBuf> {
    let mut v = vec![
        home.join(".local").join("bin"),
        home.join(".bun").join("bin"),
        home.join(".volta").join("bin"),
        home.join(".cargo").join("bin"),
        home.join("scoop").join("shims"),
    ];
    if let Some(a) = env_dir("APPDATA") {
        v.push(a.join("npm"));
    }
    if let Some(l) = env_dir("LOCALAPPDATA") {
        v.push(l.join("pnpm"));
        v.push(l.join("Volta").join("bin"));
    }
    if let Some(p) = env_dir("PNPM_HOME") {
        v.push(p);
    }
    if let Some(p) = env_dir("NVM_SYMLINK") {
        v.push(p);
    }
    if let Some(p) = env_dir("ProgramData") {
        v.push(p.join("chocolatey").join("bin"));
    }
    if let Some(p) = env_dir("ProgramFiles") {
        v.push(p.join("nodejs"));
    }
    v
}

/// Caminhos fixos conhecidos de cada ferramenta.
fn tool_paths(tool: Tool, home: &Path) -> Vec<PathBuf> {
    let local = env_dir("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData").join("Local"));
    match tool {
        Tool::Claude => vec![
            home.join(".local").join("bin").join("claude.exe"),
            home.join(".claude").join("local").join("claude.exe"),
            home.join(".claude").join("local").join("claude.cmd"),
            local.join("Programs").join("claude-code").join("claude.exe"),
            local.join("AnthropicClaude").join("claude.exe"),
        ],
        Tool::Codex => vec![
            local.join("Programs").join("OpenAI").join("Codex").join("bin").join("codex.exe"),
            local.join("Programs").join("codex").join("codex.exe"),
        ],
        Tool::Agy => vec![
            local.join("agy").join("bin").join("agy.exe"),
            local.join("Programs").join("Antigravity").join("bin").join("agy.exe"),
            local.join("Programs").join("Antigravity").join("bin").join("agy.cmd"),
            local.join("Programs").join("antigravity").join("bin").join("agy"),
        ],
    }
}

/// Executáveis embutidos em extensões de editor (pasta com versão no nome).
fn extension_paths(tool: Tool, home: &Path) -> Vec<PathBuf> {
    let (prefixes, rels): (&[&str], &[&[&str]]) = match tool {
        Tool::Claude => (&["anthropic.claude-code-"], &[&["resources", "native-binary", "claude.exe"]]),
        Tool::Codex => (
            &["openai.chatgpt-", "openai.codex-"],
            &[&["bin", "windows-x86_64", "codex.exe"], &["bin", "windows-aarch64", "codex.exe"], &["bin", "codex.exe"]],
        ),
        Tool::Agy => (&[], &[]),
    };
    let mut out = vec![];
    for ed in [".vscode", ".vscode-insiders", ".cursor", ".windsurf", ".antigravity"] {
        let Ok(rd) = fs::read_dir(home.join(ed).join("extensions")) else { continue };
        let mut dirs: Vec<PathBuf> = rd
            .filter_map(|e| e.ok())
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_lowercase();
                prefixes.iter().any(|p| n.starts_with(p))
            })
            .map(|e| e.path())
            .collect();
        // versão mais nova primeiro
        dirs.sort_by_key(|d| std::cmp::Reverse(fs::metadata(d).and_then(|m| m.modified()).ok()));
        for d in dirs {
            for rel in rels {
                out.push(rel.iter().fold(d.clone(), |p, s| p.join(s)));
            }
        }
    }
    out
}

/// Codex do `.sandbox-bin` (sem pacote completo): precisa de `--no-daemon`.
pub fn needs_no_daemon(p: &Path) -> bool {
    key(p).contains(r"\.sandbox-bin\")
}

fn key(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// Tudo o que for encontrado, sem repetições, na ordem de preferência.
pub fn candidates(tool: Tool) -> Vec<Candidate> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    let mut push = |p: PathBuf, source: Source| {
        if p.is_file() && seen.insert(key(&p)) {
            out.push(Candidate { path: p.display().to_string(), source });
        }
    };
    for d in path_dirs() {
        if let Some(p) = find_in_dir(&d, tool.command()) {
            push(p, Source::Path);
        }
    }
    for d in package_manager_dirs(&home) {
        if let Some(p) = find_in_dir(&d, tool.command()) {
            push(p, Source::Known);
        }
    }
    for p in tool_paths(tool, &home).into_iter().chain(extension_paths(tool, &home)) {
        push(p, Source::Known);
    }
    if tool == Tool::Codex {
        // Último recurso: o binário que a extensão do VS Code deixa aqui não tem o
        // pacote completo e só roda com `--no-daemon` (ver `needs_no_daemon`).
        push(home.join(".codex").join(".sandbox-bin").join("codex.exe"), Source::Known);
    }
    out
}

/// Executável que será usado para a ferramenta.
pub fn resolve(tool: Tool) -> Option<PathBuf> {
    if let Some(c) = custom(tool).map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(c);
    }
    candidates(tool).into_iter().next().map(|c| PathBuf::from(c.path))
}

pub fn info(tool: Tool) -> ExeInfo {
    let custom = custom(tool);
    let custom_ok = custom.as_ref().is_some_and(|p| Path::new(p).is_file());
    let mut candidates = candidates(tool);
    let resolved = if custom_ok {
        let c = custom.clone().unwrap();
        candidates.retain(|x| key(Path::new(&x.path)) != key(Path::new(&c)));
        Some(Candidate { path: c, source: Source::Custom })
    } else {
        candidates.first().cloned()
    };
    ExeInfo {
        tool,
        label: tool.label().into(),
        command: tool.command().into(),
        custom_missing: custom.is_some() && !custom_ok,
        custom,
        resolved,
        candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_env_vars() {
        std::env::set_var("ASM_TEST_X", r"C:\x");
        assert_eq!(expand(r"%ASM_TEST_X%\bin"), r"C:\x\bin");
        assert_eq!(expand(r"%NAO_EXISTE_ASM%\a"), r"%NAO_EXISTE_ASM%\a");
        assert_eq!(expand("sem%"), "sem%");
    }

    #[test]
    fn finds_command_in_dir() {
        let d = tempfile::tempdir().unwrap();
        assert!(find_in_dir(d.path(), "codex").is_none());
        fs::write(d.path().join(if cfg!(windows) { "codex.exe" } else { "codex" }), "").unwrap();
        assert!(find_in_dir(d.path(), "codex").is_some());
    }

    #[test]
    fn sandbox_bin_needs_no_daemon() {
        assert!(needs_no_daemon(Path::new(r"C:\Users\danie\.codex\.sandbox-bin\codex.exe")));
        assert!(!needs_no_daemon(Path::new(r"C:\Users\a\.vscode\extensions\openai.chatgpt-1\bin\windows-x86_64\codex.exe")));
    }

    #[test]
    fn cleans_pasted_paths() {
        assert_eq!(clean(r#"  "C:\a b\codex.exe"  "#), r"C:\a b\codex.exe");
    }
}
