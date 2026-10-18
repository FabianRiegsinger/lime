//! Lime engine: lists the operations available for a set of files and runs them.
//! Every operation writes new files next to its input; inputs are never modified.

mod archive;
mod document;
mod formats;
mod media;
#[cfg(target_os = "macos")]
mod pdf;
mod sheet;
mod subtitle;
mod util;

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

pub use formats::{kind, Kind};
pub use util::ffmpeg_path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wheel {
    Convert,
    Tools,
}

/// A value the user must supply before a tool runs, e.g. a time range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub prompt: &'static str,
    pub default: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Op {
    /// `to:<ext>` for conversions, a tool name otherwise.
    pub id: String,
    pub label: String,
    pub param: Option<Param>,
}

fn tool(id: &str, label: &str) -> Op {
    Op { id: id.into(), label: label.into(), param: None }
}

fn tool_with(id: &str, label: &str, prompt: &'static str, default: &'static str) -> Op {
    Op { id: id.into(), label: label.into(), param: Some(Param { prompt, default }) }
}

/// Operations offered on `wheel` for `inputs`. Empty when nothing applies.
pub fn ops(wheel: Wheel, inputs: &[PathBuf]) -> Vec<Op> {
    if inputs.is_empty() {
        return vec![];
    }
    let kinds: Vec<Kind> = inputs.iter().map(|p| kind(p)).collect();
    match wheel {
        Wheel::Convert => {
            // Only targets every input can reach.
            let mut targets = formats::targets(&inputs[0], kinds[0]);
            for (p, k) in inputs.iter().zip(&kinds).skip(1) {
                let other = formats::targets(p, *k);
                targets.retain(|t| other.contains(t));
            }
            targets
                .into_iter()
                .map(|t| Op { id: format!("to:{t}"), label: t.to_uppercase(), param: None })
                .collect()
        }
        Wheel::Tools => {
            let same = kinds.iter().all(|k| *k == kinds[0]);
            let multi = inputs.len() > 1;
            let mut out = if same { tools_for(kinds[0], &inputs[0], multi) } else { vec![] };
            out.push(tool("zip", "Zip"));
            out
        }
    }
}

fn tools_for(kind: Kind, first: &Path, multi: bool) -> Vec<Op> {
    let ext = formats::ext(first);
    let mut v = match kind {
        Kind::Image => {
            let mut v = vec![
                tool_with("compress", "Compress", "Quality (1-100)", "70"),
                tool_with("resize", "Resize", "Scale (%)", "50"),
                tool("rotate", "Rotate"),
                tool("flip", "Flip"),
                tool("square", "Crop Square"),
            ];
            if !matches!(ext.as_str(), "heic" | "avif" | "svg") {
                v.push(tool("strip", "Strip Metadata"));
            }
            v
        }
        Kind::Video => vec![
            tool_with("compress", "Compress", "Target size (% of original)", "50"),
            tool_with("trim", "Trim", "Time range (start-end, e.g. 0:05-1:30)", "0:00-0:10"),
            tool_with("split", "Split", "Number of parts", "2"),
            tool("mute", "Mute"),
            tool("rotate", "Rotate"),
            tool("720p", "720p"),
            tool("strip", "Strip Metadata"),
        ],
        Kind::Audio => vec![
            tool_with("compress", "Compress", "Bitrate (kbps)", "128"),
            tool_with("trim", "Trim", "Time range (start-end, e.g. 0:05-1:30)", "0:00-0:30"),
            tool_with("split", "Split", "Number of parts", "2"),
            tool("normalize", "Normalize"),
            tool("strip", "Strip Metadata"),
        ],
        Kind::Pdf => vec![
            tool("compress", "Compress"),
            tool("split", "Split Pages"),
            tool_with("extract", "Extract Pages", "Pages (e.g. 1-3,5)", "1"),
            tool("rotate", "Rotate"),
        ],
        Kind::Subtitle => vec![tool_with("shift", "Shift Timing", "Offset in seconds (e.g. -1.5)", "1.0")],
        Kind::Archive => vec![tool("extract", "Extract")],
        Kind::Document | Kind::Spreadsheet | Kind::Other => vec![],
    };
    if multi {
        match kind {
            Kind::Audio | Kind::Video | Kind::Pdf => v.push(tool("merge", "Merge")),
            Kind::Image => v.push(tool("merge", "Combine PDF")),
            _ => {}
        }
    }
    if cfg!(not(target_os = "macos")) && kind == Kind::Pdf {
        v.clear();
    }
    v
}

/// Runs `op_id` on `inputs`; returns the files it created.
pub fn run(op_id: &str, inputs: &[PathBuf], arg: Option<&str>) -> Result<Vec<PathBuf>> {
    if inputs.is_empty() {
        bail!("no input files");
    }
    for p in inputs {
        if !p.exists() {
            bail!("{} does not exist", p.display());
        }
    }
    if let Some(target) = op_id.strip_prefix("to:") {
        return inputs.iter().map(|p| convert(p, target)).collect();
    }
    let op = ops(Wheel::Tools, inputs)
        .into_iter()
        .find(|o| o.id == op_id)
        .ok_or_else(|| anyhow::anyhow!("tool '{op_id}' is not available for these files"))?;
    let arg = arg.or(op.param.as_ref().map(|p| p.default));
    if op_id == "zip" {
        return Ok(vec![archive::zip(inputs)?]);
    }
    if op_id == "merge" {
        return Ok(vec![match kind(&inputs[0]) {
            Kind::Audio | Kind::Video => media::merge(inputs)?,
            #[cfg(target_os = "macos")]
            Kind::Pdf => pdf::merge(inputs)?,
            #[cfg(target_os = "macos")]
            Kind::Image => pdf::from_images(inputs)?,
            _ => unreachable!("merge only offered for media, PDF and images"),
        }]);
    }
    let mut out = vec![];
    for p in inputs {
        out.extend(match kind(p) {
            Kind::Image => media::image_tool(op_id, p, arg)?,
            Kind::Audio | Kind::Video => media::av_tool(op_id, p, arg)?,
            #[cfg(target_os = "macos")]
            Kind::Pdf => pdf::tool(op_id, p, arg)?,
            Kind::Subtitle => vec![subtitle::shift(p, arg.unwrap_or("0"))?],
            Kind::Archive => vec![archive::extract(p)?],
            _ => unreachable!("tools_for offers no other tools"),
        });
    }
    Ok(out)
}

fn convert(input: &Path, target: &str) -> Result<PathBuf> {
    let k = kind(input);
    if !formats::targets(input, k).contains(&target) {
        bail!("cannot convert {} to {target}", input.display());
    }
    match k {
        Kind::Image => media::convert_image(input, target),
        Kind::Audio | Kind::Video => media::convert_av(input, target),
        #[cfg(target_os = "macos")]
        Kind::Pdf => pdf::convert(input, target),
        Kind::Document => document::convert(input, target),
        Kind::Spreadsheet => sheet::convert(input, target),
        Kind::Subtitle => subtitle::convert(input, target),
        Kind::Archive | Kind::Other => archive::convert(input, target),
        #[cfg(not(target_os = "macos"))]
        Kind::Pdf => bail!("PDF conversion needs macOS"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(w: Wheel, files: &[&str]) -> Vec<String> {
        let v: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
        ops(w, &v).into_iter().map(|o| o.id).collect()
    }

    #[test]
    fn convert_targets_exclude_own_format_and_intersect() {
        let one = ids(Wheel::Convert, &["a.png"]);
        assert!(one.contains(&"to:jpg".into()) && !one.contains(&"to:png".into()));
        let both = ids(Wheel::Convert, &["a.png", "b.jpg"]);
        assert!(!both.contains(&"to:png".into()) && !both.contains(&"to:jpg".into()));
        assert!(both.contains(&"to:webp".into()));
        assert!(ids(Wheel::Convert, &["a.png", "b.mp3"]).is_empty());
    }

    #[test]
    fn tools_depend_on_kind_and_count() {
        assert!(ids(Wheel::Tools, &["a.mp4"]).contains(&"trim".into()));
        assert!(!ids(Wheel::Tools, &["a.mp4"]).contains(&"merge".into()));
        assert!(ids(Wheel::Tools, &["a.mp4", "b.mov"]).contains(&"merge".into()));
        assert_eq!(ids(Wheel::Tools, &["a.mp4", "b.png"]), vec!["zip".to_string()]);
    }
}
