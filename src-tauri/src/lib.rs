pub mod ai;
pub mod client;
mod commands;
pub use commands::probe_connection;
pub mod dialect;
pub mod drivers;
pub use commands::AppState;
pub mod connections;
pub mod export;
pub mod notebook;
pub mod query_history;
mod query_paging;
#[cfg(feature = "integration-tests")]
mod query_paging_integration_test;
mod readonly;
mod sql_builder;
mod sql_quote;
pub mod ssh;
pub mod supervisor;
pub mod trace;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::Emitter;

pub fn run() {
    // Installs the global tracing subscriber (EnvFilter from RUST_LOG,
    // stdout + daily-rotating file, stdout-only fallback on an unwritable
    // config dir) and bridges existing `log::` macros into it. The returned
    // guard (first call only; `None` when file logging is unavailable) keeps
    // the file-writer thread alive for the process lifetime and flushes it on
    // exit.
    let _tracing_guard = trace::init_tracing();

    log::info!("Lucent starting up");

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::new())
        .setup(|app| {
            let handle = app.handle();

            let new_notebook = MenuItem::with_id(
                handle,
                "new-notebook",
                "New Notebook",
                true,
                Some("CmdOrCtrl+Shift+N"),
            )?;
            let new_query =
                MenuItem::with_id(handle, "new-query", "New Query", true, Some("CmdOrCtrl+T"))?;
            let open_notebook = MenuItem::with_id(
                handle,
                "open-notebook",
                "Open Notebook…",
                true,
                Some("CmdOrCtrl+O"),
            )?;
            let save = MenuItem::with_id(handle, "save", "Save", true, Some("CmdOrCtrl+S"))?;
            let save_as = MenuItem::with_id(
                handle,
                "save-as",
                "Save As…",
                true,
                Some("CmdOrCtrl+Shift+S"),
            )?;
            let sep = PredefinedMenuItem::separator(handle)?;

            let file = Submenu::with_items(
                handle,
                "File",
                true,
                &[
                    &new_notebook,
                    &new_query,
                    &open_notebook,
                    &sep,
                    &save,
                    &save_as,
                ],
            )?;

            // The app submenu carries Quit and the standard edit items carry
            // copy/paste — without them, macOS loses Cmd+C/Cmd+V entirely once a
            // custom menu replaces the default.
            let app_menu = Submenu::with_items(
                handle,
                "Lucent",
                true,
                &[&PredefinedMenuItem::quit(handle, None)?],
            )?;
            let edit_menu = Submenu::with_items(
                handle,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(handle, None)?,
                    &PredefinedMenuItem::redo(handle, None)?,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::cut(handle, None)?,
                    &PredefinedMenuItem::copy(handle, None)?,
                    &PredefinedMenuItem::paste(handle, None)?,
                    &PredefinedMenuItem::select_all(handle, None)?,
                ],
            )?;

            let menu = Menu::with_items(handle, &[&app_menu, &file, &edit_menu])?;
            app.set_menu(menu)?;

            // Forward every click to the frontend, which owns the tab and model state.
            app.on_menu_event(|app, event| {
                let id = event.id().0.clone();
                if let Err(e) = app.emit("menu-action", id.clone()) {
                    log::error!("failed to emit menu action {id}: {e}");
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::connection_capabilities,
            commands::execute_query,
            commands::cancel_query,
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
            commands::get_logs,
            commands::ai_chat,
            commands::ai_cancel,
            commands::close_conversation,
            commands::execute_dml,
            commands::get_ai_settings,
            commands::save_ai_settings,
            commands::get_ai_usage,
            // Connection profile commands
            commands::list_connections,
            commands::get_connection,
            commands::save_connection,
            commands::delete_connection,
            commands::duplicate_connection,
            commands::test_connection,
            // Driver descriptors for the connection form
            drivers::list_drivers,
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
            // Notebook commands
            crate::notebook::commands::notebook_open,
            crate::notebook::commands::notebook_save,
            crate::notebook::commands::notebook_attach,
            crate::notebook::commands::notebook_detach,
            crate::notebook::commands::notebook_restart_session,
            crate::notebook::commands::notebook_run_cell,
            crate::notebook::commands::notebook_cancel_cell,
            crate::notebook::commands::notebook_clear_outputs,
            crate::notebook::commands::notebook_resolve_refs,
            crate::notebook::paging::notebook_fetch_page,
            crate::notebook::paging::notebook_count_rows,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
