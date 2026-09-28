// Checks a slim FFmpeg build against everything Trimmy asks of it (see scripts/ffmpeg-configure.sh).
//
//   node scripts/ffmpeg-smoke.mjs --full <full ffmpeg> --full-probe <full ffprobe> \
//     --ffmpeg <slim ffmpeg> --ffprobe <slim ffprobe> [--work <dir>]
//
// The full build (the one fetch-ffmpeg.mjs used to bundle) makes short test clips in every
// container Trimmy opens, since the slim build can't encode video. Then, for each clip, the slim
// build must: probe it the same way the full one does, list its keyframes, stream-copy a cut
// into the same container, and compute the waveform. Exits 1 if anything fails.
import { execFileSync, spawnSync } from 'node:child_process'
import { mkdirSync, renameSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { parseArgs } from 'node:util'

const { values: opt } = parseArgs({
  options: {
    full: { type: 'string' },
    'full-probe': { type: 'string' },
    ffmpeg: { type: 'string' },
    ffprobe: { type: 'string' },
    work: { type: 'string', default: join(tmpdir(), 'trimmy-ffmpeg-smoke') },
  },
})
for (const k of ['full', 'full-probe', 'ffmpeg', 'ffprobe']) {
  if (!opt[k]) throw new Error(`missing --${k}`)
}
rmSync(opt.work, { recursive: true, force: true })
mkdirSync(opt.work, { recursive: true })

const run = (exe, args) => spawnSync(exe, args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })

// 4 s of moving test pattern with a keyframe every second and B-frames where the codec has them,
// plus a tone, so the waveform has something to measure.
const VIDEO = ['-f', 'lavfi', '-i', 'testsrc2=size=320x240:rate=30:duration=4']
const TONE = ['-f', 'lavfi', '-i', 'sine=frequency=440:duration=4']
const TONE2 = ['-f', 'lavfi', '-i', 'sine=frequency=880:duration=4']
const H264 = ['-c:v', 'libx264', '-preset', 'veryfast', '-g', '30', '-bf', '2', '-pix_fmt', 'yuv420p']

const CLIPS = [
  { name: 'h264-aac.mp4', args: [...VIDEO, ...TONE, ...H264, '-c:a', 'aac'] },
  { name: 'h264-aac.mov', args: [...VIDEO, ...TONE, ...H264, '-c:a', 'aac'] },
  { name: 'h264-aac.m4v', args: [...VIDEO, ...TONE, ...H264, '-c:a', 'aac'] },
  { name: 'h264-aac.mkv', args: [...VIDEO, ...TONE, ...H264, '-c:a', 'aac'] },
  { name: 'h264-ac3.mkv', args: [...VIDEO, ...TONE, ...H264, '-c:a', 'ac3'] },
  { name: 'h264-aac.ts', args: [...VIDEO, ...TONE, ...H264, '-c:a', 'aac'] },
  { name: 'h264-aac.m2ts', args: [...VIDEO, ...TONE, ...H264, '-c:a', 'aac'] },
  { name: 'h264-aac.flv', args: [...VIDEO, ...TONE, ...H264, '-c:a', 'aac'] },
  { name: 'h264-aac.3gp', args: [...VIDEO, ...TONE, ...H264, '-c:a', 'aac'] },
  { name: 'h264-pcm.mov', args: [...VIDEO, ...TONE, ...H264, '-c:a', 'pcm_s16le'] },
  { name: 'h264-2audio.mp4', args: [...VIDEO, ...TONE, ...TONE2, '-map', '0', '-map', '1', '-map', '2', ...H264, '-c:a', 'aac'] },
  { name: 'h264-noaudio.mp4', args: [...VIDEO, ...H264], noAudio: true },
  // Phones film portrait as landscape plus a rotation flag; -display_rotation only sticks on a copy.
  { name: 'h264-rotated.mp4', args: [...VIDEO, ...TONE, ...H264, '-c:a', 'aac'], rotate: 90 },
  { name: 'hevc-aac.mp4', args: [...VIDEO, ...TONE, '-c:v', 'libx265', '-preset', 'ultrafast', '-x265-params', 'keyint=30:log-level=error', '-tag:v', 'hvc1', '-c:a', 'aac'] },
  { name: 'vp9-opus.webm', args: [...VIDEO, ...TONE, '-c:v', 'libvpx-vp9', '-deadline', 'realtime', '-g', '30', '-c:a', 'libopus'] },
  { name: 'av1-opus.mkv', args: [...VIDEO, ...TONE, '-c:v', 'libaom-av1', '-cpu-used', '8', '-g', '30', '-c:a', 'libopus'], optional: true },
  { name: 'mpeg4-mp3.avi', args: [...VIDEO, ...TONE, '-c:v', 'mpeg4', '-g', '30', '-c:a', 'libmp3lame'] },
  { name: 'wmv2-wma.wmv', args: [...VIDEO, ...TONE, '-c:v', 'wmv2', '-g', '30', '-c:a', 'wmav2'] },
  { name: 'mpeg2-mp2.mpg', args: [...VIDEO, ...TONE, '-c:v', 'mpeg2video', '-g', '30', '-bf', '2', '-c:a', 'mp2'] },
]

const WAVEFORM =
  'aformat=channel_layouts=mono,aresample=8000,asetnsamples=n=800:p=0,' +
  'astats=metadata=1:reset=1:measure_perchannel=none:measure_overall=Peak_level,' +
  'ametadata=mode=print:key=lavfi.astats.Overall.Peak_level:file=-:direct=1'

const probe = (exe, file) => {
  const r = run(exe, ['-v', 'error', '-print_format', 'json', '-show_format', '-show_streams', file])
  if (r.status !== 0) throw new Error(`ffprobe failed: ${r.stderr.trim()}`)
  const json = JSON.parse(r.stdout)
  const video = json.streams.find((s) => s.codec_type === 'video' && !s.disposition?.attached_pic)
  return {
    codec: video?.codec_name,
    size: `${video?.width}x${video?.height}`,
    fps: video?.avg_frame_rate,
    rotation: video?.side_data_list?.find((d) => d.rotation !== undefined)?.rotation ?? 0,
    audio: json.streams.filter((s) => s.codec_type === 'audio').length,
    duration: Number(json.format.duration),
  }
}

const keyframes = (exe, file) => {
  const r = run(exe, ['-v', 'error', '-select_streams', 'V:0', '-show_entries', 'packet=pts_time,flags', '-of', 'compact', file])
  if (r.status !== 0) throw new Error(`keyframe scan failed: ${r.stderr.trim()}`)
  return r.stdout.split('\n').filter((l) => /flags=K/.test(l)).length
}

let failed = 0
const rows = []
for (const clip of CLIPS) {
  const file = join(opt.work, clip.name)
  const made = run(opt.full, ['-v', 'error', '-y', ...clip.args, '-t', '4', file])
  if (made.status !== 0) {
    if (clip.optional) {
      rows.push([clip.name, 'skipped (full build lacks the encoder)'])
      continue
    }
    throw new Error(`couldn't make ${clip.name} with the full build: ${made.stderr.trim()}`)
  }
  if (clip.rotate) {
    const plain = file.replace(/(\.[^.]+)$/, '-plain$1')
    rmSync(plain, { force: true })
    renameSync(file, plain)
    const turned = run(opt.full, ['-v', 'error', '-y', '-display_rotation', String(clip.rotate), '-i', plain, '-c', 'copy', file])
    if (turned.status !== 0) throw new Error(`couldn't rotate ${clip.name}: ${turned.stderr.trim()}`)
  }
  const problems = []
  try {
    // 1. Probe: the slim build must report what the full one does.
    const full = probe(opt['full-probe'], file)
    const slim = probe(opt.ffprobe, file)
    for (const key of ['codec', 'size', 'fps', 'rotation', 'audio']) {
      if (full[key] !== slim[key]) problems.push(`${key}: slim ${slim[key]} vs full ${full[key]}`)
    }
    if (clip.rotate && full.rotation === 0) problems.push('test clip came out without a rotation')
    if (!(Math.abs(full.duration - slim.duration) < 0.05)) problems.push(`duration: slim ${slim.duration} vs full ${full.duration}`)

    // 2. Keyframes: same list length as the full build finds.
    const kf = keyframes(opt.ffprobe, file)
    const kfFull = keyframes(opt['full-probe'], file)
    if (kf < 1 || kf !== kfFull) problems.push(`keyframes: slim ${kf} vs full ${kfFull}`)

    // 3. Export: stream-copy a cut into the same container, like export.rs.
    const ext = clip.name.split('.').pop()
    const out = join(opt.work, `cut-${clip.name.replace(/\.[^.]+$/, '')}.${ext}`)
    const cut = run(opt.ffmpeg, [
      '-hide_banner', '-nostdin', '-loglevel', 'error', '-y', '-progress', 'pipe:1',
      '-ss', '1.2', '-i', file, '-t', '1.5',
      '-map', '0:V:0', '-map', '0:a?', '-c', 'copy', '-avoid_negative_ts', 'make_zero', '-map_metadata', '0', out,
    ])
    if (cut.status !== 0) problems.push(`export failed: ${cut.stderr.trim().split('\n').pop()}`)
    else {
      const got = probe(opt['full-probe'], out)
      if (got.codec !== full.codec) problems.push(`export video: ${got.codec}`)
      if (got.audio !== full.audio) problems.push(`export audio tracks: ${got.audio} of ${full.audio}`)
      if (!(got.duration > 1 && got.duration < 3)) problems.push(`export duration: ${got.duration}`)
      if (got.rotation !== full.rotation) problems.push(`export rotation: ${got.rotation}`)
    }

    // 4. Waveform: the filter chain from waveform.rs prints one peak per 0.1 s of audio.
    const wave = run(opt.ffmpeg, ['-hide_banner', '-nostdin', '-loglevel', 'error', '-i', file, '-map', '0:a:0', '-vn', '-af', WAVEFORM, '-f', 'null', '-'])
    const peaks = (wave.stdout.match(/Peak_level=/g) ?? []).length
    if (clip.noAudio) {
      if (wave.status === 0) problems.push('waveform: expected a failure with no audio track')
    } else if (wave.status !== 0 || peaks < 30) {
      problems.push(`waveform: ${peaks} peaks, ${wave.stderr.trim().split('\n').pop() || 'exit ' + wave.status}`)
    }

    // 5. Sampled waveform, as waveform.rs does for big files: three -ss/-t inputs, each padded or
    // cut to exactly 0.25 s, joined, one peak per input.
    if (!clip.noAudio) {
      const starts = [0.2, 1.6, 3.9]
      const inputs = starts.flatMap((t) => ['-ss', String(t), '-t', '0.25', '-i', file])
      const graph =
        starts.map((_, i) => `[${i}:a:0]aformat=channel_layouts=mono,aresample=8000,apad=whole_len=2000,atrim=end_sample=2000[s${i}];`).join('') +
        starts.map((_, i) => `[s${i}]`).join('') +
        `concat=n=${starts.length}:v=0:a=1,asetnsamples=n=2000:p=0,` +
        WAVEFORM.slice(WAVEFORM.indexOf('astats')) +
        '[peaks]'
      const sampled = run(opt.ffmpeg, ['-hide_banner', '-nostdin', '-loglevel', 'error', ...inputs, '-filter_complex', graph, '-map', '[peaks]', '-f', 'null', '-'])
      const got = (sampled.stdout.match(/Peak_level=/g) ?? []).length
      if (sampled.status !== 0 || got !== starts.length) {
        problems.push(`sampled waveform: ${got} of ${starts.length} peaks, ${sampled.stderr.trim().split('\n').pop() || 'exit ' + sampled.status}`)
      }
    }
  } catch (e) {
    problems.push(String(e.message ?? e))
  }
  if (problems.length) failed++
  rows.push([clip.name, problems.length ? `FAIL  ${problems.join('; ')}` : 'ok'])
}

const width = Math.max(...rows.map(([name]) => name.length))
for (const [name, result] of rows) console.log(`${name.padEnd(width)}  ${result}`)
const version = execFileSync(opt.ffmpeg, ['-hide_banner', '-version'], { encoding: 'utf8' }).split('\n')[0]
console.log(`\n${version}\n${failed ? `${failed} clip(s) FAILED` : 'all clips passed'}`)
process.exit(failed ? 1 : 0)
