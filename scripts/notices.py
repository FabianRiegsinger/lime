#!/usr/bin/env python3
"""Writes build/Acknowledgements.txt: the license notices Lime.app must ship with.

Covers the bundled LGPL ffmpeg (with the source offer), the codec libraries linked into it,
and every Rust crate compiled into the `lime` and `lime-mac` binaries.
Run from scripts/bundle.sh after ffmpeg is built.
"""
import hashlib
import json
import pathlib
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "build" / "Acknowledgements.txt"
REPO = "https://github.com/FabianRiegsinger/lime"
# Homebrew formula -> what ffmpeg links from it, and its license.
CODECS = [
    ("lame", "LAME", "LGPL-2.0-or-later"),
    ("mpg123", "mpg123", "LGPL-2.1-only"),
    ("opus", "Opus", "BSD-3-Clause"),
    ("libvpx", "libvpx", "BSD-3-Clause"),
    ("webp", "libwebp", "BSD-3-Clause"),
    ("libvorbis", "libvorbis", "BSD-3-Clause"),
    ("libogg", "libogg", "BSD-3-Clause"),
]
LICENSE_FILE = re.compile(r"^(LICEN[SC]E|COPYING|NOTICE|UNLICENSE)", re.I)


def sh(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout.strip()


def ffmpeg_version():
    return re.search(r"^VERSION=(\S+)", (ROOT / "scripts/build-ffmpeg.sh").read_text(), re.M).group(1)


def license_texts(directory):
    return [p for p in sorted(pathlib.Path(directory).iterdir()) if p.is_file() and LICENSE_FILE.match(p.name)]


def shipped_crates():
    """Crates reachable through normal (non-dev, non-build) dependencies of the shipped binaries."""
    meta = json.loads(sh("cargo", "metadata", "--format-version", "1", "--filter-platform", "aarch64-apple-darwin", "--manifest-path", str(ROOT / "Cargo.toml")))
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    members = set(meta["workspace_members"])
    roots = [i for i in members if packages[i]["name"] in ("lime", "lime-mac")]
    seen, stack = set(), list(roots)
    while stack:
        node = nodes[stack.pop()]
        for dep in node["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]) and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                stack.append(dep["pkg"])
    return sorted((packages[i] for i in seen - members), key=lambda p: (p["name"], p["version"]))


def main():
    version = ffmpeg_version()
    lime_version = re.search(r'^version = "(.+)"', (ROOT / "Cargo.toml").read_text(), re.M).group(1)
    buildconf = sh(str(ROOT / "build/ffmpeg/bin/ffmpeg"), "-hide_banner", "-buildconf")
    out = [
        "Lime — Acknowledgements and third-party notices",
        "",
        "Lime is licensed under the Apache License 2.0.",
        "",
        "=" * 72,
        "FFmpeg",
        "=" * 72,
        f"Lime includes FFmpeg {version} (https://ffmpeg.org) as a separate program at",
        "Lime.app/Contents/Resources/ffmpeg. FFmpeg is licensed under the GNU Lesser",
        "General Public License version 2.1 or later (LGPL). This build contains no GPL",
        "or nonfree components. You may replace that program with your own build.",
        "",
        "It is statically linked with: " + ", ".join(f"{name} ({lic})" for _, name, lic in CODECS) + ".",
        "",
        "Source code: the complete corresponding source of FFmpeg and the LGPL libraries",
        "linked into it, with the exact build script, is published with every Lime",
        f"release at {REPO}/releases as lime-{lime_version}-third-party-sources.tar.gz.",
        "",
        "Written offer: for at least three years after you received this copy of Lime,",
        "anyone may request that source by opening an issue at",
        f"{REPO}/issues. It will be provided for no more than the cost of",
        "physically performing the distribution.",
        "",
        "FFmpeg build configuration:",
        buildconf,
        "",
    ]
    texts = [("FFmpeg — GNU Lesser General Public License v2.1", ROOT / f"build/ffmpeg-src/ffmpeg-{version}/COPYING.LGPLv2.1")]
    for formula, name, _ in CODECS:
        for f in license_texts(sh("brew", "--prefix", formula)):
            texts.append((f"{name} — {f.name}", f))
    for title, path in texts:
        out += ["=" * 72, title, "=" * 72, path.read_text(errors="replace").strip(), ""]

    # Crates often share a license text verbatim; print each distinct text once with everyone using it.
    groups = {}
    for p in shipped_crates():
        files = license_texts(pathlib.Path(p["manifest_path"]).parent)
        body = "\n\n".join(f.read_text(errors="replace").strip() for f in files) or f"License: {p['license']} (no license file in the crate)"
        groups.setdefault(hashlib.sha256(body.encode()).hexdigest(), (body, []))[1].append(f"{p['name']} {p['version']} ({p['license']})")
    out += ["=" * 72, "Rust crates", "=" * 72, ""]
    for body, users in groups.values():
        out += ["-" * 72, *users, "-" * 72, body, ""]

    OUT.write_text("\n".join(out) + "\n")
    print(f"{OUT.relative_to(ROOT)}: {len(groups)} distinct crate license texts")


if __name__ == "__main__":
    main()
