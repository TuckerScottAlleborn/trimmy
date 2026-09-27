# trimmy.lol

The website: one page of plain HTML and CSS, no JavaScript. `build.mjs` (Node 20+, no
dependencies) fills `index.html` in with the latest release, and the
[Site workflow](../.github/workflows/site.yml) publishes it to GitHub Pages.

```sh
node site/build.mjs --serve          # build, then preview on http://localhost:4173
node site/build.mjs --draft --serve  # same, using the newest release even if it's a draft
```

## When it updates

- A push to `main` that changes `site/`.
- A release is published, unpublished or deleted. The download button, version and file size
  come from the latest *published* release, so publishing the draft is what updates the site.
- Running the Site workflow by hand (Actions tab).

The installers are found by name: `*_x64-setup.exe` (the button), `*_x64_en-US.msi` and
`*_universal.dmg`. A release with no setup `.exe` fails the build, which leaves the live site
alone. Before any release is published, the button links to the releases page instead.

## Files

| Path | What |
| --- | --- |
| `index.html` | The page, as a template. `{{key}}` is filled in by `build.mjs`. |
| `build.mjs` | The build, plus the title, description, link-preview texts and donation link. |
| `404.html` | Served by GitHub Pages for any missing path. |
| `public/` | Copied as-is. Everything but `icon.svg` is generated; don't edit it by hand. |
| `assets/make.py` | Regenerates `public/`: fonts, screenshots, icons, link-preview card. |
| `assets/og.html` | The link-preview card, rendered to `public/og.png`. |

After changing `docs/screenshot.png`, the app icon, the card or the font, rerun
`python site/assets/make.py` (needs Pillow, fonttools and brotli, Edge or Chrome, and
`npm install` for the font) and commit `public/`.

## What the build checks

It fails rather than ship:

- a link to a file that isn't in the build;
- an image whose real size differs from its `width`/`height` or `og:image:width`/`height`
  (previews would crop wrongly), or a preview card over 300 KB (WhatsApp would drop it);
- a title over 60 characters, a description over 160, or anything but one `<h1>`;
- `index.html` over 14 KB gzipped (it must arrive in the first round trip), or a first visit
  over 100 KB.

## Link previews

Discord, Bluesky, iMessage, Slack, WhatsApp, Telegram, Signal, LinkedIn, Facebook, Reddit and
Mastodon read the Open Graph tags; X reads `twitter:card` and falls back to them. X shows only
the picture, so `og.png` spells out the name and tagline itself. `theme-color` colors Discord's
embed stripe, and Slack lists the version and price under the link.

Apps cache previews. The card's URL carries a hash of the image, so a new card is fetched as
soon as an app looks at the page again. To force that: [Facebook's
debugger](https://developers.facebook.com/tools/debug/), [LinkedIn's Post
Inspector](https://www.linkedin.com/post-inspector/), and for Bluesky
`https://cardyb.bsky.app/v1/extract?url=https://trimmy.lol`.

## Domain

`trimmy.lol` is registered at Namecheap and set as the custom domain in the repo's Pages
settings. DNS (Namecheap, Advanced DNS):

| Type | Host | Value |
| --- | --- | --- |
| A | `@` | `185.199.108.153`, `185.199.109.153`, `185.199.110.153`, `185.199.111.153` (one record each) |
| AAAA | `@` | `2606:50c0:8000::153`, `2606:50c0:8001::153`, `2606:50c0:8002::153`, `2606:50c0:8003::153` |
| CNAME | `www` | `tuckerscottalleborn.github.io.` (GitHub redirects `www` to the bare domain) |
| TXT | `_github-pages-challenge-TuckerScottAlleborn` | the code from GitHub's Pages settings (domain verification) |
