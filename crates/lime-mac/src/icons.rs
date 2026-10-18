//! SF Symbol for each wheel choice, drawn above its label.

use crate::actions::Action;
use crate::wheel::Choice;
use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSFontWeightSemibold, NSImage, NSImageSymbolConfiguration};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};

/// Symbol name for `choice`: conversions by target format, tools by what they do.
pub fn symbol(choice: &Choice) -> &'static str {
    let id = match choice {
        Choice::Action(a) => {
            return match a {
                Action::AirDrop => "dot.radiowaves.left.and.right",
                Action::Float => "pin",
                Action::Copy => "doc.on.doc",
                Action::Move => "folder",
                Action::CopyPath => "link",
                Action::Terminal => "terminal",
            }
        }
        Choice::Op(op) => op.id.as_str(),
    };
    if let Some(ext) = id.strip_prefix("to:") {
        return match ext {
            "gif" => "photo.stack",
            "pdf" => "doc.richtext",
            "mp4" | "mov" | "mkv" | "webm" | "avi" | "wmv" => "film",
            "mp3" | "m4a" | "wav" | "flac" | "ogg" | "opus" | "aiff" | "wma" => "music.note",
            "docx" | "odt" | "rtf" => "doc.text",
            "txt" => "text.alignleft",
            "md" => "number",
            "html" => "chevron.left.forwardslash.chevron.right",
            "epub" => "book",
            "xlsx" | "ods" | "csv" | "tsv" => "tablecells",
            "srt" | "vtt" => "captions.bubble",
            "zip" | "tar" | "tgz" => "doc.zipper",
            _ => "photo", // jpg, png, webp, heic, tiff, avif, bmp
        };
    }
    match id {
        "compress" => "arrow.down.right.and.arrow.up.left",
        "resize" => "arrow.up.left.and.arrow.down.right",
        "rotate" => "rotate.right",
        "flip" => "arrow.left.and.right.righttriangle.left.righttriangle.right",
        "square" => "crop",
        "strip" => "tag.slash",
        "trim" => "scissors",
        "split" => "square.split.2x1",
        "mute" => "speaker.slash",
        "720p" => "tv",
        "normalize" => "slider.horizontal.3",
        "extract" => "tray.and.arrow.up",
        "merge" => "arrow.triangle.merge",
        "shift" => "clock.arrow.circlepath",
        "zip" => "doc.zipper",
        _ => "questionmark",
    }
}

fn image(name: &str, size: f64, color: &NSColor) -> Option<Retained<NSImage>> {
    let base = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)?;
    let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(size, unsafe { NSFontWeightSemibold })
        .configurationByApplyingConfiguration(&NSImageSymbolConfiguration::configurationWithPaletteColors(&NSArray::from_slice(&[color])));
    base.imageWithSymbolConfiguration(&config)
}

/// Draws `choice`'s symbol centred on `at`, `size` points tall, in `color`.
pub fn draw(choice: &Choice, at: NSPoint, size: f64, color: &NSColor) {
    if let Some(img) = image(symbol(choice), size, color) {
        let s = img.size();
        img.drawInRect(NSRect::new(NSPoint::new(at.x - s.width / 2.0, at.y - s.height / 2.0), NSSize::new(s.width, s.height)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every choice the wheels can offer maps to a real SF Symbol.
    #[test]
    fn every_choice_has_a_symbol() {
        let mut choices: Vec<Choice> = Action::ALL.iter().copied().map(Choice::Action).collect();
        for f in ["a.png", "a.mp4", "a.mp3", "a.pdf", "a.md", "a.csv", "a.srt", "a.zip"] {
            for files in [vec![f.into()], vec![f.into(), f.into()]] {
                for wheel in [lime_core::Wheel::Convert, lime_core::Wheel::Tools] {
                    choices.extend(lime_core::ops(wheel, &files).into_iter().map(Choice::Op));
                }
            }
        }
        for c in &choices {
            let name = symbol(c);
            assert_ne!(name, "questionmark", "{c:?} has no symbol");
            assert!(NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None).is_some(), "missing SF Symbol {name}");
        }
    }
}
