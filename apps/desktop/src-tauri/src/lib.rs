mod supervisor;

use std::{path::PathBuf, sync::Arc};

use serde::Serialize;
use supervisor::{
    AgentStats, AppState, CreateAgentPayload, CreateFromTemplatePayload, DockerStatus, OpsResult,
};
use tauri::{Manager, State};

#[derive(Serialize)]
struct AppInfo {
    app_data_dir: String,
    platform: String,
    version: String,
}

#[tauri::command]
async fn get_app_info(state: State<'_, Arc<AppState>>) -> Result<AppInfo, String> {
    Ok(AppInfo {
        app_data_dir: state.base_dir().to_string_lossy().to_string(),
        platform: std::env::consts::OS.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[tauri::command]
async fn check_docker(state: State<'_, Arc<AppState>>) -> Result<DockerStatus, String> {
    state.check_docker().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_templates(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<supervisor::AgentTemplate>, String> {
    state.list_templates().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn create_agent_from_template(
    state: State<'_, Arc<AppState>>,
    payload: CreateFromTemplatePayload,
) -> Result<supervisor::Agent, String> {
    state
        .create_agent_from_template(payload)
        .await
        .map_err(|e| e.to_string())
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
    remove_volumes: bool,
) -> Result<(), String> {
    state
        .delete_agent(&agent_id, remove_volumes)
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
    folder_path: String,
    name: String,
) -> Result<supervisor::Agent, String> {
    state
        .import_agent(folder_path, name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn export_agent_bundle(
    state: State<'_, Arc<AppState>>,
    agent_id: String,
    include_secrets: bool,
) -> Result<String, String> {
    state
        .export_agent_bundle(&agent_id, include_secrets)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn import_agent_bundle(
    state: State<'_, Arc<AppState>>,
    zip_path: String,
) -> Result<supervisor::Agent, String> {
    state
        .import_agent_bundle(zip_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_agent_stats(
    state: State<'_, Arc<AppState>>,
    agent_id: String,
) -> Result<AgentStats, String> {
    state
        .get_agent_stats(&agent_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn attach_agent_shell(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    agent_id: String,
    preferred_shell: String,
) -> Result<(), String> {
    state
        .attach_agent_shell(app, &agent_id, preferred_shell)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn write_agent_shell(
    state: State<'_, Arc<AppState>>,
    agent_id: String,
    data: String,
) -> Result<(), String> {
    state
        .write_agent_shell(&agent_id, data)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn detach_agent_shell(
    state: State<'_, Arc<AppState>>,
    agent_id: String,
) -> Result<(), String> {
    state
        .detach_agent_shell(&agent_id)
        .await
        .map_err(|e| e.to_string())
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
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            list_templates,
            create_agent_from_template,
            list_agents,
            create_agent,
            delete_agent,
            duplicate_agent,
            import_agent,
            export_agent_bundle,
            import_agent_bundle,
            get_agent_stats,
            attach_agent_shell,
            write_agent_shell,
            detach_agent_shell,
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
