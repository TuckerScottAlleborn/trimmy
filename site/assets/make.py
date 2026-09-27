"""Regenerates the generated files in site/public/ from the app's own assets:

    fonts/jetbrains-mono-{400,800}.woff2   the app's font, cut down to Latin text
    img/screenshot-{640,976}.{avif,webp}   docs/screenshot.png, resized and compressed
    favicon.ico, apple-touch-icon.png      from src-tauri/icons
    og.png                                 the link-preview card, assets/og.html at 1200x630

    python site/assets/make.py [path/to/node_modules]

node_modules defaults to the repo's own (run `npm install` first); it supplies the font.
Needs Python 3.10+, Pillow with AVIF support, fonttools with brotli, and Edge or Chrome.
The outputs are committed, so building and deploying the site never needs any of this.
public/icon.svg is not generated: it's a hand-made compact copy of src-tauri/icons/icon.svg.
"""

import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from fontTools.subset import Options, Subsetter, parse_unicodes
from fontTools.ttLib import TTFont
from PIL import Image

HERE = Path(__file__).resolve().parent
SITE = HERE.parent
REPO = SITE.parent
PUBLIC = SITE / "public"

# Printable ASCII, Latin-1 (· × © é ...) and typographic punctuation (– — ‘ ’ “ ” • … ‹ ›).
UNICODES = "U+20-7E,U+A0-FF,U+2013-2014,U+2018-2019,U+201C-201D,U+2022,U+2026,U+2039-203A"
WEIGHTS = (400, 800)
# index.html's srcset lists these widths; the largest is the screenshot's own.
SCREENSHOT_WIDTHS = (640, 976)
BG = (10, 13, 14)  # --bg, #0a0d0e

BROWSERS = (
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "google-chrome",
    "chromium",
    "microsoft-edge",
)


def fonts(node_modules: Path) -> None:
    src = node_modules / "@fontsource" / "jetbrains-mono" / "files"
    out = PUBLIC / "fonts"
    out.mkdir(parents=True, exist_ok=True)
    for weight in WEIGHTS:
        # Keep the source's timestamp, so rerunning this doesn't churn the committed files.
        font = TTFont(src / f"jetbrains-mono-latin-{weight}-normal.woff2", recalcTimestamp=False)
        options = Options()
        options.layout_features = []  # no ligatures: "->" stays two characters
        subsetter = Subsetter(options)
        subsetter.populate(unicodes=parse_unicodes(UNICODES))
        subsetter.subset(font)
        font.flavor = "woff2"
        font.save(out / f"jetbrains-mono-{weight}.woff2")


def screenshots() -> None:
    shot = Image.open(REPO / "docs" / "screenshot.png").convert("RGB")
    if shot.width != SCREENSHOT_WIDTHS[-1]:
        sys.exit(f"make.py: docs/screenshot.png is {shot.width} wide; update SCREENSHOT_WIDTHS "
                 "and the srcset, width and height in index.html to match")
    out = PUBLIC / "img"
    out.mkdir(parents=True, exist_ok=True)
    for width in SCREENSHOT_WIDTHS:
        size = (width, round(shot.height * width / shot.width))
        image = shot if size == shot.size else shot.resize(size, Image.LANCZOS)
        # Full-resolution color (4:4:4) keeps the thin green UI text crisp in the AVIF.
        image.save(out / f"screenshot-{width}.avif", quality=60, subsampling="4:4:4", speed=0)
        image.save(out / f"screenshot-{width}.webp", quality=80, method=6)


def icons() -> None:
    src = REPO / "src-tauri" / "icons"
    shutil.copyfile(src / "icon.ico", PUBLIC / "favicon.ico")
    # iOS rounds the corners itself, so the touch icon is a full square on the app's background.
    tile = Image.open(src / "128x128@2x.png").convert("RGBA")
    square = Image.new("RGBA", tile.size, BG + (255,))
    square.alpha_composite(tile)
    square.convert("RGB").resize((180, 180), Image.LANCZOS).save(
        PUBLIC / "apple-touch-icon.png", optimize=True)


def browser() -> str:
    for candidate in BROWSERS:
        if found := shutil.which(candidate):
            return found
    sys.exit("make.py: no Edge or Chrome found to render the link-preview card")


def og_card() -> None:
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        shot = Path(tmp) / "og.png"
        subprocess.run([
            browser(), "--headless=new", f"--user-data-dir={Path(tmp) / 'profile'}",
            "--allow-file-access-from-files",  # the card loads the fonts from public/
            "--hide-scrollbars", "--force-device-scale-factor=1", "--window-size=1200,630",
            "--virtual-time-budget=5000", f"--screenshot={shot}", (HERE / "og.html").as_uri(),
        ], check=True, capture_output=True, timeout=120)
        # On Windows the process we started hands off to a child and exits before the
        # screenshot is written, so wait for the file to appear and stop growing.
        size = -1
        for _ in range(120):
            if shot.is_file() and shot.stat().st_size == size:
                break
            size = shot.stat().st_size if shot.is_file() else -1
            time.sleep(0.25)
        else:
            sys.exit("make.py: the browser never wrote the link-preview card")
        card = Image.open(shot).convert("RGB")
        if card.size != (1200, 630):
            sys.exit(f"make.py: the card rendered at {card.size[0]}x{card.size[1]}, not 1200x630")
        card.save(PUBLIC / "og.png", optimize=True)


def main() -> None:
    node_modules = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO / "node_modules"
    if not (node_modules / "@fontsource" / "jetbrains-mono").is_dir():
        sys.exit(f"make.py: no @fontsource/jetbrains-mono in {node_modules}; run npm install")
    fonts(node_modules)
    screenshots()
    icons()
    og_card()  # last: it uses the fonts
    for path in sorted(PUBLIC.rglob("*")):
        if path.is_file():
            print(f"{path.stat().st_size:>9,}  {path.relative_to(SITE).as_posix()}")


if __name__ == "__main__":
    main()
