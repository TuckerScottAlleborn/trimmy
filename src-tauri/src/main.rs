// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cache;
mod export;
mod index;
mod jobs;
mod keyframes;
mod probe;
mod waveform;

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use tauri::Manager;
use tauri_plugin_shell::ShellExt;

use probe::VideoInfo;

/// Files that passed `open_video`. The other commands only ever touch these, so even a
/// compromised UI can't point FFmpeg at arbitrary paths, URLs or FFmpeg protocols.
#[derive(Default)]
struct OpenedFiles(Mutex<HashSet<String>>);

impl OpenedFiles {
    fn check(&self, path: &str) -> Result<(), String> {
        match self.0.lock() {
            Ok(opened) if opened.contains(path) => Ok(()),
            _ => Err("Open that video in Trimmy first.".into()),
        }
    }
}

/// Checks that `path` is a video FFmpeg can read, lets the webview load it for the preview,
/// and returns what the UI shows about it.
#[tauri::command]
async fn open_video(
    app: tauri::AppHandle,
    opened: tauri::State<'_, OpenedFiles>,
    path: String,
) -> Result<VideoInfo, String> {
    let file = Path::new(&path);
    if !file.is_absolute() {
        let example = if cfg!(windows) {
            r"C:\Videos\clip.mp4"
        } else {
            "/Users/you/Movies/clip.mp4"
        };
        return Err(format!("Use the full path, like {example}"));
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
    let mut info = probe::parse(&path, &output.stdout)?;
    if info.duration <= 0.0 {
        // No duration in the container (an interrupted recording): measure the video itself.
        info.duration = keyframes::measure(&app, &path).await;
        if info.duration <= 0.0 {
            return Err("Trimmy can't tell how long that video is.".into());
        }
    }

    app.asset_protocol_scope()
        .allow_file(file)
        .map_err(|e| e.to_string())?;
    if let Ok(mut opened) = opened.0.lock() {
        opened.insert(path);
    }
    Ok(info)
}

/// Saves `start..end` (seconds) of `path` as a new file next to it and returns the new file's path.
#[tauri::command]
async fn export_clip(
    app: tauri::AppHandle,
    opened: tauri::State<'_, OpenedFiles>,
    path: String,
    start: f64,
    end: f64,
) -> Result<String, String> {
    opened.check(&path)?;
    export::export(&app, &path, start, end).await
}

/// Peak loudness of `path`'s first audio track in `buckets` slices, for the timeline waveform.
/// `progress` gets the raw peaks in batches while they're computed, so the waveform draws in.
#[tauri::command]
async fn waveform(
    app: tauri::AppHandle,
    opened: tauri::State<'_, OpenedFiles>,
    path: String,
    duration: f64,
    buckets: u32,
    progress: tauri::ipc::Channel<Vec<f32>>,
) -> Result<Vec<f32>, String> {
    opened.check(&path)?;
    waveform::peaks(&app, &path, duration, buckets, &progress).await
}

/// Where `path`'s keyframes are (seconds), for snapping the start handle. Empty means any start works.
#[tauri::command]
async fn keyframes(
    app: tauri::AppHandle,
    opened: tauri::State<'_, OpenedFiles>,
    path: String,
) -> Result<Vec<f64>, String> {
    opened.check(&path)?;
    Ok(keyframes::scan(&app, &path).await?.keyframes)
}

/// The user closed `path` (or opened another file): stop reading it in the background.
#[tauri::command]
fn close_video(jobs: tauri::State<jobs::Jobs>, path: String) {
    jobs.stop_file(&path);
}

/// A file macOS asked Trimmy to open (Finder's "Open With") that the UI hasn't picked up yet.
#[derive(Default)]
struct PendingOpen(Mutex<Option<String>>);

/// The file Trimmy was started with ("Open with", or `trimmy clip.mp4` in a terminal), if any.
#[tauri::command]
fn launch_path(pending: tauri::State<PendingOpen>) -> Option<String> {
    let from_finder = pending.0.lock().ok().and_then(|mut p| p.take());
    from_finder.or_else(|| {
        let arg = std::env::args().nth(1)?;
        // A path typed in a terminal can be relative to the folder the terminal is in.
        let full = std::env::current_dir().ok()?.join(arg);
        Some(full.to_string_lossy().into_owned())
    })
}

/// The window starts hidden (no flash of an empty frame); the UI calls this once it has drawn.
#[tauri::command]
fn ui_ready(window: tauri::WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

/// Runs ffprobe and ffmpeg once in the background so the OS has the (large) executables cached
/// before the user opens a file. The first run after a reboot is otherwise ~1 s slower.
fn warm_up_ffmpeg(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        for tool in ["ffprobe", "ffmpeg"] {
            if let Ok(command) = app.shell().sidecar(tool) {
                let _ = command.arg("-version").output().await;
            }
        }
    });
}

/// Shows the window after a few seconds if the UI never called `ui_ready` (for example, a
/// frontend error), so Trimmy can't end up running invisibly.
fn show_window_eventually(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(3));
        if let Some(window) = app.get_webview_window("main") {
            if !window.is_visible().unwrap_or(true) {
                let _ = window.show();
            }
        }
    });
}

fn main() {
    tauri::Builder::default()
        .manage(PendingOpen::default())
        .manage(OpenedFiles::default())
        .manage(jobs::Jobs::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            warm_up_ffmpeg(app.handle().clone());
            show_window_eventually(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_video,
            export_clip,
            waveform,
            keyframes,
            close_video,
            launch_path,
            ui_ready
        ])
        .build(tauri::generate_context!())
        .expect("error while starting Trimmy")
        .run(|_app, _event| {
            // Don't leave FFmpeg running after Trimmy is gone.
            if let tauri::RunEvent::Exit = _event {
                _app.state::<jobs::Jobs>().stop_all();
            }
            // Windows passes "Open with" files as a command-line argument; macOS sends an event
            // instead, possibly before the UI is ready, so keep it for launch_path too.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = _event {
                if let Some(path) = urls.first().and_then(|url| url.to_file_path().ok()) {
                    let path = path.to_string_lossy().into_owned();
                    if let Ok(mut pending) = _app.state::<PendingOpen>().0.lock() {
                        *pending = Some(path.clone());
                    }
                    let _ = tauri::Emitter::emit(_app, "open-file", path);
                }
            }
        });
}
