pub mod ai;
mod client;
mod commands;
pub mod connections;
pub mod export;
pub mod query_history;
mod query_paging;
#[cfg(feature = "integration-tests")]
mod query_paging_integration_test;
mod sql_quote;
pub mod ssh;
mod supervisor;

pub fn run() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("lucent=debug,warn"),
    )
    .format_timestamp_millis()
    .init();

    log::info!("Lucent starting up");

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::execute_query,
            commands::get_databases,
            commands::get_schemas,
            commands::get_schema_objects,
            commands::get_function_source,
            commands::get_view_source,
            commands::get_sequence_info,
            commands::browse_table,
            commands::count_all_rows,
            commands::describe_filters,
            commands::disconnect,
            commands::ai_chat,
            commands::ai_cancel,
            commands::close_conversation,
            commands::execute_dml,
            commands::get_ai_settings,
            commands::save_ai_settings,
            // Connection profile commands
            commands::list_connections,
            commands::get_connection,
            commands::save_connection,
            commands::delete_connection,
            commands::duplicate_connection,
            commands::test_connection,
            // SSH config commands
            commands::save_ssh_config,
            commands::list_ssh_configs,
            commands::delete_ssh_config,
            // Query history commands
            commands::list_history,
            commands::toggle_history_favorite,
            commands::delete_history_entry,
            commands::clear_history,
            // Export commands
            commands::export_results,
            commands::copy_results,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
