//! Configurações do app por perfil (ex.: parâmetros extras ao abrir a ferramenta).
//!
//! Ficam em `~/.ai-sessions/profiles.json`, fora das pastas das ferramentas:
//! `{ "claude/padrão": { "args": "--model opus" }, ... }`.

use crate::tools::Tool;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct ProfileSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
}

fn file() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".ai-sessions").join("profiles.json")
}

fn key(tool: Tool, name: &str) -> String {
    format!("{}/{name}", tool.id())
}

fn load() -> BTreeMap<String, ProfileSettings> {
    fs::read(file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(map: &BTreeMap<String, ProfileSettings>) -> Result<(), String> {
    let f = file();
    if let Some(d) = f.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = f.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(map).unwrap()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &f).map_err(|e| e.to_string())
}

pub fn args(tool: Tool, name: &str) -> Option<String> {
    load().remove(&key(tool, name)).and_then(|s| s.args).filter(|a| !a.trim().is_empty())
}

pub fn set_args(tool: Tool, name: &str, args: &str) -> Result<(), String> {
    if args.contains(['\r', '\n']) {
        return Err("Os parâmetros devem ficar numa linha só.".into());
    }
    let mut map = load();
    let a = args.trim();
    if a.is_empty() {
        map.remove(&key(tool, name));
    } else {
        map.entry(key(tool, name)).or_default().args = Some(a.to_string());
    }
    save(&map)
}

pub fn rename(tool: Tool, old: &str, new: &str) {
    let mut map = load();
    if let Some(s) = map.remove(&key(tool, old)) {
        map.insert(key(tool, new), s);
        let _ = save(&map);
    }
}

pub fn remove(tool: Tool, name: &str) {
    let mut map = load();
    if map.remove(&key(tool, name)).is_some() {
        let _ = save(&map);
    }
}
