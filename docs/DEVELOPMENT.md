# Developing Lime

## Build

Requirements: macOS 13+ on Apple silicon, Rust, the Xcode Command Line Tools, and these Homebrew packages, which ffmpeg links statically:

```sh
brew install lame mpg123 opus libvpx webp libvorbis pkgconf nasm   # nasm: Intel Macs only
scripts/bundle.sh          # builds ffmpeg once, then build/Lime.app (~27 MB)
open build/Lime.app
```

## Layout

```
crates/lime-core   engine: format registry, conversions and tools (cross-platform core; PDF/AppKit parts macOS-only)
crates/lime-cli    `lime` CLI: `lime ops convert|tools FILE…`, `lime run OP [--arg V] FILE…`
crates/lime-mac    menu-bar app: drag detection, wheels, actions, progress, settings
crates/lime-tray   Windows/Linux tray app: choose files, run a conversion or tool; on Windows also the drag wheel (Linux: libgtk-3-dev libxdo-dev libayatana-appindicator3-dev)
scripts/           build-ffmpeg.sh, bundle.sh, smoke-test.sh
```

The app runs every job by calling the bundled `lime` CLI. PDFKit and AppKit text rendering need a main thread, and a separate process keeps a failing job from taking down the app.

## Tests

```sh
cargo test --workspace
scripts/smoke-test.sh                                  # CLI end to end on generated samples
scripts/smoke-test.sh build/Lime.app/Contents/MacOS/lime  # same, against the bundle
```

## Licensing

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
