use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Serialize, Deserialize, Clone)]
pub struct WatchRule {
    pub id: String,
    pub name: String,
    pub executable: String,
    pub working_dir: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub restart_policy: String,
    pub max_retries: Option<u32>,
}

#[derive(Serialize, Clone)]
pub struct ProcessState {
    pub id: String,
    pub name: String,
    pub status: String,
    pub pid: Option<u32>,
    pub uptime_secs: u64,
    pub last_exit_code: Option<i32>,
    pub restart_count: u32,
}

struct AppState {
    processes: Mutex<HashMap<String, ProcessState>>,
    rules: Mutex<Vec<WatchRule>>,
}

#[tauri::command]
fn add_watch_rule(app: AppHandle, state: State<AppState>, rule: WatchRule) -> Result<(), String> {
    let mut rules = state.rules.lock().unwrap();
    rules.push(rule.clone());
    save_config(&app, &rules)?;
    spawn_process(app, state, rule)?;
    Ok(())
}

#[tauri::command]
fn remove_watch_rule(state: State<AppState>, id: String) -> Result<(), String> {
    let mut rules = state.rules.lock().unwrap();
    rules.retain(|r| r.id != id);
    let mut procs = state.processes.lock().unwrap();
    procs.remove(&id);
    Ok(())
}

#[tauri::command]
fn get_processes(state: State<AppState>) -> Vec<ProcessState> {
    state.processes.lock().unwrap().values().cloned().collect()
}

fn spawn_process(app: AppHandle, state: State<AppState>, rule: WatchRule) -> Result<(), String> {
    let mut cmd = Command::new(&rule.executable);
    if let Some(dir) = &rule.working_dir {
        cmd.current_dir(dir);
    }
    if let Some(envs) = &rule.env {
        for (k, v) in envs {
            cmd.env(k, v);
        }
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let pid = child.id();
    
    let mut procs = state.processes.lock().unwrap();
    procs.insert(rule.id.clone(), ProcessState {
        id: rule.id.clone(),
        name: rule.name.clone(),
        status: "Running".to_string(),
        pid: Some(pid),
        uptime_secs: 0,
        last_exit_code: None,
        restart_count: 0,
    });
    drop(procs);

    let app_clone = app.clone();
    let rule_clone = rule.clone();
    std::thread::spawn(move || {
        let status = child.wait().unwrap();
        let exit_code = status.code();
        
        let mut procs = app_clone.state::<AppState>().processes.lock().unwrap();
        if let Some(p) = procs.get_mut(&rule_clone.id) {
            p.status = "Stopped".to_string();
            p.last_exit_code = exit_code;
            p.pid = None;
        }
        drop(procs);

        let _ = app_clone.emit("process-status", &rule_clone.id);
        
        if rule_clone.restart_policy == "auto" {
            let mut procs = app_clone.state::<AppState>().processes.lock().unwrap();
            if let Some(p) = procs.get_mut(&rule_clone.id) {
                p.restart_count += 1;
                p.status = "Restarting".to_string();
            }
            drop(procs);
            let _ = app_clone.emit("process-status", &rule_clone.id);
            std::thread::sleep(std::time::Duration::from_secs(1));
            let _ = spawn_process(app_clone, app_clone.state::<AppState>(), rule_clone);
        }
    });
    Ok(())
}

fn save_config(app: &AppHandle, rules: &[WatchRule]) -> Result<(), String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("config.json");
    let json = serde_json::to_string_pretty(rules).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

fn load_config(app: &AppHandle) -> Vec<WatchRule> {
    let dir = match app.path().app_data_dir() { Ok(d) => d, Err(_) => return vec![] };
    let path = dir.join("config.json");
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(rules) = serde_json::from_str::<Vec<WatchRule>>(&content) {
                return rules;
            }
        }
    }
    vec![]
}

fn main() {
    tauri::Builder::default()
        .manage(AppState {
            processes: Mutex::new(HashMap::new()),
            rules: Mutex::new(Vec::new()),
        })
        .setup(|app| {
            let state = app.state::<AppState>();
            let rules = load_config(app.handle());
            *state.rules.lock().unwrap() =
