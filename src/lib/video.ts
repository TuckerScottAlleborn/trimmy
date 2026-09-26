import { invoke } from '@tauri-apps/api/core'

/** What the `open_video` command reports (see src-tauri/src/probe.rs). */
export interface VideoInfo {
  path: string
  name: string
  duration: number
  width: number
  height: number
  fps: number
  videoCodec: string
  audioTracks: number
}

export const openVideo = (path: string) => invoke<VideoInfo>('open_video', { path })

/** Saves `start..end` (seconds) as a new file next to the original; resolves to its path. */
export const exportClip = (path: string, start: number, end: number) =>
  invoke<string>('export_clip', { path, start, end })

/** How many bars the timeline waveform has. */
const WAVEFORM_BARS = 240

/** Peak loudness per slice of the video (0 to 1), or [] when it has no audio. */
export const loadWaveform = (video: VideoInfo) =>
  invoke<number[]>('waveform', { path: video.path, duration: video.duration, buckets: WAVEFORM_BARS })

/** The file Trimmy was started with ("Open with"), if any. */
export const launchPath = () => invoke<string | null>('launch_path')

const CODEC_NAMES: Record<string, string> = {
  h264: 'H.264',
  hevc: 'HEVC',
  av1: 'AV1',
  vp9: 'VP9',
  vp8: 'VP8',
  mpeg4: 'MPEG-4',
}

export const codecName = (codec: string) => CODEC_NAMES[codec] ?? codec.toUpperCase()

/** "1920×1080 · 30 fps · H.264 · 1 audio track · 0:15.0" */
export function describe(video: VideoInfo): string {
  // Recorders drift a little around their nominal rate (ShadowPlay's "30 fps" averages 30.02).
  const fps = Math.abs(video.fps - Math.round(video.fps)) < 0.025 ? Math.round(video.fps) : video.fps.toFixed(2)
  const audio =
    video.audioTracks === 0 ? 'no audio' : video.audioTracks === 1 ? '1 audio track' : `${video.audioTracks} audio tracks`
  return [
    `${video.width}×${video.height}`,
    video.fps > 0 && `${fps} fps`,
    codecName(video.videoCodec),
    audio,
    formatTime(video.duration),
  ]
    .filter(Boolean)
    .join(' · ')
}

/** 75.25 -> "1:15.3", 3725 -> "1:02:05.0" */
export function formatTime(seconds: number): string {
  const tenths = Math.round(seconds * 10) // round first, so 59.96 becomes 1:00.0 rather than 0:60.0
  const hours = Math.floor(tenths / 36000)
  const minutes = Math.floor((tenths % 36000) / 600)
  const secs = ((tenths % 600) / 10).toFixed(1).padStart(4, '0')
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, '0')}:${secs}` : `${minutes}:${secs}`
}
