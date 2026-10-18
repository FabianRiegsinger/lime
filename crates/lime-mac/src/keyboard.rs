//! Keyboard flow: Option-Return in Finder opens the Convert wheel for the selected files.
//! A Carbon hot key needs no Accessibility permission. It is registered only while Finder is
//! frontmost, so Option-Return keeps working in other apps.

use crate::{jobs, wheel, WheelKind};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSWorkspace};
use std::cell::Cell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[repr(C)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(target: *mut c_void, handler: Handler, count: u32, types: *const EventTypeSpec, data: *mut c_void, out: *mut *mut c_void) -> i32;
    fn RegisterEventHotKey(key: u32, modifiers: u32, id: EventHotKeyID, target: *mut c_void, options: u32, out: *mut *mut c_void) -> i32;
    fn UnregisterEventHotKey(hot_key: *mut c_void) -> i32;
}

const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const OPTION_KEY: u32 = 1 << 11;
const RETURN_KEY: u32 = 36;

static PRESSED: AtomicBool = AtomicBool::new(false);
/// Finder's selection, fetched on a background thread because the first AppleScript call waits for the user to allow it.
static SELECTION: Mutex<Option<Result<Vec<PathBuf>, String>>> = Mutex::new(None);

thread_local! {
    static HOT_KEY: Cell<*mut c_void> = const { Cell::new(std::ptr::null_mut()) };
}

extern "C" fn on_hot_key(_call: *mut c_void, _event: *mut c_void, _data: *mut c_void) -> i32 {
    PRESSED.store(true, Ordering::Relaxed);
    0
}

pub fn install() {
    let spec = EventTypeSpec { event_class: K_EVENT_CLASS_KEYBOARD, event_kind: K_EVENT_HOT_KEY_PRESSED };
    unsafe { InstallEventHandler(GetApplicationEventTarget(), on_hot_key, 1, &spec, std::ptr::null_mut(), std::ptr::null_mut()) };
}

fn finder_is_frontmost() -> bool {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .and_then(|app| app.bundleIdentifier())
        .is_some_and(|id| id.to_string() == "com.apple.finder")
}

// shortcut: one path per line, so a file name containing a newline breaks; switch to NSAppleScript descriptors if that matters.
const SELECTION_SCRIPT: &str = r#"tell application "Finder" to set s to selection as alias list
set out to ""
repeat with f in s
  set out to out & POSIX path of f & linefeed
end repeat
return out"#;

fn finder_selection() -> Result<Vec<PathBuf>, String> {
    let out = Command::new("osascript").args(["-e", SELECTION_SCRIPT]).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("Allow Lime to control Finder in System Settings › Privacy & Security › Automation".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect())
}

/// Called every tick: keeps the hot key registered only while Finder is frontmost and opens the wheel when it fires.
pub fn tick(mtm: MainThreadMarker) {
    let registered = HOT_KEY.get();
    if finder_is_frontmost() == registered.is_null() {
        if registered.is_null() {
            let mut hot_key = std::ptr::null_mut();
            let id = EventHotKeyID { signature: u32::from_be_bytes(*b"Lime"), id: 1 };
            unsafe { RegisterEventHotKey(RETURN_KEY, OPTION_KEY, id, GetApplicationEventTarget(), 0, &mut hot_key) };
            HOT_KEY.set(hot_key);
        } else {
            unsafe { UnregisterEventHotKey(registered) };
            HOT_KEY.set(std::ptr::null_mut());
        }
    }
    if PRESSED.swap(false, Ordering::Relaxed) {
        std::thread::spawn(|| *SELECTION.lock().unwrap() = Some(finder_selection()));
    }
    let Some(selection) = SELECTION.lock().unwrap().take() else { return };
    match selection {
        Ok(files) if !files.is_empty() => wheel::show(mtm, WheelKind::Convert, &files, NSEvent::mouseLocation(), true),
        Ok(_) => {}
        Err(e) => jobs::report_error(mtm, &e),
    }
}
