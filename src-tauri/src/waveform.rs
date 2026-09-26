//! The timeline's waveform: peak loudness of the first audio track, in evenly spaced buckets.

use tauri_plugin_shell::ShellExt;

const SAMPLE_RATE: f64 = 8000.0;
const KEY: &str = "lavfi.astats.Overall.Peak_level=";

/// Returns `buckets` peaks from 0.0 to 1.0 (scaled so the loudest is 1.0), or an empty list
/// when the file has no audio.
pub async fn peaks(
    app: &tauri::AppHandle,
    path: &str,
    duration: f64,
    buckets: u32,
) -> Result<Vec<f32>, String> {
    if duration <= 0.0 || buckets == 0 {
        return Ok(Vec::new());
    }
    // FFmpeg chops the audio into chunks of `per_bucket` samples and prints each chunk's peak (dB).
    let per_bucket = (duration * SAMPLE_RATE / f64::from(buckets))
        .ceil()
        .max(1.0);
    let filter = format!(
        "aformat=channel_layouts=mono,aresample={SAMPLE_RATE},asetnsamples=n={per_bucket}:p=0,\
         astats=metadata=1:reset=1:measure_perchannel=none:measure_overall=Peak_level,\
         ametadata=mode=print:key=lavfi.astats.Overall.Peak_level:file=-"
    );
    let output = app
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| e.to_string())?
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-i", path])
        .args(["-map", "0:a:0", "-vn", "-af", &filter, "-f", "null", "-"])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Ok(Vec::new()); // no audio track
    }
    Ok(parse(&String::from_utf8_lossy(&output.stdout)))
}

fn parse(text: &str) -> Vec<f32> {
    let mut peaks: Vec<f32> = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix(KEY))
        // Silence prints "-inf", which becomes 0.
        .map(|db| db.parse::<f32>().map_or(0.0, |db| 10f32.powf(db / 20.0)))
        .map(|level| if level.is_finite() { level } else { 0.0 })
        .collect();
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
        let peaks = parse(text);
        assert_eq!(peaks.len(), 3);
        assert!((peaks[0] - 0.5).abs() < 0.001);
        assert_eq!(peaks[1], 1.0);
        assert_eq!(peaks[2], 0.0);
    }
}
