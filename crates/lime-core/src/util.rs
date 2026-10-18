use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// Which input of how many is being processed, so per-file progress becomes overall progress.
static PART: Mutex<(usize, usize)> = Mutex::new((0, 1));

/// Progress lines are only written when the app asks for them, so the CLI stays quiet.
fn progress_enabled() -> bool {
    std::env::var_os("LIME_PROGRESS").is_some()
}

pub fn set_part(i: usize, n: usize) {
    *PART.lock().unwrap() = (i, n.max(1));
}

/// Reports `f` (0..1) of the current input as `progress <overall>` on stderr.
pub fn report(f: f64) {
    if progress_enabled() {
        let (i, n) = *PART.lock().unwrap();
        eprintln!("progress {:.3}", (i as f64 + f.clamp(0.0, 1.0)) / n as f64);
    }
}

/// Seconds from ffmpeg's `  Duration: 00:01:02.50, start: …` line.
fn ffmpeg_duration(line: &str) -> Option<f64> {
    let t = line.trim_start().strip_prefix("Duration: ")?.split(',').next()?;
    let mut secs = 0.0;
    for part in t.split(':') {
        secs = secs * 60.0 + part.parse::<f64>().ok()?;
    }
    (secs > 0.0).then_some(secs)
}

/// Runs ffmpeg with `-progress pipe:2`, turning its `out_time_us=` lines into progress reports.
/// Returns (success, stdout, stderr without the progress lines).
fn run_with_progress(cmd: &mut Command) -> Result<(bool, String, String)> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().with_context(|| format!("could not start {:?}", cmd.get_program()))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let (mut duration, mut err) = (None, String::new());
    for line in BufReader::new(child.stderr.take().expect("piped stderr")).lines().map_while(Result::ok) {
        if let Some(us) = line.strip_prefix("out_time_us=") {
            if let (Some(d), Ok(us)) = (duration, us.parse::<f64>()) {
                report(us / 1e6 / d);
            }
        } else if !line.contains('=') || line.contains(' ') {
            duration = duration.or_else(|| ffmpeg_duration(&line));
            err += &line;
            err.push('\n');
        }
    }
    let ok = child.wait()?.success();
    Ok((ok, reader.join().unwrap_or_default(), err))
}

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
    let (ok, stdout, err) = if progress_enabled() && cmd.get_program() == ffmpeg_path() {
        run_with_progress(cmd)?
    } else {
        let out = cmd.output().with_context(|| format!("could not start {:?}", cmd.get_program()))?;
        (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    if !ok {
        let tail: Vec<&str> = err.lines().rev().take(4).collect();
        bail!(
            "{} failed: {}",
            cmd.get_program().to_string_lossy(),
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        );
    }
    Ok(stdout)
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
    if progress_enabled() {
        c.args(["-progress", "pipe:2"]);
    }
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

    #[test]
    fn parses_ffmpeg_duration() {
        assert_eq!(ffmpeg_duration("  Duration: 00:01:02.50, start: 0.000000, bitrate: 1 kb/s"), Some(62.5));
        assert_eq!(ffmpeg_duration("  Duration: N/A, bitrate: N/A"), None);
        assert_eq!(ffmpeg_duration("Stream #0:0: Video"), None);
    }
}
