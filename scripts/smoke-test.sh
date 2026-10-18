#!/bin/sh
# End-to-end check of the lime CLI on generated sample files.
# Usage: scripts/smoke-test.sh [path/to/lime]   (default: target/debug/lime)
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
LIME=$(cd "$(dirname "${1:-$ROOT/target/debug/lime}")" && pwd)/$(basename "${1:-lime}")
FF=$ROOT/build/ffmpeg/bin/ffmpeg
# Inside Lime.app the CLI must find its bundled ffmpeg on its own.
case $LIME in *.app/*) ;; *) export LIME_FFMPEG=${LIME_FFMPEG:-$FF} ;; esac
D=$(mktemp -d)
trap 'rm -rf "$D"' EXIT
cd "$D"
fail=0

$FF -hide_banner -loglevel error -f lavfi -i testsrc=duration=4:size=320x240:rate=25 -f lavfi -i sine=duration=4 \
  -c:v h264_videotoolbox -c:a aac_at -shortest clip.mp4
$FF -hide_banner -loglevel error -f lavfi -i sine=duration=3 tone.wav
$FF -hide_banner -loglevel error -f lavfi -i testsrc=size=400x300 -frames:v 1 pic.png
printf '# Report\n\nSome *text*.\n' > doc.md
printf 'a,b\n1,2\n' > t.csv
printf '1\n00:00:01,000 --> 00:00:02,000\nHi\n' > s.srt

check() { # check DESCRIPTION -- lime args...
  desc=$1; shift 2
  if out=$("$LIME" "$@" 2>&1) && [ -n "$out" ]; then echo "ok   $desc"; else echo "FAIL $desc: $out"; fail=1; fi
}

for t in jpg webp heic tiff avif bmp gif pdf; do check "png->$t" -- run to:$t pic.png; done
check "heic->webp" -- run to:webp pic.heic
check "image compress" -- run compress pic.jpg
check "image resize" -- run resize --arg 25 pic.png
check "image square" -- run square pic.webp
check "images->pdf" -- run merge pic.png pic.jpg
for t in mov mkv webm avi wmv gif mp3; do check "mp4->$t" -- run to:$t clip.mp4; done
for t in mp3 m4a flac ogg opus aiff wma; do check "wav->$t" -- run to:$t tone.wav; done
check "video compress" -- run compress clip.mp4
check "video trim" -- run trim --arg 0:01-0:03 clip.mp4
check "video split" -- run split clip.mp4
check "video 720p" -- run 720p clip.mp4
check "audio normalize" -- run normalize tone.wav
check "audio merge" -- run merge tone.wav tone.wav
check "md->pdf" -- run to:pdf doc.md
check "md->docx" -- run to:docx doc.md
check "docx->epub" -- run to:epub doc.docx
check "csv->xlsx" -- run to:xlsx t.csv
check "csv->pdf" -- run to:pdf t.csv
check "csv->ods" -- run to:ods t.csv
check "srt->vtt" -- run to:vtt s.srt
check "pdf->txt" -- run to:txt doc.pdf
check "pdf->png" -- run to:png doc.pdf
check "pdf->docx" -- run to:docx doc.pdf
check "pdf->epub" -- run to:epub doc.pdf
check "pdf rotate" -- run rotate doc.pdf
check "pdf merge" -- run merge doc.pdf pic.pdf
check "pdf compress" -- run compress pic.pdf
check "zip" -- run zip pic.png doc.md

ls -la "$D" > /dev/null
[ $fail = 0 ] && echo "all passed" || { echo "FAILURES"; exit 1; }
