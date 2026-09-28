//! Where a video's keyframes are. Lossless export can only start on one, so the start handle snaps
//! to them and the preview shows exactly what the export will contain.
//!
//! MP4, MOV and MKV files list their keyframes in an index (`index.rs`), which takes one small
//! read. Anything else, or a file whose index is missing, gets a packet scan: ffprobe lists every
//! video packet, which reads the whole file. That scan also measures how long the video really
//! is, for files whose container never got a duration written (an interrupted OBS recording).

use serde::{Deserialize, Serialize};
use tauri_plugin_shell::{process::CommandEvent, ShellExt};

use crate::cache::Cache;
use crate::index::{self, Keyframes};
use crate::jobs::{self, Kind};

/// Bump when `Scan` or how it's computed changes, so old cache entries are ignored.
const CACHE_VERSION: u32 = 3;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scan {
    /// Keyframe times in seconds from the start of the file, sorted. Empty when every frame is a
    /// keyframe (nothing to snap to) or the file couldn't be read.
    pub keyframes: Vec<f64>,
    /// When the last video frame ends, in seconds from the start of the file. 0 if unknown, which
    /// it always is when the keyframes came from the index.
    pub end: f64,
    /// True for containers whose seeking lands on the first keyframe *after* the requested time
    /// (MPEG transport and program streams); most others land on the one before.
    #[serde(default)]
    pub seeks_forward: bool,
}

impl Scan {
    /// The keyframe at or before `t` (0 before the first): where a lossless clip starting at `t`
    /// really starts. With no keyframe list, `t` itself.
    pub fn keyframe_at_or_before(&self, t: f64) -> f64 {
        if self.keyframes.is_empty() {
            return t;
        }
        // A hair of tolerance so a start already on a keyframe stays put.
        let found = self.keyframes.iter().rev().find(|&&k| k <= t + 1e-6);
        found.copied().unwrap_or(0.0)
    }

    /// The first keyframe after `t`, if any.
    pub fn keyframe_after(&self, t: f64) -> Option<f64> {
        self.keyframes.iter().copied().find(|&k| k > t + 1e-6)
    }
}

/// The video's keyframes: from the file's index if it has one, otherwise from a packet scan.
pub async fn scan(app: &tauri::AppHandle, path: &str) -> Result<Scan, String> {
    let cache = Cache::for_file(app, "keyframes", path, CACHE_VERSION);
    if let Some(scan) = cache.as_ref().and_then(Cache::read) {
        return Ok(scan);
    }
    if let Some(scan) = from_index(app, path).await {
        if let Some(cache) = &cache {
            cache.write(&scan);
        }
        return Ok(scan);
    }
    let scan = scan_packets(app, path).await?;
    if let (Some(cache), true) = (&cache, scan.end > 0.0) {
        cache.write(&scan);
    }
    Ok(scan)
}

/// How long the video really is, measured from its packets (0 if they can't be read). For files
/// whose container doesn't say.
pub async fn measure(app: &tauri::AppHandle, path: &str) -> f64 {
    scan_packets(app, path).await.map_or(0.0, |scan| scan.end)
}

/// Keyframes from the container's index, made relative to the file's start like a packet scan's.
async fn from_index(app: &tauri::AppHandle, path: &str) -> Option<Scan> {
    let owned = path.to_owned();
    let found = tauri::async_runtime::spawn_blocking(move || index::read(&owned))
        .await
        .ok()??;
    let keyframes = match found {
        Keyframes::All => Vec::new(),
        Keyframes::At(times) => {
            let start = start_time(app, path).await?;
            let mut keyframes: Vec<f64> = times.iter().map(|t| (t - start).max(0.0)).collect();
            keyframes.dedup();
            keyframes
        }
    };
    Some(Scan {
        keyframes,
        end: 0.0,
        seeks_forward: false,
    })
}

/// The file's start time as FFmpeg sees it (what `-ss 0` means), from its header only.
async fn start_time(app: &tauri::AppHandle, path: &str) -> Option<f64> {
    let output = app
        .shell()
        .sidecar("ffprobe")
        .ok()?
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=start_time",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.trim().parse().unwrap_or(0.0))
}

/// Reads every video packet's timestamp and flags (no decoding, so it's about as fast as the disk).
async fn scan_packets(app: &tauri::AppHandle, path: &str) -> Result<Scan, String> {
    let command = app
        .shell()
        .sidecar("ffprobe")
        .map_err(|e| e.to_string())?
        // V (capital) skips cover art, which counts as a one-frame video track.
        .args(["-v", "error", "-select_streams", "V:0"])
        .args([
            "-show_entries",
            "packet=pts_time,dts_time,duration_time,flags:format=start_time,format_name",
            "-of",
            "compact",
        ])
        .arg(path);
    // Stopped (and so failing here) if the user closes the file first.
    let (mut events, _job) = jobs::spawn(app, path, Kind::Background, command)?;
    let mut text = String::new();
    let mut ok = false;
    while let Some(event) = events.recv().await {
        match event {
            CommandEvent::Stdout(line) => {
                text.push_str(&String::from_utf8_lossy(&line));
                text.push('\n');
            }
            CommandEvent::Terminated(status) => {
                ok = status.code == Some(0);
                break;
            }
            _ => {}
        }
    }
    if !ok {
        return Err("Couldn't read the video's keyframes.".into());
    }
    Ok(parse(&text))
}

/// Parses `ffprobe -of compact` lines such as
/// `packet|pts_time=1.466667|dts_time=1.400000|duration_time=0.033333|flags=K__` and
/// `format|start_time=1.445333|format_name=mpegts`.
fn parse(text: &str) -> Scan {
    let mut keyframes = Vec::new();
    let mut packets = 0usize;
    let mut end = 0f64;
    let mut start_time = 0f64;
    let mut seeks_forward = false;
    for line in text.lines() {
        let line = line.trim();
        let section = line.split('|').next();
        let value = |name: &str| {
            line.split('|')
                .find_map(|field| field.strip_prefix(name)?.strip_prefix('='))
        };
        let number = |name: &str| value(name).and_then(|v| v.parse::<f64>().ok());
        match section {
            Some("format") => {
                start_time = number("start_time").unwrap_or(0.0);
                seeks_forward = matches!(value("format_name"), Some("mpegts" | "mpeg"));
            }
            Some("packet") => {
                let Some(time) = number("pts_time").or_else(|| number("dts_time")) else {
                    continue;
                };
                packets += 1;
                end = end.max(time + number("duration_time").unwrap_or(0.0));
                if value("flags").is_some_and(|flags| flags.starts_with('K')) {
                    keyframes.push(time);
                }
            }
            _ => {}
        }
    }
    // Times are relative to the file's start, which is what the player and `-ss` use.
    let mut keyframes: Vec<f64> = keyframes
        .into_iter()
        .map(|t| (t - start_time).max(0.0))
        .collect();
    keyframes.sort_by(f64::total_cmp);
    keyframes.dedup();
    if keyframes.len() == packets {
        keyframes.clear(); // every frame is a keyframe, so any start works
    }
    Scan {
        keyframes,
        end: (end - start_time).max(0.0),
        seeks_forward,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_keyframes_relative_to_the_start_of_the_file() {
        let text = "packet|pts_time=1.466667|dts_time=1.400000|duration_time=0.033333|flags=K__\n\
                    packet|pts_time=1.566667|dts_time=1.433333|duration_time=0.033333|flags=___\n\
                    packet|pts_time=1.500000|dts_time=1.466667|duration_time=0.033333|flags=___\n\
                    packet|pts_time=2.466667|dts_time=2.400000|duration_time=0.033333|flags=K__\n\
                    packet|pts_time=2.500000|dts_time=2.433333|duration_time=0.033333|flags=___\n\
                    format|start_time=1.400000|format_name=mpegts\n";
        let scan = parse(text);
        assert!(scan.seeks_forward);
        assert_eq!(scan.keyframes.len(), 2);
        assert!((scan.keyframes[0] - 0.066667).abs() < 1e-6);
        assert!((scan.keyframes[1] - 1.066667).abs() < 1e-6);
        assert!((scan.end - 1.133333).abs() < 1e-6);
    }

    #[test]
    fn uses_dts_when_pts_is_missing() {
        let text = "packet|pts_time=N/A|dts_time=0.000000|duration_time=0.04|flags=K_\n\
                    packet|pts_time=N/A|dts_time=0.040000|duration_time=0.04|flags=__\n\
                    format|start_time=N/A\n";
        let scan = parse(text);
        assert_eq!(scan.keyframes, [0.0]);
        assert!((scan.end - 0.08).abs() < 1e-9);
    }

    #[test]
    fn all_keyframes_means_nothing_to_snap_to() {
        let text = "packet|pts_time=0.0|dts_time=0.0|duration_time=0.5|flags=K__\n\
                    packet|pts_time=0.5|dts_time=0.5|duration_time=0.5|flags=K__\n";
        let scan = parse(text);
        assert!(scan.keyframes.is_empty());
        assert_eq!(scan.end, 1.0);
    }

    #[test]
    fn snaps_to_the_keyframe_at_or_before() {
        let scan = Scan {
            keyframes: vec![0.5, 2.0, 4.0],
            ..Scan::default()
        };
        assert_eq!(scan.keyframe_at_or_before(0.2), 0.0);
        assert_eq!(scan.keyframe_at_or_before(2.0), 2.0);
        assert_eq!(scan.keyframe_at_or_before(3.99), 2.0);
        assert_eq!(scan.keyframe_at_or_before(9.0), 4.0);
        assert_eq!(scan.keyframe_after(2.0), Some(4.0));
        assert_eq!(scan.keyframe_after(4.0), None);
        assert_eq!(Scan::default().keyframe_at_or_before(3.3), 3.3);
    }

    #[test]
    fn nothing_readable_is_empty() {
        assert_eq!(parse(""), Scan::default());
    }
}
