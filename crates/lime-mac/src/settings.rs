//! Which modifier key opens which wheel, stored in user defaults.

use crate::{Delegate, WheelKind};
use objc2::rc::Retained;
use objc2::{sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSEventModifierFlags, NSPopUpButton, NSTextField, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString, NSUserDefaults};
use std::cell::RefCell;

const KEYS: [&str; 4] = ["Shift", "Command", "Control", "Option"];

fn flag(key: &str) -> NSEventModifierFlags {
    match key {
        "Command" => NSEventModifierFlags::Command,
        "Control" => NSEventModifierFlags::Control,
        "Option" => NSEventModifierFlags::Option,
        _ => NSEventModifierFlags::Shift,
    }
}

fn defaults_key(kind: WheelKind) -> String {
    format!("wheelKey.{}", kind.title())
}

fn default_key(kind: WheelKind) -> &'static str {
    match kind {
        WheelKind::Convert => "Shift",
        WheelKind::Tools => "Command",
        WheelKind::Actions => "Control",
    }
}

pub fn key_for(kind: WheelKind) -> String {
    NSUserDefaults::standardUserDefaults()
        .stringForKey(&NSString::from_str(&defaults_key(kind)))
        .map(|s| s.to_string())
        .filter(|k| KEYS.contains(&k.as_str()))
        .unwrap_or_else(|| default_key(kind).into())
}

/// The wheel whose key is held. Tools and Actions win over Convert, so Shift+Command opens Tools.
pub fn wheel_for(flags: NSEventModifierFlags) -> Option<WheelKind> {
    [WheelKind::Tools, WheelKind::Actions, WheelKind::Convert].into_iter().find(|k| flags.contains(flag(&key_for(*k))))
}

thread_local! {
    static WINDOW: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
}

pub fn open(mtm: MainThreadMarker, delegate: &Delegate) {
    WINDOW.with_borrow_mut(|w| {
        let window = w.get_or_insert_with(|| build(mtm, delegate));
        NSApplication::sharedApplication(mtm).activate();
        window.center();
        window.makeKeyAndOrderFront(None);
    });
}

fn build(mtm: MainThreadMarker, delegate: &Delegate) -> Retained<NSWindow> {
    let size = NSSize::new(380.0, 200.0);
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), size),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("Lime Settings"));
    let content = window.contentView().expect("window has a content view");

    let note = NSTextField::wrappingLabelWithString(
        &NSString::from_str("Drag a file in Finder and hold a key to open its wheel. The wheel stays open when you let go of the key; press another key to switch."),
        mtm,
    );
    note.setFrame(NSRect::new(NSPoint::new(20.0, 140.0), NSSize::new(340.0, 48.0)));
    content.addSubview(&note);

    let titles: Vec<_> = KEYS.iter().map(|k| NSString::from_str(k)).collect();
    let titles = NSArray::from_retained_slice(&titles);
    for (row, kind) in WheelKind::ALL.into_iter().enumerate() {
        let y = 96.0 - row as f64 * 36.0;
        let label = NSTextField::labelWithString(&NSString::from_str(&format!("{} wheel", kind.title())), mtm);
        label.setFrame(NSRect::new(NSPoint::new(20.0, y + 4.0), NSSize::new(140.0, 20.0)));
        content.addSubview(&label);
        let popup = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(mtm),
            NSRect::new(NSPoint::new(170.0, y), NSSize::new(180.0, 28.0)),
            false,
        );
        popup.addItemsWithTitles(&titles);
        popup.selectItemWithTitle(&NSString::from_str(&key_for(kind)));
        popup.setTag(row as isize);
        unsafe {
            popup.setTarget(Some(delegate));
            popup.setAction(Some(sel!(settingChanged:)));
        }
        content.addSubview(&popup);
    }
    window
}

pub fn changed(popup: &NSPopUpButton) {
    let Some(kind) = WheelKind::ALL.get(popup.tag() as usize) else { return };
    if let Some(title) = popup.titleOfSelectedItem() {
        unsafe { NSUserDefaults::standardUserDefaults().setObject_forKey(Some(&title), &NSString::from_str(&defaults_key(*kind))) };
    }
}
