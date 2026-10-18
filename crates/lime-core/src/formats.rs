use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Audio,
    Video,
    Pdf,
    Document,
    Spreadsheet,
    Subtitle,
    Archive,
    /// Folders and files Lime has no converter for; they can still be archived.
    Other,
}

/// Lowercased extension, with `jpeg` folded into `jpg` and `.tar.gz` into `tgz`.
pub fn ext(path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    if name.ends_with(".tar.gz") {
        return "tgz".into();
    }
    match path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() {
        Some("jpeg") => "jpg".into(),
        Some("aif") => "aiff".into(),
        Some("htm") => "html".into(),
        Some(e) => e.into(),
        None => String::new(),
    }
}

pub fn kind(path: &Path) -> Kind {
    if path.is_dir() {
        return Kind::Other;
    }
    match ext(path).as_str() {
        "jpg" | "png" | "webp" | "heic" | "tiff" | "tif" | "avif" | "bmp" | "gif" | "svg" | "jp2" | "jxl" => Kind::Image,
        "mp3" | "m4a" | "wav" | "flac" | "ogg" | "opus" | "aiff" | "wma" | "aac" | "caf" => Kind::Audio,
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "m4v" => Kind::Video,
        "pdf" => Kind::Pdf,
        "docx" | "doc" | "rtf" | "rtfd" | "txt" | "md" | "markdown" | "html" | "odt" | "epub" | "webarchive" => Kind::Document,
        "xlsx" | "xls" | "ods" | "csv" | "tsv" => Kind::Spreadsheet,
        "srt" | "vtt" => Kind::Subtitle,
        "zip" | "tar" | "tgz" | "gz" | "rar" | "7z" => Kind::Archive,
        _ => Kind::Other,
    }
}

/// Formats `path` (of kind `k`) can be converted to, excluding its own format.
pub fn targets(path: &Path, k: Kind) -> Vec<&'static str> {
    let all: &[&str] = match k {
        Kind::Image => &["jpg", "png", "webp", "heic", "tiff", "avif", "bmp", "gif", "pdf"],
        Kind::Audio => &["mp3", "m4a", "wav", "flac", "ogg", "opus", "aiff", "wma"],
        Kind::Video => &["mp4", "mov", "mkv", "webm", "avi", "wmv", "gif", "mp3"],
        Kind::Pdf if cfg!(target_os = "macos") => &["txt", "md", "jpg", "png"],
        Kind::Pdf => &[],
        Kind::Document => &["docx", "odt", "rtf", "txt", "md", "html", "epub", "pdf"],
        Kind::Spreadsheet => &["xlsx", "csv", "tsv", "pdf"],
        Kind::Subtitle => &["srt", "vtt", "txt"],
        Kind::Archive | Kind::Other => &["zip", "tar", "tgz"],
    };
    let own = ext(path);
    let own = if own == "tif" { "tiff".to_string() } else { own };
    let mut v: Vec<&'static str> = all.iter().copied().filter(|t| *t != own).collect();
    // Document → PDF goes through AppKit.
    if !cfg!(target_os = "macos") && matches!(k, Kind::Document | Kind::Spreadsheet) {
        v.retain(|t| *t != "pdf");
    }
    // Plain gzip/rar/7z are extract-only sources but can be repacked; a lone .gz holds one file.
    if own == "gz" {
        v.clear();
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_normalizes() {
        assert_eq!(ext(Path::new("A.JPEG")), "jpg");
        assert_eq!(ext(Path::new("x.tar.gz")), "tgz");
        assert_eq!(kind(Path::new("x.tar.gz")), Kind::Archive);
        assert_eq!(kind(Path::new("noext")), Kind::Other);
    }
}
