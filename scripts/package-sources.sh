#!/bin/sh
# Collects the source that LGPL requires Lime to offer for its bundled ffmpeg into
# build/lime-<version>-third-party-sources.tar.gz. Attach that file to every GitHub release.
# Run after scripts/build-ffmpeg.sh, with the same Homebrew packages installed.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
LIME_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
FFMPEG_VERSION=$(sed -n 's/^VERSION=//p' scripts/build-ffmpeg.sh)
NAME=lime-$LIME_VERSION-third-party-sources
DIR=build/$NAME
rm -rf "$DIR" && mkdir -p "$DIR/codecs"

cp "build/ffmpeg-src/ffmpeg-$FFMPEG_VERSION.tar.xz" scripts/build-ffmpeg.sh "$DIR/"
# The codec libraries are linked into the ffmpeg executable; ship their exact sources and
# Homebrew build recipes (which list any patches) so the executable can be rebuilt.
for formula in lame mpg123 opus libvpx webp libvorbis libogg; do
  brew fetch --quiet --build-from-source "$formula" >/dev/null
  src=$(brew --cache --build-from-source "$formula")
  cp "$src" "$DIR/codecs/$(basename "$src" | sed 's/^[0-9a-f]*--//')"
  brew cat "$formula" > "$DIR/codecs/$formula.rb"
  echo "$formula $(brew list --versions "$formula" | cut -d' ' -f2)" >> "$DIR/codecs/VERSIONS"
done
cat > "$DIR/README.txt" <<TXT
Sources for the FFmpeg program bundled in Lime $LIME_VERSION (Lime.app/Contents/Resources/ffmpeg).

ffmpeg-$FFMPEG_VERSION.tar.xz   FFmpeg $FFMPEG_VERSION, unmodified upstream release (LGPL-2.1-or-later)
build-ffmpeg.sh            the exact configure options and build steps used
codecs/                    sources and Homebrew recipes of the libraries linked into it (VERSIONS lists them)

To rebuild: install the codec libraries (brew install lame mpg123 opus libvpx webp libvorbis pkgconf),
run build-ffmpeg.sh, and replace Lime.app/Contents/Resources/ffmpeg with the result.
TXT
tar -czf "build/$NAME.tar.gz" -C build "$NAME"
du -sh "build/$NAME.tar.gz"
