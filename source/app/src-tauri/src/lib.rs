//! Stitchup Tauri host — owns the app window and the single in-memory session
//! over `stitchup-core`. All structural state lives in the Rust session
//! (`SessionState`), mutated by `page_op`/`set_metadata`; `open_document`
//! returns CURRENT session state for the opened path (so refreshPages sees
//! in-memory mutations, never a stale disk re-read).

mod commands;

use commands::SessionState;

use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let session = Mutex::new(SessionState::default());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(session)
        .invoke_handler(tauri::generate_handler![
            commands::open_document,
            commands::page_op,
            commands::cmd_set_metadata,
            commands::commit_export,
            commands::source_hash,
            commands::render_bytes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Stitchup");
}