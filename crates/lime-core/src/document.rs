//! Text documents via macOS `textutil`, plus Markdown and EPUB in and out.

use crate::formats::ext;
use crate::util::{out_path, produce, run, stem};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Files textutil should read for `input`, and whether they must be read as HTML.
fn sources(input: &Path, tmp: &Path) -> Result<(Vec<PathBuf>, bool)> {
    match ext(input).as_str() {
        "md" | "markdown" => {
            let md = std::fs::read_to_string(input)?;
            let mut html = String::from("<html><head><meta charset=\"utf-8\"></head><body>");
            pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new(&md));
            html.push_str("</body></html>");
            let p = tmp.join("doc.html");
            std::fs::write(&p, html)?;
            Ok((vec![p], false))
        }
        "epub" => Ok((epub_chapters(input, tmp)?, true)),
        // A Finder text clipping is a property list holding the text under its type identifier.
        // shortcut: plain text only; older clippings that keep their text in a resource fork are rejected.
        "textclipping" => {
            let p = tmp.join("doc.txt");
            run(Command::new("plutil").args(["-extract", r"UTI-Data.public\.utf8-plain-text", "raw", "-o"]).arg(&p).arg(input))
                .context("text clipping holds no plain text")?;
            Ok((vec![p], false))
        }
        _ => Ok((vec![input.to_path_buf()], false)),
    }
}

/// Concatenates `input` into `format` (a textutil format) at `out`.
fn textutil(input: &Path, tmp: &Path, format: &str, out: &Path) -> Result<()> {
    let (files, html) = sources(input, tmp)?;
    let mut cmd = Command::new("textutil");
    if html {
        cmd.args(["-format", "html"]);
    }
    cmd.args(["-cat", format, "-output"]).arg(out).args(files);
    run(&mut cmd).map(drop)
}

pub fn convert(input: &Path, target: &str) -> Result<PathBuf> {
    let tmp = tempfile::tempdir()?;
    let tmp = tmp.path();
    produce(out_path(input, "", target), |out| match target {
        "docx" | "odt" | "rtf" | "html" | "txt" => textutil(input, tmp, target, out),
        // shortcut: Markdown export is the plain text (valid Markdown, no formatting); add an HTML→MD pass if styling matters.
        "md" => textutil(input, tmp, "txt", out),
        "epub" => {
            let txt = tmp.join("doc.txt");
            textutil(input, tmp, "txt", &txt)?;
            write_epub(&stem(input), &std::fs::read_to_string(txt)?, out)
        }
        #[cfg(target_os = "macos")]
        "pdf" => {
            let html = tmp.join("doc.html");
            textutil(input, tmp, "html", &html)?;
            crate::pdf::render_document(&html, out)
        }
        _ => bail!("cannot convert document to {target}"),
    })
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Minimal valid EPUB 3: one XHTML chapter, one paragraph per text line.
pub fn write_epub(title: &str, text: &str, out: &Path) -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let d = tmp.path();
    let title = escape(title);
    let body: String = text.lines().filter(|l| !l.trim().is_empty()).map(|l| format!("<p>{}</p>\n", escape(l))).collect();
    std::fs::create_dir_all(d.join("META-INF"))?;
    std::fs::create_dir_all(d.join("OEBPS"))?;
    std::fs::write(d.join("mimetype"), "application/epub+zip")?;
    std::fs::write(
        d.join("META-INF/container.xml"),
        r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
    )?;
    let id = format!("urn:lime:{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis());
    std::fs::write(
        d.join("OEBPS/content.opf"),
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">{id}</dc:identifier><dc:title>{title}</dc:title><dc:language>en</dc:language><meta property="dcterms:modified">2000-01-01T00:00:00Z</meta></metadata>
<manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="text" href="text.xhtml" media-type="application/xhtml+xml"/></manifest>
<spine><itemref idref="text"/></spine>
</package>"#
        ),
    )?;
    let page = |inner: &str| {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>{title}</title></head><body>{inner}</body></html>"
        )
    };
    std::fs::write(d.join("OEBPS/nav.xhtml"), page(&format!("<nav epub:type=\"toc\"><ol><li><a href=\"text.xhtml\">{title}</a></li></ol></nav>")))?;
    std::fs::write(d.join("OEBPS/text.xhtml"), page(&format!("<h1>{title}</h1>\n{body}")))?;
    let out = std::path::absolute(out)?;
    // mimetype must be the first entry, stored uncompressed.
    run(Command::new("zip").current_dir(d).args(["-X0q"]).arg(&out).arg("mimetype"))?;
    run(Command::new("zip").current_dir(d).args(["-Xr9q"]).arg(&out).args(["META-INF", "OEBPS"]))?;
    Ok(())
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let i = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    Some(&tag[i..i + tag[i..].find('"')?])
}

fn tags<'a>(xml: &'a str, open: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    xml.match_indices(open).filter_map(move |(i, _)| xml[i..].find('>').map(|end| &xml[i..i + end]))
}

/// Unpacks an EPUB and returns its chapter files in reading (spine) order.
fn epub_chapters(input: &Path, tmp: &Path) -> Result<Vec<PathBuf>> {
    let root = tmp.join("epub");
    run(Command::new("unzip").args(["-qo"]).arg(input).arg("-d").arg(&root))?;
    let container = std::fs::read_to_string(root.join("META-INF/container.xml")).context("not an EPUB: no container.xml")?;
    let opf_rel = tags(&container, "<rootfile ").find_map(|t| attr(t, "full-path")).context("EPUB has no rootfile")?;
    let opf_path = root.join(opf_rel);
    let opf = std::fs::read_to_string(&opf_path)?;
    let base = opf_path.parent().unwrap_or(&root);
    let manifest: Vec<(&str, &str)> = tags(&opf, "<item ").filter_map(|t| Some((attr(t, "id")?, attr(t, "href")?))).collect();
    let chapters: Vec<PathBuf> = tags(&opf, "<itemref ")
        .filter_map(|t| attr(t, "idref"))
        .filter_map(|id| manifest.iter().find(|(i, _)| *i == id))
        .map(|(_, href)| base.join(href))
        .filter(|p| p.exists())
        .collect();
    if chapters.is_empty() {
        bail!("EPUB has no readable chapters");
    }
    Ok(chapters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_to_txt_and_epub_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("notes.md");
        std::fs::write(&md, "# Title\n\nHello **world** & 1 < 2.\n").unwrap();
        let txt = convert(&md, "txt").unwrap();
        assert!(std::fs::read_to_string(&txt).unwrap().contains("Hello world & 1 < 2."));

        let epub = convert(&md, "epub").unwrap();
        let back = convert(&epub, "txt").unwrap();
        assert_eq!(back.file_name().unwrap(), "notes 2.txt");
        assert!(std::fs::read_to_string(back).unwrap().contains("Hello world & 1 < 2."));
    }

    #[test]
    fn text_clipping_to_txt() {
        let dir = tempfile::tempdir().unwrap();
        let plist = dir.path().join("Note.textClipping");
        std::fs::write(
            &plist,
            r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>UTI-Data</key><dict><key>public.utf8-plain-text</key><string>Clip &amp; paste</string></dict></dict></plist>"#,
        )
        .unwrap();
        let txt = convert(&plist, "txt").unwrap();
        assert!(std::fs::read_to_string(txt).unwrap().contains("Clip & paste"));
    }

    #[test]
    fn markdown_to_docx() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("a.md");
        std::fs::write(&md, "Hi").unwrap();
        let docx = convert(&md, "docx").unwrap();
        assert_eq!(&std::fs::read(docx).unwrap()[..2], b"PK");
    }
}
