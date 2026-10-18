use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Name without extension; `.tar.gz` counts as one extension.
pub fn stem(input: &Path) -> String {
    let name = input.file_name().and_then(|n| n.to_str()).unwrap_or("output");
    if let Some(s) = name.strip_suffix(".tar.gz").or_else(|| name.strip_suffix(".TAR.GZ")) {
        return s.into();
    }
    if input.is_dir() {
        return name.into();
    }
    input.file_stem().and_then(|s| s.to_str()).unwrap_or("output").into()
}

/// A free path next to `input`: `<stem><suffix>.<ext>`, then `<stem><suffix> 2.<ext>`, …
/// An empty `ext` yields a folder-style name.
pub fn out_path(input: &Path, suffix: &str, ext: &str) -> PathBuf {
    let dir = input.parent().unwrap_or(Path::new("."));
    let base = format!("{}{suffix}", stem(input));
    let dot = if ext.is_empty() { String::new() } else { format!(".{ext}") };
    let mut candidate = dir.join(format!("{base}{dot}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{base} {n}{dot}"));
        n += 1;
    }
    candidate
}

/// Runs a command; on failure returns the tail of its stderr as the error.
pub fn run(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().with_context(|| format!("could not start {:?}", cmd.get_program()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(4).collect();
        bail!(
            "{} failed: {}",
            cmd.get_program().to_string_lossy(),
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Bundled ffmpeg (`Lime.app/Contents/Resources/ffmpeg`), `$LIME_FFMPEG`, or `ffmpeg` on PATH.
pub fn ffmpeg_path() -> PathBuf {
    if let Some(p) = std::env::var_os("LIME_FFMPEG") {
        return p.into();
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bundled) = exe.parent().map(|d| d.join("../Resources/ffmpeg")) {
            if bundled.exists() {
                return bundled;
            }
        }
    }
    "ffmpeg".into()
}

pub fn ffmpeg() -> Command {
    let mut c = Command::new(ffmpeg_path());
    c.args(["-hide_banner", "-nostdin", "-y"]);
    c
}

/// Removes a half-written output when `f` fails, so failures never leave junk next to the input.
pub fn produce(out: PathBuf, f: impl FnOnce(&Path) -> Result<()>) -> Result<PathBuf> {
    match f(&out) {
        Ok(()) if out.exists() => Ok(out),
        Ok(()) => bail!("no output was produced"),
        Err(e) => {
            let _ = if out.is_dir() { std::fs::remove_dir_all(&out) } else { std::fs::remove_file(&out) };
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_path_avoids_collisions() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("photo.png");
        assert_eq!(out_path(&input, "", "jpg"), dir.path().join("photo.jpg"));
        std::fs::write(dir.path().join("photo.jpg"), "").unwrap();
        assert_eq!(out_path(&input, "", "jpg"), dir.path().join("photo 2.jpg"));
        assert_eq!(out_path(&dir.path().join("a.tar.gz"), "", "zip"), dir.path().join("a.zip"));
    }
}
