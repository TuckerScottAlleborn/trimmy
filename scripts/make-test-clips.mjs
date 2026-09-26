// Builds Trimmy's test clip set from one real recording (a ShadowPlay / ReLive / OBS clip):
//   npm run test-clips -- "C:\path\to\clip.mp4" ["C:\path\to\long-audio.mp3"]
// Writes test-clips/ (gitignored): the untouched original plus one file per format case. With a
// long audio file (a DJ mix, a podcast), also builds a long video: the clip looped underneath it.
import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync } from 'node:fs'
import { dirname, extname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const binDir = join(root, 'src-tauri', 'bin')
const outDir = join(root, 'test-clips')

const [source, longAudio] = process.argv.slice(2)
if (!source || !existsSync(source) || (longAudio && !existsSync(longAudio))) {
  console.error('usage: npm run test-clips -- <source video> [<long audio file>]')
  process.exit(1)
}
const ffmpegFile = existsSync(binDir) && readdirSync(binDir).find((f) => f.startsWith('ffmpeg-'))
if (!ffmpegFile) {
  console.error('make-test-clips: FFmpeg is missing; run `node scripts/fetch-ffmpeg.mjs` first.')
  process.exit(1)
}
const ffmpeg = join(binDir, ffmpegFile)
const ffprobe = join(binDir, ffmpegFile.replace('ffmpeg', 'ffprobe'))
const out = (file) => join(outDir, file)
const encode = (args) => execFileSync(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-y', ...args], { stdio: 'inherit' })

const aac = ['-c:a', 'aac', '-b:a', '160k']
const copyFrom = (file) => ['-i', out(file), '-c', 'copy']

// Encodes run straight from the source. Container variants stream-copy h264-aac.mp4 /
// hevc-aac.mp4, so each name stays true whatever the source was.
const CASES = [
  // x264 defaults put a keyframe every 250 frames, like OBS: the worst case for fast lossless cuts.
  ['h264-aac.mp4', 'H.264 + AAC, the most common recording format', ['-i', source, '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '20', ...aac]],
  ['hevc-aac.mp4', 'HEVC, the ShadowPlay / ReLive "H.265" option', ['-i', source, '-c:v', 'libx265', '-preset', 'fast', '-crf', '24', '-tag:v', 'hvc1', ...aac]],
  ['av1-aac.mp4', 'AV1, recorded by RTX 40 / RX 7000 cards', ['-i', source, '-c:v', 'libaom-av1', '-usage', 'realtime', '-cpu-used', '8', '-row-mt', '1', '-crf', '32', '-b:v', '0', ...aac]],
  ['vp9-opus.webm', 'VP9 + Opus WebM, common for web downloads', ['-i', source, '-c:v', 'libvpx-vp9', '-deadline', 'realtime', '-cpu-used', '8', '-row-mt', '1', '-b:v', '6M', '-c:a', 'libopus', '-b:a', '128k']],
  ['mpeg4-mp3.avi', 'MPEG-4 Part 2 + MP3 AVI, the legacy case', ['-i', source, '-c:v', 'mpeg4', '-q:v', '3', '-c:a', 'libmp3lame', '-q:a', '2']],
  ['h264-aac.mkv', 'MKV, the OBS default container', copyFrom('h264-aac.mp4')],
  ['h264-aac.mov', 'MOV (QuickTime)', copyFrom('h264-aac.mp4')],
  ['h264-aac.ts', 'MPEG-TS, the other OBS recording container', copyFrom('h264-aac.mp4')],
  ['hevc-aac.mov', 'HEVC in MOV, how iPhones record', [...copyFrom('hevc-aac.mp4'), '-tag:v', 'hvc1']],
  ['h264-2audio.mp4', 'separate game + mic tracks, a ShadowPlay option', [
    '-i', out('h264-aac.mp4'), '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000,volume=0.2',
    '-map', '0:v', '-map', '0:a', '-map', '1:a', '-shortest', '-c:v', 'copy', '-c:a:0', 'copy',
    '-c:a:1', 'aac', '-ac:a:1', '1', '-metadata:s:a:0', 'title=Game', '-metadata:s:a:1', 'title=Mic',
  ]],
  ['h264-noaudio.mp4', 'no audio track at all', ['-i', out('h264-aac.mp4'), '-map', '0:v', '-c', 'copy']],
]

mkdirSync(outDir, { recursive: true })
const original = `original${extname(source).toLowerCase()}`
copyFileSync(source, out(original))
console.log(`${original.padEnd(18)} untouched copy of the source`)

function step(file, why, make) {
  const started = Date.now()
  make()
  console.log(`${file.padEnd(18)} ${why} (${((Date.now() - started) / 1000).toFixed(1)}s)`)
}

for (const [file, why, args] of CASES) step(file, why, () => encode([...args, out(file)]))

if (longAudio) {
  const seconds = execFileSync(ffprobe, ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', longAudio])
    .toString()
    .trim()
  step('long-h264-aac.mp4', `a ${Math.round(seconds / 60)}-minute recording`, () => {
    // Stream-copy loops of a short 720p segment: fast, and only ~7 MB a minute on disk.
    // The audio's duration (-t) is what ends the otherwise endless loop.
    const segment = out('.long-segment.mp4')
    encode(['-i', source, '-an', '-vf', 'scale=-2:720', '-c:v', 'libx264', '-preset', 'veryfast', '-b:v', '800k', segment])
    encode([
      '-stream_loop', '-1', '-i', segment, '-i', longAudio, '-map', '0:v', '-map', '1:a', '-t', seconds,
      '-c:v', 'copy', '-c:a', 'aac', '-b:a', '128k', out('long-h264-aac.mp4'),
    ])
    rmSync(segment)
  })
}
console.log(`make-test-clips: done, see test-clips/`)
