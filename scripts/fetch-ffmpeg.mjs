// Downloads the FFmpeg build that Trimmy bundles as Tauri sidecars into src-tauri/bin/.
// Runs automatically before every `npm run tauri ...` (the "pretauri" script) and exits
// immediately once the pinned version is already in place. Every download is pinned by SHA-256.
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const VERSION = '9.0.2'

// Windows: gyan.dev's "essentials" GPL build (https://github.com/GyanD/codexffmpeg), which
// includes the NVENC / AMF / QSV hardware encoders.
const GYAN = `ffmpeg-${VERSION}-essentials_build`
const WINDOWS = [
  {
    url: `https://github.com/GyanD/codexffmpeg/releases/download/${VERSION}/${GYAN}.zip`,
    sha256: '60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba',
    unzip: {
      [`${GYAN}/bin/ffmpeg.exe`]: 'ffmpeg-x86_64-pc-windows-msvc.exe',
      [`${GYAN}/bin/ffprobe.exe`]: 'ffprobe-x86_64-pc-windows-msvc.exe',
      [`${GYAN}/LICENSE`]: 'FFmpeg-LICENSE.txt',
    },
  },
]

// macOS: Martin Riedl's static, signed builds (https://ffmpeg.martin-riedl.de), one zip per tool
// and CPU. Both CPUs are fetched so a universal (Apple Silicon + Intel) app can be built.
const riedl = (arch, build, triple, tool, sha256) => ({
  url: `https://ffmpeg.martin-riedl.de/download/macos/${arch}/${build}_${VERSION}/${tool}.zip`,
  sha256,
  unzip: { [tool]: `${tool}-${triple}` },
})
const MACOS = [
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
]
// Tauri's universal build looks for <tool>-universal-apple-darwin.
const MACOS_UNIVERSAL = ['ffmpeg', 'ffprobe']

const isMac = process.platform === 'darwin'
const downloads = isMac ? MACOS : process.platform === 'win32' && process.arch === 'x64' ? WINDOWS : null
if (!downloads) {
  console.error(`fetch-ffmpeg: no pinned FFmpeg build for ${process.platform}-${process.arch} yet (add one in scripts/fetch-ffmpeg.mjs).`)
  process.exit(1)
}

const outDir = join(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'bin')
const stampFile = join(outDir, '.ffmpeg-version')
const outputs = [
  ...downloads.flatMap((d) => (d.save ? [d.save] : Object.values(d.unzip))),
  ...(isMac ? MACOS_UNIVERSAL.map((tool) => `${tool}-universal-apple-darwin`) : []),
]

const installed = existsSync(stampFile) ? readFileSync(stampFile, 'utf8').trim() : null
if (installed === VERSION && outputs.every((file) => existsSync(join(outDir, file)))) process.exit(0)

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
      if (!isMac) continue
      chmodSync(join(outDir, file), 0o755)
    }
  }
  if (isMac) {
    for (const tool of MACOS_UNIVERSAL) {
      const [arm, intel, universal] = ['aarch64', 'x86_64', 'universal'].map((cpu) => join(outDir, `${tool}-${cpu}-apple-darwin`))
      execFileSync('lipo', ['-create', '-output', universal, arm, intel], { stdio: 'inherit' })
    }
  }
  writeFileSync(stampFile, VERSION + '\n')
} finally {
  rmSync(work, { recursive: true, force: true })
}
console.log(`fetch-ffmpeg: FFmpeg ${VERSION} is ready in src-tauri/bin/`)
