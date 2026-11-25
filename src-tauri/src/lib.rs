mod audit;
mod commands;
mod connections;
mod mcp;
mod models;
mod policy;
mod redaction;
mod sessions;
mod state;
mod vault;

use tauri::Manager;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let state = AppState::load(data_dir)?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::save_connection,
            commands::delete_connection,
            commands::preview_command,
            commands::run_command,
            commands::resolve_approval,
            commands::list_audit_entries,
            commands::list_sessions,
            commands::close_session,
            commands::mcp_server_info,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ShellX");
}
