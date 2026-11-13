//! Stitchup Tauri host — owns the app window and the single in-memory session
//! over `stitchup-core`. All structural state lives in the Rust session
//! (`SessionState`), mutated by `page_op`/`set_metadata`; `open_document`
//! returns CURRENT session state for the opened path (so refreshPages sees
//! in-memory mutations, never a stale disk re-read).

pub mod commands;

use commands::SessionState;

use std::sync::Mutex;

use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let session = Mutex::new(SessionState::default());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(session)
        .invoke_handler(tauri::generate_handler![
            commands::open_document,
            commands::page_op,
            commands::merge_document,
            commands::cmd_set_metadata,
            commands::undo,
            commands::redo,
            commands::unlock_document,
            commands::commit_export,
            commands::source_hash,
            commands::render_bytes,
            commands::edit_text,
            commands::export_pages,
            commands::split_document,
            commands::new_document,
            commands::save_document,
            commands::add_watermark,
            commands::set_crop,
            commands::save_encrypted,
            commands::save_compressed,
            commands::list_attachments,
            commands::save_attachment,
            commands::write_file,
            commands::print_window,
            commands::combine_documents,
            commands::take_pending_open,
            commands::adopt_saved_path,
            commands::log_message,
            commands::add_annotation,
            commands::delete_annotation,
            commands::move_annotation,
            commands::redact_area,
            commands::list_objects,
            commands::object_op,
            commands::insert_shape,
            commands::sign_document,
            commands::open_url,
            commands::list_fields,
            commands::set_field,
            commands::insert_image,
            commands::read_image_file,
            commands::quit_app,
        ])
        .setup(|_app| {
            // `stitchup file.pdf` from a shell.
            let args: Vec<String> = std::env::args().skip(1).filter(|a| a.to_lowercase().ends_with(".pdf")).collect();
            if let Ok(mut q) = commands::PENDING_OPEN.lock() {
                q.extend(args);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Stitchup")
        .run(|app, event| {
            // Finder double-click / "Open With" / drop on the Dock icon.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = event {
                let paths: Vec<String> = urls
                    .iter()
                    .filter_map(|u| u.to_file_path().ok())
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                if let Ok(mut q) = commands::PENDING_OPEN.lock() {
                    q.extend(paths.iter().cloned());
                }
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.emit("open-files", paths);
                }
            }
        });
}
