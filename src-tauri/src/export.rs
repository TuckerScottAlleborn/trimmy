//! Cuts a clip out of a video with FFmpeg, losslessly (stream copy).

use std::path::{Path, PathBuf};

use tauri::Emitter;
use tauri_plugin_shell::{process::CommandEvent, ShellExt};

/// Copies `start..end` (seconds) of `input` to a new file next to it and returns that file's path.
/// Emits `export-progress` events (0.0 to 1.0) while FFmpeg runs.
///
/// Stream copy can only start on a keyframe, so the clip begins at the last keyframe at or
/// before `start` (within about a second for ShadowPlay clips).
pub async fn export(
    app: &tauri::AppHandle,
    input: &str,
    start: f64,
    end: f64,
) -> Result<String, String> {
    let length = end - start;
    if !(start >= 0.0 && length > 0.0) {
        return Err("Pick a start that comes before the end.".into());
    }
    let output = reserve_output(Path::new(input))?;
    let output_str = output.to_string_lossy().into_owned();
    let result = run_ffmpeg(app, input, start, length, &output_str).await;
    if result.is_err() {
        // Only ever deletes the file this export reserved, never someone else's.
        let _ = std::fs::remove_file(&output);
    }
    result.map(|()| output_str)
}

/// Claims the output name by creating the file, so a file that appears in the meantime (another
/// export, a sync client) is never overwritten or deleted.
fn reserve_output(input: &Path) -> Result<PathBuf, String> {
    loop {
        let candidate = output_path(input, |p| p.exists());
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Couldn't create {}: {e}", candidate.display())),
        }
    }
}

async fn run_ffmpeg(
    app: &tauri::AppHandle,
    input: &str,
    start: f64,
    length: f64,
    output: &str,
) -> Result<(), String> {
    let (mut events, _child) = app
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| e.to_string())?
        // -y: the output is the empty file reserve_output just created for this export.
        .args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-nostats",
            "-y",
        ])
        .args(["-progress", "pipe:1"])
        .args([
            "-ss",
            &start.to_string(),
            "-i",
            input,
            "-t",
            &length.to_string(),
        ])
        // First video track plus every audio track (game + mic stay separate).
        .args(["-map", "0:v:0", "-map", "0:a?", "-c", "copy"])
        .args(["-avoid_negative_ts", "make_zero", "-map_metadata", "0"])
        .arg(output)
        .spawn()
        .map_err(|e| e.to_string())?;

    let mut errors = String::new();
    while let Some(event) = events.recv().await {
        match event {
            CommandEvent::Stdout(line) => {
                let line = String::from_utf8_lossy(&line);
                if let Some(Ok(us)) = line
                    .trim()
                    .strip_prefix("out_time_us=")
                    .map(str::parse::<f64>)
                {
                    let _ = app.emit("export-progress", (us / 1e6 / length).clamp(0.0, 1.0));
                }
            }
            CommandEvent::Stderr(line) => errors.push_str(&String::from_utf8_lossy(&line)),
            CommandEvent::Terminated(status) if status.code == Some(0) => return Ok(()),
            CommandEvent::Terminated(_) => break,
            _ => {}
        }
    }
    Err(format!("Export failed: {}", errors.trim()))
}

/// "clip.mp4" -> "clip_trimmed.mp4", then "clip_trimmed_2.mp4" and so on, never an existing file.
fn output_path(input: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let dir = input.parent().unwrap_or(Path::new(""));
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let ext = input
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (1..)
        .map(|n| match n {
            1 => dir.join(format!("{stem}_trimmed{ext}")),
            n => dir.join(format!("{stem}_trimmed_{n}{ext}")),
        })
        .find(|candidate| !exists(candidate))
        .expect("some numbered name is free")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_output_next_to_the_input() {
        let out = output_path(Path::new("C:/Videos/WARDOGS.DVR.mp4"), |_| false);
        assert_eq!(out, Path::new("C:/Videos/WARDOGS.DVR_trimmed.mp4"));
    }

    #[test]
    fn never_overwrites() {
        let taken = [
            PathBuf::from("C:/Videos/clip_trimmed.mkv"),
            PathBuf::from("C:/Videos/clip_trimmed_2.mkv"),
        ];
        let out = output_path(Path::new("C:/Videos/clip.mkv"), |p| {
            taken.iter().any(|t| t == p)
        });
        assert_eq!(out, Path::new("C:/Videos/clip_trimmed_3.mkv"));
    }
}
