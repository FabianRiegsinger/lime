//! Images (sips on macOS, with ffmpeg for WebP; ffmpeg only elsewhere) and audio/video (ffmpeg).

use crate::formats::{ext, kind, Kind};
use crate::util::{ffmpeg, out_path, produce, run, stem};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------- images ----------

/// `sips` ships with macOS only; elsewhere every image goes through ffmpeg.
const SIPS: bool = cfg!(target_os = "macos");

/// sips output format name, when sips can write `ext`.
fn sips_format(ext: &str) -> Option<&'static str> {
    if !SIPS {
        return None;
    }
    Some(match ext {
        "jpg" => "jpeg",
        "png" => "png",
        "heic" => "heic",
        "tiff" | "tif" => "tiff",
        "avif" => "avif",
        "bmp" => "bmp",
        "gif" => "gif",
        "pdf" => "pdf",
        "jp2" => "jp2",
        _ => return None,
    })
}

/// Inputs ffmpeg decodes directly; on macOS others go through a sips PNG first.
fn ffmpeg_reads(ext: &str) -> bool {
    !SIPS || matches!(ext, "jpg" | "png" | "webp" | "bmp" | "tiff" | "tif" | "gif")
}

/// ffmpeg encoder args for an image written as `ext`.
fn ffmpeg_args(ext: &str) -> &'static [&'static str] {
    match ext {
        "webp" => WEBP,
        "jpg" => &["-q:v", "2"],
        _ => &[],
    }
}

/// Formats ffmpeg writes as images; edits of other formats become PNG.
fn ffmpeg_writes(ext: &str) -> bool {
    matches!(ext, "jpg" | "png" | "webp" | "bmp" | "tiff" | "gif")
}

fn sips(input: &Path, out: &Path, args: &[&str]) -> Result<()> {
    run(Command::new("sips").args(args).arg(input).arg("--out").arg(out)).map(drop)
}

/// Runs ffmpeg on an image, decoding unsupported inputs (HEIC, AVIF, SVG…) via sips first.
fn ffmpeg_image(input: &Path, out: &Path, args: &[&str]) -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let src = if ffmpeg_reads(&ext(input)) {
        input.to_path_buf()
    } else {
        let png = tmp.path().join("src.png");
        sips(input, &png, &["-s", "format", "png"])?;
        png
    };
    run(ffmpeg().arg("-i").arg(&src).args(args).arg(out)).map(drop)
}

const WEBP: &[&str] = &["-c:v", "libwebp", "-quality", "85"];

pub fn convert_image(input: &Path, target: &str) -> Result<PathBuf> {
    produce(out_path(input, "", target), |out| match sips_format(target) {
        Some(fmt) => sips(input, out, &["-s", "format", fmt]),
        None => ffmpeg_image(input, out, ffmpeg_args(target)),
    })
}

fn dims(input: &Path) -> Result<(u32, u32)> {
    let text = run(Command::new("sips").args(["-g", "pixelWidth", "-g", "pixelHeight"]).arg(input))?;
    let get = |key: &str| -> Result<u32> {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key))
            .and_then(|v| v.trim().parse().ok())
            .with_context(|| format!("could not read image {key}"))
    };
    Ok((get("pixelWidth:")?, get("pixelHeight:")?))
}

/// Applies an edit expressed both as sips args and as an equivalent ffmpeg filter.
/// Keeps the input format when possible; formats nobody can write (SVG, JXL) become PNG.
fn edit_image(input: &Path, suffix: &str, sips_args: &[&str], filter: &str) -> Result<PathBuf> {
    let e = ext(input);
    if e == "webp" || !SIPS {
        let out_ext = if ffmpeg_writes(&e) { e.as_str() } else { "png" };
        let mut args = vec!["-vf", filter];
        args.extend_from_slice(ffmpeg_args(out_ext));
        return produce(out_path(input, suffix, out_ext), |out| ffmpeg_image(input, out, &args));
    }
    let (out_ext, mut args) = match sips_format(&e) {
        Some(_) => (e.as_str(), vec![]),
        None => ("png", vec!["-s", "format", "png"]),
    };
    args.extend_from_slice(sips_args);
    produce(out_path(input, suffix, out_ext), |out| sips(input, out, &args))
}

fn number(arg: Option<&str>, min: f64, max: f64) -> Result<f64> {
    let s = arg.unwrap_or("").trim().trim_end_matches('%');
    let n: f64 = s.parse().with_context(|| format!("'{s}' is not a number"))?;
    if !(min..=max).contains(&n) {
        bail!("{n} is outside {min}-{max}");
    }
    Ok(n)
}

pub fn image_tool(op: &str, input: &Path, arg: Option<&str>) -> Result<Vec<PathBuf>> {
    let e = ext(input);
    let out = match op {
        "compress" => {
            let q = number(arg, 1.0, 100.0)?;
            let qs = format!("{q:.0}");
            match e.as_str() {
                "jpg" | "heic" | "avif" if SIPS => produce(out_path(input, "-compressed", &e), |out| {
                    sips(input, out, &["-s", "format", sips_format(&e).unwrap(), "-s", "formatOptions", &qs])
                })?,
                "webp" => produce(out_path(input, "-compressed", "webp"), |out| {
                    ffmpeg_image(input, out, &["-c:v", "libwebp", "-quality", &qs])
                })?,
                // Lossless PNG: quantize to a 256-colour palette (keeps transparency), like pngquant.
                "png" => produce(out_path(input, "-compressed", "png"), |out| {
                    ffmpeg_image(input, out, &["-vf", "split[a][b];[a]palettegen[p];[b][p]paletteuse"])
                })?,
                // shortcut: other formats have no quality knob, so they become JPEG; upgrade per-format if users mind.
                _ if SIPS => produce(out_path(input, "-compressed", "jpg"), |out| {
                    sips(input, out, &["-s", "format", "jpeg", "-s", "formatOptions", &qs])
                })?,
                // ffmpeg's JPEG quality runs from 2 (best) to 31.
                _ => produce(out_path(input, "-compressed", "jpg"), |out| {
                    let qv = format!("{:.0}", 31.0 - (q - 1.0) * 29.0 / 99.0);
                    ffmpeg_image(input, out, &["-q:v", &qv])
                })?,
            }
        }
        "resize" => {
            let pct = number(arg, 1.0, 1000.0)?;
            // Only sips needs the size up front; ffmpeg computes it from the filter.
            let nw = if SIPS { ((dims(input)?.0 as f64 * pct / 100.0).round() as u32).max(1).to_string() } else { String::new() };
            edit_image(input, "-resized", &["--resampleWidth", &nw], &format!("scale='max(1,round(iw*{pct}/100))':-1"))?
        }
        "rotate" => edit_image(input, "-rotated", &["-r", "90"], "transpose=1")?,
        "flip" => edit_image(input, "-flipped", &["-f", "horizontal"], "hflip")?,
        "square" => {
            let s = if SIPS { let (w, h) = dims(input)?; w.min(h).to_string() } else { String::new() };
            edit_image(input, "-square", &["-c", &s, &s], "crop='min(iw,ih)':'min(iw,ih)'")?
        }
        "strip" => produce(out_path(input, "-clean", &e), |out| {
            let mut args = vec!["-map_metadata", "-1"];
            match e.as_str() {
                "jpg" => args.extend(["-q:v", "2"]),
                "webp" => args.extend(["-c:v", "libwebp", "-lossless", "1"]),
                _ => {}
            }
            ffmpeg_image(input, out, &args)
        })?,
        _ => bail!("unknown image tool '{op}'"),
    };
    Ok(vec![out])
}

// ---------- audio / video ----------

/// ffmpeg encoder arguments for an output extension.
fn encode_args(target: &str) -> Result<Vec<&'static str>> {
    // VideoToolbox's constant-quality mode exists only on Apple silicon; Intel Macs get a bitrate.
    // shortcut: a fixed 8 Mbit/s on Intel, regardless of resolution; derive it from the input size if that matters.
    let h264_rate: [&str; 2] = if cfg!(target_arch = "aarch64") { ["-q:v", "65"] } else { ["-b:v", "8M"] };
    let h264 = ["-c:v", "h264_videotoolbox", "-allow_sw", "1", h264_rate[0], h264_rate[1], "-pix_fmt", "yuv420p", "-c:a", "aac_at", "-b:a", "192k"];
    let video: &[&str] = match target {
        "mp4" | "mov" | "m4v" | "mkv" => &h264,
        "webm" => &["-c:v", "libvpx-vp9", "-crf", "32", "-b:v", "0", "-row-mt", "1", "-deadline", "good", "-cpu-used", "4", "-c:a", "libopus"],
        // Motion JPEG and MP3 are patent-free; ffmpeg's MPEG-4 Part 2 encoder is left out of the build.
        "avi" => &["-c:v", "mjpeg", "-q:v", "3", "-pix_fmt", "yuvj420p", "-c:a", "libmp3lame", "-q:a", "4"],
        "wmv" => &["-c:v", "wmv2", "-b:v", "3M", "-c:a", "wmav2", "-b:a", "192k"],
        "gif" => &["-vf", "fps=12,scale='min(640,iw)':-1:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse", "-loop", "0", "-an"],
        _ => &[],
    };
    if !video.is_empty() {
        let mut v = video.to_vec();
        if matches!(target, "mp4" | "mov" | "m4v") {
            v.extend(["-movflags", "+faststart"]);
        }
        return Ok(v);
    }
    let audio: &[&str] = match target {
        "mp3" => &["-c:a", "libmp3lame", "-q:a", "2"],
        "m4a" | "aac" | "caf" => &["-c:a", "aac_at", "-b:a", "192k"],
        "wav" => &["-c:a", "pcm_s16le"],
        "flac" => &["-c:a", "flac"],
        "ogg" => &["-c:a", "libvorbis", "-q:a", "5"],
        "opus" => &["-c:a", "libopus", "-b:a", "128k"],
        "aiff" => &["-c:a", "pcm_s16be"],
        "wma" => &["-c:a", "wmav2", "-b:a", "192k"],
        _ => bail!("no encoder for .{target}"),
    };
    let mut v = vec!["-vn"];
    v.extend_from_slice(audio);
    Ok(v)
}

pub fn convert_av(input: &Path, target: &str) -> Result<PathBuf> {
    let args = encode_args(target)?;
    produce(out_path(input, "", target), |out| run(ffmpeg().arg("-i").arg(input).args(&args).arg(out)).map(drop))
}

struct Probe {
    seconds: f64,
    kbps: Option<f64>,
}

/// Duration and overall bitrate, parsed from `ffmpeg -i` (no ffprobe in the bundle).
fn probe(input: &Path) -> Result<Probe> {
    let out = ffmpeg().arg("-i").arg(input).output()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let line = text.lines().find(|l| l.trim_start().starts_with("Duration:")).context("could not read media duration")?;
    let dur = line.trim_start().trim_start_matches("Duration:").split(',').next().unwrap_or("").trim();
    let seconds = parse_time(dur)?;
    let kbps = line.split("bitrate:").nth(1).and_then(|b| b.trim().split(' ').next()?.parse().ok());
    Ok(Probe { seconds, kbps })
}

/// `90`, `1:30`, `0:01:30.5` → seconds.
fn parse_time(s: &str) -> Result<f64> {
    let mut secs = 0.0;
    for part in s.trim().split(':') {
        let v: f64 = part.trim().parse().with_context(|| format!("'{s}' is not a time"))?;
        secs = secs * 60.0 + v;
    }
    Ok(secs)
}

fn parse_range(s: &str) -> Result<(f64, f64)> {
    let (a, b) = s.split_once('-').with_context(|| format!("'{s}' is not a range like 0:05-1:30"))?;
    let (a, b) = (parse_time(a)?, parse_time(b)?);
    if b <= a {
        bail!("range end must be after its start");
    }
    Ok((a, b))
}

fn ff(input: &Path, out: &Path, args: &[&str]) -> Result<()> {
    run(ffmpeg().arg("-i").arg(input).args(args).arg(out)).map(drop)
}

pub fn av_tool(op: &str, input: &Path, arg: Option<&str>) -> Result<Vec<PathBuf>> {
    let e = ext(input);
    let is_video = kind(input) == Kind::Video;
    let out = match op {
        "compress" if is_video => {
            let pct = number(arg, 1.0, 100.0)?;
            let p = probe(input)?;
            let total = p.kbps.context("could not read the video bitrate")?;
            let video_kbps = ((total * pct / 100.0) - 128.0).max(100.0);
            let target = if matches!(e.as_str(), "mp4" | "mov" | "m4v" | "mkv") { e.as_str() } else { "mp4" };
            let vb = format!("{video_kbps:.0}k");
            produce(out_path(input, "-compressed", target), |out| {
                ff(input, out, &["-c:v", "h264_videotoolbox", "-allow_sw", "1", "-b:v", &vb, "-pix_fmt", "yuv420p", "-c:a", "aac_at", "-b:a", "128k"])
            })?
        }
        "compress" => {
            let kbps = format!("{:.0}k", number(arg, 8.0, 512.0)?);
            let (target, codec) = match e.as_str() {
                "mp3" => ("mp3", "libmp3lame"),
                "m4a" | "aac" => (e.as_str(), "aac_at"),
                "opus" => ("opus", "libopus"),
                "ogg" => ("ogg", "libvorbis"),
                _ => ("mp3", "libmp3lame"),
            };
            produce(out_path(input, "-compressed", target), |out| ff(input, out, &["-vn", "-c:a", codec, "-b:a", &kbps]))?
        }
        "trim" => {
            let (a, b) = parse_range(arg.unwrap_or(""))?;
            let (a, b) = (a.to_string(), b.to_string());
            // shortcut: stream copy cuts on keyframes (fast, lossless); re-encode if frame accuracy is needed.
            produce(out_path(input, "-trimmed", &e), |out| {
                run(ffmpeg().args(["-ss", &a, "-to", &b, "-i"]).arg(input).args(["-map", "0", "-c", "copy"]).arg(out)).map(drop)
            })?
        }
        "split" => {
            let parts = number(arg, 2.0, 100.0)?;
            let seg = format!("{:.3}", probe(input)?.seconds / parts.round());
            produce(out_path(input, " parts", ""), |dir| {
                let pattern = dir.join(format!("{}-%02d.{e}", stem(input)));
                ff(input, &pattern, &["-map", "0", "-c", "copy", "-f", "segment", "-segment_time", &seg, "-reset_timestamps", "1"])
            })?
        }
        "mute" => produce(out_path(input, "-muted", &e), |out| ff(input, out, &["-c:v", "copy", "-an"]))?,
        "rotate" | "720p" => {
            let filter = if op == "rotate" { "transpose=1" } else { "scale=-2:'min(720,ih)'" };
            let mut args = vec!["-vf", filter];
            args.extend(encode_args(&e)?);
            produce(out_path(input, &format!("-{op}"), &e), |out| ff(input, out, &args))?
        }
        "normalize" => {
            let mut args = vec!["-af", "loudnorm"];
            args.extend(encode_args(&e)?);
            produce(out_path(input, "-normalized", &e), |out| ff(input, out, &args))?
        }
        "strip" => produce(out_path(input, "-clean", &e), |out| {
            ff(input, out, &["-map", "0", "-map_metadata", "-1", "-map_chapters", "-1", "-c", "copy"])
        })?,
        _ => bail!("unknown tool '{op}'"),
    };
    Ok(vec![out])
}

pub fn merge(inputs: &[PathBuf]) -> Result<PathBuf> {
    let e = ext(&inputs[0]);
    let tmp = tempfile::tempdir()?;
    let list = tmp.path().join("list.txt");
    let body: String = inputs
        .iter()
        .map(|p| Ok(format!("file '{}'\n", p.canonicalize()?.display().to_string().replace('\'', "'\\''"))))
        .collect::<Result<_>>()?;
    std::fs::write(&list, body)?;
    let args = encode_args(&e)?;
    // shortcut: concat demuxer assumes matching resolution/sample rate across inputs; use the concat filter if mixed sources matter.
    produce(out_path(&inputs[0], "-merged", &e), |out| {
        run(ffmpeg().args(["-f", "concat", "-safe", "0", "-i"]).arg(&list).args(&args).arg(out)).map(drop)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_ranges() {
        assert_eq!(parse_time("90").unwrap(), 90.0);
        assert_eq!(parse_time("1:30").unwrap(), 90.0);
        assert_eq!(parse_time("01:01:30.5").unwrap(), 3690.5);
        assert_eq!(parse_range("0:05-1:00").unwrap(), (5.0, 60.0));
        assert!(parse_range("10-5").is_err());
        assert!(parse_range("abc").is_err());
    }
}
