#!/usr/bin/env bash
# Configures Trimmy's slim FFmpeg: only what Trimmy runs, LGPL, static. Run it from an empty
# build directory:
#
#   scripts/ffmpeg-configure.sh <ffmpeg source dir> [platform flags...]
#
# The platform flags (cross-compiling for Windows, the macOS CPU) come from
# .github/workflows/ffmpeg.yml. Everything Trimmy depends on is here, so both platforms ship the
# same components. docs/slim-ffmpeg.md explains each choice; keep the two in sync.
#
# What Trimmy runs:
#   probe      ffprobe -show_format -show_streams (JSON), and -show_entries packet=... (compact)
#   export     ffmpeg -ss -i -t -map 0:V:0 -map 0:a? -c copy (stream copy: demux, parse, mux)
#   waveform   ffmpeg -map 0:a:0 -af aformat,aresample,asetnsamples,astats,ametadata -f null -
set -euo pipefail

src=${1:?usage: ffmpeg-configure.sh <ffmpeg source dir> [platform flags...]}
shift

# Containers Trimmy opens and writes (it always writes the input's own container). Configure
# names, not -f names: mpegps demuxes .mpg, mpeg1system and mpeg2vob mux it, tgp is .3gp, and
# .m4v output goes through ipod.
demuxers=mov,matroska,mpegts,avi,flv,asf,mpegps,ogg
# MPEG-TS and MPEG-PS often don't say what a stream holds, so FFmpeg identifies it by running
# these raw elementary-stream demuxers' probes on its data. Without them, a .mpg has no video and
# some .m2ts files have no audio.
demuxers+=,aac,ac3,eac3,mp3,dts,truehd,h264,hevc,mpegvideo,m4v,vc1
muxers=mov,mp4,ipod,tgp,matroska,webm,mpegts,avi,flv,asf,mpeg1system,mpeg2vob,ogg,null

# Parsers let stream copy find keyframes and read sizes and frame rates without decoding video.
parsers=h264,hevc,av1,vp8,vp9,mpeg4video,mpegvideo,h263,vc1,mjpeg,aac,aac_latm,ac3,mpegaudio,opus,vorbis,flac,amr,dca

# Only the waveform decodes anything: audio, never video.
decoders=aac,aac_latm,mp3,mp3float,mp2,mp2float,mp1,mp1float,opus,vorbis,ac3,eac3,dca,flac,alac
decoders+=,amrnb,amrwb,wmav1,wmav2,wmapro,pcm_s16le,pcm_s16be,pcm_s24le,pcm_s24be,pcm_s32le
decoders+=,pcm_f32le,pcm_u8,pcm_alaw,pcm_mulaw,pcm_dvd,pcm_bluray

"$src/configure" \
  --disable-everything \
  --disable-autodetect \
  --disable-doc \
  --disable-network \
  --disable-avdevice \
  --disable-swscale \
  --disable-ffplay \
  --enable-ffmpeg \
  --enable-ffprobe \
  --enable-small \
  --disable-debug \
  --enable-static \
  --disable-shared \
  --enable-zlib \
  --enable-protocol=file,pipe \
  --enable-demuxer="$demuxers" \
  --enable-muxer="$muxers" \
  --enable-parser="$parsers" \
  --enable-bsf=h264_mp4toannexb,hevc_mp4toannexb,aac_adtstoasc,vp9_superframe,extract_extradata,null \
  --enable-decoder="$decoders" \
  --enable-encoder=pcm_s16le \
  --enable-filter=aformat,aresample,asetnsamples,astats,ametadata,anull,abuffer,abuffersink \
  "$@"
