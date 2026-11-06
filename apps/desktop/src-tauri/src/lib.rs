pub mod commands;
pub mod types;

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::scan_and_find_duplicates,
            commands::cancel_scan,
            commands::generate_cleanup_plan,
            commands::execute_quarantine_plan,
            commands::undo_session,
            commands::get_session_history,
            commands::recover_journal,
            commands::reveal_in_file_manager,
            commands::check_cow_support,
            commands::execute_cow_deduplication
        ])
        .run(tauri::generate_context!())
        .expect("error while running reclaim desktop application");
}
