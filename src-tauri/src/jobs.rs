//! Every FFmpeg/ffprobe run Trimmy starts, so it can stop them: a file's background work (the
//! waveform, the keyframe scan) when the user closes that file, and everything when Trimmy quits.
//! Without this, closing a long video left FFmpeg reading it for as long as it took, even after
//! Trimmy itself was gone.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri::async_runtime::Receiver;
use tauri::Manager;
use tauri_plugin_shell::process::{Command, CommandChild, CommandEvent};

/// What a run is for, which decides when it's stopped.
#[derive(Clone, PartialEq, Eq)]
pub enum Kind {
    /// Reading a file for the editor (waveform, keyframes): stopped when that file is closed.
    Background,
    /// Writing a clip: finishes even if the file is closed, and is stopped only when Trimmy quits,
    /// in which case its half-written output is deleted.
    Export { output: PathBuf },
}

struct Running {
    path: String,
    kind: Kind,
    child: CommandChild,
}

#[derive(Default)]
pub struct Jobs {
    next_id: AtomicU64,
    running: Mutex<HashMap<u64, Running>>,
    /// Files the user closed. Background work on them is refused until they're opened again, so
    /// work that was still to start (the next batch of a sampled waveform, the keyframe lookup
    /// that waits for the waveform) doesn't start after all.
    closed: Mutex<HashSet<String>>,
    /// One lock per file for the work that reads all of it (the full waveform, the packet scan),
    /// so two such reads never run at once: on a hard drive, two readers of one big file make it
    /// seek back and forth and each crawls.
    readers: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// A started run. Dropping it (when the run ends, however it ends) forgets it.
pub struct Job {
    app: tauri::AppHandle,
    id: u64,
}

impl Drop for Job {
    fn drop(&mut self) {
        if let Ok(mut running) = self.app.state::<Jobs>().running.lock() {
            running.remove(&self.id);
        }
    }
}

/// Why background work didn't start: the user closed the file.
pub const CLOSED: &str = "That file was closed.";

/// Starts `command` for `path` and remembers it until the returned `Job` is dropped. Background
/// work on a closed file is refused with `CLOSED`.
pub fn spawn(
    app: &tauri::AppHandle,
    path: &str,
    kind: Kind,
    command: Command,
) -> Result<(Receiver<CommandEvent>, Job), String> {
    let jobs = app.state::<Jobs>();
    let background = kind == Kind::Background;
    if background && jobs.is_closed(path) {
        return Err(CLOSED.into());
    }
    let (events, child) = command.spawn().map_err(|e| e.to_string())?;
    let id = jobs.next_id.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut running) = jobs.running.lock() {
        running.insert(
            id,
            Running {
                path: path.to_owned(),
                kind,
                child,
            },
        );
    }
    let job = Job {
        app: app.clone(),
        id,
    };
    // The file may have been closed while this was starting; then stop it straight away.
    if background && jobs.is_closed(path) {
        jobs.stop_file(path);
    }
    Ok((events, job))
}

impl Jobs {
    /// The user closed `path`: stops its background work (waveform, keyframes) and refuses more
    /// until it's opened again. Exports carry on.
    pub fn close_file(&self, path: &str) {
        if let Ok(mut closed) = self.closed.lock() {
            closed.insert(path.to_owned());
        }
        self.stop_file(path);
    }

    /// `path` was opened (again): background work on it is allowed.
    pub fn open_file(&self, path: &str) {
        if let Ok(mut closed) = self.closed.lock() {
            closed.remove(path);
        }
    }

    /// The lock to hold while reading the whole of `path`.
    pub fn reader(&self, path: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut readers = self.readers.lock().unwrap_or_else(|e| e.into_inner());
        readers.entry(path.to_owned()).or_default().clone()
    }

    pub fn is_closed(&self, path: &str) -> bool {
        self.closed.lock().is_ok_and(|closed| closed.contains(path))
    }

    fn stop_file(&self, path: &str) {
        self.stop(|job| job.path == path && job.kind == Kind::Background);
    }

    /// Stops everything, deleting any half-written export. Called as Trimmy quits.
    pub fn stop_all(&self) {
        self.stop(|_| true);
    }

    fn stop(&self, which: impl Fn(&Running) -> bool) {
        let Ok(mut running) = self.running.lock() else {
            return;
        };
        let ids: Vec<u64> = running
            .iter()
            .filter(|(_, job)| which(job))
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            if let Some(job) = running.remove(&id) {
                let _ = job.child.kill();
                if let Kind::Export { output } = job.kind {
                    // FFmpeg may need a moment to let go of the file after being killed.
                    for _ in 0..20 {
                        if std::fs::remove_file(&output).is_ok() || !output.exists() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
            }
        }
    }
}
