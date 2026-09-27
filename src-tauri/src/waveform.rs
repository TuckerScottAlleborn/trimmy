//! The timeline's waveform: peak loudness of the first audio track, in evenly spaced buckets.
//!
//! Results are cached on disk under the app's cache dir (`waveforms/`), keyed by a hash of the
//! file's path plus its size, modified time and the bucket count, so reopening a file skips FFmpeg.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use tauri::Manager;
use tauri_plugin_shell::ShellExt;

const SAMPLE_RATE: f64 = 8000.0;
const KEY: &str = "lavfi.astats.Overall.Peak_level=";
/// The most cache entries kept; the oldest are deleted past this.
const MAX_CACHED: usize = 200;

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
    let key = CacheKey::for_file(path, buckets);
    let dir = app
        .path()
        .app_cache_dir()
        .ok()
        .map(|dir| dir.join("waveforms"));
    if let (Some(key), Some(dir)) = (&key, &dir) {
        if let Some(peaks) = read_cache(dir, key) {
            return Ok(peaks);
        }
    }
    let peaks = compute(app, path, duration, buckets).await?;
    // An empty result may be a failed FFmpeg run rather than a silent file, so it isn't cached.
    if let (Some(key), Some(dir), false) = (&key, &dir, peaks.is_empty()) {
        if write_cache(dir, key, &peaks).is_some() {
            prune(dir, MAX_CACHED);
        }
    }
    Ok(peaks)
}

/// Runs FFmpeg over the file and parses its per-bucket peaks.
async fn compute(
    app: &tauri::AppHandle,
    path: &str,
    duration: f64,
    buckets: u32,
) -> Result<Vec<f32>, String> {
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

/// What a cached waveform depends on; if any of it changes, the waveform is recomputed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CacheKey {
    /// FNV-1a of the file's path. Only the hash is stored, so the cache folder doesn't keep a
    /// readable list of every video the user has opened.
    path_hash: u64,
    size: u64,
    /// Modified time in nanoseconds since the UNIX epoch.
    modified: u64,
    buckets: u32,
}

/// A cache file: the key it was made for, so a hash collision reads as a miss, and the peaks.
#[derive(Serialize, Deserialize)]
struct CacheEntry {
    key: CacheKey,
    peaks: Vec<f32>,
}

impl CacheKey {
    /// The key for a file on disk, or `None` when its metadata can't be read.
    fn for_file(path: &str, buckets: u32) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        let since_epoch = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        Some(Self {
            path_hash: fnv1a(path.as_bytes()),
            size: meta.len(),
            modified: u64::try_from(since_epoch.as_nanos()).ok()?,
            buckets,
        })
    }

    /// The cache file's name: hex of a stable hash of every key field.
    fn file_name(&self) -> String {
        let mut bytes = self.path_hash.to_le_bytes().to_vec();
        bytes.extend_from_slice(&self.size.to_le_bytes());
        bytes.extend_from_slice(&self.modified.to_le_bytes());
        bytes.extend_from_slice(&self.buckets.to_le_bytes());
        format!("{:016x}.json", fnv1a(&bytes))
    }
}

/// 64-bit FNV-1a: tiny, and unlike `DefaultHasher` it never changes between Rust versions.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The cached peaks for `key`, or `None` on a miss or any read error.
fn read_cache(dir: &Path, key: &CacheKey) -> Option<Vec<f32>> {
    let text = fs::read(dir.join(key.file_name())).ok()?;
    let entry: CacheEntry = serde_json::from_slice(&text).ok()?;
    (entry.key == *key).then_some(entry.peaks)
}

/// Stores the peaks for `key`; returns `None` if that failed.
fn write_cache(dir: &Path, key: &CacheKey, peaks: &[f32]) -> Option<()> {
    fs::create_dir_all(dir).ok()?;
    let entry = CacheEntry {
        key: key.clone(),
        peaks: peaks.to_vec(),
    };
    let json = serde_json::to_vec(&entry).ok()?;
    fs::write(dir.join(key.file_name()), json).ok()
}

/// Deletes the oldest files (by modified time) until at most `keep` remain.
fn prune(dir: &Path, keep: usize) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = read
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((meta.modified().ok()?, entry.path()))
        })
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort();
    let excess = files.len() - keep;
    for (_, path) in files.into_iter().take(excess) {
        fs::remove_file(path).ok();
    }
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

    fn key() -> CacheKey {
        CacheKey {
            path_hash: fnv1a(b"C:/videos/clip.mp4"),
            size: 123_456_789,
            modified: 1_700_000_000_000_000_000,
            buckets: 1200,
        }
    }

    /// A fresh, empty folder under the system temp dir.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("trimmy-{name}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn fnv1a_matches_the_reference_values() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn file_name_is_stable_and_depends_on_every_field() {
        let base = key();
        assert_eq!(base.file_name(), key().file_name());
        assert!(base.file_name().ends_with(".json"));
        let changed = [
            CacheKey {
                path_hash: fnv1a(b"C:/videos/other.mp4"),
                ..key()
            },
            CacheKey { size: 1, ..key() },
            CacheKey {
                modified: 1,
                ..key()
            },
            CacheKey {
                buckets: 600,
                ..key()
            },
        ];
        for other in changed {
            assert_ne!(base.file_name(), other.file_name());
        }
    }

    #[test]
    fn cache_round_trips_and_a_mismatched_key_misses() {
        let dir = temp_dir("waveform-cache");
        let peaks = vec![0.0, 0.25, 0.5, 1.0, 0.123_456_79];
        assert_eq!(read_cache(&dir, &key()), None);
        assert_eq!(write_cache(&dir, &key(), &peaks), Some(()));
        assert_eq!(read_cache(&dir, &key()), Some(peaks.clone()));

        // A different key stored under this key's file name (a hash collision) is a miss.
        let other = CacheKey { size: 1, ..key() };
        let entry = CacheEntry {
            key: other,
            peaks: peaks.clone(),
        };
        fs::write(
            dir.join(key().file_name()),
            serde_json::to_vec(&entry).unwrap(),
        )
        .unwrap();
        assert_eq!(read_cache(&dir, &key()), None);

        // A corrupt file is a miss too.
        fs::write(dir.join(key().file_name()), b"not json").unwrap();
        assert_eq!(read_cache(&dir, &key()), None);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prune_keeps_only_the_newest() {
        let dir = temp_dir("waveform-prune");
        fs::create_dir_all(&dir).unwrap();
        let base = std::time::SystemTime::now();
        for i in 0..5u64 {
            let path = dir.join(format!("{i}.json"));
            fs::write(&path, b"{}").unwrap();
            let file = fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(base + std::time::Duration::from_secs(i))
                .unwrap();
        }
        prune(&dir, 3);
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["2.json", "3.json", "4.json"]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
