# Lime

An offline file converter for macOS, modelled on [Tangerine](https://tangerineformac.com/).
Drag a file in Finder, hold a key, and drop it on a wheel that appears under the pointer.
Results are saved as new files next to the original; originals are never changed. Nothing is uploaded.

| Key (while dragging) | Wheel   | What it offers |
|----------------------|---------|----------------|
| Shift                | Convert | Target formats for the dragged file(s) |
| Command              | Tools   | Compress, resize, trim, split, merge, … |
| Control              | Actions | AirDrop, Float, Copy, Move, Copy Path, Terminal |

The wheel stays open after you release the key. Press another wheel key to switch wheels, or drop on the centre to cancel.

**Keyboard:** select files in Finder and press **Option-Return** to open the Convert wheel. Use the arrow keys to choose, Return to apply and Escape to cancel. Hold a wheel key to switch wheels. The first time, macOS asks whether Lime may control Finder; Lime needs this to read the selection. (Plain Return would clash with renaming in Finder.)
You can change the keys under **Settings…** in the 🍋‍🟩 menu-bar menu. Settings also has:

- **Appearance:** System, Light or Dark.
- **Glassiness:** how see-through the wheel's background is, from solid to clear glass. It needs macOS 26 or later; older systems show a plain grey wheel and the slider is disabled.

## Formats

- **Images:** JPG, PNG, WebP, HEIC, TIFF, AVIF, BMP, GIF ⇄ each other, plus PDF export. SVG, JPEG 2000 and JPEG XL are input only.
- **Audio:** MP3, M4A, WAV, FLAC, OGG, Opus, AIFF, WMA.
- **Video:** MP4, MOV, MKV, WebM, AVI, WMV, plus GIF and MP3 export.
- **PDF:** TXT, Markdown, DOCX and EPUB (text only), and JPG/PNG (every page, 300 DPI).
- **Documents:** DOCX, DOC, ODT, RTF, TXT, Markdown, HTML, EPUB, text clippings → DOCX, ODT, RTF, TXT, MD, HTML, EPUB, PDF.
- **Spreadsheets:** XLSX, XLS, ODS, CSV, TSV → XLSX, ODS, CSV, TSV, PDF.
- **Subtitles:** SRT ⇄ VTT, plus TXT export.
- **Archives:** ZIP, TAR, TGZ ⇄ each other. RAR, 7z and GZ are input only. Any file or folder can be archived.

**Tools** (they depend on the file type):

- **Images:** compress (quality), resize (%), rotate, flip, crop square, strip metadata, and combine several images into one PDF.
- **Video:** compress (% of the original size), trim, split, mute, rotate, scale to 720p, strip metadata, merge.
- **Audio:** compress (bitrate), trim, split, normalize loudness, strip metadata, merge.
- **PDF:** compress, split into pages, extract pages, rotate, merge.
- **Subtitles:** shift timing.
- **Archives:** extract.
- **Any selection:** zip.

## Build

Requirements: macOS 13+ on Apple silicon, Rust, the Xcode Command Line Tools, and these Homebrew packages, which ffmpeg links statically:

```sh
brew install lame mpg123 opus libvpx webp libvorbis pkgconf
scripts/bundle.sh          # builds ffmpeg once, then build/Lime.app (~27 MB)
open build/Lime.app
```

Dragging needs no special permissions; the keyboard flow asks once for permission to control Finder. To start it at login, add `Lime.app` under System Settings › General › Login Items.

## Layout

```
crates/lime-core   engine: format registry, conversions and tools (cross-platform core; PDF/AppKit parts macOS-only)
crates/lime-cli    `lime` CLI: `lime ops convert|tools FILE…`, `lime run OP [--arg V] FILE…`
crates/lime-mac    menu-bar app: drag detection, wheels, actions, progress, settings
scripts/           build-ffmpeg.sh, bundle.sh, smoke-test.sh
```

The app runs every job by calling the bundled `lime` CLI. PDFKit and AppKit text rendering need a main thread, and a separate process keeps a failing job from taking down the app.

## Tests

```sh
cargo test --workspace
scripts/smoke-test.sh                                  # CLI end to end on generated samples
scripts/smoke-test.sh build/Lime.app/Contents/MacOS/lime  # same, against the bundle
```

## Not (yet) in Lime

These Tangerine features are missing:

- The interactive photo, background, video and audio editors. Lime has parameter-based tools instead.
- PDF → DOCX layout reconstruction (Lime exports the text only), and PDF → PPTX/XLSX.
- PowerPoint rendering, and Pages files.
- The touch flow.
- Enter alone on a Finder selection; Lime uses Option-Return so renaming still works.

Markdown export is plain text.

## License

Lime is licensed under Apache-2.0.

The bundled `ffmpeg` is built LGPL-only (no GPL components; see `scripts/build-ffmpeg.sh` for the exact configuration). It ships as a separate executable inside the app. If you distribute Lime.app, include the FFmpeg LGPL notice and offer the FFmpeg source.
