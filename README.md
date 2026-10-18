# Lime

An offline file converter for macOS.
Drag a file in Finder, hold a key, and drop it on a wheel that appears under the pointer.
Results are saved as new files next to the original; originals are never changed. Nothing is uploaded.

| Key (while dragging) | Wheel   | What it offers |
|----------------------|---------|----------------|
| Shift                | Convert | Target formats for the dragged file(s) |
| Command              | Tools   | Compress, resize, trim, split, merge, … |
| Control              | Actions | AirDrop, Float, Copy, Move, Copy Path, Terminal |

After you drop, the chosen button fills with lime from its inner to its outer edge as the job runs, and the wheel closes when it is done. Failures show in a small panel.

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

## Limits

- Tools take parameters (quality, time range, …); there are no interactive editors.
- PDF → DOCX and EPUB export the text only, without layout. No PDF → PPTX/XLSX, PowerPoint rendering or Pages files.
- Markdown export is plain text.

## License

Lime is licensed under Apache-2.0.

The bundled `ffmpeg` is built LGPL-2.1-only: no GPL, nonfree or LGPLv3 parts, which `scripts/build-ffmpeg.sh` checks. It ships as a separate, replaceable executable inside the app. `scripts/bundle.sh` writes all third-party license notices, with the LGPL source offer, to `Lime.app/Contents/Resources/Acknowledgements.txt`, shown under **Acknowledgements…** in the menu.

## Releasing

1. Build signed and notarized, with a Developer ID certificate and a `notarytool` keychain profile:
   ```sh
   SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" NOTARY_PROFILE=lime scripts/bundle.sh
   ```
   This gives `build/Lime.zip`. Without `SIGN_IDENTITY` the app is signed ad hoc, and Gatekeeper blocks it on other Macs.
2. Run `scripts/package-sources.sh` and attach `build/lime-<version>-third-party-sources.tar.gz` to the same GitHub release as the app. The acknowledgements promise it there; LGPL requires it.
3. Bump `version` in `Cargo.toml` for every release, so the notice names the right source archive.

Codec patents: AAC and AC-3 are decoded and AAC/H.264 encoded by Apple's licensed AudioToolbox and VideoToolbox. AVI export uses Motion JPEG and MP3, which are patent-free. ffmpeg's own H.264, HEVC and other video decoders are still used to read video; that is the remaining patent exposure, shared with most ffmpeg-based apps.

The Mac App Store is not a target: Lime watches the system drag pasteboard, runs a helper executable and scripts Finder, which the App Store sandbox does not allow.
