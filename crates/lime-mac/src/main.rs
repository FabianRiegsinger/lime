//! Lime menu-bar app: watches for file drags, shows a wheel when a modifier key is held,
//! and runs the chosen operation through the bundled `lime` CLI.

mod actions;
mod icons;
mod jobs;
mod keyboard;
mod settings;
mod wheel;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSEvent, NSImage, NSMenu, NSMenuItem, NSPasteboard,
    NSPasteboardNameDrag, NSPasteboardTypeFileURL, NSPopUpButton, NSSlider, NSStatusBar, NSStatusItem, NSVariableStatusItemLength, NSWorkspace,
};
use objc2_foundation::{ns_string, NSData, NSNotification, NSObject, NSObjectProtocol, NSSize, NSString, NSTimer, NSURL};
use std::cell::RefCell;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelKind {
    Convert,
    Tools,
    Actions,
}

impl WheelKind {
    pub const ALL: [WheelKind; 3] = [WheelKind::Convert, WheelKind::Tools, WheelKind::Actions];

    pub fn title(self) -> &'static str {
        match self {
            WheelKind::Convert => "Convert",
            WheelKind::Tools => "Tools",
            WheelKind::Actions => "Actions",
        }
    }
}

/// Main-thread app state; AppKit objects are not Send, so it lives in a thread-local.
#[derive(Default)]
struct State {
    delegate: Option<Retained<Delegate>>,
    status_item: Option<Retained<NSStatusItem>>,
    /// Drag pasteboard change count already seen; a new value means a new drag started.
    drag_count: isize,
    dragging: bool,
    files: Vec<PathBuf>,
    /// Ticks since the mouse was released while a wheel was shown.
    released_ticks: u32,
    pending: Option<(wheel::Choice, Vec<PathBuf>)>,
    /// An action waiting for its button to finish filling, so a dialog it opens does not cover the wheel.
    pending_action: Option<(actions::Action, Vec<PathBuf>)>,
    /// The job whose progress the wheel shows.
    wheel_job: Option<usize>,
    /// Runs at 60 fps only while the wheel animates; the regular tick is too coarse.
    animation: Option<Retained<NSTimer>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// File paths on a pasteboard, resolving Finder's file-reference URLs.
pub fn files_on(pb: &NSPasteboard) -> Vec<PathBuf> {
    let Some(items) = pb.pasteboardItems() else { return vec![] };
    items
        .iter()
        .filter_map(|item| {
            let s = item.stringForType(unsafe { NSPasteboardTypeFileURL })?;
            NSURL::URLWithString(&s)?.filePathURL()?.to_file_path()
        })
        .collect()
}

/// Starts the animation timer unless it is already running.
pub fn animate() {
    STATE.with_borrow_mut(|s| {
        if s.animation.is_none() {
            let delegate = s.delegate.as_ref().expect("delegate is set in main");
            s.animation = Some(unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(1.0 / 60.0, delegate, sel!(animate:), None, true)
            });
        }
    });
}

/// Queues a dropped choice; it runs on the next tick so the drag session can finish first.
pub fn queue(choice: wheel::Choice, files: Vec<PathBuf>) {
    STATE.with_borrow_mut(|s| s.pending = Some((choice, files)));
}

fn tick(mtm: MainThreadMarker) {
    if let Some((choice, files)) = STATE.with_borrow_mut(|s| s.pending.take()) {
        match choice {
            wheel::Choice::Op(op) => match jobs::start(mtm, op, files) {
                Some(job) => STATE.with_borrow_mut(|s| s.wheel_job = Some(job)),
                None => wheel::hide(),
            },
            // Actions are instant: fill the button first, then act.
            wheel::Choice::Action(a) => {
                wheel::finish();
                STATE.with_borrow_mut(|s| s.pending_action = Some((a, files)));
            }
        }
    }
    if !wheel::is_busy() {
        if let Some((a, files)) = STATE.with_borrow_mut(|s| s.pending_action.take()) {
            actions::run(mtm, a, &files);
        }
    }
    if let Some(job) = STATE.with_borrow(|s| s.wheel_job) {
        let done = match jobs::progress(job) {
            jobs::Progress::Running(f) => {
                wheel::progress(f);
                false
            }
            jobs::Progress::Done => {
                wheel::finish();
                true
            }
            jobs::Progress::Failed => {
                wheel::hide();
                true
            }
        };
        if done {
            STATE.with_borrow_mut(|s| s.wheel_job = None);
        }
    }
    jobs::refresh(mtm);
    keyboard::tick(mtm);
    wheel::tick();

    let mouse_down = NSEvent::pressedMouseButtons() & 1 != 0;
    let count = NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag }).changeCount();
    let new_drag = STATE.with_borrow_mut(|s| {
        if !mouse_down {
            s.dragging = false;
            s.released_ticks += 1;
            // Give performDragOperation a moment before closing the wheel on a miss or cancel.
            if s.released_ticks == 10 && !wheel::is_busy() {
                wheel::hide();
            }
            return false;
        }
        s.released_ticks = 0;
        let new = count != s.drag_count;
        s.drag_count = count;
        new
    });
    if new_drag {
        let files = files_on(&NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag }));
        STATE.with_borrow_mut(|s| {
            s.dragging = !files.is_empty();
            s.files = files;
        });
    }
    let (dragging, files) = STATE.with_borrow(|s| (s.dragging, s.files.clone()));
    if !dragging {
        return;
    }
    // The wheel stays open after the key is released; another key switches it.
    if let Some(kind) = settings::wheel_for(NSEvent::modifierFlags_class()) {
        if wheel::shown() != Some(kind) {
            wheel::show(mtm, kind, &files, NSEvent::mouseLocation(), false);
        }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "LimeAppDelegate"]
    pub struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _n: &NSNotification) {
            let mtm = self.mtm();
            settings::apply_appearance(mtm);
            keyboard::install();
            STATE.with_borrow_mut(|s| {
                s.drag_count = NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag }).changeCount();
                s.status_item = Some(status_item(mtm, self));
            });
            unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(0.03, self, sel!(tick:), None, true);
            }
        }

        #[unsafe(method(applicationDidResignActive:))]
        fn did_resign_active(&self, _n: &NSNotification) {
            settings::hide();
        }
    }

    impl Delegate {
        #[unsafe(method(tick:))]
        fn tick(&self, _t: &NSTimer) {
            tick(self.mtm());
        }

        #[unsafe(method(animate:))]
        fn animate(&self, timer: &NSTimer) {
            if !wheel::animate() {
                timer.invalidate();
                STATE.with_borrow_mut(|s| s.animation = None);
            }
        }

        /// Third-party license notices, bundled by scripts/bundle.sh.
        #[unsafe(method(openAcknowledgements:))]
        fn open_acknowledgements(&self, _sender: Option<&AnyObject>) {
            let file = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("../Resources/Acknowledgements.txt")));
            if let Some(url) = file.filter(|f| f.exists()).and_then(|f| NSURL::from_file_path(f.canonicalize().ok()?)) {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            settings::open(self.mtm(), self);
        }

        #[unsafe(method(settingChanged:))]
        fn setting_changed(&self, sender: &NSPopUpButton) {
            settings::changed(sender);
        }

        #[unsafe(method(glassChanged:))]
        fn glass_changed(&self, sender: &NSSlider) {
            settings::glass_changed(sender);
        }
    }
);

fn status_item(mtm: MainThreadMarker, delegate: &Delegate) -> Retained<NSStatusItem> {
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        // Template image: macOS draws it black on light menu bars and white on dark ones.
        let data = NSData::with_bytes(include_bytes!("../../../assets/menubar.png"));
        if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
            image.setSize(NSSize::new(22.0, 22.0));
            image.setTemplate(true);
            button.setImage(Some(&image));
        }
    }
    let menu = NSMenu::new(mtm);
    let add = |title: &NSString, action, key: &NSString| {
        let mi = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), title, action, key) };
        menu.addItem(&mi);
        mi
    };
    add(ns_string!("Drag a file, then hold a wheel key"), None, ns_string!(""));
    add(ns_string!("Or select files in Finder and press ⌥↩"), None, ns_string!(""));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let settings_item = add(ns_string!("Settings…"), Some(sel!(openSettings:)), ns_string!(","));
    unsafe { settings_item.setTarget(Some(delegate)) };
    let notices = add(ns_string!("Acknowledgements…"), Some(sel!(openAcknowledgements:)), ns_string!(""));
    unsafe { notices.setTarget(Some(delegate)) };
    add(ns_string!("Quit Lime"), Some(sel!(terminate:)), ns_string!("q"));
    item.setMenu(Some(&menu));
    item
}

fn main() {
    let mtm = MainThreadMarker::new().expect("must start on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    STATE.with_borrow_mut(|s| s.delegate = Some(delegate));
    app.run();
}
