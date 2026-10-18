//! PDF via PDFKit, and document → PDF via AppKit printing (macOS only).
//! Must run on the main thread; the Lime app runs every job through the `lime` CLI for that reason.

use crate::util::{out_path, produce, run, stem};
use anyhow::{bail, Context, Result};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSAttributedStringDocumentFormats, NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSPrintInfo, NSPrintJobSavingURL, NSPrintOperation, NSPrintSaveJob,
    NSPrintingPaginationMode, NSTextView,
};
use objc2_foundation::{NSAttributedString, NSDictionary, NSNumber, NSPoint, NSRect, NSSize, NSString, NSURL};
use objc2_pdf_kit::{PDFDisplayBox, PDFDocument, PDFDocumentOptimizeImagesForScreenOption, PDFDocumentSaveImagesAsJPEGOption, PDFPage};
use std::path::{Path, PathBuf};

fn url(p: &Path) -> Retained<NSURL> {
    NSURL::from_file_path(p).expect("absolute file path")
}

fn open(p: &Path) -> Result<Retained<PDFDocument>> {
    let p = std::path::absolute(p)?;
    unsafe { PDFDocument::initWithURL(PDFDocument::alloc(), &url(&p)) }.with_context(|| format!("could not open PDF {}", p.display()))
}

fn save(doc: &PDFDocument, out: &Path) -> Result<()> {
    let out = std::path::absolute(out)?;
    if !unsafe { doc.writeToURL(&url(&out)) } {
        bail!("could not write {}", out.display());
    }
    Ok(())
}

fn page(doc: &PDFDocument, i: usize) -> Result<Retained<PDFPage>> {
    unsafe { doc.pageAtIndex(i) }.with_context(|| format!("PDF has no page {}", i + 1))
}

/// Builds a new document from `pages` (0-based) of `src`.
fn subset(src: &Path, pages: &[usize]) -> Result<Retained<PDFDocument>> {
    // Re-open per call: PDFKit moves a page out of its document when inserting it elsewhere.
    let from = open(src)?;
    let doc = unsafe { PDFDocument::new() };
    for (n, &i) in pages.iter().enumerate() {
        unsafe { doc.insertPage_atIndex(&*page(&from, i)?, n) };
    }
    Ok(doc)
}

fn page_count(doc: &PDFDocument) -> usize {
    unsafe { doc.pageCount() }
}

fn write_image(img: &NSImage, file_type: NSBitmapImageFileType, out: &Path) -> Result<()> {
    let tiff = img.TIFFRepresentation().context("could not render page")?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff).context("could not render page")?;
    let data = unsafe { rep.representationUsingType_properties(file_type, &NSDictionary::new()) }.context("could not encode image")?;
    if !data.writeToURL_atomically(&url(&std::path::absolute(out)?), true) {
        bail!("could not write {}", out.display());
    }
    Ok(())
}

pub fn convert(input: &Path, target: &str) -> Result<PathBuf> {
    let doc = open(input)?;
    match target {
        // shortcut: Markdown export is the extracted text; PDFKit gives no heading/list structure.
        "txt" | "md" => {
            let text = unsafe { doc.string() }.map(|s| s.to_string()).unwrap_or_default();
            produce(out_path(input, "", target), |out| Ok(std::fs::write(out, text)?))
        }
        // shortcut: text only, one paragraph per line; no layout, images or tables.
        "docx" | "epub" => {
            let text = unsafe { doc.string() }.map(|s| s.to_string()).unwrap_or_default();
            produce(out_path(input, "", target), |out| {
                if target == "epub" {
                    return crate::document::write_epub(&stem(input), &text, out);
                }
                let tmp = tempfile::tempdir()?;
                let txt = tmp.path().join("doc.txt");
                std::fs::write(&txt, text)?;
                run(std::process::Command::new("textutil").args(["-convert", "docx", "-output"]).arg(out).arg(&txt)).map(drop)
            })
        }
        "jpg" | "png" => {
            let ty = if target == "png" { NSBitmapImageFileType::PNG } else { NSBitmapImageFileType::JPEG };
            let n = page_count(&doc);
            let render = |i: usize, out: &Path| -> Result<()> {
                let p = page(&doc, i)?;
                let b = unsafe { p.boundsForBox(PDFDisplayBox::MediaBox) };
                // 300 DPI: PDF points are 1/72 inch.
                let size = NSSize::new(b.size.width * 300.0 / 72.0, b.size.height * 300.0 / 72.0);
                write_image(&*unsafe { p.thumbnailOfSize_forBox(size, PDFDisplayBox::MediaBox) }, ty, out)
            };
            if n == 1 {
                return produce(out_path(input, "", target), |out| render(0, out));
            }
            produce(out_path(input, &format!(" {}", target.to_uppercase()), ""), |dir| {
                std::fs::create_dir(dir)?;
                (0..n).try_for_each(|i| render(i, &dir.join(format!("{}-{:03}.{target}", stem(input), i + 1))))
            })
        }
        _ => bail!("cannot convert PDF to {target}"),
    }
}

/// `1-3,5` → [0, 1, 2, 4], checked against `count`.
fn parse_pages(spec: &str, count: usize) -> Result<Vec<usize>> {
    let mut v = vec![];
    for part in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (a, b) = part.split_once('-').unwrap_or((part, part));
        let (a, b): (usize, usize) = (a.trim().parse()?, b.trim().parse()?);
        if a < 1 || b < a || b > count {
            bail!("page range '{part}' is outside 1-{count}");
        }
        v.extend(a - 1..b);
    }
    if v.is_empty() {
        bail!("no pages given");
    }
    Ok(v)
}

pub fn tool(op: &str, input: &Path, arg: Option<&str>) -> Result<Vec<PathBuf>> {
    let doc = open(input)?;
    let n = page_count(&doc);
    let out = match op {
        "compress" => produce(out_path(input, "-compressed", "pdf"), |out| {
            let yes = NSNumber::new_bool(true);
            let keys = unsafe { [PDFDocumentSaveImagesAsJPEGOption, PDFDocumentOptimizeImagesForScreenOption] };
            let opts = NSDictionary::<NSString, AnyObject>::from_slices(&keys, &[yes.as_ref(), yes.as_ref()]);
            if !unsafe { doc.writeToURL_withOptions(&url(&std::path::absolute(out)?), Some(&opts)) } {
                bail!("could not write compressed PDF");
            }
            Ok(())
        })?,
        "split" => produce(out_path(input, " pages", ""), |dir| {
            std::fs::create_dir(dir)?;
            (0..n).try_for_each(|i| save(&*subset(input, &[i])?, &dir.join(format!("{}-{:03}.pdf", stem(input), i + 1))))
        })?,
        "extract" => {
            let pages = parse_pages(arg.unwrap_or(""), n)?;
            produce(out_path(input, "-pages", "pdf"), |out| save(&*subset(input, &pages)?, out))?
        }
        "rotate" => produce(out_path(input, "-rotated", "pdf"), |out| {
            for i in 0..n {
                let p = page(&doc, i)?;
                unsafe { p.setRotation((p.rotation() + 90) % 360) };
            }
            save(&doc, out)
        })?,
        _ => bail!("unknown PDF tool '{op}'"),
    };
    Ok(vec![out])
}

pub fn merge(inputs: &[PathBuf]) -> Result<PathBuf> {
    let doc = unsafe { PDFDocument::new() };
    for p in inputs {
        let src = open(p)?;
        for i in 0..page_count(&src) {
            unsafe { doc.insertPage_atIndex(&*page(&src, i)?, page_count(&doc)) };
        }
    }
    produce(out_path(&inputs[0], "-merged", "pdf"), |out| save(&doc, out))
}

/// One page per image, in the given order.
pub fn from_images(inputs: &[PathBuf]) -> Result<PathBuf> {
    let doc = unsafe { PDFDocument::new() };
    for p in inputs {
        let img = NSImage::initWithContentsOfURL(NSImage::alloc(), &url(&std::path::absolute(p)?))
            .with_context(|| format!("could not read image {}", p.display()))?;
        let pg = unsafe { PDFPage::initWithImage(PDFPage::alloc(), &img) }.context("could not make PDF page")?;
        unsafe { doc.insertPage_atIndex(&pg, page_count(&doc)) };
    }
    produce(out_path(&inputs[0], "-combined", "pdf"), |out| save(&doc, out))
}

/// Paginates any document AppKit can read (HTML, RTF, DOCX…) into a PDF.
pub fn render_document(src: &Path, out: &Path) -> Result<()> {
    let mtm = MainThreadMarker::new().context("PDF rendering must run on the main thread")?;
    let _app = NSApplication::sharedApplication(mtm);
    let text = unsafe {
        NSAttributedString::initWithURL_options_documentAttributes_error(NSAttributedString::alloc(), &url(&std::path::absolute(src)?), &NSDictionary::new(), None)
    }
    .map_err(|e| anyhow::anyhow!("could not read document: {}", e.localizedDescription()))?;

    let info = NSPrintInfo::sharedPrintInfo();
    let paper = info.paperSize();
    let width = paper.width - info.leftMargin() - info.rightMargin();
    let view = NSTextView::initWithFrame(NSTextView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, 1.0)));
    unsafe { view.textStorage() }.context("text view has no storage")?.setAttributedString(&text);
    view.sizeToFit();

    unsafe {
        info.setJobDisposition(NSPrintSaveJob);
        info.dictionary().setObject_forKey(&url(&std::path::absolute(out)?), ProtocolObject::from_ref(NSPrintJobSavingURL));
    }
    info.setHorizontalPagination(NSPrintingPaginationMode::Fit);
    info.setVerticalPagination(NSPrintingPaginationMode::Automatic);
    let op = NSPrintOperation::printOperationWithView_printInfo(&view, &info);
    op.setShowsPrintPanel(false);
    op.setShowsProgressPanel(false);
    if !op.runOperation() {
        bail!("printing to PDF failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ranges() {
        assert_eq!(parse_pages("1-3,5", 5).unwrap(), vec![0, 1, 2, 4]);
        assert!(parse_pages("0", 5).is_err());
        assert!(parse_pages("4-6", 5).is_err());
        assert!(parse_pages("", 5).is_err());
    }
}
