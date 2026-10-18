//! `lime` CLI. The Lime app also runs every job through it (PDFKit/AppKit need a main thread).

use anyhow::{bail, Result};
use lime_core::Wheel;
use std::path::PathBuf;

const USAGE: &str = "usage:
  lime ops convert|tools FILE...        list operations for FILEs (id<TAB>label<TAB>prompt<TAB>default)
  lime run OP [--arg VALUE] FILE...     run OP (e.g. to:webp, compress) and print created paths";

fn main() {
    if let Err(e) = real_main() {
        eprintln!("lime: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("ops") => {
            let wheel = match args.next().as_deref() {
                Some("convert") => Wheel::Convert,
                Some("tools") => Wheel::Tools,
                _ => bail!("{USAGE}"),
            };
            let files: Vec<PathBuf> = args.map(PathBuf::from).collect();
            for op in lime_core::ops(wheel, &files) {
                let (prompt, default) = op.param.map(|p| (p.prompt, p.default)).unwrap_or_default();
                println!("{}\t{}\t{prompt}\t{default}", op.id, op.label);
            }
        }
        Some("run") => {
            let op = args.next().ok_or_else(|| anyhow::anyhow!("{USAGE}"))?;
            let mut rest: Vec<String> = args.collect();
            let arg = match rest.iter().position(|a| a == "--arg") {
                Some(i) if i + 1 < rest.len() => {
                    let v = rest.remove(i + 1);
                    rest.remove(i);
                    Some(v)
                }
                Some(_) => bail!("--arg needs a value"),
                None => None,
            };
            let files: Vec<PathBuf> = rest.into_iter().map(PathBuf::from).collect();
            for p in lime_core::run(&op, &files, arg.as_deref())? {
                println!("{}", p.display());
            }
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
}
