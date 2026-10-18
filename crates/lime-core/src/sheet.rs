//! Spreadsheets: read XLSX/XLS/ODS (calamine) and CSV/TSV, write XLSX, ODS, CSV, TSV and PDF.

use crate::formats::ext;
use crate::util::{out_path, produce, run, stem};
use anyhow::{bail, Result};
use calamine::Reader;
use std::path::{Path, PathBuf};
use std::process::Command;

type Table = Vec<Vec<String>>;

fn read(input: &Path) -> Result<Vec<(String, Table)>> {
    let e = ext(input);
    if e == "csv" || e == "tsv" {
        let mut r = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .delimiter(if e == "tsv" { b'\t' } else { b',' })
            .from_path(input)?;
        let rows = r.records().map(|rec| Ok(rec?.iter().map(String::from).collect())).collect::<Result<Table>>()?;
        return Ok(vec![(stem(input), rows)]);
    }
    let mut wb = calamine::open_workbook_auto(input)?;
    let mut sheets = vec![];
    for name in wb.sheet_names() {
        let range = wb.worksheet_range(&name)?;
        sheets.push((name, range.rows().map(|r| r.iter().map(|c| c.to_string()).collect()).collect()));
    }
    Ok(sheets)
}

fn write_delimited(rows: &Table, delim: u8, out: &Path) -> Result<()> {
    let mut w = csv::WriterBuilder::new().delimiter(delim).flexible(true).from_path(out)?;
    for row in rows {
        w.write_record(row)?;
    }
    w.flush()?;
    Ok(())
}

fn write_xlsx(sheets: &[(String, Table)], out: &Path) -> Result<()> {
    let mut wb = rust_xlsxwriter::Workbook::new();
    for (name, rows) in sheets {
        let ws = wb.add_worksheet();
        // Excel caps sheet names at 31 chars and forbids some characters.
        let clean: String = name.chars().filter(|c| !"[]:*?/\\".contains(*c)).take(31).collect();
        if !clean.is_empty() {
            ws.set_name(clean)?;
        }
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let (r, c) = (r as u32, c as u16);
                match cell.parse::<f64>() {
                    Ok(n) if n.is_finite() => ws.write_number(r, c, n)?,
                    _ => ws.write_string(r, c, cell)?,
                };
            }
        }
    }
    wb.save(out)?;
    Ok(())
}

/// Minimal OpenDocument spreadsheet: numbers as floats, everything else as strings.
fn write_ods(sheets: &[(String, Table)], out: &Path) -> Result<()> {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let mut body = String::new();
    for (name, rows) in sheets {
        body += &format!("<table:table table:name=\"{}\">", esc(name));
        for row in rows {
            body += "<table:table-row>";
            for cell in row {
                body += &match cell.parse::<f64>() {
                    Ok(n) if n.is_finite() => format!("<table:table-cell office:value-type=\"float\" office:value=\"{n}\"><text:p>{}</text:p></table:table-cell>", esc(cell)),
                    _ => format!("<table:table-cell office:value-type=\"string\"><text:p>{}</text:p></table:table-cell>", esc(cell)),
                };
            }
            body += "</table:table-row>";
        }
        body += "</table:table>";
    }
    let tmp = tempfile::tempdir()?;
    let d = tmp.path();
    std::fs::create_dir(d.join("META-INF"))?;
    std::fs::write(d.join("mimetype"), "application/vnd.oasis.opendocument.spreadsheet")?;
    std::fs::write(
        d.join("META-INF/manifest.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#,
    )?;
    std::fs::write(
        d.join("content.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2"><office:body><office:spreadsheet>{body}</office:spreadsheet></office:body></office:document-content>"#
        ),
    )?;
    let out = std::path::absolute(out)?;
    // mimetype must be the first entry, stored uncompressed.
    run(Command::new("zip").current_dir(d).args(["-X0q"]).arg(&out).arg("mimetype"))?;
    run(Command::new("zip").current_dir(d).args(["-Xr9q"]).arg(&out).args(["META-INF", "content.xml"]))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn write_pdf(sheets: &[(String, Table)], out: &Path) -> Result<()> {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;");
    let mut html = String::from("<html><head><meta charset=\"utf-8\"></head><body style=\"font-family:Helvetica;font-size:10px\">");
    for (name, rows) in sheets {
        html += &format!("<h3>{}</h3><table border=\"1\" cellspacing=\"0\" cellpadding=\"3\">", esc(name));
        for row in rows {
            html += "<tr>";
            for cell in row {
                html += &format!("<td>{}</td>", esc(cell));
            }
            html += "</tr>";
        }
        html += "</table>";
    }
    html += "</body></html>";
    let tmp = tempfile::tempdir()?;
    let src = tmp.path().join("sheet.html");
    std::fs::write(&src, html)?;
    crate::pdf::render_document(&src, out)
}

pub fn convert(input: &Path, target: &str) -> Result<PathBuf> {
    let sheets = read(input)?;
    if sheets.is_empty() {
        bail!("spreadsheet has no sheets");
    }
    produce(out_path(input, "", target), |out| match target {
        // shortcut: CSV/TSV hold one table, so only the first sheet is exported.
        "csv" => write_delimited(&sheets[0].1, b',', out),
        "tsv" => write_delimited(&sheets[0].1, b'\t', out),
        "xlsx" => write_xlsx(&sheets, out),
        "ods" => write_ods(&sheets, out),
        #[cfg(target_os = "macos")]
        "pdf" => write_pdf(&sheets, out),
        _ => bail!("cannot convert spreadsheet to {target}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_xlsx_tsv_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("data.csv");
        std::fs::write(&csv, "name,qty\n\"a, b\",3\nc,4.5\n").unwrap();
        let xlsx = convert(&csv, "xlsx").unwrap();
        let tsv = convert(&xlsx, "tsv").unwrap();
        assert_eq!(std::fs::read_to_string(tsv).unwrap(), "name\tqty\na, b\t3\nc\t4.5\n");
    }

    #[test]
    fn ods_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("data.csv");
        std::fs::write(&csv, "name,qty\n<a & b>,3\n").unwrap();
        let ods = convert(&csv, "ods").unwrap();
        let back = convert(&ods, "csv").unwrap();
        assert_eq!(std::fs::read_to_string(back).unwrap(), "name,qty\n<a & b>,3\n");
    }
}
