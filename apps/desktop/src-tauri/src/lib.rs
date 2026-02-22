mod supervisor;

use std::{path::PathBuf, sync::Arc};

use serde::Serialize;
use supervisor::{AppState, CreateAgentPayload, DockerStatus, ImportAgentPayload, OpsResult};
use tauri::{Manager, State};

#[derive(Serialize)]
struct AppInfo {
    app_data_dir: String,
    platform: String,
}

#[tauri::command]
async fn get_app_info(state: State<'_, Arc<AppState>>) -> Result<AppInfo, String> {
    Ok(AppInfo {
        app_data_dir: state.base_dir().to_string_lossy().to_string(),
        platform: std::env::consts::OS.to_string(),
    })
}

#[tauri::command]
async fn check_docker(state: State<'_, Arc<AppState>>) -> Result<DockerStatus, String> {
    state.check_docker().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_agents(state: State<'_, Arc<AppState>>) -> Result<Vec<supervisor::Agent>, String> {
    state.list_agents().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn create_agent(
    state: State<'_, Arc<AppState>>,
    payload: CreateAgentPayload,
) -> Result<supervisor::Agent, String> {
    state.create_agent(payload).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_agent(
    state: State<'_, Arc<AppState>>,
    agent_id: String,
    down_with_volumes: bool,
) -> Result<(), String> {
    state
        .delete_agent(&agent_id, down_with_volumes)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn duplicate_agent(
    state: State<'_, Arc<AppState>>,
    agent_id: String,
    new_name: String,
) -> Result<supervisor::Agent, String> {
    state
        .duplicate_agent(&agent_id, new_name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn import_agent(
    state: State<'_, Arc<AppState>>,
    payload: ImportAgentPayload,
) -> Result<supervisor::Agent, String> {
    state.import_agent(payload).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_agent(state: State<'_, Arc<AppState>>, agent_id: String) -> Result<(), String> {
    state
        .start_agent(&agent_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn stop_agent(state: State<'_, Arc<AppState>>, agent_id: String) -> Result<(), String> {
    state.stop_agent(&agent_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn restart_agent(state: State<'_, Arc<AppState>>, agent_id: String) -> Result<(), String> {
    state
        .restart_agent(&agent_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn sync_agent_status(
    state: State<'_, Arc<AppState>>,
    agent_id: String,
) -> Result<(), String> {
    state
        .sync_agent_status(&agent_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn sync_all_statuses(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.sync_all_statuses().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn stream_logs_start(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    agent_id: String,
) -> Result<(), String> {
    state
        .stream_logs_start(app, &agent_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn stream_logs_stop(state: State<'_, Arc<AppState>>, agent_id: String) -> Result<(), String> {
    state
        .stream_logs_stop(&agent_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn run_ops_command(
    state: State<'_, Arc<AppState>>,
    agent_id: String,
    cmd: String,
) -> Result<OpsResult, String> {
    state
        .run_ops_command(&agent_id, &cmd)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn open_agent_folder(
    state: State<'_, Arc<AppState>>,
    agent_id: String,
) -> Result<(), String> {
    state
        .open_agent_folder(&agent_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_agent_config(
    state: State<'_, Arc<AppState>>,
    agent_id: String,
) -> Result<supervisor::AgentConfigResponse, String> {
    state
        .get_agent_config(&agent_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn reset_app_data(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.reset_app_data().await.map_err(|e| e.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_dir: PathBuf = app.path().app_data_dir()?.join("openclaw-mc");
            std::fs::create_dir_all(&app_dir)?;
            let runtime = tauri::async_runtime::block_on(AppState::new(app_dir))?;
            app.manage(Arc::new(runtime));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            check_docker,
            list_agents,
            create_agent,
            delete_agent,
            duplicate_agent,
            import_agent,
            start_agent,
            stop_agent,
            restart_agent,
            sync_agent_status,
            sync_all_statuses,
            stream_logs_start,
            stream_logs_stop,
            run_ops_command,
            open_agent_folder,
            get_agent_config,
            reset_app_data
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
