// Downloads the FFmpeg build that Trimmy bundles as Tauri sidecars into src-tauri/bin/.
// Runs automatically before every `npm run tauri ...` (the "pretauri" script) and exits
// immediately once the pinned version is already in place.
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

// Pinned release from https://github.com/GyanD/codexffmpeg ("essentials" GPL build,
// which includes the NVENC / AMF / QSV hardware encoders).
const VERSION = '9.0.2'
const BUILDS = {
  'win32-x64': {
    target: 'x86_64-pc-windows-msvc',
    url: `https://github.com/GyanD/codexffmpeg/releases/download/${VERSION}/ffmpeg-${VERSION}-essentials_build.zip`,
    sha256: '60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba',
    dirInZip: `ffmpeg-${VERSION}-essentials_build`,
    exe: '.exe',
  },
}
const TOOLS = ['ffmpeg', 'ffprobe']

const platform = `${process.platform}-${process.arch}`
const build = BUILDS[platform]
if (!build) {
  console.error(`fetch-ffmpeg: no pinned FFmpeg build for ${platform} yet (add one in scripts/fetch-ffmpeg.mjs).`)
  process.exit(1)
}

const outDir = join(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'bin')
const stampFile = join(outDir, '.ffmpeg-version')
// Tauri looks sidecars up as <name>-<target triple><ext>.
const sidecar = (tool) => join(outDir, `${tool}-${build.target}${build.exe}`)
// FFmpeg's GPL license ships inside the installer next to the binaries.
const license = join(outDir, 'FFmpeg-LICENSE.txt')

const installed = existsSync(stampFile) ? readFileSync(stampFile, 'utf8').trim() : null
if (installed === VERSION && existsSync(license) && TOOLS.every((tool) => existsSync(sidecar(tool)))) process.exit(0)

console.log(`fetch-ffmpeg: downloading FFmpeg ${VERSION}...`)
const res = await fetch(build.url)
if (!res.ok) throw new Error(`download failed: ${res.status} ${res.statusText} (${build.url})`)
const zip = Buffer.from(await res.arrayBuffer())

const actual = createHash('sha256').update(zip).digest('hex')
if (actual !== build.sha256) throw new Error(`checksum mismatch for ${build.url}: got ${actual}`)

const work = join(outDir, '.download')
rmSync(work, { recursive: true, force: true })
mkdirSync(work, { recursive: true })
try {
  const zipPath = join(work, 'ffmpeg.zip')
  writeFileSync(zipPath, zip)
  // Windows' built-in bsdtar reads zip files; call it by path so a GNU tar on PATH can't shadow it.
  const tar = join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe')
  const members = [...TOOLS.map((tool) => `${build.dirInZip}/bin/${tool}${build.exe}`), `${build.dirInZip}/LICENSE`]
  execFileSync(tar, ['-xf', zipPath, '-C', work, ...members], { stdio: 'inherit' })
  for (const tool of TOOLS) copyFileSync(join(work, build.dirInZip, 'bin', tool + build.exe), sidecar(tool))
  copyFileSync(join(work, build.dirInZip, 'LICENSE'), license)
  writeFileSync(stampFile, VERSION + '\n')
} finally {
  rmSync(work, { recursive: true, force: true })
}
console.log(`fetch-ffmpeg: FFmpeg ${VERSION} is ready in src-tauri/bin/`)
