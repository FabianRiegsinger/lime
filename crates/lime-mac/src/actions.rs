//! File actions (AirDrop, Float, Copy, Move, Copy Path, Terminal). They work on any files and folders.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSColor, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession,
    NSDraggingSource, NSEvent, NSFloatingWindowLevel, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSOpenPanel,
    NSPanel, NSPasteboard, NSPasteboardTypeString, NSPasteboardWriting, NSSharingService, NSSharingServiceNameSendViaAirDrop,
    NSStringDrawing, NSView, NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{ns_string, NSArray, NSDictionary, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    AirDrop,
    Float,
    Copy,
    Move,
    CopyPath,
    Terminal,
}

impl Action {
    pub const ALL: [Action; 6] = [Action::AirDrop, Action::Float, Action::Copy, Action::Move, Action::CopyPath, Action::Terminal];

    pub fn label(self) -> &'static str {
        match self {
            Action::AirDrop => "AirDrop",
            Action::Float => "Float",
            Action::Copy => "Copy",
            Action::Move => "Move",
            Action::CopyPath => "Copy Path",
            Action::Terminal => "Terminal",
        }
    }
}

fn url(p: &Path) -> Retained<NSURL> {
    NSURL::from_file_path(p).expect("absolute path")
}

pub fn run(mtm: MainThreadMarker, action: Action, files: &[PathBuf]) {
    if files.is_empty() {
        return;
    }
    let urls: Vec<Retained<NSURL>> = files.iter().map(|p| url(p)).collect();
    match action {
        Action::AirDrop => {
            NSApplication::sharedApplication(mtm).activate();
            if let Some(svc) = NSSharingService::sharingServiceNamed(unsafe { NSSharingServiceNameSendViaAirDrop }) {
                let items: Vec<Retained<AnyObject>> = urls.into_iter().map(|u| u.into()).collect();
                unsafe { svc.performWithItems(&NSArray::from_retained_slice(&items)) };
            }
        }
        Action::Float => {
            for (i, f) in files.iter().enumerate() {
                float(mtm, f, i);
            }
        }
        Action::Copy => {
            let pb = NSPasteboard::generalPasteboard();
            pb.clearContents();
            let writers: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
                urls.into_iter().map(ProtocolObject::from_retained).collect();
            pb.writeObjects(&NSArray::from_retained_slice(&writers));
        }
        Action::Move => {
            let panel = NSOpenPanel::openPanel(mtm);
            panel.setCanChooseDirectories(true);
            panel.setCanChooseFiles(false);
            panel.setCanCreateDirectories(true);
            panel.setPrompt(Some(ns_string!("Move Here")));
            NSApplication::sharedApplication(mtm).activate();
            if panel.runModal() == 1 {
                if let Some(dest) = panel.URL().and_then(|u| u.to_file_path()) {
                    move_files(mtm, files, &dest);
                }
            }
        }
        Action::CopyPath => {
            let pb = NSPasteboard::generalPasteboard();
            pb.clearContents();
            let text: Vec<String> = files.iter().map(|p| p.display().to_string()).collect();
            pb.setString_forType(&NSString::from_str(&text.join("\n")), unsafe { NSPasteboardTypeString });
        }
        Action::Terminal => {
            let dir = if files[0].is_dir() { files[0].clone() } else { files[0].parent().unwrap_or(Path::new("/")).to_path_buf() };
            let _ = Command::new("open").args(["-a", "Terminal"]).arg(dir).status();
        }
    }
}

fn move_files(mtm: MainThreadMarker, files: &[PathBuf], dest: &Path) {
    for f in files {
        // mv handles folders and cross-volume moves; -n refuses to overwrite.
        let ok = Command::new("mv").arg("-n").arg(f).arg(dest).status().map(|s| s.success()).unwrap_or(false);
        if !ok || f.exists() {
            crate::jobs::report_error(mtm, &format!("Could not move {} (already exists?)", f.display()));
        }
    }
}

// ---------- Float: a small always-on-top window you can drag the file out of again ----------

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "LimeFloatView"]
    #[ivars = PathBuf]
    struct FloatView;

    impl FloatView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let b = self.bounds();
            let icon = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&self.ivars().display().to_string()));
            icon.drawInRect(NSRect::new(NSPoint::new((b.size.width - 96.0) / 2.0, b.size.height - 104.0), NSSize::new(96.0, 96.0)));
            let name = self.ivars().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
            let font = NSFont::systemFontOfSize(11.0);
            let color = NSColor::labelColor();
            let values: [&AnyObject; 2] = [&font, &color];
            let attrs = NSDictionary::from_slices(&keys, &values);
            let s = NSString::from_str(&name);
            unsafe {
                let size = s.sizeWithAttributes(Some(&attrs));
                s.drawAtPoint_withAttributes(NSPoint::new(((b.size.width - size.width) / 2.0).max(4.0), 4.0), Some(&attrs));
            }
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _e: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if event.clickCount() == 2 {
                NSWorkspace::sharedWorkspace().openURL(&url(self.ivars()));
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let u = url(self.ivars());
            let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*u));
            let icon = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&self.ivars().display().to_string()));
            unsafe { item.setDraggingFrame_contents(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(64.0, 64.0)), Some(&icon)) };
            self.beginDraggingSessionWithItems_event_source(&NSArray::from_retained_slice(&[item]), event, ProtocolObject::from_ref(self));
        }
    }

    unsafe impl NSObjectProtocol for FloatView {}

    unsafe impl NSDraggingSource for FloatView {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn operation_mask(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
            NSDragOperation::Copy | NSDragOperation::Generic
        }
    }
);

thread_local! {
    // shortcut: closed float panels stay allocated until quit; fine for a handful, add a close delegate if it grows.
    static FLOATS: RefCell<Vec<Retained<NSPanel>>> = const { RefCell::new(Vec::new()) };
}

fn float(mtm: MainThreadMarker, path: &Path, index: usize) {
    let size = NSSize::new(150.0, 140.0);
    let at = NSEvent::mouseLocation();
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        NSRect::new(NSPoint::new(at.x + 20.0 + index as f64 * 24.0, at.y - size.height - index as f64 * 24.0), size),
        NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::UtilityWindow | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setTitle(&NSString::from_str(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    panel.setLevel(NSFloatingWindowLevel);
    panel.setHidesOnDeactivate(false);
    panel.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary);
    unsafe { panel.setReleasedWhenClosed(false) };
    let view = FloatView::alloc(mtm).set_ivars(path.to_path_buf());
    let view: Retained<FloatView> = unsafe { msg_send![super(view), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), size)] };
    panel.setContentView(Some(&view));
    panel.orderFrontRegardless();
    FLOATS.with_borrow_mut(|f| f.push(panel));
}
