//! Runs operations through the `lime` CLI on background threads and shows their progress.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSBackingStoreType, NSColor, NSFloatingWindowLevel, NSPanel, NSScreen, NSTextField,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{ns_string, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

enum Status {
    Running,
    Done(String),
    Failed(String),
}

struct Job {
    title: String,
    status: Status,
    finished: Option<Instant>,
}

static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());

thread_local! {
    static PANEL: RefCell<Option<(Retained<NSPanel>, Retained<NSTextField>)>> = const { RefCell::new(None) };
    static LAST_TEXT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The `lime` CLI next to this executable (Lime.app/Contents/MacOS/lime, or target/<profile>/lime).
fn cli() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("lime"))).unwrap_or_else(|| "lime".into())
}

/// Asks for a tool's value; None when cancelled.
fn ask(mtm: MainThreadMarker, op: &lime_core::Op, param: &lime_core::Param) -> Option<String> {
    NSApplication::sharedApplication(mtm).activate();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&op.label));
    alert.setInformativeText(&NSString::from_str(param.prompt));
    alert.addButtonWithTitle(ns_string!("Run"));
    alert.addButtonWithTitle(ns_string!("Cancel"));
    let field = NSTextField::textFieldWithString(&NSString::from_str(param.default), mtm);
    field.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(240.0, 24.0)));
    alert.setAccessoryView(Some(&field));
    alert.window().setInitialFirstResponder(Some(&field));
    (alert.runModal() == NSAlertFirstButtonReturn).then(|| field.stringValue().to_string())
}

fn summary(files: &[PathBuf]) -> String {
    let first = files[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match files.len() {
        1 => first,
        n => format!("{first} + {} more", n - 1),
    }
}

pub fn start(mtm: MainThreadMarker, op: lime_core::Op, files: Vec<PathBuf>) {
    if files.is_empty() {
        return;
    }
    let arg = match &op.param {
        Some(p) => match ask(mtm, &op, p) {
            Some(v) => Some(v),
            None => return,
        },
        None => None,
    };
    let index = {
        let mut jobs = JOBS.lock().unwrap();
        jobs.push(Job { title: format!("{} → {}", summary(&files), op.label), status: Status::Running, finished: None });
        jobs.len() - 1
    };
    std::thread::spawn(move || {
        let mut cmd = Command::new(cli());
        cmd.arg("run").arg(&op.id);
        if let Some(a) = &arg {
            cmd.args(["--arg", a]);
        }
        cmd.args(&files);
        let status = match cmd.output() {
            Ok(out) if out.status.success() => {
                let made: Vec<String> = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .filter_map(|l| PathBuf::from(l).file_name().map(|n| n.to_string_lossy().into_owned()))
                    .collect();
                Status::Done(made.join(", "))
            }
            Ok(out) => Status::Failed(String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("lime: ").to_string()),
            Err(e) => Status::Failed(format!("could not start lime: {e}")),
        };
        let mut jobs = JOBS.lock().unwrap();
        if let Some(job) = jobs.get_mut(index) {
            job.status = status;
            job.finished = Some(Instant::now());
        }
    });
}

pub fn report_error(_mtm: MainThreadMarker, message: &str) {
    JOBS.lock().unwrap().push(Job { title: message.into(), status: Status::Failed(String::new()), finished: Some(Instant::now()) });
}

/// Called every tick: redraws the progress panel and drops finished jobs after a while.
pub fn refresh(mtm: MainThreadMarker) {
    let text = {
        let mut jobs = JOBS.lock().unwrap();
        // Indexes held by running threads must stay valid, so only clear once everything is done.
        let all_expired = jobs.iter().all(|j| match (&j.status, j.finished) {
            (Status::Running, _) | (_, None) => false,
            (Status::Failed(_), Some(t)) => t.elapsed() > Duration::from_secs(12),
            (_, Some(t)) => t.elapsed() > Duration::from_secs(4),
        });
        if all_expired {
            jobs.clear();
        }
        jobs.iter()
            .map(|j| match &j.status {
                Status::Running => format!("⏳  {}", j.title),
                Status::Done(made) => format!("✅  {}\n      {made}", j.title),
                Status::Failed(e) if e.is_empty() => format!("⚠️  {}", j.title),
                Status::Failed(e) => format!("⚠️  {}\n      {e}", j.title),
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    if LAST_TEXT.with_borrow(|t| *t == text) {
        return;
    }
    LAST_TEXT.with_borrow_mut(|t| t.clone_from(&text));
    PANEL.with_borrow_mut(|p| {
        let (panel, label) = p.get_or_insert_with(|| make_panel(mtm));
        if text.is_empty() {
            panel.orderOut(None);
            return;
        }
        label.setStringValue(&NSString::from_str(&text));
        let height = label.fittingSize().height + 24.0;
        let width = 380.0;
        let screen = NSScreen::mainScreen(mtm).map(|s| s.visibleFrame()).unwrap_or(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)));
        let origin = NSPoint::new(screen.origin.x + screen.size.width - width - 16.0, screen.origin.y + screen.size.height - height - 16.0);
        panel.setFrame_display(NSRect::new(origin, NSSize::new(width, height)), true);
        label.setFrame(NSRect::new(NSPoint::new(12.0, 12.0), NSSize::new(width - 24.0, height - 24.0)));
        panel.orderFrontRegardless();
    });
}

fn make_panel(mtm: MainThreadMarker) -> (Retained<NSPanel>, Retained<NSTextField>) {
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(380.0, 60.0)),
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setLevel(NSFloatingWindowLevel);
    panel.setBackgroundColor(Some(&NSColor::colorWithWhite_alpha(0.1, 0.92)));
    panel.setHidesOnDeactivate(false);
    panel.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces);
    unsafe { panel.setReleasedWhenClosed(false) };
    let label = NSTextField::wrappingLabelWithString(ns_string!(""), mtm);
    label.setSelectable(false);
    label.setTextColor(Some(&NSColor::whiteColor()));
    if let Some(content) = panel.contentView() {
        content.addSubview(&label);
    }
    (panel, label)
}
