# Future: a slim FFmpeg build

Status: idea, not started. Nothing here is implemented yet.

Trimmy ships a general-purpose FFmpeg and uses maybe 1% of it. A custom build with only the
parts Trimmy calls would make the installer several times smaller and the first run after a
reboot faster.

## Why

| | Today |
|---|---|
| Windows `ffmpeg.exe` / `ffprobe.exe` | ~100 MB each (gyan.dev 9.0.2 "essentials", GPL, static) |
| macOS `ffmpeg` / `ffprobe` | ~160 MB each (Martin Riedl 9.0.2 static, arm64 + x86_64 joined with `lipo`) |
| NSIS installer | 58 MB |
| macOS `.dmg` | 125 MB |
| First FFmpeg run after a reboot (author's PC) | ~1.2 s |
| Same, warm | ~0.15 s |

`trimmy.exe` itself is ~12 MB before compression, so almost all of the installer is FFmpeg.
The cold-start cost is most likely the OS (and Windows Defender) reading and scanning a 100 MB
image the first time. A ~5 MB binary should get close to the warm number (not measured yet).

## What Trimmy actually calls

- **Probe** (`probe.rs`): `ffprobe -print_format json -show_format -show_streams`
- **Export** (`export.rs`): `-ss S -i IN -t T -map 0:v:0 -map 0:a? -c copy -avoid_negative_ts make_zero -map_metadata 0 -progress pipe:1 OUT`,
  where `OUT` keeps the input's extension, so the muxer is picked from that extension.
- **Waveform** (`waveform.rs`): `-map 0:a:0 -vn -af aformat,aresample,asetnsamples,astats,ametadata -f null -`,
  with `ametadata ... file=-`, which opens `pipe:1`.
- **Planned:** a "precise" export that re-encodes, on the GPU where possible.

Stream copy needs demuxers, muxers, parsers and bitstream filters, but no video decoders and no
encoders. The waveform needs audio decoders, a few filters, and one encoder: the `null` muxer's
default audio codec is `pcm_s16le` (checked in `libavformat/nullenc.c`, n9.0.2).

## What to keep (tier 1: today's features)

Component names below were checked against the n9.0.2 sources (`allformats.c`, `parsers.c`,
`allcodecs.c`, `bitstream_filters.c`, `configure`). Note that configure names are the internal
ones, not the `-f` names: the MPEG-PS demuxer is `mpegps`, the `.mpg` muxer is `mpeg1system`,
`.vob` is `mpeg2vob`, `.3gp` is `tgp`, and `.m4v` output goes through `ipod`.

```sh
./configure \
  --disable-everything --disable-autodetect \
  --disable-ffplay --enable-ffmpeg --enable-ffprobe \
  --disable-doc --disable-network --disable-avdevice --disable-swscale \
  --enable-small --disable-debug --enable-lto \
  --enable-static --disable-shared \
  --enable-zlib \
  --enable-protocol=file,pipe \
  --enable-demuxer=mov,matroska,mpegts,avi,flv,asf,mpegps \
  --enable-muxer=mov,mp4,ipod,tgp,matroska,webm,mpegts,avi,flv,asf,mpeg1system,mpeg2vob,null \
  --enable-parser=h264,hevc,av1,vp8,vp9,mpeg4video,mpegvideo,h263,vc1,aac,aac_latm,ac3,mpegaudio,opus,vorbis,flac,amr,dca \
  --enable-bsf=h264_mp4toannexb,hevc_mp4toannexb,aac_adtstoasc,vp9_superframe,extract_extradata,null \
  --enable-decoder=aac,aac_latm,mp3,mp3float,mp2,mp2float,opus,vorbis,ac3,eac3,flac,alac,amrnb,amrwb,wmav1,wmav2,wmapro,pcm_s16le,pcm_s16be,pcm_s24le,pcm_s24be,pcm_s32le,pcm_f32le,pcm_u8,pcm_alaw,pcm_mulaw,pcm_dvd,pcm_bluray \
  --enable-encoder=pcm_s16le \
  --enable-filter=aformat,aresample,asetnsamples,astats,ametadata,anull,abuffer,abuffersink
```

Notes:

- `--disable-everything` turns off every component, including the input/output devices. The
  `ffmpeg` program then re-selects what it hard-depends on (`aformat anull atrim crop format hflip
  null rotate transpose trim vflip`, from `ffmpeg_select` in `configure`). `buffer`/`abuffer` and
  their sinks are always built; listing them does no harm.
- `aresample` needs libswresample, which stays enabled. Nothing needs libswscale in tier 1
  (unverified that fftools builds cleanly without it; drop `--disable-swscale` if not).
- Muxers pull in some bitstream filters on their own (`mpegts_muxer_select` adds the
  `*_mp4toannexb` filters; `mov` and `matroska` add `aac_adtstoasc` and `vp9_superframe`). The
  explicit `--enable-bsf` list is belt and braces.
- Parsers are what let stream copy find keyframes and fill in width, height and frame rate
  without a decoder. `vc1`/`h263` and the WMA/AMR decoders are for WMV and 3GP; `dca` is DTS
  audio in MKV/TS and can be dropped if size matters.
- `--disable-autodetect` stops configure from quietly linking whatever it finds on the build
  machine (SDL, iconv, the GPU SDKs). Anything wanted, like zlib (compressed MKV headers and
  QuickTime `cmov`), has to be enabled by name.
- Verify the result with `ffmpeg -hide_banner -buildconf`, `-demuxers`, `-muxers`, `-bsfs` and
  `-filters`.

## Tier 2 (optional): precise export

Only when re-encoding ships. Adds video decoders, libswscale and the encoders:

```sh
# both platforms (drop --disable-swscale from tier 1)
--enable-swscale --enable-libdav1d
--enable-decoder=h264,hevc,vp8,vp9,mpeg4,mpeg2video,libdav1d,vc1,wmv3
--enable-filter=scale,format,setpts,trim,atrim
--enable-encoder=aac

# Windows
--enable-ffnvcodec --enable-nvenc --enable-amf --enable-libvpl --enable-mediafoundation
--enable-encoder=h264_nvenc,hevc_nvenc,h264_amf,hevc_amf,h264_qsv,hevc_qsv,h264_mf

# macOS
--enable-videotoolbox --enable-audiotoolbox
--enable-encoder=h264_videotoolbox,hevc_videotoolbox
```

Since tier 1 uses `--disable-autodetect`, every hardware API has to be enabled by name, and its
headers or SDK (nv-codec-headers, AMF headers, libvpl, dav1d) must be in the build environment.
Hardware *decoding* (`-hwaccel`) is left out; CPU decoding is fine for short clips.

NVENC and AMF are loaded at run time from the GPU driver (`nvenc_deps_any="libdl LoadLibrary"`,
same for `amf`), so they add little size. The software fallback without x264 would be
`h264_mf` (Media Foundation's built-in encoder) on Windows and VideoToolbox on macOS, which
always has a software path. `libopenh264` (BSD) is another option if those turn out poor.

## Licensing

Not legal advice; read [ffmpeg.org/legal.html](https://ffmpeg.org/legal.html) before shipping.

Today's gyan.dev build is **GPLv3**: it links libx264, libx265 and other GPL libraries. A build
without `--enable-gpl`, `--enable-version3` or `--enable-nonfree` is **LGPL 2.1 or later**.
Nothing Trimmy needs requires GPL code: stream copy uses no encoders, the waveform uses FFmpeg's
own decoders, and the NVENC/AMF/QSV/VideoToolbox/Media Foundation wrappers are LGPL-compatible
(the NVIDIA headers are MIT, libvpl is MIT). Do not add CUDA NPP or anything that needs
`--enable-nonfree`; that makes the binary non-redistributable.

What changes, as far as we understand it:

- **Either way,** Trimmy runs FFmpeg as a separate program, so Trimmy's own MIT code is not a
  derivative of it. We ship FFmpeg's license text and must make the matching source available.
- **GPL build:** the source offer covers FFmpeg *and* every GPL library in it (x264, x265, ...),
  at the exact versions used, plus the build scripts. We currently lean on gyan.dev for that.
- **LGPL build:** the source offer is just FFmpeg (plus any LGPL/BSD libs like dav1d) and our
  configure line, which we would own and publish anyway. It also leaves the door open to linking
  FFmpeg's libraries into Trimmy later (for example to probe in-process) without pulling the app
  under the GPL; for static linking the LGPL has extra relinking conditions, so dynamic linking
  would be the safer route.
- Patents are a separate question that neither license settles. A tier-1 build carries no video
  codec implementations at all, only parsers.

## Building it in CI

A separate workflow, `.github/workflows/ffmpeg.yml`, run by hand (`workflow_dispatch`), never on
app tags:

1. Download `https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz` and check its SHA-256 (pinned in
   the workflow). Keep the configure flags in one shared file, e.g. `scripts/ffmpeg-configure.sh`,
   so both platforms build the same thing.
2. **Windows:** either `windows-latest` with [msys2/setup-msys2](https://github.com/msys2/setup-msys2)
   (UCRT64 toolchain + `nasm`, `--extra-ldflags=-static --pkg-config-flags=--static`), or
   cross-compile on `ubuntu-latest` with mingw-w64 (`--target-os=mingw32 --arch=x86_64
   --cross-prefix=x86_64-w64-mingw32-`). The cross-compile is what
   [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds) does, in Docker, and is easier to
   pin. See also the [MinGW compilation guide](https://trac.ffmpeg.org/wiki/CompilationGuide/MinGW).
3. **macOS:** on `macos-latest` (arm64), build twice, natively for arm64 and with
   `--enable-cross-compile --arch=x86_64 --extra-cflags="-arch x86_64" --extra-ldflags="-arch x86_64"`,
   then `lipo -create`. Martin Riedl's [build scripts](https://git.martin-riedl.de/ffmpeg/build-script)
   (the source of today's mac binaries) show a working static macOS setup. Intel runners are
   being retired, which is why the x86_64 build is a cross-compile (untested).
4. Smoke-test each binary (`-version`, `-buildconf`, probe and export one tiny generated clip),
   then zip `ffmpeg`, `ffprobe`, the LGPL text and the configure line, and publish them as assets
   on a release in this repo tagged e.g. `ffmpeg-9.0.2-trimmy.1`. Also attach the source tarball,
   which covers the source offer.
5. Mark that release as a pre-release / not "latest" (`make_latest: false`), because the README's
   download link points at `releases/latest`.

`fetch-ffmpeg.mjs` then changes only its URLs and hashes; the download, checksum and `lipo`
logic stays as is:

```js
const OURS = `https://github.com/TuckerScottAlleborn/trimmy/releases/download/ffmpeg-${VERSION}-trimmy.1`
const WINDOWS = [
  {
    url: `${OURS}/ffmpeg-windows-x86_64.zip`,
    sha256: '<printed by the ffmpeg workflow>',
    unzip: {
      'ffmpeg.exe': 'ffmpeg-x86_64-pc-windows-msvc.exe',
      'ffprobe.exe': 'ffprobe-x86_64-pc-windows-msvc.exe',
      'LICENSE.md': 'FFmpeg-LICENSE.txt',
    },
  },
]
```

The macOS zips could ship already-universal binaries, which would make the `lipo` step in
`fetch-ffmpeg.mjs` unnecessary. Bump the stamp (e.g. `9.0.2-trimmy.1`) so existing checkouts
re-download. Builds will not be bit-for-bit reproducible (toolchains move); pinning the source
hash, the flags and the runner image is the practical bar.

A cheaper stopgap: BtbN publishes `win64-lgpl` builds. They are still full-featured (so not much
smaller), and their daily builds are only kept for 14 days (monthly ones for two years), so they
would have to be mirrored into our own release to pin them.

## Estimated result

All numbers are **estimates**, not measurements.

| | Today | Tier 1 | Tier 1 + 2 |
|---|---|---|---|
| Windows `ffmpeg.exe` | 100 MB | 3-6 MB | 8-14 MB |
| Windows `ffprobe.exe` | 100 MB | 2-4 MB | 2-4 MB (keep it tier 1) |
| macOS `ffmpeg` (universal) | 160 MB | 6-12 MB | 15-25 MB |
| NSIS installer | 58 MB | 7-10 MB | 10-15 MB |
| macOS `.dmg` | 125 MB | 15-25 MB | 20-35 MB |

### One binary instead of two?

Each static binary carries its own copy of libavformat/libavcodec. Options:

- **Keep both, same trimmed config.** Simplest; ffprobe adds only a few MB. Recommended.
- **Drop ffprobe, probe with `ffmpeg -i`.** Saves a few MB, but `ffmpeg -i` prints
  human-oriented text to stderr and exits with an error; parsing it is fragile across versions.
- **Shared build** (`--enable-shared`): both programs use the same DLLs/dylibs. Saves the
  duplication and makes LGPL compliance trivial, but Tauri sidecars are single executables, so
  the libraries would have to be bundled as resources and found at run time. Not worth it yet.

## Test plan

1. Build the slim binaries, drop them into `src-tauri/bin/` under the usual sidecar names.
2. `npm run test-clips -- <a real ShadowPlay/OBS clip> [<long audio>]`. Note that
   `make-test-clips.mjs` *encodes* with libx264, libx265, libaom, libvpx, libopus and libmp3lame,
   so it must keep using the full gyan.dev build (generate the clips first, or point the script
   at a separate full FFmpeg).
3. For every file in `test-clips/`: open it in Trimmy, check duration, size, fps and codec,
   check the waveform draws (and that `h264-noaudio.mp4` shows none), export a cut, and open the
   exported file. `h264-2audio.mp4` must keep both audio tracks.
4. Add cases the current set lacks: WMV/ASF, FLV, 3GP, MPEG-PS (`.mpg`), AC-3/E-AC-3 and PCM
   audio, Opus in MKV, and an MPEG-TS whose first packet is not a keyframe. These can be made
   with the full build via stream copy or cheap encodes.
5. Compare `ffprobe` JSON from the slim and full builds for each file; `width`, `height`,
   `avg_frame_rate`, `codec_name` and `duration` must match (the slim build may lack `pix_fmt`
   without video decoders, which Trimmy does not read).

## Risks

- **A missing component breaks someone's odd file.** A demuxer, parser or audio decoder we did
  not list means "file won't open" or "no waveform". Mitigate: map FFmpeg's "Invalid data found",
  "Unknown decoder" and "Could not find codec parameters" errors to a plain message ("Trimmy
  can't read this kind of file yet, please report it"), and keep the list generous; demuxers and
  parsers are small.
- **Stream copy without video decoders** relies on parsers to fill in codec parameters. For
  MPEG-TS and PS this is usually enough, but it is not verified for every codec. Test it.
- **Maintenance.** We own the build now: every FFmpeg bump means re-running the workflow,
  re-testing, and updating hashes. Toolchain drift (MSYS2 is rolling) can break builds with no
  change on our side.
- **macOS**: the x86_64 cross-build and code signing of our own binaries are untested, like the
  rest of the macOS build.
