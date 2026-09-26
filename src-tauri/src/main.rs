// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod export;
mod probe;
mod waveform;

use std::path::Path;

use tauri::Manager;
use tauri_plugin_shell::ShellExt;

use probe::VideoInfo;

/// Checks that `path` is a video FFmpeg can read, lets the webview load it for the preview,
/// and returns what the UI shows about it.
#[tauri::command]
async fn open_video(app: tauri::AppHandle, path: String) -> Result<VideoInfo, String> {
    let file = Path::new(&path);
    if !file.is_absolute() {
        return Err(r"Use the full path, like C:\Videos\clip.mp4".into());
    }
    if file.is_dir() {
        return Err("That's a folder, not a video file.".into());
    }
    if !file.is_file() {
        return Err(format!("There's no file at {path}"));
    }

    let output = app
        .shell()
        .sidecar("ffprobe")
        .map_err(|e| e.to_string())?
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(&path)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("That doesn't look like a video file.".into());
    }
    let info = probe::parse(&path, &output.stdout)?;

    app.asset_protocol_scope()
        .allow_file(file)
        .map_err(|e| e.to_string())?;
    Ok(info)
}

/// Saves `start..end` (seconds) of `path` as a new file next to it and returns the new file's path.
#[tauri::command]
async fn export_clip(
    app: tauri::AppHandle,
    path: String,
    start: f64,
    end: f64,
) -> Result<String, String> {
    export::export(&app, &path, start, end).await
}

/// Peak loudness of `path`'s first audio track in `buckets` slices, for the timeline waveform.
#[tauri::command]
async fn waveform(
    app: tauri::AppHandle,
    path: String,
    duration: f64,
    buckets: u32,
) -> Result<Vec<f32>, String> {
    waveform::peaks(&app, &path, duration, buckets).await
}

/// The file Trimmy was started with ("Open with", or `trimmy.exe clip.mp4`), if any.
#[tauri::command]
fn launch_path() -> Option<String> {
    std::env::args().nth(1)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            open_video,
            export_clip,
            waveform,
            launch_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running Trimmy");
}
