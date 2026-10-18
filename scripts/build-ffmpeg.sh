#!/bin/sh
# Builds a minimal, LGPL-only, statically linked ffmpeg into build/ffmpeg/bin/ffmpeg.
# Codec libs come from Homebrew: brew install lame mpg123 opus libvpx webp libvorbis pkgconf
set -eu
VERSION=7.1.1
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK="$ROOT/build/ffmpeg-src"
OUT="$ROOT/build/ffmpeg"
DEPS="$ROOT/build/ffmpeg-deps"
BREW=$(brew --prefix)

mkdir -p "$WORK" "$DEPS"
# Only .a files in this dir, searched first, so the linker never picks Homebrew dylibs.
for lib in mp3lame mpg123 opus vpx webp webpmux sharpyuv vorbis vorbisenc ogg; do
  ln -sf "$BREW/lib/lib$lib.a" "$DEPS/lib$lib.a"
done

cd "$WORK"
[ -f "ffmpeg-$VERSION.tar.xz" ] || curl -fLO "https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz"
[ -d "ffmpeg-$VERSION" ] || tar -xf "ffmpeg-$VERSION.tar.xz"
cd "ffmpeg-$VERSION"

# Relative paths only: ffmpeg embeds its configure line (shown by -buildconf), so absolute ones would leak the build machine's home folder.
./configure \
  --disable-shared --enable-static \
  --disable-doc --disable-ffplay --disable-ffprobe --disable-network \
  --disable-autodetect --enable-videotoolbox --enable-audiotoolbox --enable-zlib \
  --enable-libmp3lame --enable-libopus --enable-libvpx --enable-libwebp --enable-libvorbis \
  `# Patent-pooled audio codecs go through Apple's licensed AudioToolbox (aac_at, ac3_at, eac3_at) instead.` \
  --disable-decoder=aac,aac_fixed,aac_latm,ac3,ac3_fixed,eac3 \
  --disable-encoder=aac,ac3,ac3_fixed,eac3,mpeg4 \
  --pkg-config-flags=--static \
  --extra-cflags="-I$BREW/include" --extra-ldflags="-L../../ffmpeg-deps" --extra-libs="-lmpg123"
make -j"$(sysctl -n hw.ncpu)" ffmpeg
mkdir -p "$OUT/bin"
cp ffmpeg "$OUT/bin/ffmpeg"

# Fail if anything besides system libraries got linked dynamically.
if otool -L "$OUT/bin/ffmpeg" | tail -n +2 | grep -v -e '^\s*/usr/lib/' -e '^\s*/System/'; then
  echo "error: ffmpeg links non-system dylibs" >&2; exit 1
fi
# Selling Lime relies on ffmpeg staying LGPL: no GPL or nonfree parts.
if "$OUT/bin/ffmpeg" -hide_banner -buildconf | grep -e '--enable-gpl' -e '--enable-nonfree' -e '--enable-version3'; then
  echo "error: ffmpeg build is not LGPL-2.1-only" >&2; exit 1
fi
"$OUT/bin/ffmpeg" -hide_banner -L | head -3
ls -lh "$OUT/bin/ffmpeg"
