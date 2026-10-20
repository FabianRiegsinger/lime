//! Archives through libarchive's bsdtar (the system `tar` on macOS and Windows, `bsdtar` on Linux): zip, tar, tgz, rar, 7z.

use crate::formats::ext;
use crate::util::{out_path, produce, run};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

fn tar() -> Command {
    // GNU tar (the Linux default) cannot read or write zip, rar or 7z.
    let mut c = Command::new(if cfg!(target_os = "linux") { "bsdtar" } else { "tar" });
    // Keep macOS AppleDouble (._*) files out of archives.
    c.env("COPYFILE_DISABLE", "1");
    c
}

/// Packs `items` (all in one folder) into `out`; the format follows its extension.
fn pack(items: &[PathBuf], out: &Path) -> Result<()> {
    let dir = match items[0].parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let mut cmd = tar();
    cmd.arg("-a").arg("-cf").arg(out).arg("-C").arg(dir);
    for p in items {
        cmd.arg(p.file_name().context("bad file name")?);
    }
    run(&mut cmd).map(drop)
}

pub fn extract(input: &Path) -> Result<PathBuf> {
    produce(out_path(input, "", ""), |dir| {
        if ext(input) == "gz" {
            // A bare .gz holds one compressed file, not a tar.
            let name = input.file_stem().context("bad file name")?;
            let mut gz = flate2::read::MultiGzDecoder::new(std::fs::File::open(input)?);
            std::io::copy(&mut gz, &mut std::fs::File::create(dir.join(name))?).context("not a valid .gz file")?;
            return Ok(());
        }
        run(tar().arg("-xf").arg(input).arg("-C").arg(dir)).map(drop)
    })
}

/// Archive inputs are repacked; anything else (files, folders) is packed as-is.
pub fn convert(input: &Path, target: &str) -> Result<PathBuf> {
    let is_archive = matches!(ext(input).as_str(), "zip" | "tar" | "tgz" | "rar" | "7z");
    produce(out_path(input, "", target), |out| {
        if !is_archive {
            return pack(&[input.to_path_buf()], out);
        }
        let tmp = tempfile::tempdir()?;
        run(tar().arg("-xf").arg(input).arg("-C").arg(tmp.path()))?;
        let entries = std::fs::read_dir(tmp.path())?.map(|e| Ok(e?.path())).collect::<Result<Vec<_>>>()?;
        if entries.is_empty() {
            anyhow::bail!("archive is empty");
        }
        pack(&entries, out)
    })
}

/// One zip of all inputs: `<name>.zip` for one item, `Archive.zip` for several.
pub fn zip(inputs: &[PathBuf]) -> Result<PathBuf> {
    let first_dir = inputs[0].parent();
    if inputs.iter().any(|p| p.parent() != first_dir) {
        anyhow::bail!("zip needs all items in the same folder");
    }
    let out = if inputs.len() == 1 {
        out_path(&inputs[0], "", "zip")
    } else {
        out_path(&inputs[0].with_file_name("Archive"), "", "zip")
    };
    produce(out, |out| pack(inputs, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_repack_extract() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("stuff");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("a.txt"), "hello").unwrap();
        let z = zip(&[folder.clone()]).unwrap();
        assert_eq!(z, dir.path().join("stuff.zip"));
        let tgz = convert(&z, "tgz").unwrap();
        assert_eq!(tgz, dir.path().join("stuff.tgz"));
        let out = extract(&tgz).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("stuff/a.txt")).unwrap(), "hello");
    }

    #[test]
    fn extracts_bare_gz() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let gz = dir.path().join("note.txt.gz");
        let mut enc = flate2::write::GzEncoder::new(std::fs::File::create(&gz).unwrap(), flate2::Compression::default());
        enc.write_all(b"hello").unwrap();
        enc.finish().unwrap();
        let out = extract(&gz).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("note.txt")).unwrap(), "hello");
    }
}
