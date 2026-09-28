//! Turns `ffprobe -print_format json -show_format -show_streams` output into what the UI shows.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// What Trimmy knows about an opened video.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    pub path: String,
    pub name: String,
    /// Seconds; 0 when the container doesn't say.
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    /// 0 when unknown.
    pub fps: f64,
    /// FFmpeg's codec name, e.g. "h264" or "hevc".
    pub video_codec: String,
    pub audio_tracks: usize,
}

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
    format: Option<Format>,
}

#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    #[serde(default)]
    disposition: Disposition,
    #[serde(default)]
    side_data_list: Vec<SideData>,
}

#[derive(Deserialize)]
struct SideData {
    /// Degrees the player turns the picture (phones film portrait as rotated landscape).
    rotation: Option<f64>,
}

#[derive(Deserialize, Default)]
struct Disposition {
    #[serde(default)]
    attached_pic: u8,
}

#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
}

pub fn parse(path: &str, json: &[u8]) -> Result<VideoInfo, String> {
    let probe: Probe =
        serde_json::from_slice(json).map_err(|e| format!("Couldn't read ffprobe's output: {e}"))?;
    let is = |stream: &Stream, kind: &str| stream.codec_type.as_deref() == Some(kind);
    // Album art in audio files shows up as a one-frame video stream, so skip it.
    let video = probe
        .streams
        .iter()
        .find(|s| is(s, "video") && s.disposition.attached_pic == 0)
        .ok_or("That file has no video in it.")?;
    let (width, height) = (video.width.unwrap_or(0), video.height.unwrap_or(0));
    // Report the size as it's shown: a quarter turn swaps width and height.
    let turned = video
        .side_data_list
        .iter()
        .filter_map(|side| side.rotation)
        .any(|degrees| (degrees.abs() % 180.0 - 90.0).abs() < 1.0);
    let (width, height) = if turned {
        (height, width)
    } else {
        (width, height)
    };
    Ok(VideoInfo {
        path: path.to_owned(),
        name: Path::new(path).file_name().map_or_else(
            || path.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        ),
        duration: probe
            .format
            .and_then(|f| f.duration)
            .and_then(|d| d.parse().ok())
            .unwrap_or(0.0),
        width,
        height,
        fps: video.avg_frame_rate.as_deref().map_or(0.0, parse_rate),
        video_codec: video.codec_name.clone().unwrap_or_default(),
        audio_tracks: probe.streams.iter().filter(|s| is(s, "audio")).count(),
    })
}

/// Turns an FFmpeg rate like "30000/1001" into frames per second (0 when unknown).
fn parse_rate(rate: &str) -> f64 {
    let (num, den) = rate.split_once('/').unwrap_or((rate, "1"));
    match (num.parse::<f64>(), den.parse::<f64>()) {
        (Ok(num), Ok(den)) if den != 0.0 => num / den,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_game_clip() {
        let json = br#"{
            "streams": [
                {"codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080,
                 "avg_frame_rate": "224955/7493", "disposition": {"default": 1, "attached_pic": 0}},
                {"codec_type": "audio", "codec_name": "aac"},
                {"codec_type": "audio", "codec_name": "aac"}
            ],
            "format": {"duration": "14.997313"}
        }"#;
        let info = parse("C:/Videos/WARDOGS.DVR.mp4", json).unwrap();
        assert_eq!(info.name, "WARDOGS.DVR.mp4");
        assert_eq!((info.width, info.height), (1920, 1080));
        assert_eq!(info.video_codec, "h264");
        assert_eq!(info.audio_tracks, 2);
        assert!((info.fps - 30.02).abs() < 0.01);
        assert!((info.duration - 14.997).abs() < 0.001);
    }

    #[test]
    fn rejects_audio_with_album_art() {
        let json = br#"{
            "streams": [
                {"codec_type": "audio", "codec_name": "mp3"},
                {"codec_type": "video", "codec_name": "mjpeg", "disposition": {"attached_pic": 1}}
            ],
            "format": {"duration": "180.0"}
        }"#;
        assert!(parse("C:/Music/song.mp3", json).is_err());
    }

    #[test]
    fn swaps_the_size_of_a_rotated_phone_video() {
        let json = br#"{
            "streams": [
                {"codec_type": "video", "codec_name": "hevc", "width": 1920, "height": 1080,
                 "side_data_list": [{"side_data_type": "Display Matrix", "rotation": -90}]}
            ],
            "format": {"duration": "3.0"}
        }"#;
        let info = parse("/Users/me/IMG_0001.MOV", json).unwrap();
        assert_eq!((info.width, info.height), (1080, 1920));
    }

    #[test]
    fn parses_frame_rates() {
        assert_eq!(parse_rate("60/1"), 60.0);
        assert!((parse_rate("30000/1001") - 29.97).abs() < 0.01);
        assert_eq!(parse_rate("0/0"), 0.0);
        assert_eq!(parse_rate("25"), 25.0);
    }
}
