//! The radial wheel: a transparent panel under the pointer that accepts the dragged files.

use crate::actions::Action;
use crate::WheelKind;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSBackingStoreType, NSBezierPath, NSColor, NSDragOperation, NSDraggingInfo, NSEvent, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSPanel, NSPasteboardTypeFileURL, NSWindow, NSGlassEffectView, NSGlassEffectViewStyle, NSPopUpMenuWindowLevel, NSStringDrawing, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::path::PathBuf;

const SIZE: f64 = 340.0;
const OUTER: f64 = 160.0;
/// Radius of the background disc: a thin border around the segments.
const DISC: f64 = OUTER - 6.0;
const INNER: f64 = 52.0;
const RIND: f64 = 7.0;
const PITH: f64 = 5.0;
/// Half the white membrane between segments, and the corner radius of each segment.
const MEMBRANE: f64 = 2.5;
const ROUND: f64 = 12.0;

/// Neutral tints laid over the glass. `glass` (0..1) only fades the background disc from solid to see-through.
/// Without Liquid Glass (`None`) the wheel is a solid grey disc with slightly greyer segments.
struct Palette {
    disc: Option<Retained<NSColor>>,
    pulp: Retained<NSColor>,
    pulp_hover: Retained<NSColor>,
    hub: Retained<NSColor>,
    text: Retained<NSColor>,
    dim: Retained<NSColor>,
}

fn rgb(r: f64, g: f64, b: f64, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a)
}

impl Palette {
    fn new(dark: bool, glass: Option<f64>) -> Self {
        let Some(glass) = glass else {
            let grey = |w: f64, a: f64| NSColor::colorWithWhite_alpha(w, a);
            return if dark {
                Palette {
                    disc: Some(grey(0.18, 0.97)),
                    pulp: grey(0.27, 1.0),
                    pulp_hover: grey(0.40, 1.0),
                    hub: grey(0.18, 1.0),
                    text: grey(1.0, 0.95),
                    dim: grey(1.0, 0.6),
                }
            } else {
                Palette {
                    disc: Some(grey(0.95, 0.97)),
                    pulp: grey(0.85, 1.0),
                    pulp_hover: grey(0.74, 1.0),
                    hub: grey(0.95, 1.0),
                    text: grey(0.0, 0.85),
                    dim: grey(0.0, 0.5),
                }
            };
        };
        let disc = Some(NSColor::windowBackgroundColor().colorWithAlphaComponent(1.0 - glass));
        if dark {
            Palette {
                disc,
                pulp: rgb(0.30, 0.30, 0.31, 0.82),
                pulp_hover: rgb(0.42, 0.42, 0.43, 0.9),
                hub: rgb(0.0, 0.0, 0.0, 0.18),
                text: rgb(1.0, 1.0, 1.0, 0.95),
                dim: rgb(1.0, 1.0, 1.0, 0.6),
            }
        } else {
            Palette {
                disc,
                pulp: rgb(0.96, 0.96, 0.97, 0.82),
                pulp_hover: rgb(1.0, 1.0, 1.0, 0.97),
                hub: rgb(1.0, 1.0, 1.0, 0.35),
                text: rgb(0.0, 0.0, 0.0, 0.85),
                dim: rgb(0.0, 0.0, 0.0, 0.5),
            }
        }
    }
}

fn is_dark(view: &NSView) -> bool {
    let (aqua, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    view.effectiveAppearance()
        .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[aqua, dark]))
        .is_some_and(|name| name.isEqualToString(dark))
}

fn circle(c: NSPoint, r: f64) -> Retained<NSBezierPath> {
    NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(c.x - r, c.y - r), NSSize::new(r * 2.0, r * 2.0)))
}

fn polar(c: NSPoint, r: f64, degrees: f64) -> NSPoint {
    let a = degrees.to_radians();
    NSPoint::new(c.x + r * a.cos(), c.y + r * a.sin())
}

/// One segment from `start` clockwise to `end` (degrees), its sides kept `MEMBRANE` away from the
/// radial dividers so the gaps stay even, with `ROUND` corners (smaller when the segment is narrow). Fill only: a stroke would double the
/// alpha along the edge of a translucent fill.
fn segment(c: NSPoint, ri: f64, ro: f64, start: f64, end: f64) -> Option<Retained<NSBezierPath>> {
    let (off_o, off_i) = ((MEMBRANE / ro).asin().to_degrees(), (MEMBRANE / ri).asin().to_degrees());
    let round = ROUND.min(((start - end) / 2.0 - off_i).to_radians() * ri * 0.8);
    if round < 1.0 {
        return None;
    }
    let (r_o, r_i) = ((round / ro).to_degrees(), (round / ri).to_degrees());
    let (p1, p2) = (polar(c, ro, start - off_o), polar(c, ro, end + off_o));
    let (p3, p4) = (polar(c, ri, end + off_i), polar(c, ri, start - off_i));
    let path = NSBezierPath::bezierPath();
    path.moveToPoint(polar(c, ro, start - off_o - r_o));
    path.appendBezierPathWithArcWithCenter_radius_startAngle_endAngle_clockwise(c, ro, start - off_o - r_o, end + off_o + r_o, true);
    path.appendBezierPathWithArcFromPoint_toPoint_radius(p2, p3, round);
    path.appendBezierPathWithArcFromPoint_toPoint_radius(p3, polar(c, ri, end + off_i + r_i), round);
    path.appendBezierPathWithArcWithCenter_radius_startAngle_endAngle_clockwise(c, ri, end + off_i + r_i, start - off_i - r_i, false);
    path.appendBezierPathWithArcFromPoint_toPoint_radius(p4, p1, round);
    path.appendBezierPathWithArcFromPoint_toPoint_radius(p1, polar(c, ro, start - off_o - r_o), round);
    path.closePath();
    Some(path)
}

#[derive(Clone, Debug)]
pub enum Choice {
    Op(lime_core::Op),
    Action(Action),
}

impl Choice {
    fn label(&self) -> String {
        match self {
            Choice::Op(op) => op.label.clone(),
            Choice::Action(a) => a.label().into(),
        }
    }
}

#[derive(Default)]
pub struct WheelData {
    kind: Option<WheelKind>,
    choices: Vec<Choice>,
    hovered: Option<usize>,
    /// Glassiness, or `None` when Liquid Glass is unavailable.
    glass: Option<f64>,
    /// Files chosen in Finder when the wheel was opened from the keyboard; `None` during a drag.
    files: Option<Vec<PathBuf>>,
}

/// The choice `delta` steps clockwise from `current`, wrapping around; the first one when nothing is selected.
fn step(current: Option<usize>, n: usize, delta: isize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    Some(current.map_or(0, |i| (i as isize + delta).rem_euclid(n as isize) as usize))
}

const KEY_RETURN: u16 = 36;
const KEY_ENTER: u16 = 76;
const KEY_ESCAPE: u16 = 53;
const KEY_LEFT: u16 = 123;
const KEY_RIGHT: u16 = 124;
const KEY_DOWN: u16 = 125;
const KEY_UP: u16 = 126;

define_class!(
    /// Borderless panels refuse key status by default; the keyboard flow needs it.
    #[unsafe(super(NSPanel, NSWindow))]
    #[thread_kind = MainThreadOnly]
    #[name = "LimeWheelPanel"]
    struct WheelPanel;

    impl WheelPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            true
        }

        /// Clicking elsewhere closes a keyboard-opened wheel.
        #[unsafe(method(resignKeyWindow))]
        fn resign_key(&self) {
            unsafe { msg_send![super(self), resignKeyWindow] }
            hide();
        }
    }
);

/// Segment under `p` (view coordinates), counted clockwise from 12 o'clock.
fn segment_at(p: NSPoint, n: usize) -> Option<usize> {
    let (dx, dy) = (p.x - SIZE / 2.0, p.y - SIZE / 2.0);
    let dist = dx.hypot(dy);
    if n == 0 || !(INNER..=DISC).contains(&dist) {
        return None;
    }
    let clockwise_from_top = (90.0 - dy.atan2(dx).to_degrees()).rem_euclid(360.0);
    Some(((clockwise_from_top / (360.0 / n as f64)) as usize).min(n - 1))
}

fn attrs(size: f64, color: &NSColor, bold: bool) -> Retained<NSDictionary<NSString, AnyObject>> {
    let font = if bold { NSFont::boldSystemFontOfSize(size) } else { NSFont::systemFontOfSize(size) };
    let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    let values: [&AnyObject; 2] = [&font, color];
    NSDictionary::from_slices(&keys, &values)
}

/// Draws `text` centred on `at`.
fn draw_centered(text: &str, at: NSPoint, attrs: &NSDictionary<NSString, AnyObject>) {
    let s = NSString::from_str(text);
    unsafe {
        let size = s.sizeWithAttributes(Some(attrs));
        s.drawAtPoint_withAttributes(NSPoint::new(at.x - size.width / 2.0, at.y - size.height / 2.0), Some(attrs));
    }
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "LimeWheelView"]
    #[ivars = RefCell<WheelData>]
    pub struct WheelView;

    impl WheelView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let data = self.ivars().borrow();
            let c = NSPoint::new(SIZE / 2.0, SIZE / 2.0);
            let n = data.choices.len();
            let p = Palette::new(is_dark(self), data.glass);
            let (pulp_out, pulp_in) = (OUTER - RIND - PITH, INNER + 1.0);
            if let Some(disc) = &p.disc {
                disc.setFill();
                circle(c, DISC).fill();
            }

            let step = 360.0 / n.max(1) as f64;
            let label_attrs = attrs(if n > 9 { 11.0 } else { 13.0 }, &p.text, true);
            if n <= 1 {
                let ring = circle(c, pulp_out);
                ring.appendBezierPath(&circle(c, pulp_in));
                ring.setWindingRule(objc2_app_kit::NSWindingRule::EvenOdd);
                if data.hovered == Some(0) { p.pulp_hover.setFill() } else { p.pulp.setFill() };
                ring.fill();
            }
            for (i, choice) in data.choices.iter().enumerate() {
                let (start, end) = (90.0 - i as f64 * step, 90.0 - (i + 1) as f64 * step);
                let color = if data.hovered == Some(i) { &p.pulp_hover } else { &p.pulp };
                if n > 1 {
                    if let Some(seg) = segment(c, pulp_in, pulp_out, start, end) {
                        color.setFill();
                        seg.fill();
                    }
                }
                draw_centered(&choice.label(), polar(c, (INNER + pulp_out) / 2.0, (start + end) / 2.0), &label_attrs);
            }

            p.hub.setFill();
            circle(c, INNER - 4.0).fill();
            let title = data.kind.map(WheelKind::title).unwrap_or("");
            if n == 0 {
                draw_centered("Nothing to do", NSPoint::new(c.x, c.y + INNER + 40.0), &attrs(13.0, &p.text, true));
            }
            draw_centered(title, c, &attrs(12.0, &p.dim, false));
        }

        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.track(info)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.track(info)
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _info: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            self.set_hovered(None);
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> Bool {
            let choice = {
                let data = self.ivars().borrow();
                data.hovered.and_then(|i| data.choices.get(i).cloned())
            };
            hide();
            let Some(choice) = choice else { return Bool::NO };
            crate::queue(choice, crate::files_on(&info.draggingPasteboard()));
            Bool::YES
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let (hovered, n) = {
                let data = self.ivars().borrow();
                (data.hovered, data.choices.len())
            };
            match event.keyCode() {
                KEY_RIGHT | KEY_DOWN => self.set_hovered(step(hovered, n, 1)),
                KEY_LEFT | KEY_UP => self.set_hovered(step(hovered, n, -1)),
                KEY_RETURN | KEY_ENTER => {
                    let picked = {
                        let data = self.ivars().borrow();
                        data.hovered.and_then(|i| data.choices.get(i).cloned()).zip(data.files.clone())
                    };
                    hide();
                    if let Some((choice, files)) = picked {
                        crate::queue(choice, files);
                    }
                }
                KEY_ESCAPE => hide(),
                _ => {}
            }
        }

        /// A wheel key switches wheels, as during a drag.
        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            let (kind, files) = {
                let data = self.ivars().borrow();
                (data.kind, data.files.clone())
            };
            let Some(files) = files else { return };
            if let Some(next) = crate::settings::wheel_for(event.modifierFlags()).filter(|k| Some(*k) != kind) {
                show(self.mtm(), next, &files, NSPoint::new(0.0, 0.0), true);
            }
        }
    }
);

impl WheelView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::default());
        let view: Retained<Self> =
            unsafe { msg_send![super(this), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(SIZE, SIZE))] };
        view.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
        view
    }

    fn track(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
        let p = self.convertPoint_fromView(info.draggingLocation(), None);
        let n = self.ivars().borrow().choices.len();
        let hit = segment_at(p, n);
        self.set_hovered(hit);
        if hit.is_some() {
            NSDragOperation::Copy
        } else {
            NSDragOperation::None
        }
    }

    fn set_hovered(&self, h: Option<usize>) {
        let changed = {
            let mut data = self.ivars().borrow_mut();
            std::mem::replace(&mut data.hovered, h) != h
        };
        if changed {
            self.setNeedsDisplay(true);
        }
    }
}

thread_local! {
    static WHEEL: RefCell<Option<(Retained<NSPanel>, Retained<WheelView>, Option<Retained<NSGlassEffectView>>)>> = const { RefCell::new(None) };
}

fn choices(kind: WheelKind, files: &[PathBuf]) -> Vec<Choice> {
    match kind {
        WheelKind::Convert => lime_core::ops(lime_core::Wheel::Convert, files).into_iter().map(Choice::Op).collect(),
        WheelKind::Tools => lime_core::ops(lime_core::Wheel::Tools, files).into_iter().map(Choice::Op).collect(),
        WheelKind::Actions => Action::ALL.iter().copied().map(Choice::Action).collect(),
    }
}

/// Liquid Glass (`NSGlassEffectView`) exists from macOS 26; older systems get the grey wheel.
pub fn has_glass() -> bool {
    objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some()
}

pub fn shown() -> Option<WheelKind> {
    WHEEL.with_borrow(|w| {
        w.as_ref().filter(|(panel, ..)| panel.isVisible()).and_then(|(_, view, _)| view.ivars().borrow().kind)
    })
}

/// Opens the wheel centred on `at`. With `keyboard` it takes key focus and remembers `files`,
/// since there is no drag pasteboard to read them from.
pub fn show(mtm: MainThreadMarker, kind: WheelKind, files: &[PathBuf], at: NSPoint, keyboard: bool) {
    let choices = choices(kind, files);
    WHEEL.with_borrow_mut(|w| {
        let (panel, view, glass) = w.get_or_insert_with(|| {
            let panel: Retained<WheelPanel> = unsafe {
                msg_send![
                    WheelPanel::alloc(mtm),
                    initWithContentRect: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(SIZE, SIZE)),
                    styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                    backing: NSBackingStoreType::Buffered,
                    defer: false
                ]
            };
            let panel = Retained::into_super(panel);
            panel.setOpaque(false);
            panel.setBackgroundColor(Some(&NSColor::clearColor()));
            panel.setHasShadow(!has_glass()); // the glass draws its own

            panel.setLevel(NSPopUpMenuWindowLevel);
            panel.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary);
            unsafe { panel.setReleasedWhenClosed(false) };
            // Liquid Glass disc behind the drawn wheel; the wheel view sits on top and takes the drops.
            let root = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(SIZE, SIZE)));
            let inset = SIZE / 2.0 - DISC;
            let glass = has_glass().then(|| {
                let glass = NSGlassEffectView::initWithFrame(
                    NSGlassEffectView::alloc(mtm),
                    NSRect::new(NSPoint::new(inset, inset), NSSize::new(DISC * 2.0, DISC * 2.0)),
                );
                glass.setCornerRadius(DISC);
                glass.setStyle(NSGlassEffectViewStyle::Clear);
                root.addSubview(&glass);
                glass
            });
            let view = WheelView::new(mtm);
            root.addSubview(&view);
            panel.setContentView(Some(&root));
            (panel, view, glass)
        });
        // Keep the wheel where it first appeared when switching kinds mid-drag.
        if !panel.isVisible() {
            panel.setFrameOrigin(NSPoint::new(at.x - SIZE / 2.0, at.y - SIZE / 2.0));
        }
        // The glass stays clear; the view paints a solid disc over it that fades out as glassiness rises.
        let g = glass.as_ref().map(|_| crate::settings::glassiness());
        let hovered = if keyboard && !choices.is_empty() { Some(0) } else { None };
        let files = keyboard.then(|| files.to_vec());
        *view.ivars().borrow_mut() = WheelData { kind: Some(kind), choices, hovered, glass: g, files };
        view.setNeedsDisplay(true);
        view.displayIfNeeded();
        panel.invalidateShadow(); // the shadow follows the drawn circle, not the square panel
        if keyboard {
            panel.makeKeyAndOrderFront(None);
            panel.makeFirstResponder(Some(view));
        } else {
            panel.orderFrontRegardless();
        }
    });
}

pub fn hide() {
    // try_borrow: ordering out resigns key, and resignKeyWindow calls back in here.
    WHEEL.with(|w| {
        if let Ok(w) = w.try_borrow() {
            if let Some((panel, ..)) = &*w {
                panel.orderOut(None);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_clockwise_from_top() {
        let c = SIZE / 2.0;
        let r = (INNER + OUTER) / 2.0;
        assert_eq!(segment_at(NSPoint::new(c, c + r), 4), Some(0)); // top
        assert_eq!(segment_at(NSPoint::new(c + r, c - 1.0), 4), Some(1)); // right, just below horizontal
        assert_eq!(segment_at(NSPoint::new(c - 1.0, c - r), 4), Some(2)); // bottom
        assert_eq!(segment_at(NSPoint::new(c - r, c + 1.0), 4), Some(3)); // left, just above horizontal
        assert_eq!(segment_at(NSPoint::new(c, c), 4), None); // centre cancels
        assert_eq!(segment_at(NSPoint::new(c + r, c), 0), None);
        assert_eq!(segment_at(NSPoint::new(c + DISC + 1.0, c), 4), None); // just outside the disc
    }

    #[test]
    fn arrow_keys_wrap_around() {
        assert_eq!(step(None, 4, 1), Some(0));
        assert_eq!(step(Some(3), 4, 1), Some(0));
        assert_eq!(step(Some(0), 4, -1), Some(3));
        assert_eq!(step(Some(1), 0, 1), None);
    }

    #[test]
    fn segments_stay_inside_their_ring() {
        let c = NSPoint::new(SIZE / 2.0, SIZE / 2.0);
        let (ri, ro) = (INNER + 1.0, OUTER - RIND - PITH);
        for n in [2, 4, 8, 16] {
            let step = 360.0 / n as f64;
            let b = segment(c, ri, ro, 90.0, 90.0 - step).expect("segment fits").bounds();
            assert!(b.size.width > 0.0 && b.size.height > 0.0, "n={n}");
            assert!(b.origin.x >= c.x - ro - 0.5 && b.origin.x + b.size.width <= c.x + ro + 0.5, "n={n}");
            assert!(b.origin.y + b.size.height <= c.y + ro + 0.5, "n={n}");
        }
    }
}
