//! SRT ⇄ VTT, plain-text export and timing shifts.

use crate::formats::ext;
use crate::util::{out_path, produce};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

struct Cue {
    start: f64,
    end: f64,
    text: Vec<String>,
}

/// `00:01:02,500` or `01:02.500` → seconds.
fn parse_ts(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let mut secs = 0.0;
    for part in s.split(':') {
        secs = secs * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(secs)
}

fn fmt_ts(t: f64, sep: char) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}{sep}{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

fn parse(src: &str) -> Vec<Cue> {
    let src = src.replace("\r\n", "\n");
    let mut cues = vec![];
    for block in src.split("\n\n") {
        let mut lines = block.lines().skip_while(|l| !l.contains("-->"));
        let Some(timing) = lines.next() else { continue };
        let (a, rest) = timing.split_once("-->").unwrap();
        // VTT cue settings follow the end time.
        let b = rest.split_whitespace().next().unwrap_or("");
        if let (Some(start), Some(end)) = (parse_ts(a), parse_ts(b)) {
            cues.push(Cue { start, end, text: lines.map(String::from).collect() });
        }
    }
    cues
}

fn render(cues: &[Cue], format: &str) -> String {
    let mut out = String::new();
    if format == "vtt" {
        out.push_str("WEBVTT\n\n");
    }
    for (i, c) in cues.iter().enumerate() {
        match format {
            "srt" => out += &format!("{}\n{} --> {}\n", i + 1, fmt_ts(c.start, ','), fmt_ts(c.end, ',')),
            "vtt" => out += &format!("{} --> {}\n", fmt_ts(c.start, '.'), fmt_ts(c.end, '.')),
            _ => {}
        }
        out += &c.text.join("\n");
        out += if format == "txt" { "\n" } else { "\n\n" };
    }
    out
}

fn load(input: &Path) -> Result<Vec<Cue>> {
    let cues = parse(&std::fs::read_to_string(input)?);
    if cues.is_empty() {
        bail!("no subtitle cues found in {}", input.display());
    }
    Ok(cues)
}

pub fn convert(input: &Path, target: &str) -> Result<PathBuf> {
    let text = render(&load(input)?, target);
    produce(out_path(input, "", target), |out| Ok(std::fs::write(out, text)?))
}

pub fn shift(input: &Path, offset: &str) -> Result<PathBuf> {
    let d: f64 = offset.trim().parse().with_context(|| format!("'{offset}' is not a number of seconds"))?;
    let mut cues = load(input)?;
    for c in &mut cues {
        c.start += d;
        c.end += d;
    }
    let e = ext(input);
    produce(out_path(input, "-shifted", &e), |out| Ok(std::fs::write(out, render(&cues, &e))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str = "1\r\n00:00:01,000 --> 00:00:02,500\r\nHello\r\nthere\r\n\r\n2\r\n00:01:00,000 --> 00:01:01,000\r\nBye\r\n";

    #[test]
    fn srt_vtt_txt() {
        let cues = parse(SRT);
        assert_eq!(render(&cues, "vtt"), "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\nHello\nthere\n\n00:01:00.000 --> 00:01:01.000\nBye\n\n");
        let back = parse(&render(&cues, "vtt"));
        assert_eq!(render(&back, "srt"), SRT.replace("\r\n", "\n") + "\n");
        assert_eq!(render(&cues, "txt"), "Hello\nthere\nBye\n");
    }

    #[test]
    fn vtt_settings_and_short_timestamps() {
        let cues = parse("WEBVTT\n\nNOTE hi\n\nid1\n01:02.250 --> 01:03.000 align:start\nX\n");
        assert_eq!(cues.len(), 1);
        assert_eq!((cues[0].start, cues[0].end), (62.25, 63.0));
    }
}
