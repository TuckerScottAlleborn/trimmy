// Downloads the FFmpeg that Trimmy bundles as Tauri sidecars into src-tauri/bin/.
// Runs automatically before every `npm run tauri ...` (the "pretauri" script) and exits
// immediately once the pinned build is already in place. Every download is pinned by SHA-256.
//
//   node scripts/fetch-ffmpeg.mjs          the build Trimmy bundles (see BUNDLED below)
//   node scripts/fetch-ffmpeg.mjs --full   full general-purpose builds into src-tauri/bin/full/,
//                                          for the tools that need encoders: make-test-clips.mjs
//                                          and the FFmpeg workflow's smoke test
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const VERSION = '9.0.2'

// ---- Slim: Trimmy's own LGPL build with only what it uses (scripts/ffmpeg-configure.sh), made by
// .github/workflows/ffmpeg.yml and published as a pre-release of this repository.
const SLIM_BUILD = `${VERSION}-trimmy.1`
const SLIM_URL = `https://github.com/TuckerScottAlleborn/trimmy/releases/download/ffmpeg-${SLIM_BUILD}`
const SLIM = {
  stamp: SLIM_BUILD,
  windows: [
    {
      url: `${SLIM_URL}/ffmpeg-windows-x86_64.zip`,
      sha256: 'SLIM_WINDOWS_SHA256',
      unzip: {
        'ffmpeg.exe': 'ffmpeg-x86_64-pc-windows-msvc.exe',
        'ffprobe.exe': 'ffprobe-x86_64-pc-windows-msvc.exe',
        'LICENSE.txt': 'FFmpeg-LICENSE.txt',
      },
    },
  ],
  // Already universal (Apple Silicon + Intel), so the per-CPU names are copies of the same file.
  macos: [
    {
      url: `${SLIM_URL}/ffmpeg-macos-universal.zip`,
      sha256: 'SLIM_MACOS_SHA256',
      unzip: {
        ffmpeg: 'ffmpeg-universal-apple-darwin',
        ffprobe: 'ffprobe-universal-apple-darwin',
        'LICENSE.txt': 'FFmpeg-LICENSE.txt',
      },
      copies: {
        'ffmpeg-universal-apple-darwin': ['ffmpeg-aarch64-apple-darwin', 'ffmpeg-x86_64-apple-darwin'],
        'ffprobe-universal-apple-darwin': ['ffprobe-aarch64-apple-darwin', 'ffprobe-x86_64-apple-darwin'],
      },
    },
  ],
}

// ---- Full: general-purpose GPL builds with every encoder. Trimmy bundled these up to 1.0.
// Windows: gyan.dev's "essentials" build (https://github.com/GyanD/codexffmpeg).
const GYAN = `ffmpeg-${VERSION}-essentials_build`
// macOS: Martin Riedl's static, signed builds (https://ffmpeg.martin-riedl.de), one zip per tool
// and CPU, joined into universal binaries with lipo.
const riedl = (arch, build, triple, tool, sha256) => ({
  url: `https://ffmpeg.martin-riedl.de/download/macos/${arch}/${build}_${VERSION}/${tool}.zip`,
  sha256,
  unzip: { [tool]: `${tool}-${triple}` },
})
const FULL = {
  stamp: VERSION,
  windows: [
    {
      url: `https://github.com/GyanD/codexffmpeg/releases/download/${VERSION}/${GYAN}.zip`,
      sha256: '60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba',
      unzip: {
        [`${GYAN}/bin/ffmpeg.exe`]: 'ffmpeg-x86_64-pc-windows-msvc.exe',
        [`${GYAN}/bin/ffprobe.exe`]: 'ffprobe-x86_64-pc-windows-msvc.exe',
        [`${GYAN}/LICENSE`]: 'FFmpeg-LICENSE.txt',
      },
    },
  ],
  macos: [
    riedl('arm64', '1789931890', 'aarch64-apple-darwin', 'ffmpeg', 'c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924'),
    riedl('arm64', '1789931890', 'aarch64-apple-darwin', 'ffprobe', 'fcbe839537485eaee7a7a8bc5cbc0f90d53617e80943e8a5b2e31cb851197ea6'),
    riedl('amd64', '1789931006', 'x86_64-apple-darwin', 'ffmpeg', '7c6b4125b191cbf773832dc51f424cf2b6bb7da43007d1e066f95909e47cacd4'),
    riedl('amd64', '1789931006', 'x86_64-apple-darwin', 'ffprobe', '2322438ed2f6319a691291b247d09c69dcaa3a982460d1f269a7e1af335cfdfd'),
    // These builds don't ship a license file, so bundle FFmpeg's own GPLv3 text from the same release.
    {
      url: `https://raw.githubusercontent.com/FFmpeg/FFmpeg/n${VERSION}/COPYING.GPLv3`,
      sha256: '8ceb4b9ee5adedde47b31e975c1d90c73ad27b6b165a1dcd80c7c545eb65b903',
      save: 'FFmpeg-LICENSE.txt',
    },
  ],
  // Tauri's universal build looks for <tool>-universal-apple-darwin.
  lipo: ['ffmpeg', 'ffprobe'],
}

// What `npm run tauri ...` bundles.
const BUNDLED = FULL

const full = process.argv.includes('--full')
const build = full ? FULL : BUNDLED
const isMac = process.platform === 'darwin'
const downloads = isMac ? build.macos : process.platform === 'win32' && process.arch === 'x64' ? build.windows : null
if (!downloads) {
  console.error(`fetch-ffmpeg: no pinned FFmpeg build for ${process.platform}-${process.arch} yet (add one in scripts/fetch-ffmpeg.mjs).`)
  process.exit(1)
}

const binDir = join(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'bin')
const outDir = full ? join(binDir, 'full') : binDir
const stampFile = join(outDir, '.ffmpeg-version')
const lipo = isMac ? (build.lipo ?? []) : []
const outputs = [
  ...downloads.flatMap((d) => (d.save ? [d.save] : [...Object.values(d.unzip), ...Object.values(d.copies ?? {}).flat()])),
  ...lipo.map((tool) => `${tool}-universal-apple-darwin`),
]

const installed = existsSync(stampFile) ? readFileSync(stampFile, 'utf8').trim() : null
if (installed === build.stamp && outputs.every((file) => existsSync(join(outDir, file)))) process.exit(0)

const work = join(outDir, '.download')
rmSync(work, { recursive: true, force: true })
mkdirSync(work, { recursive: true })
try {
  for (const [i, download] of downloads.entries()) {
    console.log(`fetch-ffmpeg: downloading ${download.url}`)
    const res = await fetch(download.url)
    if (!res.ok) throw new Error(`download failed: ${res.status} ${res.statusText} (${download.url})`)
    const data = Buffer.from(await res.arrayBuffer())
    const actual = createHash('sha256').update(data).digest('hex')
    if (actual !== download.sha256) throw new Error(`checksum mismatch for ${download.url}: got ${actual}`)

    if (download.save) {
      writeFileSync(join(outDir, download.save), data)
      continue
    }
    const zipPath = join(work, `${i}.zip`)
    const into = join(work, String(i))
    mkdirSync(into)
    writeFileSync(zipPath, data)
    // bsdtar reads zip files: Windows ships it as System32\tar.exe (called by path so a GNU tar on
    // PATH can't shadow it) and it is the default tar on macOS.
    const tar = isMac ? '/usr/bin/tar' : join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe')
    execFileSync(tar, ['-xf', zipPath, '-C', into, ...Object.keys(download.unzip)], { stdio: 'inherit' })
    for (const [member, file] of Object.entries(download.unzip)) {
      copyFileSync(join(into, member), join(outDir, file))
      if (isMac) chmodSync(join(outDir, file), 0o755)
    }
    for (const [from, targets] of Object.entries(download.copies ?? {})) {
      for (const to of targets) {
        copyFileSync(join(outDir, from), join(outDir, to))
        if (isMac) chmodSync(join(outDir, to), 0o755)
      }
    }
  }
  for (const tool of lipo) {
    const [arm, intel, universal] = ['aarch64', 'x86_64', 'universal'].map((cpu) => join(outDir, `${tool}-${cpu}-apple-darwin`))
    execFileSync('lipo', ['-create', '-output', universal, arm, intel], { stdio: 'inherit' })
  }
  writeFileSync(stampFile, build.stamp + '\n')
} finally {
  rmSync(work, { recursive: true, force: true })
}
console.log(`fetch-ffmpeg: FFmpeg ${build.stamp} is ready in ${full ? 'src-tauri/bin/full/' : 'src-tauri/bin/'}`)
