//! Cuts a clip out of a video with FFmpeg, losslessly (stream copy).

use std::path::{Path, PathBuf};

use tauri::Emitter;
use tauri_plugin_shell::{process::CommandEvent, ShellExt};

use crate::keyframes::{self, Scan};

/// Copies `start..end` (seconds) of `input` to a new file next to it and returns that file's path.
/// Emits `export-progress` events (0.0 to 1.0) while FFmpeg runs.
///
/// Stream copy can only start on a keyframe, so the clip begins at the last keyframe at or
/// before `start`. The UI snaps the start handle to keyframes, so that is normally `start` itself.
pub async fn export(
    app: &tauri::AppHandle,
    input: &str,
    start: f64,
    end: f64,
) -> Result<String, String> {
    if !(start >= 0.0 && end > start) {
        return Err("Pick a start that comes before the end.".into());
    }
    // Cached from when the file was opened, so this is instant. If it fails, the scan is empty
    // and FFmpeg's own keyframe choice stands.
    let scan = keyframes::scan(app, input).await.unwrap_or_default();
    let start = scan.keyframe_at_or_before(start);
    let offset = seek_offset(&scan, start, end - start);
    let output = reserve_output(Path::new(input))?;
    let output_str = output.to_string_lossy().into_owned();
    let result = run_ffmpeg(
        app,
        input,
        start + offset,
        end - start - offset,
        &output_str,
    )
    .await;
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

/// Runs the stream copy. FFmpeg keeps everything from the keyframe it lands on when seeking to
/// `seek` (see `seek_offset`) and stops `length` seconds after `seek`.
async fn run_ffmpeg(
    app: &tauri::AppHandle,
    input: &str,
    seek: f64,
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
            &seek.to_string(),
            "-i",
            input,
            "-t",
            &length.to_string(),
        ])
        // First video track (V skips cover art) plus every audio track (game + mic stay separate).
        .args(["-map", "0:V:0", "-map", "0:a?", "-c", "copy"])
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

/// How far past the keyframe at `start` to point `-ss` so that FFmpeg starts on exactly that
/// keyframe. Aiming at the keyframe itself isn't enough:
///
/// - ffprobe rounds times to the microsecond, so a keyframe at 0.9333333 s reads as 0.933333,
///   just before itself. Hence at least a millisecond.
/// - For containers that seek by decode time (Matroska among them) and video with B-frames,
///   FFmpeg seeks 3/23 s (about 0.13 s) earlier than asked, which can land on the keyframe
///   before. Aiming 0.2 s in (or halfway to the next keyframe, if that's closer) absorbs it.
/// - MPEG-TS and MPEG-PS seek forward to the next keyframe instead, so for them the extra
///   0.2 s would skip the right one. They get the millisecond only.
///
/// Checked on every test format: the export's first frame is the keyframe's frame.
fn seek_offset(scan: &Scan, start: f64, length: f64) -> f64 {
    if start <= 0.0 {
        return 0.0;
    }
    let offset = if scan.seeks_forward || scan.keyframes.is_empty() {
        0.001
    } else {
        let gap = scan
            .keyframe_after(start)
            .map_or(f64::INFINITY, |next| next - start);
        (gap / 2.0).min(0.2)
    };
    // The clip itself may be shorter than that.
    offset.min(length / 2.0)
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
    fn seeks_just_past_the_keyframe() {
        let scan = Scan {
            keyframes: vec![0.0, 1.0, 1.1, 5.0],
            ..Scan::default()
        };
        assert_eq!(seek_offset(&scan, 0.0, 3.0), 0.0);
        assert_eq!(seek_offset(&scan, 1.1, 3.0), 0.2);
        assert!((seek_offset(&scan, 1.0, 3.0) - 0.05).abs() < 1e-9); // half the gap to 1.1
        assert!((seek_offset(&scan, 5.0, 0.1) - 0.05).abs() < 1e-9); // half a short clip
        let transport_stream = Scan {
            seeks_forward: true,
            ..scan
        };
        assert_eq!(seek_offset(&transport_stream, 1.1, 3.0), 0.001);
        assert_eq!(seek_offset(&Scan::default(), 2.5, 3.0), 0.001);
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
