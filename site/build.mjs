// Builds trimmy.lol into site/dist/. Plain Node 20+, no dependencies:
//
//   node site/build.mjs            build from the latest published release
//   node site/build.mjs --draft    use the newest release even if it's still a draft (a local
//                                  preview: a draft's download links only work for you)
//   node site/build.mjs --serve    build, then serve site/dist on http://localhost:4173
//
// index.html and 404.html are templates: {{key}} is replaced HTML-escaped, {{{key}}} raw, and an
// <!--if:key-->...<!--/if:key--> block is dropped when key is empty. public/ is copied as-is.
// The build fails if a page links to a file that isn't there, if a declared image size is wrong,
// or if the page outgrows its budget.

import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { cp, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { createServer } from 'node:http'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { gzipSync } from 'node:zlib'

const SITE = 'https://trimmy.lol'
const REPO = 'TuckerScottAlleborn/trimmy'
const REPO_URL = `https://github.com/${REPO}`
// A donation page (Ko-fi, Buy Me a Coffee, GitHub Sponsors). The footer's "buy me a coffee" line
// appears once this is set.
const DONATE_URL = ''

// What search results and link previews say. This is the only place these texts live.
const TITLE = 'Trimmy: a comically simple video trimmer for Windows'
const DESCRIPTION =
  'Open a clip, drag the start and end, export. Trimmy trims ShadowPlay, ReLive and OBS clips in about a second with zero quality loss. Free for Windows.'
const OG_TITLE = 'Trimmy: a comically simple video trimmer'
const OG_DESCRIPTION =
  "Open a clip. Drag the start and end. Export. That's the whole app. Free, lossless and made for game clips."
const OG_IMAGE_ALT =
  'Glitch the pixel ghost above the word trimmy, the tagline "a comically simple video trimmer" and a green audio waveform between two trim handles.'

// index.html, gzipped, has to fit in the first round trip of a new connection (about 14 KB), so
// the browser can draw the page before anything else arrives.
const PAGE_BUDGET = 14 * 1024
// Everything a first visit on a desktop downloads: the page, both fonts, the screenshot, the icon.
const VISIT_BUDGET = 100 * 1024

const HERE = path.dirname(fileURLToPath(import.meta.url))
const DIST = path.join(HERE, 'dist')
const flags = new Set(process.argv.slice(2))

async function github(route) {
  const headers = { accept: 'application/vnd.github+json', 'user-agent': 'trimmy-site' }
  const token = process.env.GITHUB_TOKEN || ghToken()
  if (token) headers.authorization = `Bearer ${token}`
  const res = await fetch(`https://api.github.com/repos/${REPO}/${route}`, {
    headers,
    signal: AbortSignal.timeout(30_000),
  })
  if (res.status === 404) return null
  if (!res.ok) throw new Error(`GitHub API ${route}: ${res.status} ${await res.text()}`)
  return res.json()
}

// Locally, borrow the GitHub CLI's login (seeing drafts needs one). Without it the API still
// answers, just rate-limited.
function ghToken() {
  try {
    const options = { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }
    return execFileSync('gh', ['auth', 'token'], options).trim()
  } catch {
    return ''
  }
}

async function latestRelease() {
  if (!flags.has('--draft')) return github('releases/latest')
  const [newest] = (await github('releases?per_page=1')) ?? []
  return newest ?? null
}

/** What the page needs from a release. With none published yet, the buttons go to the releases page. */
function downloads(release) {
  if (!release) {
    return {
      version: '',
      released: '',
      winUrl: `${REPO_URL}/releases/latest`,
      winName: 'Trimmy_x.y.z_x64-setup.exe',
      winSize: '',
      msiUrl: '',
      macUrl: '',
      notesUrl: `${REPO_URL}/releases`,
    }
  }
  const asset = (pattern) => release.assets.find((a) => pattern.test(a.name))
  const win = asset(/_x64-setup\.exe$/)
  if (!win) throw new Error(`release ${release.tag_name} has no *_x64-setup.exe to offer`)
  return {
    version: release.tag_name.replace(/^v/, ''),
    released: (release.published_at ?? release.created_at).slice(0, 10),
    winUrl: win.browser_download_url,
    winName: win.name,
    winSize: `${Math.round(win.size / 2 ** 20)} MB`,
    msiUrl: asset(/_x64_en-US\.msi$/)?.browser_download_url ?? '',
    macUrl: asset(/_universal\.dmg$/)?.browser_download_url ?? '',
    notesUrl: release.html_url,
  }
}

function structuredData(d, ogImage) {
  const app = {
    '@type': 'SoftwareApplication',
    '@id': `${SITE}/#app`,
    name: 'Trimmy',
    url: `${SITE}/`,
    description: DESCRIPTION,
    applicationCategory: 'MultimediaApplication',
    applicationSubCategory: 'Video trimmer',
    operatingSystem: d.macUrl ? 'Windows 10, Windows 11, macOS 11 or later' : 'Windows 10, Windows 11',
    offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
    isAccessibleForFree: true,
    license: 'https://opensource.org/licenses/MIT',
    image: ogImage,
    screenshot: `${SITE}/img/screenshot-976.webp`,
    author: {
      '@type': 'Person',
      '@id': 'https://www.tuckerscottalleborn.com/#person',
      name: 'Tucker Scott Alleborn',
      url: 'https://www.tuckerscottalleborn.com/',
    },
    sameAs: [REPO_URL],
  }
  if (d.version) {
    Object.assign(app, {
      softwareVersion: d.version,
      downloadUrl: d.winUrl,
      fileSize: d.winSize.replace(' ', ''),
      releaseNotes: d.notesUrl,
    })
  }
  const website = {
    '@type': 'WebSite',
    '@id': `${SITE}/#website`,
    url: `${SITE}/`,
    name: 'Trimmy',
    inLanguage: 'en-US',
    about: { '@id': app['@id'] },
  }
  // With "<" escaped, nothing in the data can close the <script> it sits in.
  const graph = { '@context': 'https://schema.org', '@graph': [website, app] }
  return JSON.stringify(graph).replaceAll('<', '\\u003c')
}

function render(template, data) {
  const html = template
    .replace(/<!--if:(\w+)-->([\s\S]*?)<!--\/if:\1-->/g, (_, key, body) => (field(data, key) ? body : ''))
    .replace(/<!--(?!\/?if:)[\s\S]*?-->/g, '')
    .replace(/{{{(\w+)}}}/g, (_, key) => field(data, key))
    .replace(/{{(\w+)}}/g, (_, key) => escapeHtml(field(data, key)))
  if (/<!--\/?if:|{{/.test(html)) throw new Error('a template has a nested or unclosed if block, or a stray {{')
  return html
}

function field(data, key) {
  if (!(key in data)) throw new Error(`a template uses "${key}", which build.mjs doesn't provide`)
  return String(data[key])
}

function escapeHtml(text) {
  return text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;')
}

// Drops indentation and squeezes the CSS (render already dropped the comments). Safe because
// the pages have no <pre>.
function minify(html) {
  return html
    .replace(/<style>([\s\S]*?)<\/style>/g, (_, css) => `<style>${minifyCss(css)}</style>`)
    .replace(/^\s+/gm, '')
}

function minifyCss(css) {
  return css
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/\s+/g, ' ')
    .replace(/\s*([{}:;,])\s*/g, '$1')
    .replaceAll(';}', '}')
    .trim()
}

function llmsTxt(d) {
  const get = d.version
    ? `- Latest version: ${d.version}, released ${d.released}
- Download for Windows 10 and 11 (${d.winSize}): ${d.winUrl}
${d.macUrl ? `- macOS build (Apple Silicon and Intel, untested): ${d.macUrl}\n` : ''}`
    : ''
  return `# Trimmy

> A comically simple video trimmer for Windows: open one video, drag the start and end, export. Lossless and near-instant (an FFmpeg stream copy, no re-encoding). Free and open source (MIT). Made for game clips from NVIDIA ShadowPlay, AMD ReLive, OBS and Xbox Game Bar.

${get}- All releases: ${REPO_URL}/releases

## What it does

- Opens MP4, MKV, MOV, WebM, AVI, MPEG-TS, M4V, WMV, FLV and more, in H.264, HEVC, AV1, VP9 and others
- Shows the audio waveform on a timeline with start and end handles
- Exports in about a second with no quality loss, keeping every audio track
- Saves the clip next to the original as name_trimmed.ext and never overwrites anything
- Runs entirely on the PC: no account, no upload, no watermark

## What it doesn't do

Titles, transitions, filters, music, multi-clip timelines, projects or accounts. It only trims.

A stream copy starts on a keyframe, so a clip can begin slightly before the chosen start (about 1 s for ShadowPlay, up to several seconds with OBS's defaults). The end is exact.

## Links

- [Website](${SITE}/)
- [Source code and full documentation](${REPO_URL})
- [Report a bug](${REPO_URL}/issues)
`
}

function sitemap(lastmod) {
  return `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
<url><loc>${SITE}/</loc><lastmod>${lastmod}</lastmod></url>
</urlset>
`
}

// Everyone is welcome: search engines, link previews and AI crawlers alike.
const ROBOTS = `User-agent: *
Allow: /

Sitemap: ${SITE}/sitemap.xml
`

// When site/ last changed. In CI's shallow clone that's the commit being built, close enough.
function lastCommitDate() {
  try {
    const options = { cwd: HERE, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }
    return execFileSync('git', ['log', '-1', '--format=%cI', '--', '.'], options).trim().slice(0, 10)
  } catch {
    return ''
  }
}

async function isFile(file) {
  return (await stat(file).catch(() => null))?.isFile() ?? false
}

/** Width and height from a PNG or WebP file's header. */
function imageSize(bytes) {
  if (bytes.toString('ascii', 1, 4) === 'PNG') return [bytes.readUInt32BE(16), bytes.readUInt32BE(20)]
  const chunk = bytes.toString('ascii', 12, 16)
  if (chunk === 'VP8X') return [1 + bytes.readUIntLE(24, 3), 1 + bytes.readUIntLE(27, 3)]
  if (chunk === 'VP8L') {
    const bits = bytes.readUInt32LE(21)
    return [1 + (bits & 0x3fff), 1 + ((bits >> 14) & 0x3fff)]
  }
  return [bytes.readUInt16LE(26) & 0x3fff, bytes.readUInt16LE(28) & 0x3fff]
}

async function check(html) {
  const problems = []
  const refs = [
    ...[...html.matchAll(/(?:src|href)="([^"]+)"/g)].map((m) => m[1]),
    ...[...html.matchAll(/srcset="([^"]+)"/g)].flatMap((m) => m[1].split(',').map((c) => c.trim().split(' ')[0])),
    ...[...html.matchAll(/url\(([^)]+)\)/g)].map((m) => m[1]),
  ]
  for (const ref of new Set(refs)) {
    if (/^(?:[a-z]+:|#|\/$)/.test(ref)) continue
    if (!(await isFile(path.join(DIST, ref)))) problems.push(`links to ${ref}, which isn't in the build`)
  }

  // Link previews crop by the declared size, and the layout reserves space by it.
  const images = [...html.matchAll(/<img src="([^"]+)"[^>]*? width="(\d+)" height="(\d+)"/g)].map((m) => m.slice(1))
  const og = html.match(/property="og:image" content="([^"]+)"/)?.[1]
  if (og) {
    const width = html.match(/property="og:image:width" content="(\d+)"/)?.[1]
    const height = html.match(/property="og:image:height" content="(\d+)"/)?.[1]
    images.push([new URL(og).pathname, width, height])
  }
  for (const [src, width, height] of images) {
    const file = path.join(DIST, src)
    if (!(await isFile(file))) continue // reported above
    const [realWidth, realHeight] = imageSize(await readFile(file))
    if (realWidth !== Number(width) || realHeight !== Number(height)) {
      problems.push(`says ${src} is ${width}x${height}, but it's ${realWidth}x${realHeight}`)
    }
  }
  // WhatsApp skips a preview whose image is much over 300 KB.
  const preview = og && path.join(DIST, new URL(og).pathname)
  if (preview && (await isFile(preview)) && (await stat(preview)).size > 300 * 1024) {
    problems.push('has a link-preview image over 300 KB, too big for WhatsApp')
  }

  const title = html.match(/<title>([^<]*)<\/title>/)?.[1] ?? ''
  if (!title || title.length > 60) problems.push(`needs a title of 1-60 characters (it's ${title.length})`)
  const description = html.match(/name="description" content="([^"]*)"/)?.[1]
  if (description && description.length > 160) problems.push(`has a description over 160 characters`)
  if ((html.match(/<h1[\s>]/g) ?? []).length !== 1) problems.push('needs exactly one <h1>')
  return problems
}

const kb = (bytes) => `${(bytes / 1024).toFixed(1)} KB`

async function report(page) {
  const pageGz = gzipSync(page, { level: 9 }).length
  const icon = gzipSync(await readFile(path.join(DIST, 'icon.svg'))).length
  const others = ['fonts/jetbrains-mono-400.woff2', 'fonts/jetbrains-mono-800.woff2', 'img/screenshot-976.avif']
  const sizes = await Promise.all(others.map(async (file) => (await stat(path.join(DIST, file))).size))
  const visit = pageGz + icon + sizes.reduce((sum, size) => sum + size, 0)
  console.log(`  index.html   ${kb(page.length)}, ${kb(pageGz)} gzipped (budget ${kb(PAGE_BUDGET)})`)
  console.log(`  first visit  ${kb(visit)}: page, fonts, screenshot, icon (budget ${kb(VISIT_BUDGET)})`)
  const over = []
  if (pageGz > PAGE_BUDGET) over.push(`index.html is ${kb(pageGz)} gzipped, over its ${kb(PAGE_BUDGET)} budget`)
  if (visit > VISIT_BUDGET) over.push(`a first visit costs ${kb(visit)}, over its ${kb(VISIT_BUDGET)} budget`)
  return over
}

const TYPES = {
  '.avif': 'image/avif',
  '.html': 'text/html; charset=utf-8',
  '.ico': 'image/x-icon',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.txt': 'text/plain; charset=utf-8',
  '.webp': 'image/webp',
  '.woff2': 'font/woff2',
  '.xml': 'application/xml',
}

function serve(port = 4173) {
  createServer(async (req, res) => {
    const { pathname } = new URL(req.url, 'http://localhost')
    let file = path.join(DIST, decodeURIComponent(pathname))
    if (pathname.endsWith('/')) file = path.join(file, 'index.html')
    const found = file.startsWith(DIST) && (await isFile(file))
    if (!found) file = path.join(DIST, '404.html')
    res.writeHead(found ? 200 : 404, { 'content-type': TYPES[path.extname(file)] ?? 'application/octet-stream' })
    res.end(await readFile(file))
  }).listen(port, () => console.log(`serving site/dist on http://localhost:${port}`))
}

const release = await latestRelease()
const d = downloads(release)

await rm(DIST, { recursive: true, force: true })
await cp(path.join(HERE, 'public'), DIST, { recursive: true })

// A new card gets a new URL, so apps that cached the old picture fetch it again.
const card = await readFile(path.join(DIST, 'og.png'))
const ogImage = `${SITE}/og.png?v=${createHash('sha256').update(card).digest('hex').slice(0, 10)}`
const data = {
  ...d,
  url: `${SITE}/`,
  repoUrl: REPO_URL,
  donateUrl: DONATE_URL,
  title: TITLE,
  description: DESCRIPTION,
  ogTitle: OG_TITLE,
  ogDescription: OG_DESCRIPTION,
  ogImage,
  ogImageAlt: OG_IMAGE_ALT,
  jsonLd: structuredData(d, ogImage),
}

const problems = []
let page = ''
for (const name of ['index.html', '404.html']) {
  const html = minify(render(await readFile(path.join(HERE, name), 'utf8'), data))
  await writeFile(path.join(DIST, name), html)
  if (name === 'index.html') page = html
  problems.push(...(await check(html)).map((problem) => `${name} ${problem}`))
}

const changed = [d.released, lastCommitDate()].filter(Boolean).sort().at(-1) ?? new Date().toISOString().slice(0, 10)
await writeFile(path.join(DIST, 'sitemap.xml'), sitemap(changed))
await writeFile(path.join(DIST, 'robots.txt'), ROBOTS)
await writeFile(path.join(DIST, 'llms.txt'), llmsTxt(d))

const from = release ? `${release.tag_name}${release.draft ? ' (a draft)' : ''}` : 'no published release yet'
console.log(`trimmy.lol built into site/dist from ${from}`)
problems.push(...(await report(page)))
if (problems.length) {
  console.error(`\nbuild failed:\n${problems.map((problem) => `  ${problem}`).join('\n')}`)
  process.exit(1)
}
if (flags.has('--serve')) serve()
