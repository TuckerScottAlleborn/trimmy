//! The timeline's waveform: peak loudness of the first audio track, in evenly spaced buckets.
//!
//! While FFmpeg works, the peaks stream to the UI in batches, so a long file's waveform draws
//! left to right instead of appearing all at once. Results are cached on disk (see `cache.rs`),
//! so reopening a file skips FFmpeg.

use std::time::{Duration, Instant};

use tauri::ipc::Channel;
use tauri_plugin_shell::{process::CommandEvent, ShellExt};

use crate::cache::Cache;

const SAMPLE_RATE: f64 = 8000.0;
const KEY: &str = "lavfi.astats.Overall.Peak_level=";
/// How often a batch of new peaks goes to the UI: smooth to watch, few enough messages to be free.
const SEND_EVERY: Duration = Duration::from_millis(80);

/// Returns `buckets` peaks from 0.0 to 1.0 (scaled so the loudest is 1.0), or an empty list
/// when the file has no audio. Meanwhile sends `progress` the raw, unscaled peaks as FFmpeg
/// produces them, in order; nothing is sent when the result comes from the cache.
pub async fn peaks(
    app: &tauri::AppHandle,
    path: &str,
    duration: f64,
    buckets: u32,
    progress: &Channel<Vec<f32>>,
) -> Result<Vec<f32>, String> {
    if duration <= 0.0 || buckets == 0 {
        return Ok(Vec::new());
    }
    let cache = Cache::for_file(app, "waveforms", path, buckets);
    if let Some(peaks) = cache.as_ref().and_then(Cache::read) {
        return Ok(peaks);
    }
    let peaks = compute(app, path, duration, buckets, progress).await?;
    // An empty result may be a failed FFmpeg run rather than a silent file, so it isn't cached.
    if let (Some(cache), false) = (&cache, peaks.is_empty()) {
        cache.write(&peaks);
    }
    Ok(peaks)
}

/// Runs FFmpeg over the file, streaming its per-bucket peaks to `progress` as they arrive.
async fn compute(
    app: &tauri::AppHandle,
    path: &str,
    duration: f64,
    buckets: u32,
    progress: &Channel<Vec<f32>>,
) -> Result<Vec<f32>, String> {
    // FFmpeg chops the audio into chunks of `per_bucket` samples and prints each chunk's peak (dB).
    let per_bucket = (duration * SAMPLE_RATE / f64::from(buckets))
        .ceil()
        .max(1.0);
    let filter = format!(
        "aformat=channel_layouts=mono,aresample={SAMPLE_RATE},asetnsamples=n={per_bucket}:p=0,\
         astats=metadata=1:reset=1:measure_perchannel=none:measure_overall=Peak_level,\
         ametadata=mode=print:key=lavfi.astats.Overall.Peak_level:file=-:direct=1"
    );
    let (mut events, _child) = app
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| e.to_string())?
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-i", path])
        .args(["-map", "0:a:0", "-vn", "-af", &filter, "-f", "null", "-"])
        .spawn()
        .map_err(|e| e.to_string())?;

    let mut peaks = Vec::new();
    let mut sent = 0;
    let mut last_send = Instant::now();
    while let Some(event) = events.recv().await {
        match event {
            CommandEvent::Stdout(line) => {
                if let Some(level) = level(&String::from_utf8_lossy(&line)) {
                    peaks.push(level);
                }
                if peaks.len() > sent && last_send.elapsed() >= SEND_EVERY {
                    // The UI may have closed the file already; then nobody is listening.
                    let _ = progress.send(peaks[sent..].to_vec());
                    sent = peaks.len();
                    last_send = Instant::now();
                }
            }
            CommandEvent::Terminated(status) if status.code == Some(0) => {
                return Ok(normalize(peaks));
            }
            CommandEvent::Terminated(_) => break,
            _ => {}
        }
    }
    Ok(Vec::new()) // no audio track, or FFmpeg failed
}

/// One peak from a line of FFmpeg's output, as a linear level (1.0 = full scale), if it is one.
fn level(line: &str) -> Option<f32> {
    let db = line.trim().strip_prefix(KEY)?;
    // Silence prints "-inf", which becomes 0.
    let level = db.parse::<f32>().map_or(0.0, |db| 10f32.powf(db / 20.0));
    Some(if level.is_finite() { level } else { 0.0 })
}

/// Scales the peaks so the loudest is 1.0.
fn normalize(mut peaks: Vec<f32>) -> Vec<f32> {
    let loudest = peaks.iter().copied().fold(0.0, f32::max);
    if loudest > 0.0 {
        peaks.iter_mut().for_each(|p| *p /= loudest);
    }
    peaks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_peaks_and_scales_to_the_loudest() {
        let text = "frame:0 pts:0 pts_time:0\n\
                    lavfi.astats.Overall.Peak_level=-6.020600\n\
                    frame:1 pts:400 pts_time:0.05\n\
                    lavfi.astats.Overall.Peak_level=0.000000\n\
                    frame:2 pts:800 pts_time:0.1\n\
                    lavfi.astats.Overall.Peak_level=-inf\n";
        let peaks = normalize(text.lines().filter_map(level).collect());
        assert_eq!(peaks.len(), 3);
        assert!((peaks[0] - 0.5).abs() < 0.001);
        assert_eq!(peaks[1], 1.0);
        assert_eq!(peaks[2], 0.0);
    }
}
