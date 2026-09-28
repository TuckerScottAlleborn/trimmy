//! Keyframe times read straight from a container's own index, so a big file doesn't have to be
//! read end to end to find them:
//!
//! - MP4 / MOV / M4V / 3GP: the video track's sync-sample table (`stss`), with its timing tables
//!   (`stts`, `ctts`) and edit list (`elst`).
//! - MKV / WebM: the `Cues` index. Every cue point is a keyframe; a file may list fewer than all
//!   of them, which only means fewer places for the start handle to snap to.
//!
//! Times are presentation timestamps in seconds as the demuxer reports them; `keyframes.rs` makes
//! them relative to the file's start, exactly as it does for a packet scan. Returns `None` for
//! anything it can't read confidently (another container, a fragmented MP4, an MKV without cues,
//! a truncated file), and the caller falls back to scanning packets.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// The most index data read into memory. A two-hour MP4's tables are a few MB.
const MAX_INDEX_BYTES: u64 = 256 << 20;
/// The most video frames an MP4 index is trusted to describe: a day at 120 fps. A damaged table
/// claiming more is left to the packet scan rather than filling memory.
const MAX_SAMPLES: usize = 24 * 3600 * 120;

/// What an index says about the video track's keyframes.
#[derive(Debug, PartialEq)]
pub enum Keyframes {
    /// Every frame is a keyframe (no sync-sample table): any start works.
    All,
    /// Keyframe times in seconds, sorted.
    At(Vec<f64>),
}

pub fn read(path: &str) -> Option<Keyframes> {
    let mut file = File::open(path).ok()?;
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;
    if magic[..4] == [0x1A, 0x45, 0xDF, 0xA3] {
        mkv(&mut file)
    } else if matches!(
        &magic[4..8],
        b"ftyp" | b"moov" | b"mdat" | b"wide" | b"free" | b"skip"
    ) {
        mp4(&mut file)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// MP4

/// One box inside an in-memory buffer: its type and contents.
fn boxes(mut data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        if data.len() < 8 {
            return None;
        }
        let size32 = u32::from_be_bytes(data[0..4].try_into().ok()?) as u64;
        let kind: [u8; 4] = data[4..8].try_into().ok()?;
        let (header, size) = match size32 {
            1 => (16, u64::from_be_bytes(data.get(8..16)?.try_into().ok()?)),
            0 => (8, data.len() as u64),
            n => (8, n),
        };
        let size = usize::try_from(size).ok()?;
        if size < header || size > data.len() {
            return None;
        }
        let body = &data[header..size];
        data = &data[size..];
        Some((kind, body))
    })
}

fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data).find(|(k, _)| k == kind).map(|(_, body)| body)
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(data: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(data.get(at..at + 8)?.try_into().ok()?))
}

/// Finds the top-level `moov` box (often after a huge `mdat`) and reads it into memory.
fn read_moov(file: &mut File) -> Option<Vec<u8>> {
    let len = file.metadata().ok()?.len();
    let mut at = 0u64;
    while at + 8 <= len {
        file.seek(SeekFrom::Start(at)).ok()?;
        let mut header = [0u8; 16];
        file.read_exact(&mut header[..8]).ok()?;
        let size32 = u32::from_be_bytes(header[0..4].try_into().ok()?) as u64;
        let (header_len, size) = match size32 {
            1 => {
                file.read_exact(&mut header[8..16]).ok()?;
                (16, u64::from_be_bytes(header[8..16].try_into().ok()?))
            }
            0 => (8, len - at),
            n => (8, n),
        };
        if size < header_len {
            return None;
        }
        if &header[4..8] == b"moov" {
            let body = size - header_len;
            if body > MAX_INDEX_BYTES {
                return None;
            }
            let mut moov = vec![0u8; usize::try_from(body).ok()?];
            file.read_exact(&mut moov).ok()?;
            return Some(moov);
        }
        // A fragmented MP4 keeps its samples in moof boxes, which this doesn't read.
        if &header[4..8] == b"moof" {
            return None;
        }
        at += size;
    }
    None
}

fn mp4(file: &mut File) -> Option<Keyframes> {
    let moov = read_moov(file)?;
    if child(&moov, b"mvex").is_some() {
        return None; // fragmented
    }
    let movie_timescale = child(&moov, b"mvhd").and_then(|mvhd| match mvhd.first()? {
        1 => u32_at(mvhd, 20),
        _ => u32_at(mvhd, 12),
    })?;
    // The first video track with more than one sample; a one-frame track is cover art.
    let found = boxes(&moov)
        .filter(|(kind, _)| kind == b"trak")
        .find_map(|(_, trak)| video_track(trak, movie_timescale));
    found
}

fn video_track(trak: &[u8], movie_timescale: u32) -> Option<Keyframes> {
    let mdia = child(trak, b"mdia")?;
    let hdlr = child(mdia, b"hdlr")?;
    if hdlr.get(8..12)? != b"vide" {
        return None;
    }
    let mdhd = child(mdia, b"mdhd")?;
    let timescale = match mdhd.first()? {
        1 => u32_at(mdhd, 20)?,
        _ => u32_at(mdhd, 12)?,
    };
    if timescale == 0 || movie_timescale == 0 {
        return None;
    }
    let stbl = child(child(mdia, b"minf")?, b"stbl")?;

    // Decode time of every sample, from the run-length stts table.
    let stts = child(stbl, b"stts")?;
    let mut dts = Vec::new();
    let mut t: i64 = 0;
    for entry in 0..u32_at(stts, 4)? as usize {
        let count = u32_at(stts, 8 + entry * 8)?;
        let delta = u32_at(stts, 12 + entry * 8)?;
        if dts.len() + count as usize > MAX_SAMPLES {
            return None;
        }
        for _ in 0..count {
            dts.push(t);
            t += i64::from(delta);
        }
    }
    if dts.len() < 2 {
        return None;
    }

    // Composition offsets (B-frames): presentation = decode + offset. Read as signed, like FFmpeg.
    let mut offsets = vec![0i64; dts.len()];
    if let Some(ctts) = child(stbl, b"ctts") {
        let mut sample = 0usize;
        for entry in 0..u32_at(ctts, 4)? as usize {
            let count = u32_at(ctts, 8 + entry * 8)? as usize;
            let offset = i64::from(u32_at(ctts, 12 + entry * 8)? as i32);
            for slot in offsets.iter_mut().skip(sample).take(count) {
                *slot = offset;
            }
            sample += count;
        }
    }

    // The edit list shifts the track: leading empty edits delay it, and the one real edit says
    // which media time is shown first. More than one real edit (a file re-cut in place) plays
    // pieces out of order, which only the packet scan gets right.
    let (mut delay, mut media_start) = (0i64, 0i64);
    if let Some(elst) = child(trak, b"edts").and_then(|edts| child(edts, b"elst")) {
        let version = *elst.first()?;
        let (entry_size, count) = (if version == 1 { 20 } else { 12 }, u32_at(elst, 4)?);
        let mut edits = 0;
        for entry in 0..count as usize {
            let at = 8 + entry * entry_size;
            let (duration, media_time) = if version == 1 {
                (u64_at(elst, at)? as i64, u64_at(elst, at + 8)? as i64)
            } else {
                (
                    i64::from(u32_at(elst, at)?),
                    i64::from(u32_at(elst, at + 4)? as i32),
                )
            };
            if media_time == -1 {
                if edits > 0 {
                    return None; // a gap after the start: a re-cut file
                }
                // Empty edit, in movie timescale units: a delay before the track starts.
                delay += duration * i64::from(timescale) / i64::from(movie_timescale);
            } else {
                media_start = media_time;
                edits += 1;
            }
        }
        if edits != 1 && count > 0 {
            return None;
        }
    }

    let seconds = |sample: usize| {
        (dts[sample] + offsets[sample] - media_start + delay) as f64 / f64::from(timescale)
    };
    let Some(stss) = child(stbl, b"stss") else {
        return Some(Keyframes::All);
    };
    let mut times = Vec::new();
    for entry in 0..u32_at(stss, 4)? as usize {
        let sample = u32_at(stss, 8 + entry * 4)? as usize;
        if sample == 0 || sample > dts.len() {
            return None;
        }
        times.push(seconds(sample - 1));
    }
    times.sort_by(f64::total_cmp);
    Some(Keyframes::At(times))
}

// ---------------------------------------------------------------------------------------------
// Matroska / WebM

const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549_A966;
const TIMESTAMP_SCALE: u32 = 0x2AD7B1;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const CUES: u32 = 0x1C53_BB6B;
const CUE_POINT: u32 = 0xBB;
const CUE_TIME: u32 = 0xB3;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_TRACK: u32 = 0xF7;
const CLUSTER: u32 = 0x1F43_B675;

/// An EBML variable-length integer: returns (value, length). IDs keep their marker bit.
fn vint(data: &[u8], keep_marker: bool) -> Option<(u64, usize)> {
    let first = *data.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || data.len() < len {
        return None;
    }
    let mut value = if keep_marker {
        u64::from(first)
    } else {
        u64::from(first) & ((1u64 << (8 - len)) - 1)
    };
    for &byte in &data[1..len] {
        value = (value << 8) | u64::from(byte);
    }
    Some((value, len))
}

/// Elements in an in-memory buffer: (id, contents).
fn elements(mut data: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    std::iter::from_fn(move || {
        let (id, id_len) = vint(data, true)?;
        let (size, size_len) = vint(data.get(id_len..)?, false)?;
        let start = id_len + size_len;
        let end = start.checked_add(usize::try_from(size).ok()?)?;
        if end > data.len() {
            return None;
        }
        let body = &data[start..end];
        data = &data[end..];
        Some((u32::try_from(id).ok()?, body))
    })
}

fn uint(data: &[u8]) -> u64 {
    data.iter()
        .fold(0, |value, &byte| (value << 8) | u64::from(byte))
}

/// Reads the element header at `at`: (id, body start, body size). Size is None when unknown.
fn header_at(file: &mut File, at: u64) -> Option<(u32, u64, Option<u64>)> {
    file.seek(SeekFrom::Start(at)).ok()?;
    let mut buf = [0u8; 16];
    let n = file.read(&mut buf).ok()?;
    let (id, id_len) = vint(&buf[..n], true)?;
    let (size, size_len) = vint(buf.get(id_len..n)?, false)?;
    let unknown = size == (1u64 << (7 * size_len)) - 1;
    Some((
        u32::try_from(id).ok()?,
        at + (id_len + size_len) as u64,
        (!unknown).then_some(size),
    ))
}

fn read_body(file: &mut File, start: u64, size: u64) -> Option<Vec<u8>> {
    if size > MAX_INDEX_BYTES {
        return None;
    }
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut body = vec![0u8; usize::try_from(size).ok()?];
    file.read_exact(&mut body).ok()?;
    Some(body)
}

fn mkv(file: &mut File) -> Option<Keyframes> {
    // Skip the EBML header to the Segment.
    let (_, ebml_start, ebml_size) = header_at(file, 0)?;
    let (id, segment, _) = header_at(file, ebml_start + ebml_size?)?;
    if id != SEGMENT {
        return None;
    }

    // Walk the Segment's top-level elements up to the first Cluster, noting Info, Tracks, Cues
    // and where the SeekHead says Cues is (usually after all the clusters).
    let (mut scale, mut video_track, mut cues, mut cues_at) = (1_000_000u64, None, None, None);
    let mut at = segment;
    for _ in 0..64 {
        let Some((id, start, size)) = header_at(file, at) else {
            break;
        };
        if id == CLUSTER {
            break;
        }
        let size = size?;
        match id {
            SEEK_HEAD => {
                for (_, seek) in
                    elements(&read_body(file, start, size)?).filter(|(id, _)| *id == SEEK)
                {
                    let fields: Vec<_> = elements(seek).collect();
                    let target = fields
                        .iter()
                        .find(|(id, _)| *id == SEEK_ID)
                        .map(|(_, v)| uint(v));
                    let position = fields
                        .iter()
                        .find(|(id, _)| *id == SEEK_POSITION)
                        .map(|(_, v)| uint(v));
                    if target == Some(u64::from(CUES)) {
                        cues_at = position.map(|p| segment + p);
                    }
                }
            }
            INFO => {
                if let Some((_, v)) =
                    elements(&read_body(file, start, size)?).find(|(id, _)| *id == TIMESTAMP_SCALE)
                {
                    scale = uint(v);
                }
            }
            TRACKS => {
                video_track = elements(&read_body(file, start, size)?)
                    .filter(|(id, _)| *id == TRACK_ENTRY)
                    .find_map(|(_, entry)| {
                        let fields: Vec<_> = elements(entry).collect();
                        let kind = fields
                            .iter()
                            .find(|(id, _)| *id == TRACK_TYPE)
                            .map(|(_, v)| uint(v));
                        let number = fields
                            .iter()
                            .find(|(id, _)| *id == TRACK_NUMBER)
                            .map(|(_, v)| uint(v));
                        (kind == Some(1)).then_some(number).flatten()
                    });
            }
            CUES => cues = Some(read_body(file, start, size)?),
            _ => {}
        }
        at = start + size;
    }
    let cues = match (cues, cues_at) {
        (Some(cues), _) => cues,
        (None, Some(cues_at)) => {
            let (id, start, size) = header_at(file, cues_at)?;
            if id != CUES {
                return None;
            }
            read_body(file, start, size?)?
        }
        (None, None) => return None, // no index (an interrupted recording)
    };
    let video_track = video_track?;
    if scale == 0 {
        return None;
    }

    let mut times: Vec<f64> = elements(&cues)
        .filter(|(id, _)| *id == CUE_POINT)
        .filter_map(|(_, point)| {
            let fields: Vec<_> = elements(point).collect();
            let time = fields
                .iter()
                .find(|(id, _)| *id == CUE_TIME)
                .map(|(_, v)| uint(v))?;
            let for_video = fields
                .iter()
                .filter(|(id, _)| *id == CUE_TRACK_POSITIONS)
                .any(|(_, positions)| {
                    elements(positions).any(|(id, v)| id == CUE_TRACK && uint(v) == video_track)
                });
            for_video.then(|| time as f64 * scale as f64 / 1e9)
        })
        .collect();
    if times.is_empty() {
        return None;
    }
    times.sort_by(f64::total_cmp);
    times.dedup();
    Some(Keyframes::At(times))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ebml_integers() {
        assert_eq!(vint(&[0x81], false), Some((1, 1)));
        assert_eq!(vint(&[0x40, 0x02], false), Some((2, 2)));
        assert_eq!(
            vint(&[0x1A, 0x45, 0xDF, 0xA3], true),
            Some((0x1A45_DFA3, 4))
        );
        assert_eq!(vint(&[0x00], false), None);
    }

    /// Compares the index with ffprobe's packet scan for every video in a folder. Opt-in:
    /// `TRIMMY_TEST_CLIPS=<folder> TRIMMY_FFPROBE=<ffprobe> cargo test -- --ignored`
    #[test]
    #[ignore]
    fn index_matches_the_packet_scan() {
        let (Ok(dir), Ok(ffprobe)) = (
            std::env::var("TRIMMY_TEST_CLIPS"),
            std::env::var("TRIMMY_FFPROBE"),
        ) else {
            panic!("set TRIMMY_TEST_CLIPS and TRIMMY_FFPROBE");
        };
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path().to_string_lossy().into_owned();
            let Some(found) = read(&path) else {
                println!("{path}: no index (packet scan)");
                continue;
            };
            let output = std::process::Command::new(&ffprobe)
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "V:0",
                    "-show_entries",
                    "packet=pts_time,flags",
                    "-of",
                    "csv=p=0",
                ])
                .arg(&path)
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&output.stdout);
            let mut all = 0;
            let mut scanned: Vec<f64> = text
                .lines()
                .inspect(|_| all += 1)
                .filter(|line| line.contains('K'))
                .filter_map(|line| line.split(',').next()?.parse().ok())
                .collect();
            scanned.sort_by(f64::total_cmp);
            match found {
                Keyframes::All => assert_eq!(
                    scanned.len(),
                    all,
                    "{path}: index says every frame is a keyframe"
                ),
                Keyframes::At(times) => {
                    assert!(!times.is_empty(), "{path}");
                    // Every indexed keyframe is a real one (an index may list fewer than all).
                    for t in &times {
                        assert!(
                            scanned.iter().any(|k| (k - t).abs() < 0.0015),
                            "{path}: {t} is not a keyframe; scan found {scanned:?}"
                        );
                    }
                    println!(
                        "{path}: {} of {} keyframes indexed",
                        times.len(),
                        scanned.len()
                    );
                }
            }
            checked += 1;
        }
        assert!(checked > 0);
    }

    #[test]
    fn walks_mp4_boxes() {
        let data = [
            0, 0, 0, 12, b'f', b'r', b'e', b'e', 1, 2, 3, 4, //
            0, 0, 0, 9, b'm', b'o', b'o', b'v', 7,
        ];
        let found: Vec<_> = boxes(&data)
            .map(|(kind, body)| (kind, body.len()))
            .collect();
        assert_eq!(found, [(*b"free", 4), (*b"moov", 1)]);
        assert_eq!(child(&data, b"moov"), Some(&[7u8][..]));
    }
}
