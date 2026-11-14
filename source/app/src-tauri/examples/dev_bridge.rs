//! Dev-only bridge for end-to-end UI tests (never shipped: it is a cargo
//! example, which `tauri build` does not compile into the app).
//!
//! Serves the built frontend (`source/app/dist`) and answers
//! `POST /ipc/<command>` with the REAL session-logic functions from
//! `commands.rs`, so a headless browser drives the real UI against the real
//! Rust core — only the native window and file dialogs are missing (the test
//! stubs those).
//!
//!   cargo run --release --example dev_bridge -- <port> <dist dir>

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use stitchup_lib::commands::*;

fn arg<T: serde::de::DeserializeOwned>(v: &Value, k: &str) -> Result<T, String> {
    serde_json::from_value(v.get(k).cloned().unwrap_or(Value::Null)).map_err(|e| format!("arg {k}: {e}"))
}

fn dispatch(st: &Mutex<SessionState>, cmd: &str, a: &Value) -> Result<Value, Value> {
    let e = |e: stitchup_core::EngineError| serde_json::to_value(e).unwrap();
    let bad = |m: String| json!({"code": "validation_failed", "message": m});
    let v = |r: Result<Value, serde_json::Error>| r.map_err(|x| bad(x.to_string()));
    macro_rules! go {
        ($call:expr) => {
            v(serde_json::to_value($call.map_err(e)?))
        };
    }
    match cmd {
        "open_document" => go!(open_document_inner(st, arg(a, "path").map_err(bad)?)),
        "page_op" => go!(page_op_inner(st, arg(a, "kind").map_err(bad)?, arg(a, "data").map_err(bad)?)),
        "merge_document" => go!(merge_document_inner(st, arg(a, "path").map_err(bad)?)),
        "cmd_set_metadata" => go!(cmd_set_metadata_inner(st, arg(a, "meta").map_err(bad)?)),
        "undo" => go!(undo_inner(st)),
        "redo" => go!(redo_inner(st)),
        "unlock_document" => go!(unlock_document_inner(st, arg(a, "password").map_err(bad)?)),
        "commit_export" => go!(commit_export_inner(st, arg(a, "target").map_err(bad)?, arg(a, "overwrite").map_err(bad)?)),
        "source_hash" => go!(source_hash_inner(st)),
        "render_bytes" => go!(render_bytes_inner(st)),
        "edit_text" => go!(edit_text_inner(st, arg(a, "data").map_err(bad)?)),
        "export_pages" => go!(export_pages_inner(
            st,
            arg(a, "pages").map_err(bad)?,
            arg(a, "target").map_err(bad)?,
            arg(a, "overwrite").map_err(bad)?
        )),
        "split_document" => go!(split_document_inner(
            st,
            arg(a, "every").map_err(bad)?,
            arg(a, "dir").map_err(bad)?,
            arg(a, "stem").map_err(bad)?,
            arg(a, "overwrite").map_err(bad)?
        )),
        "new_document" => go!(new_document_inner(st, arg(a, "widthPt").map_err(bad)?, arg(a, "heightPt").map_err(bad)?)),
        "save_document" => go!(save_document_inner(st)),
        "add_watermark" => go!(add_watermark_inner(st, arg(a, "spec").map_err(bad)?)),
        "set_crop" => go!(set_crop_inner(st, arg(a, "boxes").map_err(bad)?)),
        "save_encrypted" => go!(save_encrypted_inner(
            st,
            arg(a, "target").map_err(bad)?,
            arg(a, "userPassword").map_err(bad)?,
            arg(a, "ownerPassword").map_err(bad)?
        )),
        "save_compressed" => go!(save_compressed_inner(st, arg(a, "target").map_err(bad)?)),
        "list_attachments" => go!(list_attachments_inner(st)),
        "save_attachment" => go!(save_attachment_inner(st, arg(a, "index").map_err(bad)?, arg(a, "target").map_err(bad)?)),
        "write_file" => go!(write_file_inner(st, arg(a, "path").map_err(bad)?, arg(a, "b64").map_err(bad)?)),
        "print_window" => Ok(Value::Null),
        "combine_documents" => go!(combine_documents_inner(st, arg(a, "paths").map_err(bad)?)),
        "take_pending_open" => Ok(json!([])),
        "adopt_saved_path" => go!(adopt_saved_path_inner(st, arg(a, "path").map_err(bad)?)),
        "quit_app" | "log_message" | "open_url" => Ok(Value::Null),
        "add_annotation" => go!(add_annotation_inner(st, arg(a, "page").map_err(bad)?, arg(a, "spec").map_err(bad)?)),
        "delete_annotation" => go!(delete_annotation_inner(st, arg(a, "page").map_err(bad)?, arg(a, "obj").map_err(bad)?)),
        "move_annotation" => go!(move_annotation_inner(st, arg(a, "page").map_err(bad)?, arg(a, "obj").map_err(bad)?, arg(a, "dx").map_err(bad)?, arg(a, "dy").map_err(bad)?)),
        "redact_area" => go!(redact_area_inner(st, arg(a, "page").map_err(bad)?, arg(a, "rect").map_err(bad)?)),
        "list_objects" => go!(list_objects_inner(st, arg(a, "page").map_err(bad)?)),
        "object_op" => go!(object_op_inner(st, arg(a, "page").map_err(bad)?, arg(a, "data").map_err(bad)?)),
        "insert_shape" => go!(insert_shape_inner(st, arg(a, "page").map_err(bad)?, arg(a, "spec").map_err(bad)?)),
        "list_fields" => go!(list_fields_inner(st)),
        "set_field" => go!(set_field_inner(st, arg(a, "name").map_err(bad)?, arg(a, "value").map_err(bad)?)),
        "insert_image" => go!(insert_image_inner(st, arg(a, "page").map_err(bad)?, arg(a, "rect").map_err(bad)?, arg(a, "image").map_err(bad)?)),
        "read_image_file" => go!(read_image_file(arg(a, "path").map_err(bad)?)),
        "tab_new" => go!(tab_new_inner(st)),
        "tab_switch" => go!(tab_switch_inner(st, arg(a, "id").map_err(bad)?)),
        "tab_close" => go!(tab_close_inner(st, arg(a, "id").map_err(bad)?)),
        "move_pages_to_tab" => go!(move_pages_to_tab_inner(st, arg(a, "pages").map_err(bad)?, arg(a, "to").map_err(bad)?)),
        "list_system_fonts" => go!(Ok::<_, stitchup_core::EngineError>(list_system_fonts())),
        "ocr_recognize" => go!(ocr_recognize(arg(a, "b64").map_err(bad)?, arg(a, "languages").ok())),
        "ocr_apply" => go!(ocr_apply_inner(st, arg(a, "pages").map_err(bad)?)),
        "save_recovery" => go!(save_recovery_inner(st)),
        "clear_recovery" => go!(clear_recovery()),
        "list_recovery" => go!(list_recovery()),
        "restore_recovery" => go!(restore_recovery_inner(st, arg(a, "file").map_err(bad)?)),
        "discard_recovery" => go!(discard_recovery()),
        "sign_document" => go!(sign_document_inner(st, arg(a, "target").map_err(bad)?, arg(a, "spec").map_err(bad)?)),
        other => Err(bad(format!("unknown command {other}"))),
    }
}

fn mime(p: &str) -> &'static str {
    match p.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ttf" => "font/ttf",
        "pfb" | "bcmap" => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

fn respond(s: &mut TcpStream, code: u16, ctype: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {code} OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn handle(mut s: TcpStream, st: Arc<Mutex<SessionState>>, dist: Arc<PathBuf>) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    if r.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/").to_string());
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; len];
    let _ = r.read_exact(&mut body);

    if method == "POST" {
        if let Some(cmd) = path.strip_prefix("/ipc/") {
            let args: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let out = match dispatch(&st, cmd, &args) {
                Ok(v) => json!({"ok": v}),
                Err(e) => json!({"err": e}),
            };
            return respond(&mut s, 200, "application/json", out.to_string().as_bytes());
        }
    }
    let rel = path.split('?').next().unwrap_or("/").trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.contains("..") {
        return respond(&mut s, 403, "text/plain", b"forbidden");
    }
    match std::fs::read(dist.join(rel)) {
        Ok(b) => respond(&mut s, 200, mime(rel), &b),
        Err(_) => respond(&mut s, 404, "text/plain", b"not found"),
    }
}

fn main() {
    let mut a = std::env::args().skip(1);
    let port: u16 = a.next().and_then(|p| p.parse().ok()).unwrap_or(1430);
    let dist = Arc::new(PathBuf::from(a.next().unwrap_or_else(|| "../dist".into())));
    let st = Arc::new(Mutex::new(SessionState::default()));
    std::thread::spawn(|| {
        let _ = stitchup_core::fonts::system_faces();
    });
    if let Ok(dir) = std::env::var("STITCHUP_RECOVERY_DIR") {
        *RECOVERY_DIR.lock().unwrap() = Some(PathBuf::from(dir));
    }
    let l = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    println!("dev_bridge listening on http://127.0.0.1:{port}");
    for c in l.incoming().flatten() {
        let (st, dist) = (st.clone(), dist.clone());
        std::thread::spawn(move || handle(c, st, dist));
    }
}
