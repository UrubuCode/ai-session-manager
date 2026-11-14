mod conversations;
mod env;
mod explorer;
mod links;
mod profiles;
mod settings;
mod skills;
mod terminal;
mod tools;
mod tray;
mod updates;

use profiles::{Store, ToolState};
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::AppHandle;
use tools::Tool;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub tools: Vec<ToolState>,
}

fn tool_state(tool: Tool) -> ToolState {
    let env = tool.env_var().and_then(env::get);
    let mut s = Store::from_env(tool).state(env);
    for p in &mut s.profiles {
        p.extra_args = settings::args(tool, &p.name);
    }
    s
}

fn state() -> AppState {
    AppState { tools: Tool::ALL.into_iter().map(tool_state).collect() }
}

/// Mantém o menu do Explorer e a bandeja em dia com a lista de perfis.
fn sync_menu(app: &AppHandle) -> Result<AppState, String> {
    let s = state();
    explorer::enable(&s.tools)?;
    tray::refresh(app, &s);
    Ok(s)
}

fn set_tool_env(tool: Tool, value: Option<&str>) -> Result<(), String> {
    match tool.env_var() {
        Some(var) => env::set(var, value),
        None => Ok(()),
    }
}

pub(crate) fn do_set_default(app: &AppHandle, tool: Tool, name: &str) -> Result<AppState, String> {
    let v = Store::from_env(tool).env_for(name)?;
    set_tool_env(tool, v.as_deref())?;
    sync_menu(app)
}

pub(crate) fn do_open(tool: Tool, name: &str, cwd: Option<&Path>) -> Result<(), String> {
    terminal::open(&Store::from_env(tool), name, cwd)
}

#[tauri::command]
fn get_state() -> AppState {
    state()
}

/// Define o perfil usado por terminais/apps abertos a partir de agora.
#[tauri::command]
fn set_default(app: AppHandle, tool: Tool, name: String) -> Result<AppState, String> {
    do_set_default(&app, tool, &name)
}

#[derive(serde::Serialize)]
struct Created {
    name: String,
    warnings: Vec<String>,
    state: AppState,
}

#[tauri::command]
fn create_profile(app: AppHandle, tool: Tool, name: String) -> Result<Created, String> {
    let s = Store::from_env(tool);
    let (name, warnings) = s.create(&name)?;
    // Abre a ferramenta no perfil novo para o usuário fazer login.
    if tool.installed() {
        terminal::open(&s, &name, None)?;
    }
    Ok(Created { name, warnings, state: sync_menu(&app)? })
}

fn is_selected(tool: Tool, name: &str) -> bool {
    tool_state(tool).profiles.iter().any(|p| p.name == name && p.is_selected)
}

#[tauri::command]
fn rename_profile(app: AppHandle, tool: Tool, old: String, new: String) -> Result<AppState, String> {
    let s = Store::from_env(tool);
    let was_selected = tool.supports_default() && is_selected(tool, &old);
    let n = s.rename(&old, &new)?;
    settings::rename(tool, &old, &n);
    if was_selected {
        set_tool_env(tool, s.env_for(&n)?.as_deref())?;
    }
    sync_menu(&app)
}

#[tauri::command]
fn delete_profile(app: AppHandle, tool: Tool, name: String) -> Result<AppState, String> {
    let s = Store::from_env(tool);
    let was_selected = tool.supports_default() && is_selected(tool, &name);
    s.delete(&name)?;
    settings::remove(tool, &name);
    if was_selected {
        set_tool_env(tool, None)?;
    }
    sync_menu(&app)
}

// ---------------------------------------------------------------- skills globais

#[tauri::command]
fn get_skills() -> skills::SkillsState {
    skills::Skills::from_env().state()
}

#[tauri::command]
fn set_skill_link(name: String, tool: Tool, enabled: bool) -> Result<skills::SkillsState, String> {
    let sk = skills::Skills::from_env();
    sk.set_link(&name, tool, enabled)?;
    Ok(sk.state())
}

#[tauri::command]
fn make_skill_global(tool: Tool, name: String) -> Result<skills::SkillsState, String> {
    let sk = skills::Skills::from_env();
    sk.make_global(tool, &name)?;
    Ok(sk.state())
}

#[tauri::command]
fn create_skill(name: String, description: String) -> Result<skills::SkillsState, String> {
    let sk = skills::Skills::from_env();
    let n = sk.create(&name, &description)?;
    let _ = reveal(&sk.dir_of(&n));
    Ok(sk.state())
}

#[tauri::command]
fn open_skill_folder(name: Option<String>) -> Result<(), String> {
    let sk = skills::Skills::from_env();
    let dir = match name {
        Some(n) => sk.dir_of(&n),
        None => sk.root().to_path_buf(),
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    reveal(&dir)
}

// ---------------------------------------------------------------- conversas

#[tauri::command]
fn get_conversations(tool: Tool) -> conversations::ConversationsState {
    conversations::Conversations::from_env(tool).state()
}

#[tauri::command]
fn set_conversation_sync(
    tool: Tool,
    project: String,
    id: String,
    profile: String,
    enabled: bool,
) -> Result<conversations::ConversationsState, String> {
    let c = conversations::Conversations::from_env(tool);
    c.set(&project, &id, &profile, enabled)?;
    Ok(c.state())
}

#[tauri::command]
fn resync_conversation(tool: Tool, project: String, id: String) -> Result<conversations::ConversationsState, String> {
    let c = conversations::Conversations::from_env(tool);
    c.resync(&project, &id)?;
    Ok(c.state())
}

/// Parâmetros extras usados sempre que o perfil é aberto.
#[tauri::command]
fn set_profile_args(app: AppHandle, tool: Tool, name: String, args: String) -> Result<AppState, String> {
    Store::from_env(tool).dir_of(&name)?; // valida
    settings::set_args(tool, &name, &args)?;
    sync_menu(&app)
}

#[tauri::command]
fn open_tool(tool: Tool, name: String) -> Result<(), String> {
    do_open(tool, &name, None)
}

#[tauri::command]
fn open_folder(tool: Tool, name: String) -> Result<(), String> {
    let dir = Store::from_env(tool).dir_of(&name)?;
    reveal(&dir)
}

fn reveal(p: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = "xdg-open";
    Command::new(cmd).arg(p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Modo linha de comando usado pelo menu do Explorer e pelo instalador:
/// - `--open <ferramenta> <perfil> <pasta>` (forma antiga sem ferramenta = Claude)
/// - `--menu on|off`
///
/// Retorna `true` se tratou.
pub fn handle_cli() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--menu") {
        let r = match args.get(2).map(String::as_str) {
            Some("off") => explorer::disable(),
            _ => explorer::enable(&state().tools),
        };
        if let Err(e) = r {
            show_error(&e);
            std::process::exit(1);
        }
        return true;
    }
    if args.get(1).map(String::as_str) != Some(explorer::CLI_FLAG) {
        return false;
    }
    let (tool, name, dir) = match args.get(2).and_then(|a| Tool::from_id(a)) {
        Some(t) => (t, args.get(3), args.get(4)),
        None => (Tool::Claude, args.get(2), args.get(3)), // menus gravados por versões antigas
    };
    let name = name.cloned().unwrap_or_default();
    let dir = dir.map(|d| PathBuf::from(explorer::fix_dir_arg(d)));
    if let Err(e) = do_open(tool, &name, dir.as_deref()) {
        show_error(&e);
    }
    true
}

#[cfg(windows)]
pub(crate) fn show_error(msg: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let w = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (m, t) = (w(msg), w("AI Session Manager"));
    unsafe { MessageBoxW(std::ptr::null_mut(), m.as_ptr(), t.as_ptr(), MB_OK | MB_ICONERROR) };
}
#[cfg(not(windows))]
pub(crate) fn show_error(msg: &str) {
    eprintln!("{msg}");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Menu do Explorer sempre ligado: regrava a cada abertura com o caminho atual
    // do executável (reinstalação/atualização nunca deixa o menu quebrado).
    let _ = explorer::enable(&state().tools);

    tauri::Builder::default()
        // Precisa ser o primeiro plugin: uma segunda abertura só traz a janela existente.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_window(app);
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            tray::create(app.handle())?;
            updates::start(app.handle());
            Ok(())
        })
        // Fechar a janela só esconde: o app continua na bandeja ("Sair" encerra).
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            set_default,
            create_profile,
            rename_profile,
            delete_profile,
            open_tool,
            open_folder,
            set_profile_args,
            get_skills,
            set_skill_link,
            make_skill_global,
            create_skill,
            open_skill_folder,
            get_conversations,
            set_conversation_sync,
            resync_conversation
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o aplicativo");
}
