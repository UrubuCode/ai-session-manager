//! Sincronização de conversas entre perfis.
//!
//! Uma conversa "em sync" em vários perfis é o MESMO arquivo (hard link) na
//! pasta de cada perfil: continuar em qualquer conta grava para todas. Isso só
//! funciona porque as ferramentas acrescentam ao arquivo em vez de recriá-lo;
//! se um dia o link se desfizer, a conversa aparece como "fora de sync" e pode
//! ser juntada de novo (guardando a versão antiga como backup).
//!
//! Cada ferramenta tem um adaptador que diz onde ficam as conversas dela:
//!   Claude Code → `<perfil>/projects/<projeto>/<id>.jsonl` (+ pasta `<id>/` de subagentes)
//!   Codex, Antigravity → ainda não suportados.

use crate::links::{is_link, make_dir_link, remove_link};
use crate::profiles::{ProfileInfo, Store};
use crate::tools::Tool;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    /// Em sync: o perfil tem o mesmo arquivo que os demais.
    On,
    /// O perfil não tem esta conversa.
    Off,
    /// O perfil tem uma cópia separada (o link se desfez).
    Diverged,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePresence {
    pub profile: String,
    pub state: Presence,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    /// Chave da pasta do projeto (ex.: `C--Users-x-Documents-zun`).
    pub project: String,
    /// Pasta real do projeto, quando registrada na conversa.
    pub cwd: Option<String>,
    /// `None` enquanto a conversa ainda não foi analisada (ver `analyze`).
    pub title: Option<String>,
    pub updated_at: i64,
    pub size: u64,
    pub profiles: Vec<ProfilePresence>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationsState {
    pub tool: Tool,
    pub supported: bool,
    /// Por que não é suportado (quando `supported` é falso).
    pub note: Option<String>,
    pub profiles: Vec<String>,
    pub conversations: Vec<Conversation>,
    /// Quantas conversas ainda não têm título (serão analisadas em segundo plano).
    pub pending: usize,
}

/// Título/pasta descobertos de uma conversa (enviados em lotes à interface).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Described {
    pub project: String,
    pub id: String,
    pub title: String,
    pub cwd: Option<String>,
}

// ------------------------------------------------------------------ cache de títulos

/// Cache em `~/.ai-sessions/conversation-cache.json`: chave = caminho + data +
/// tamanho do arquivo, então uma conversa alterada é analisada de novo.
#[derive(Default, Serialize, Deserialize)]
struct Cache(HashMap<String, (String, Option<String>)>);

fn default_cache_file() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".ai-sessions").join("conversation-cache.json")
}

fn cache_key(file: &Path) -> String {
    let m = fs::metadata(file).ok();
    format!(
        "{}|{}|{}",
        file.display(),
        m.as_ref().map(|m| m.len()).unwrap_or(0),
        modified_ms(file)
    )
}

impl Cache {
    fn load(file: &Path) -> Self {
        fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    fn save(&self, f: &Path) {
        if let Some(d) = f.parent() {
            let _ = fs::create_dir_all(d);
        }
        let tmp = f.with_extension("json.tmp");
        if fs::write(&tmp, serde_json::to_vec(self).unwrap_or_default()).is_ok() {
            let _ = fs::rename(&tmp, f);
        }
    }
}

// ------------------------------------------------------------------ adaptadores

/// Uma conversa encontrada na pasta de um perfil.
struct Found {
    project: String,
    id: String,
    file: PathBuf,
}

fn supported(tool: Tool) -> Result<(), String> {
    match tool {
        Tool::Claude => Ok(()),
        Tool::Codex => Err("O Codex guarda as conversas em arquivos e num banco SQLite; o sync ainda não foi implementado.".into()),
        Tool::Agy => Err("O Antigravity guarda as conversas em formato próprio e SQLite; o sync ainda não foi implementado.".into()),
    }
}

/// Pasta onde a ferramenta guarda as conversas dentro de um perfil.
fn conversations_root(tool: Tool, profile_dir: &Path) -> PathBuf {
    match tool {
        Tool::Claude | Tool::Codex | Tool::Agy => profile_dir.join("projects"),
    }
}

fn file_path(tool: Tool, profile_dir: &Path, project: &str, id: &str) -> PathBuf {
    conversations_root(tool, profile_dir).join(project).join(format!("{id}.jsonl"))
}

fn extra_dir_path(tool: Tool, profile_dir: &Path, project: &str, id: &str) -> PathBuf {
    conversations_root(tool, profile_dir).join(project).join(id)
}

fn scan(tool: Tool, profile_dir: &Path) -> Vec<Found> {
    let root = conversations_root(tool, profile_dir);
    let mut out = vec![];
    let Ok(projects) = fs::read_dir(&root) else { return out };
    for p in projects.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()) {
        let project = p.file_name().to_string_lossy().into_owned();
        let Ok(files) = fs::read_dir(p.path()) else { continue };
        for f in files.filter_map(|e| e.ok()) {
            let path = f.path();
            if path.extension().is_none_or(|x| x != "jsonl") || !path.is_file() {
                continue;
            }
            let id = path.file_stem().unwrap().to_string_lossy().into_owned();
            out.push(Found { project: project.clone(), id, file: path });
        }
    }
    out
}

/// Até onde ler cada conversa para achar o título (o resto pode ter MB).
const DESCRIBE_BYTES: u64 = 128 * 1024;
/// Linhas maiores que isso (saídas de ferramentas) são puladas sem analisar.
const MAX_LINE: usize = 32 * 1024;

/// Título e pasta do projeto a partir do começo do arquivo.
fn describe(file: &Path) -> (String, Option<String>) {
    let mut title: Option<String> = None;
    let mut summary: Option<String> = None;
    let mut cwd: Option<String> = None;
    if let Ok(f) = fs::File::open(file) {
        for line in BufReader::new(f.take(DESCRIBE_BYTES)).lines().map_while(Result::ok) {
            if line.len() > MAX_LINE {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            if cwd.is_none() {
                cwd = v.get("cwd").and_then(|c| c.as_str()).map(String::from);
            }
            match v.get("type").and_then(|t| t.as_str()) {
                Some("summary") if summary.is_none() => {
                    summary = v.get("summary").and_then(|s| s.as_str()).map(String::from);
                }
                Some("user") if title.is_none() && v.get("isMeta").and_then(|m| m.as_bool()) != Some(true) => {
                    let content = v.get("message").and_then(|m| m.get("content"));
                    let text = match content {
                        Some(serde_json::Value::String(s)) => Some(s.clone()),
                        Some(serde_json::Value::Array(a)) => a
                            .iter()
                            .find_map(|c| (c.get("type")?.as_str()? == "text").then(|| c.get("text")?.as_str().map(String::from))?),
                        _ => None,
                    };
                    title = text
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty() && !t.starts_with('<') && !t.starts_with("Caveat:"));
                }
                _ => {}
            }
            if title.is_some() && cwd.is_some() {
                break;
            }
        }
    }
    let mut t = summary.or(title).unwrap_or_else(|| "(sem título)".into());
    t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > 140 {
        t = t.chars().take(140).collect::<String>() + "…";
    }
    (t, cwd)
}

fn modified_ms(p: &Path) -> i64 {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn same_file(a: &Path, b: &Path) -> bool {
    same_file::is_same_file(a, b).unwrap_or(false)
}

/// (projeto, id), arquivos por perfil e o arquivo principal.
type Indexed = ((String, String), Vec<(String, PathBuf)>, PathBuf);

// ------------------------------------------------------------------ operações

pub struct Conversations {
    tool: Tool,
    profiles: Vec<(String, PathBuf)>,
    cache_file: PathBuf,
}

impl Conversations {
    pub fn new(tool: Tool, profiles: &[ProfileInfo]) -> Self {
        Self {
            tool,
            profiles: profiles.iter().map(|p| (p.name.clone(), PathBuf::from(&p.path))).collect(),
            cache_file: default_cache_file(),
        }
    }

    pub fn from_env(tool: Tool) -> Self {
        let st = Store::from_env(tool).state(None);
        Self::new(tool, &st.profiles)
    }

    fn dir_of(&self, profile: &str) -> Result<&Path, String> {
        self.profiles
            .iter()
            .find(|(n, _)| n == profile)
            .map(|(_, d)| d.as_path())
            .ok_or_else(|| format!("Perfil '{profile}' não encontrado."))
    }

    /// (projeto, id) → arquivos por perfil, e o arquivo principal de cada um.
    fn index(&self) -> Vec<Indexed> {
        let mut map: BTreeMap<(String, String), Vec<(String, PathBuf)>> = BTreeMap::new();
        for (name, dir) in &self.profiles {
            for f in scan(self.tool, dir) {
                map.entry((f.project, f.id)).or_default().push((name.clone(), f.file));
            }
        }
        map.into_iter()
            .map(|(k, files)| {
                // a versão "principal" é a modificada mais recentemente
                let main = files.iter().max_by_key(|(_, p)| modified_ms(p)).unwrap().1.clone();
                (k, files, main)
            })
            .collect()
    }

    /// Lista rápida: só metadados dos arquivos + títulos já em cache. Não lê o
    /// conteúdo das conversas (isso fica para `analyze`, em segundo plano).
    pub fn state(&self) -> ConversationsState {
        let names: Vec<String> = self.profiles.iter().map(|(n, _)| n.clone()).collect();
        if let Err(note) = supported(self.tool) {
            return ConversationsState {
                tool: self.tool,
                supported: false,
                note: Some(note),
                profiles: names,
                conversations: vec![],
                pending: 0,
            };
        }
        let cache = Cache::load(&self.cache_file);
        let mut conversations: Vec<Conversation> = self
            .index()
            .into_iter()
            .map(|((project, id), files, main)| {
                let (title, cwd) = match cache.0.get(&cache_key(&main)) {
                    Some((t, c)) => (Some(t.clone()), c.clone()),
                    None => (None, None),
                };
                let profiles = names
                    .iter()
                    .map(|n| {
                        let state = match files.iter().find(|(pn, _)| pn == n) {
                            None => Presence::Off,
                            Some((_, p)) if same_file(p, &main) => Presence::On,
                            Some(_) => Presence::Diverged,
                        };
                        ProfilePresence { profile: n.clone(), state }
                    })
                    .collect();
                Conversation {
                    updated_at: modified_ms(&main),
                    size: fs::metadata(&main).map(|m| m.len()).unwrap_or(0),
                    id,
                    project,
                    cwd,
                    title,
                    profiles,
                }
            })
            .collect();
        conversations.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
        let pending = conversations.iter().filter(|c| c.title.is_none()).count();
        ConversationsState { tool: self.tool, supported: true, note: None, profiles: names, conversations, pending }
    }

    /// Analisa (lê o começo) das conversas sem título em cache, das mais
    /// recentes para as mais antigas, chamando `on_batch` a cada `batch`
    /// conversas. `cancel` permite parar (ex.: usuário trocou de aba).
    pub fn analyze(
        &self,
        batch: usize,
        mut on_batch: impl FnMut(Vec<Described>, usize),
        cancel: impl Fn() -> bool,
    ) {
        if supported(self.tool).is_err() {
            return;
        }
        let mut cache = Cache::load(&self.cache_file);
        let mut todo: Vec<_> = self
            .index()
            .into_iter()
            .filter(|(_, _, main)| !cache.0.contains_key(&cache_key(main)))
            .collect();
        todo.sort_by_key(|(_, _, main)| std::cmp::Reverse(modified_ms(main)));
        let mut done = 0;
        let mut buf = vec![];
        for ((project, id), _, main) in todo {
            if cancel() {
                break;
            }
            let (title, cwd) = describe(&main);
            cache.0.insert(cache_key(&main), (title.clone(), cwd.clone()));
            buf.push(Described { project, id, title, cwd });
            done += 1;
            if buf.len() >= batch {
                on_batch(std::mem::take(&mut buf), done);
            }
        }
        if !buf.is_empty() {
            on_batch(buf, done);
        }
        // remove do cache arquivos que não existem mais
        cache.0.retain(|k, _| k.split('|').next().is_some_and(|p| Path::new(p).is_file()));
        cache.save(&self.cache_file);
    }

    /// Arquivo principal (mais recente) e o perfil que o tem.
    fn main_copy(&self, project: &str, id: &str) -> Result<(String, PathBuf), String> {
        self.profiles
            .iter()
            .map(|(n, d)| (n.clone(), file_path(self.tool, d, project, id)))
            .filter(|(_, p)| p.is_file())
            .max_by_key(|(_, p)| modified_ms(p))
            .ok_or_else(|| "Conversa não encontrada em nenhum perfil.".into())
    }

    /// Coloca (ou tira) a conversa de um perfil.
    pub fn set(&self, project: &str, id: &str, profile: &str, on: bool) -> Result<(), String> {
        supported(self.tool)?;
        let dir = self.dir_of(profile)?;
        let dst = file_path(self.tool, dir, project, id);
        let dst_extra = extra_dir_path(self.tool, dir, project, id);
        let (main_profile, main) = self.main_copy(project, id)?;
        if on {
            if dst.is_file() {
                return if same_file(&dst, &main) {
                    Ok(())
                } else {
                    Err("Este perfil tem uma cópia diferente da conversa. Use \"Sincronizar\" para juntar.".into())
                };
            }
            fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
            fs::hard_link(&main, &dst).map_err(|e| format!("Não foi possível ligar a conversa: {e}"))?;
            let main_extra = extra_dir_path(self.tool, self.dir_of(&main_profile)?, project, id);
            if main_extra.is_dir() && fs::symlink_metadata(&dst_extra).is_err() {
                let _ = make_dir_link(&main_extra, &dst_extra);
            }
            Ok(())
        } else {
            if !dst.is_file() {
                return Ok(());
            }
            // nunca remover a última cópia
            let others = self
                .profiles
                .iter()
                .filter(|(n, _)| n != profile)
                .map(|(_, d)| file_path(self.tool, d, project, id))
                .any(|p| p.is_file() && same_file(&p, &dst));
            if !others {
                return Err("Esta é a única cópia da conversa; ela não pode ser removida daqui.".into());
            }
            fs::remove_file(&dst).map_err(|e| format!("Não foi possível remover (a conversa está aberta?): {e}"))?;
            if is_link(&dst_extra) {
                let _ = remove_link(&dst_extra);
            }
            Ok(())
        }
    }

    /// Junta cópias divergentes: todos os perfis passam a usar a versão mais
    /// recente; as outras ficam salvas como `<id>.jsonl.bak-<data>`.
    pub fn resync(&self, project: &str, id: &str) -> Result<usize, String> {
        supported(self.tool)?;
        let (_, main) = self.main_copy(project, id)?;
        let stamp = modified_ms(&main);
        let mut fixed = 0;
        for (_, d) in &self.profiles {
            let p = file_path(self.tool, d, project, id);
            if p.is_file() && !same_file(&p, &main) {
                let bak = p.with_extension(format!("jsonl.bak-{stamp}"));
                fs::rename(&p, &bak).map_err(|e| format!("Não foi possível guardar a cópia antiga: {e}"))?;
                fs::hard_link(&main, &p).map_err(|e| e.to_string())?;
                fixed += 1;
            }
        }
        Ok(fixed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(dir: &Path, project: &str, id: &str, text: &str) -> PathBuf {
        let p = dir.join("projects").join(project);
        fs::create_dir_all(&p).unwrap();
        let f = p.join(format!("{id}.jsonl"));
        fs::write(
            &f,
            format!("{{\"type\":\"user\",\"cwd\":\"C:\\\\proj\",\"message\":{{\"role\":\"user\",\"content\":\"{text}\"}}}}\n"),
        )
        .unwrap();
        f
    }

    fn setup() -> (tempfile::TempDir, Conversations) {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        conv(&a, "proj", "s1", "implementar o goal");
        let cache_file = d.path().join("cache.json");
        let c = Conversations { tool: Tool::Claude, profiles: vec![("a".into(), a), ("b".into(), b)], cache_file };
        (d, c)
    }

    #[test]
    fn link_sync_and_unlink() {
        let (d, c) = setup();
        let st = c.state();
        assert_eq!(st.conversations.len(), 1);
        assert!(st.conversations[0].title.is_none(), "lista rápida não lê o conteúdo");
        assert_eq!(st.pending, 1);
        let mut got = vec![];
        c.analyze(10, |b, n| got.push((b, n)), || false);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0[0].title, "implementar o goal");
        let st = c.state();
        assert_eq!(st.pending, 0, "título veio do cache");
        let cv = &st.conversations[0];
        assert_eq!(cv.title.as_deref(), Some("implementar o goal"));
        assert_eq!(cv.cwd.as_deref(), Some(r"C:\proj"));
        assert_eq!(cv.profiles[1].state, Presence::Off);

        c.set("proj", "s1", "b", true).unwrap();
        assert_eq!(c.state().conversations[0].profiles[1].state, Presence::On);

        // gravar por um perfil aparece no outro (mesmo arquivo)
        let fb = d.path().join("b/projects/proj/s1.jsonl");
        let mut f = fs::OpenOptions::new().append(true).open(&fb).unwrap();
        use std::io::Write;
        writeln!(f, "{{\"type\":\"assistant\"}}").unwrap();
        drop(f);
        let fa = fs::read_to_string(d.path().join("a/projects/proj/s1.jsonl")).unwrap();
        assert!(fa.contains("assistant"));

        // tira de b; a última cópia (a) não pode sair
        c.set("proj", "s1", "b", false).unwrap();
        assert!(!fb.exists());
        assert!(c.set("proj", "s1", "a", false).is_err());
    }

    #[test]
    fn detects_and_fixes_divergence() {
        let (d, c) = setup();
        c.set("proj", "s1", "b", true).unwrap();
        // simula o link se desfazendo: b vira um arquivo separado e mais novo
        let fb = d.path().join("b/projects/proj/s1.jsonl");
        let content = fs::read_to_string(&fb).unwrap() + "{\"type\":\"assistant\"}\n";
        fs::remove_file(&fb).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&fb, content).unwrap();
        let st = c.state();
        assert!(st.conversations[0].profiles.iter().any(|p| p.state == Presence::Diverged));
        assert_eq!(c.resync("proj", "s1").unwrap(), 1);
        let st = c.state();
        assert!(st.conversations[0].profiles.iter().all(|p| p.state == Presence::On));
        // a versão antiga ficou guardada
        let baks = fs::read_dir(d.path().join("a/projects/proj")).unwrap().filter(|e| {
            e.as_ref().unwrap().file_name().to_string_lossy().contains(".bak-")
        });
        assert_eq!(baks.count(), 1);
    }

    #[test]
    fn unsupported_tools_report_note() {
        let c = Conversations { tool: Tool::Codex, profiles: vec![], cache_file: PathBuf::from("nao-usado.json") };
        let st = c.state();
        assert!(!st.supported && st.note.is_some());
    }
}
