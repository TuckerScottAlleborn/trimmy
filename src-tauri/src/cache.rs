//! A small on-disk cache for things Trimmy works out by reading a whole video (the waveform, the
//! keyframe list), so reopening a file skips FFmpeg.
//!
//! Entries live under the app's cache dir, one folder per kind, keyed by a hash of the file's path
//! plus its size, modified time and a per-kind variant (the waveform's bar count, for example).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tauri::Manager;

/// The most entries kept per kind; the oldest are deleted past this.
const MAX_CACHED: usize = 200;

/// Where one video's cached result for one kind lives.
pub struct Cache {
    dir: PathBuf,
    key: Key,
}

impl Cache {
    /// The cache slot for `path` in the `kind` folder, or `None` when the app's cache dir or the
    /// file's metadata can't be read (Trimmy then just recomputes).
    pub fn for_file(app: &tauri::AppHandle, kind: &str, path: &str, variant: u32) -> Option<Self> {
        let dir = app.path().app_cache_dir().ok()?.join(kind);
        Some(Self {
            dir,
            key: Key::for_file(path, variant)?,
        })
    }

    /// The cached value, or `None` on a miss or any read error.
    pub fn read<T: DeserializeOwned>(&self) -> Option<T> {
        read(&self.dir, &self.key)
    }

    /// Stores `value`, then trims the folder to the newest entries. Failures are ignored.
    pub fn write<T: Serialize>(&self, value: &T) {
        if write(&self.dir, &self.key, value).is_some() {
            prune(&self.dir, MAX_CACHED);
        }
    }
}

/// What a cached result depends on; if any of it changes, the result is recomputed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Key {
    /// FNV-1a of the file's path. Only the hash is stored, so the cache folder doesn't keep a
    /// readable list of every video the user has opened.
    path_hash: u64,
    size: u64,
    /// Modified time in nanoseconds since the UNIX epoch.
    modified: u64,
    variant: u32,
}

/// A cache file: the key it was made for, so a hash collision reads as a miss, and the value.
#[derive(Serialize, Deserialize)]
struct Entry<T> {
    key: Key,
    value: T,
}

impl Key {
    /// The key for a file on disk, or `None` when its metadata can't be read.
    fn for_file(path: &str, variant: u32) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        let since_epoch = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        Some(Self {
            path_hash: fnv1a(path.as_bytes()),
            size: meta.len(),
            modified: u64::try_from(since_epoch.as_nanos()).ok()?,
            variant,
        })
    }

    /// The cache file's name: hex of a stable hash of every key field.
    fn file_name(&self) -> String {
        let mut bytes = self.path_hash.to_le_bytes().to_vec();
        bytes.extend_from_slice(&self.size.to_le_bytes());
        bytes.extend_from_slice(&self.modified.to_le_bytes());
        bytes.extend_from_slice(&self.variant.to_le_bytes());
        format!("{:016x}.json", fnv1a(&bytes))
    }
}

/// 64-bit FNV-1a: tiny, and unlike `DefaultHasher` it never changes between Rust versions.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn read<T: DeserializeOwned>(dir: &Path, key: &Key) -> Option<T> {
    let text = fs::read(dir.join(key.file_name())).ok()?;
    let entry: Entry<T> = serde_json::from_slice(&text).ok()?;
    (entry.key == *key).then_some(entry.value)
}

fn write<T: Serialize>(dir: &Path, key: &Key, value: &T) -> Option<()> {
    fs::create_dir_all(dir).ok()?;
    let entry = Entry {
        key: key.clone(),
        value,
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

    fn key() -> Key {
        Key {
            path_hash: fnv1a(b"C:/videos/clip.mp4"),
            size: 123_456_789,
            modified: 1_700_000_000_000_000_000,
            variant: 1200,
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
            Key {
                path_hash: fnv1a(b"C:/videos/other.mp4"),
                ..key()
            },
            Key { size: 1, ..key() },
            Key {
                modified: 1,
                ..key()
            },
            Key {
                variant: 600,
                ..key()
            },
        ];
        for other in changed {
            assert_ne!(base.file_name(), other.file_name());
        }
    }

    #[test]
    fn round_trips_and_a_mismatched_key_misses() {
        let dir = temp_dir("cache");
        let peaks = vec![0.0f32, 0.25, 0.5, 1.0, 0.123_456_79];
        assert_eq!(read::<Vec<f32>>(&dir, &key()), None);
        assert_eq!(write(&dir, &key(), &peaks), Some(()));
        assert_eq!(read(&dir, &key()), Some(peaks.clone()));

        // A different key stored under this key's file name (a hash collision) is a miss.
        let entry = Entry {
            key: Key { size: 1, ..key() },
            value: peaks.clone(),
        };
        fs::write(
            dir.join(key().file_name()),
            serde_json::to_vec(&entry).unwrap(),
        )
        .unwrap();
        assert_eq!(read::<Vec<f32>>(&dir, &key()), None);

        // A corrupt file is a miss too.
        fs::write(dir.join(key().file_name()), b"not json").unwrap();
        assert_eq!(read::<Vec<f32>>(&dir, &key()), None);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prune_keeps_only_the_newest() {
        let dir = temp_dir("cache-prune");
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
