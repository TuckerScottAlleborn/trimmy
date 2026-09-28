//! Every FFmpeg/ffprobe run Trimmy starts, so it can stop them: a file's background work (the
//! waveform, the keyframe scan) when the user closes that file, and everything when Trimmy quits.
//! Without this, closing a long video left FFmpeg reading it for as long as it took, even after
//! Trimmy itself was gone.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

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

/// Starts `command` for `path` and remembers it until the returned `Job` is dropped.
pub fn spawn(
    app: &tauri::AppHandle,
    path: &str,
    kind: Kind,
    command: Command,
) -> Result<(Receiver<CommandEvent>, Job), String> {
    let (events, child) = command.spawn().map_err(|e| e.to_string())?;
    let jobs = app.state::<Jobs>();
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
    Ok((
        events,
        Job {
            app: app.clone(),
            id,
        },
    ))
}

impl Jobs {
    /// Stops the background work (waveform, keyframes) for `path`; exports carry on.
    pub fn stop_file(&self, path: &str) {
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
