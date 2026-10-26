//! Archives through libarchive's bsdtar (the system `tar` on macOS and Windows, `bsdtar` on Linux): zip, tar, tgz, rar, 7z.

use crate::formats::ext;
use crate::util::{out_path, produce, run, run_secret};
use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tar() -> Command {
    // GNU tar (the Linux default) cannot read or write zip, rar or 7z.
    let mut c = Command::new(if cfg!(target_os = "linux") { "bsdtar" } else { "tar" });
    // Keep macOS AppleDouble (._*) files out of archives.
    c.env("COPYFILE_DISABLE", "1");
    c
}

/// The folder `items` share and their names in it.
fn names(items: &[PathBuf]) -> Result<(&Path, Vec<&OsStr>)> {
    let dir = match items[0].parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    Ok((dir, items.iter().map(|p| p.file_name().context("bad file name")).collect::<Result<_>>()?))
}

/// bsdtar packing `items` (all in one folder) into `out`, the format following its extension; `options` go to `--options`.
fn pack_cmd(items: &[PathBuf], out: &Path, options: &str) -> Result<Command> {
    let (dir, names) = names(items)?;
    let mut cmd = tar();
    if !options.is_empty() {
        cmd.arg("--options").arg(options);
    }
    cmd.arg("-a").arg("-cf").arg(out).arg("-C").arg(dir).args(names);
    Ok(cmd)
}

fn pack(items: &[PathBuf], out: &Path) -> Result<()> {
    run(&mut pack_cmd(items, out, "")?).map(drop)
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

/// Where one archive of all inputs goes: `<name>.<ext>` for one item, `Archive.<ext>` for several.
fn archive_out(inputs: &[PathBuf], ext: &str) -> Result<PathBuf> {
    let first_dir = inputs[0].parent();
    if inputs.iter().any(|p| p.parent() != first_dir) {
        anyhow::bail!("archives need all items in the same folder");
    }
    Ok(if inputs.len() == 1 { out_path(&inputs[0], "", ext) } else { out_path(&inputs[0].with_file_name("Archive"), "", ext) })
}

/// One zip of all inputs.
pub fn zip(inputs: &[PathBuf]) -> Result<PathBuf> {
    produce(archive_out(inputs, "zip")?, |out| pack(inputs, out))
}

/// One archive of all inputs in `format` (a `pack:` id from `compress_ops`). `arg` is the
/// level, part size or password the format asks for.
pub fn compress(inputs: &[PathBuf], format: &str, arg: Option<&str>) -> Result<Vec<PathBuf>> {
    let arg = arg.unwrap_or("").trim();
    let password = || if arg.is_empty() { anyhow::bail!("the password is empty") } else { Ok(arg) };
    let ext = match format {
        "zip-aes" | "zip-split" => "zip",
        "dmg-aes" => "dmg",
        f => f,
    };
    // Check the arguments before reserving the output path.
    let level = (format == "zip").then(|| arg.parse::<u8>().ok().filter(|l| *l <= 9).context("the level must be 0-9")).transpose()?;
    let part_mb = (format == "zip-split").then(|| arg.parse::<f64>().ok().filter(|mb| *mb > 0.0).context("the part size must be a number of MB")).transpose()?;
    let pw = if format.ends_with("-aes") { password()? } else { "" };
    if format == "iso" && !(inputs.len() == 1 && inputs[0].is_dir()) {
        anyhow::bail!("ISO needs a single folder");
    }
    let out = archive_out(inputs, ext)?;
    let made = produce(out, |out| match format {
        "zip" => run(&mut pack_cmd(inputs, out, &format!("zip:compression-level={}", level.unwrap_or(9)))?).map(drop),
        "zip-aes" => run_secret(&mut pack_cmd(inputs, out, "zip:encryption=aes256")?, pw).map(drop),
        "dmg" | "dmg-aes" => {
            let (dir, names) = names(inputs)?;
            let mut cmd = Command::new("hdiutil");
            cmd.args(["create", "-quiet", "-ov", "-format", "UDZO"]);
            for n in names {
                cmd.arg("-srcfolder").arg(dir.join(n));
            }
            cmd.arg(out);
            if format == "dmg" {
                return run(&mut cmd).map(drop);
            }
            run_secret(cmd.args(["-encryption", "AES-256", "-stdinpass"]), pw).map(drop)
        }
        "iso" => run(Command::new("hdiutil").args(["makehybrid", "-quiet", "-ov", "-iso", "-joliet", "-o"]).arg(out).arg(&inputs[0])).map(drop),
        "aar" => {
            let (dir, names) = names(inputs)?;
            let mut cmd = Command::new("aa");
            cmd.arg("archive").arg("-d").arg(dir).arg("-o").arg(out);
            // aa honours only one -include-path, and it matches prefixes; an anchored regex takes exactly these items.
            let mut alts = vec![];
            for n in names {
                let n = n.to_str().context("file name is not UTF-8")?;
                alts.push(n.chars().map(|c| if "\\.[]()*+?{}|^$".contains(c) { format!("\\{c}") } else { c.into() }).collect::<String>());
            }
            run(cmd.arg("-include-regex").arg(format!("^({})(/|$)", alts.join("|")))).map(drop)
        }
        _ => pack(inputs, out),
    })?;
    match part_mb {
        Some(mb) => {
            let parts = split(&made, (mb * 1e6) as u64);
            let _ = std::fs::remove_file(&made);
            parts
        }
        None => Ok(vec![made]),
    }
}

/// Cuts `file` into `<file>.001`, `<file>.002`, … of `size` bytes each; Keka and 7-Zip join them again.
fn split(file: &Path, size: u64) -> Result<Vec<PathBuf>> {
    let mut src = std::fs::File::open(file)?;
    let mut parts = vec![];
    let result = (|| loop {
        let mut name = file.as_os_str().to_owned();
        name.push(format!(".{:03}", parts.len() + 1));
        let part = PathBuf::from(name);
        let mut dst = std::fs::File::create_new(&part).with_context(|| format!("{} already exists", part.display()))?;
        parts.push(part);
        if std::io::copy(&mut (&mut src).take(size.max(1)), &mut dst)? == 0 {
            std::fs::remove_file(parts.pop().expect("just pushed"))?;
            return Ok(());
        }
    })();
    if let Err(e) = result {
        for p in &parts {
            let _ = std::fs::remove_file(p);
        }
        return Err(e);
    }
    Ok(parts)
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
    fn compress_formats() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "hello".repeat(1000)).unwrap();
        std::fs::write(&b, "world").unwrap();
        let both = [a.clone(), b.clone()];
        for f in ["7z", "tar.xz", "tar.zst", "zip"] {
            let out = compress(&both, f, Some("5")).unwrap();
            assert_eq!(out, vec![dir.path().join(format!("Archive.{f}"))], "{f}");
            let x = extract(&out[0]).unwrap();
            assert_eq!(std::fs::read_to_string(x.join("b.txt")).unwrap(), "world", "{f}");
        }
        assert!(compress(&both, "zip", Some("12")).is_err());

        let parts = compress(&[a.clone()], "zip-split", Some("0.00005")).unwrap();
        assert!(parts.len() > 1 && parts[0].to_string_lossy().ends_with("a.zip.001"), "{parts:?}");
        assert!(!dir.path().join("a.zip").exists());

        if cfg!(unix) {
            assert!(compress(&[b.clone()], "zip-aes", Some("")).is_err());
            let z = compress(&[b.clone()], "zip-aes", Some("pw")).unwrap().remove(0);
            let read = |pw: &str| tar().arg("-xOf").arg(&z).arg("--passphrase").arg(pw).output().unwrap();
            assert_eq!(read("pw").stdout, b"world");
            assert!(!read("nope").status.success());
        }
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
