//! Which modifier key opens which wheel, and the app's appearance, stored in user defaults.

use crate::{Delegate, WheelKind};
use objc2::rc::Retained;
use objc2::{sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSBackingStoreType, NSEventModifierFlags, NSFloatingWindowLevel, NSPopUpButton, NSSlider, NSTextField, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString, NSUserDefaults};
use std::cell::RefCell;

const KEYS: [&str; 4] = ["Shift", "Command", "Control", "Option"];
const APPEARANCES: [&str; 3] = ["System", "Light", "Dark"];
const APPEARANCE_KEY: &str = "appearance";
const GLASS_KEY: &str = "glassiness";
/// Popup tag for the appearance row; wheel rows use their index.
const APPEARANCE_TAG: isize = 100;

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

fn appearance() -> String {
    NSUserDefaults::standardUserDefaults()
        .stringForKey(&NSString::from_str(APPEARANCE_KEY))
        .map(|s| s.to_string())
        .filter(|a| APPEARANCES.contains(&a.as_str()))
        .unwrap_or_else(|| "System".into())
}

/// How see-through the wheel is, 0 (frosted) to 1 (clear glass).
pub fn glassiness() -> f64 {
    let defaults = NSUserDefaults::standardUserDefaults();
    let key = NSString::from_str(GLASS_KEY);
    if defaults.objectForKey(&key).is_none() {
        return 0.6;
    }
    defaults.doubleForKey(&key).clamp(0.0, 1.0)
}

/// Forces light or dark on every Lime window, or follows the system when set to System.
pub fn apply_appearance(mtm: MainThreadMarker) {
    let name = match appearance().as_str() {
        "Light" => Some(unsafe { NSAppearanceNameAqua }),
        "Dark" => Some(unsafe { NSAppearanceNameDarkAqua }),
        _ => None,
    };
    let appearance = name.and_then(NSAppearance::appearanceNamed);
    NSApplication::sharedApplication(mtm).setAppearance(appearance.as_deref());
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

/// Hides the window; called when the user clicks into another app.
pub fn hide() {
    WINDOW.with_borrow(|w| {
        if let Some(window) = w {
            window.orderOut(None);
        }
    });
}

fn build(mtm: MainThreadMarker, delegate: &Delegate) -> Retained<NSWindow> {
    let size = NSSize::new(380.0, 276.0);
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
    window.setLevel(NSFloatingWindowLevel);
    window.setTitle(&NSString::from_str("Lime Settings"));
    let content = window.contentView().expect("window has a content view");

    let note = NSTextField::wrappingLabelWithString(
        &NSString::from_str("Drag a file in Finder and hold a key to open its wheel. The wheel stays open when you let go of the key; press another key to switch."),
        mtm,
    );
    note.setFrame(NSRect::new(NSPoint::new(20.0, 216.0), NSSize::new(340.0, 48.0)));
    content.addSubview(&note);

    let label = NSTextField::labelWithString(&NSString::from_str("Appearance"), mtm);
    label.setFrame(NSRect::new(NSPoint::new(20.0, 172.0), NSSize::new(140.0, 20.0)));
    content.addSubview(&label);
    let popup = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        NSRect::new(NSPoint::new(170.0, 168.0), NSSize::new(180.0, 28.0)),
        false,
    );
    let names: Vec<_> = APPEARANCES.iter().map(|a| NSString::from_str(a)).collect();
    popup.addItemsWithTitles(&NSArray::from_retained_slice(&names));
    popup.selectItemWithTitle(&NSString::from_str(&appearance()));
    popup.setTag(APPEARANCE_TAG);
    unsafe {
        popup.setTarget(Some(delegate));
        popup.setAction(Some(sel!(settingChanged:)));
    }
    content.addSubview(&popup);

    let label = NSTextField::labelWithString(&NSString::from_str("Glassiness"), mtm);
    label.setFrame(NSRect::new(NSPoint::new(20.0, 136.0), NSSize::new(140.0, 20.0)));
    content.addSubview(&label);
    let slider = unsafe { NSSlider::sliderWithValue_minValue_maxValue_target_action(glassiness(), 0.0, 1.0, Some(delegate), Some(sel!(glassChanged:)), mtm) };
    slider.setFrame(NSRect::new(NSPoint::new(172.0, 134.0), NSSize::new(176.0, 24.0)));
    slider.setEnabled(crate::wheel::has_glass());
    content.addSubview(&slider);

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
    if popup.tag() == APPEARANCE_TAG {
        if let Some(title) = popup.titleOfSelectedItem() {
            unsafe { NSUserDefaults::standardUserDefaults().setObject_forKey(Some(&title), &NSString::from_str(APPEARANCE_KEY)) };
            apply_appearance(popup.mtm());
        }
        return;
    }
    let Some(kind) = WheelKind::ALL.get(popup.tag() as usize) else { return };
    if let Some(title) = popup.titleOfSelectedItem() {
        unsafe { NSUserDefaults::standardUserDefaults().setObject_forKey(Some(&title), &NSString::from_str(&defaults_key(*kind))) };
    }
}

pub fn glass_changed(slider: &NSSlider) {
    NSUserDefaults::standardUserDefaults().setDouble_forKey(slider.doubleValue(), &NSString::from_str(GLASS_KEY));
}
