//! The timeline's waveform: peak loudness of the first audio track, in evenly spaced buckets.
//!
//! Two ways to get it:
//! - **Full** (most files): FFmpeg reads the whole audio track once and reports each bucket's
//!   peak. Exact, and quick for game clips and anything else up to a few GB.
//! - **Sampled** (big or long files): FFmpeg jumps to the middle of each bucket and measures half
//!   a second there. A 38 GB movie on a USB hard drive takes about half a minute this way instead
//!   of an hour, since it reads a few hundred small pieces instead of every byte of the video.
//!   With a bucket per 30 s of a two-hour film, the overview looks the same.
//!
//! Either way the peaks stream to the UI as they arrive, so the waveform draws left to right, and
//! results are cached on disk (see `cache.rs`), so reopening a file skips FFmpeg.

use std::time::{Duration, Instant};

use tauri::ipc::Channel;
use tauri_plugin_shell::{
    process::{Command, CommandEvent},
    ShellExt,
};

use crate::cache::Cache;
use crate::jobs::{self, Kind};

const SAMPLE_RATE: f64 = 8000.0;
const KEY: &str = "lavfi.astats.Overall.Peak_level=";
/// How often a batch of new peaks goes to the UI: smooth to watch, few enough messages to be free.
const SEND_EVERY: Duration = Duration::from_millis(80);

/// Files bigger than this, or longer than `SAMPLE_OVER_SECONDS`, get the sampled waveform.
const SAMPLE_OVER_BYTES: u64 = 2_000_000_000;
const SAMPLE_OVER_SECONDS: f64 = 45.0 * 60.0;
/// How much audio the sampled waveform measures in each bucket.
const SLICE_SECONDS: f64 = 0.5;
/// Buckets per FFmpeg run in sampled mode. Each run costs about a second to open the file; more
/// buckets share that cost, but past ~16 the parallel reads make a hard drive thrash.
const SAMPLES_PER_RUN: u32 = 12;

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
    let size = std::fs::metadata(path).map_or(0, |m| m.len());
    let sampled = size > SAMPLE_OVER_BYTES || duration > SAMPLE_OVER_SECONDS;
    // Sampled and full waveforms of the same file are cached separately.
    let variant = if sampled { buckets | 1 << 31 } else { buckets };
    let cache = Cache::for_file(app, "waveforms", path, variant);
    if let Some(peaks) = cache.as_ref().and_then(Cache::read) {
        return Ok(peaks);
    }
    let raw = if sampled {
        sample(app, path, duration, buckets, progress).await
    } else {
        read_all(app, path, duration, buckets, progress).await
    };
    let peaks = normalize(raw);
    // An empty result may be a failed or stopped FFmpeg run rather than a silent file, so it
    // isn't cached.
    if let (Some(cache), false) = (&cache, peaks.is_empty()) {
        cache.write(&peaks);
    }
    Ok(peaks)
}

/// Full: FFmpeg reads the whole audio track, chopped into `buckets` chunks.
async fn read_all(
    app: &tauri::AppHandle,
    path: &str,
    duration: f64,
    buckets: u32,
    progress: &Channel<Vec<f32>>,
) -> Vec<f32> {
    let per_bucket = (duration * SAMPLE_RATE / f64::from(buckets))
        .ceil()
        .max(1.0);
    let filter = format!(
        "aformat=channel_layouts=mono,aresample={SAMPLE_RATE},asetnsamples=n={per_bucket}:p=0,{}",
        measure()
    );
    let Ok(command) = app.shell().sidecar("ffmpeg") else {
        return Vec::new();
    };
    let command = command
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-i", path])
        .args(["-map", "0:a:0", "-vn", "-af", &filter, "-f", "null", "-"]);
    let mut peaks = Vec::new();
    let ok = run(app, path, command, &mut peaks, progress).await;
    if ok {
        peaks
    } else {
        Vec::new() // no audio track, FFmpeg failed, or the file was closed
    }
}

/// Sampled: for each bucket, half a second from its middle. `SAMPLES_PER_RUN` buckets per FFmpeg
/// run, each its own `-ss`/`-t` input (so FFmpeg seeks straight there), padded or trimmed to
/// exactly one slice and joined in order, so each prints exactly one peak.
async fn sample(
    app: &tauri::AppHandle,
    path: &str,
    duration: f64,
    buckets: u32,
    progress: &Channel<Vec<f32>>,
) -> Vec<f32> {
    let slice = SLICE_SECONDS.min(duration / f64::from(buckets));
    let samples = (slice * SAMPLE_RATE).round().max(1.0);
    let mut peaks = Vec::with_capacity(buckets as usize);
    for first in (0..buckets).step_by(SAMPLES_PER_RUN as usize) {
        let points: Vec<f64> = (first..buckets.min(first + SAMPLES_PER_RUN))
            .map(|i| {
                let middle = (f64::from(i) + 0.5) * duration / f64::from(buckets);
                (middle - slice / 2.0).clamp(0.0, (duration - slice).max(0.0))
            })
            .collect();
        let Ok(mut command) = app.shell().sidecar("ffmpeg") else {
            return Vec::new();
        };
        command = command.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
        for start in &points {
            command = command
                .args(["-ss", &format!("{start:.3}"), "-t", &format!("{slice:.3}")])
                .args(["-i", path]);
        }
        let command = command
            .args(["-filter_complex", &sample_filter(points.len(), samples)])
            .args(["-f", "null", "-"]);
        let before = peaks.len();
        let ok = run(app, path, command, &mut peaks, progress).await;
        if !ok || peaks.len() != before + points.len() {
            // The first run failing means no audio (or the file was closed): no waveform. A later
            // one failing leaves a gap rather than throwing away what's already drawn.
            if before == 0 {
                return Vec::new();
            }
            peaks.resize(before + points.len(), 0.0);
        }
    }
    peaks
}

/// The filter graph for one sampled run: each input's first audio track, mono at 8 kHz, cut or
/// padded to exactly `samples`, then all of them in order through the peak meter.
fn sample_filter(inputs: usize, samples: f64) -> String {
    let mut graph = String::new();
    for i in 0..inputs {
        graph += &format!(
            "[{i}:a:0]aformat=channel_layouts=mono,aresample={SAMPLE_RATE},\
             apad=whole_len={samples},atrim=end_sample={samples}[s{i}];"
        );
    }
    for i in 0..inputs {
        graph += &format!("[s{i}]");
    }
    graph += &format!(
        "concat=n={inputs}:v=0:a=1,asetnsamples=n={samples}:p=0,{}",
        measure()
    );
    graph
}

/// Measures each chunk's peak and prints it, unbuffered so it can be streamed.
fn measure() -> &'static str {
    "astats=metadata=1:reset=1:measure_perchannel=none:measure_overall=Peak_level,\
     ametadata=mode=print:key=lavfi.astats.Overall.Peak_level:file=-:direct=1"
}

/// Runs one FFmpeg command, appending its peaks to `peaks` and streaming new ones to `progress`.
/// True if FFmpeg finished successfully.
async fn run(
    app: &tauri::AppHandle,
    path: &str,
    command: Command,
    peaks: &mut Vec<f32>,
    progress: &Channel<Vec<f32>>,
) -> bool {
    let Ok((mut events, _job)) = jobs::spawn(app, path, Kind::Background, command) else {
        return false;
    };
    let mut sent = peaks.len();
    let mut last_send = Instant::now();
    let mut ok = false;
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
            CommandEvent::Terminated(status) => {
                ok = status.code == Some(0);
                break;
            }
            _ => {}
        }
    }
    if peaks.len() > sent {
        let _ = progress.send(peaks[sent..].to_vec());
    }
    ok
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

    #[test]
    fn sample_filter_joins_every_input_in_order() {
        let graph = sample_filter(3, 4000.0);
        assert!(graph.starts_with("[0:a:0]aformat=channel_layouts=mono,aresample=8000,"));
        assert!(graph.contains("apad=whole_len=4000,atrim=end_sample=4000[s2];"));
        assert!(graph.contains("[s0][s1][s2]concat=n=3:v=0:a=1,asetnsamples=n=4000:p=0,astats"));
    }
}
