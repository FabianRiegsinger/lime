//! The radial wheel: a transparent panel under the pointer that accepts the dragged files.

use crate::actions::Action;
use crate::WheelKind;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAffineTransformNSAppKitAdditions, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSBackingStoreType, NSBezierPath, NSColor, NSDragOperation, NSDraggingInfo, NSEvent, NSFont, NSFontAttributeName, NSGradient, NSShadow,
    NSForegroundColorAttributeName, NSPanel, NSPasteboardTypeFileURL, NSWindow, NSGlassEffectView, NSGlassEffectViewStyle, NSGraphicsContext, NSPopUpMenuWindowLevel, NSStringDrawing, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSAffineTransform, NSArray, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long the wheel takes to grow out of its centre.
const OPEN_SECS: f64 = 0.2;

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
/// The compress ring around the wheel, opened by resting on Zip during a drag.
const RING_DISC: f64 = 250.0;
const RING_SIZE: f64 = 2.0 * (RING_DISC + 16.0);
/// How long the pointer rests on the Compress button before the ring opens, and stays away from it before it closes.
const OPEN_HOLD: Duration = Duration::from_millis(500);
const CLOSE_HOLD: Duration = Duration::from_millis(1500);

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

/// Matte lime for the hovered button and the progress fill.
struct Lime(f64, f64, f64);
const LIME_LIGHT: Lime = Lime(0.69, 0.83, 0.33);
const LIME_DARK: Lime = Lime(0.45, 0.62, 0.18);

impl Lime {
    fn rgb(&self) -> Retained<NSColor> {
        rgb(self.0, self.1, self.2, 1.0)
    }
}

impl Palette {
    fn new(dark: bool, glass: Option<f64>) -> Self {
        let Some(glass) = glass else {
            let grey = |w: f64, a: f64| NSColor::colorWithWhite_alpha(w, a);
            return if dark {
                Palette {
                    disc: Some(grey(0.18, 0.97)),
                    pulp: grey(0.33, 1.0),
                    pulp_hover: LIME_DARK.rgb(),
                    hub: grey(0.18, 1.0),
                    text: grey(1.0, 0.95),
                    dim: grey(1.0, 0.6),
                }
            } else {
                Palette {
                    disc: Some(grey(0.95, 0.97)),
                    pulp: grey(0.80, 1.0),
                    pulp_hover: LIME_LIGHT.rgb(),
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
                pulp: rgb(0.38, 0.38, 0.40, 0.92),
                pulp_hover: LIME_DARK.rgb(),
                hub: rgb(0.0, 0.0, 0.0, 0.18),
                text: rgb(1.0, 1.0, 1.0, 0.95),
                dim: rgb(1.0, 1.0, 1.0, 0.6),
            }
        } else {
            Palette {
                disc,
                pulp: rgb(0.78, 0.78, 0.80, 0.92),
                pulp_hover: LIME_LIGHT.rgb(),
                hub: rgb(1.0, 1.0, 1.0, 0.35),
                text: rgb(0.0, 0.0, 0.0, 0.85),
                dim: rgb(0.0, 0.0, 0.0, 0.5),
            }
        }
    }
}

fn shadow(dx: f64, dy: f64, blur: f64, color: &NSColor) -> Retained<NSShadow> {
    let s = NSShadow::new();
    s.setShadowOffset(NSSize::new(dx, dy));
    s.setShadowBlurRadius(blur);
    s.setShadowColor(Some(color));
    s
}

/// Soft drop shadow under `shape`; `lift` (points) makes a hovered button float higher.
fn drop_shadow(shape: &NSBezierPath, color: &NSColor, lift: f64, dark: bool) {
    NSGraphicsContext::saveGraphicsState_class();
    shadow(0.0, -lift, lift * 2.5, &NSColor::colorWithWhite_alpha(0.0, if dark { 0.5 } else { 0.25 })).set();
    color.setFill();
    shape.fill();
    NSGraphicsContext::restoreGraphicsState_class();
}

/// Matte top-lit face: a gentle vertical gradient plus a fine light rim just inside the edge.
fn face(shape: &NSBezierPath, color: &NSColor, dark: bool) {
    if let (Some(top), Some(bottom)) = (color.highlightWithLevel(0.18), color.shadowWithLevel(0.08)) {
        if let Some(g) = NSGradient::initWithStartingColor_endingColor(NSGradient::alloc(), &bottom, &top) {
            g.drawInBezierPath_angle(shape, 90.0);
        }
    }
    NSGraphicsContext::saveGraphicsState_class();
    shape.addClip();
    NSColor::colorWithWhite_alpha(1.0, if dark { 0.12 } else { 0.5 }).setStroke();
    shape.setLineWidth(2.0);
    shape.stroke();
    NSGraphicsContext::restoreGraphicsState_class();
}

/// Sinks `shape` into the surface with a shadow falling inside its top edge.
fn inset(shape: &NSBezierPath, c: NSPoint, r: f64, dark: bool) {
    NSGraphicsContext::saveGraphicsState_class();
    shape.addClip();
    shadow(0.0, -2.0, 5.0, &NSColor::colorWithWhite_alpha(0.0, if dark { 0.55 } else { 0.3 })).set();
    NSColor::blackColor().setStroke();
    let ring = circle(c, r + 6.0);
    ring.setLineWidth(12.0);
    ring.stroke();
    NSGraphicsContext::restoreGraphicsState_class();
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
    /// When the wheel opened, and how far its grow-in animation is (0..1).
    opened: Option<Instant>,
    scale: f64,
    busy: Option<Busy>,
    /// Compress formats in the outer ring; empty while it is closed. `hovered` and `busy`
    /// indexes past `choices` point into it.
    ring: Vec<Choice>,
    /// What the ring shows once it opens.
    ring_choices: Vec<Choice>,
    /// When the ring opened, and how far it has grown (0..1).
    ring_opened: Option<Instant>,
    ring_scale: f64,
    /// Since when the pointer rests on Zip, and since when it has been away from the ring.
    on_zip: Option<Instant>,
    away: Option<Instant>,
}

impl WheelData {
    fn choice(&self, i: usize) -> Option<&Choice> {
        self.choices.get(i).or_else(|| self.ring.get(i.checked_sub(self.choices.len())?))
    }

    /// Radius of the glass behind the wheel, and the ring when it is open.
    fn radius(&self) -> f64 {
        if self.ring.is_empty() {
            DISC * self.scale
        } else {
            DISC + (RING_DISC - DISC) * self.ring_scale
        }
    }

    /// Updates the Zip and away timers for the segment now under the pointer.
    fn hold(&mut self, hit: Option<usize>, now: Instant) {
        let on_zip = hit.and_then(|i| self.choices.get(i)).is_some_and(|c| matches!(c, Choice::Op(op) if op.id == "zip"));
        let in_ring = hit.is_some_and(|i| i >= self.choices.len());
        self.on_zip = if on_zip && self.ring.is_empty() && !self.ring_choices.is_empty() { self.on_zip.or(Some(now)) } else { None };
        // Resting on Zip keeps the ring too, so it does not close and reopen under the pointer.
        self.away = if in_ring || on_zip { None } else { self.away.or(Some(now)) };
    }

    /// `Some(true)` when the ring should open at `now`, `Some(false)` when it should close.
    fn ring_due(&self, now: Instant) -> Option<bool> {
        let held = |since: Option<Instant>, hold| since.is_some_and(|t| now.duration_since(t) >= hold);
        if self.busy.is_some() {
            None
        } else if self.ring.is_empty() {
            held(self.on_zip, OPEN_HOLD).then_some(true)
        } else {
            held(self.away, CLOSE_HOLD).then_some(false)
        }
    }
}

/// A chosen button filling from its inner to its outer edge while the job runs.
struct Busy {
    index: usize,
    /// Fill drawn now, easing towards `target`.
    shown: f64,
    target: f64,
    done: Option<Instant>,
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

        /// Clicking elsewhere closes a keyboard-opened wheel, unless it is showing progress.
        #[unsafe(method(resignKeyWindow))]
        fn resign_key(&self) {
            unsafe { msg_send![super(self), resignKeyWindow] }
            if !is_busy() {
                hide();
            }
        }
    }
);

/// Which of `n` segments between radii `inner` and `outer` around `c` lies under `p`, counted clockwise from 12 o'clock.
fn segment_at(p: NSPoint, c: NSPoint, inner: f64, outer: f64, n: usize) -> Option<usize> {
    let (dx, dy) = (p.x - c.x, p.y - c.y);
    let dist = dx.hypot(dy);
    if n == 0 || !(inner..=outer).contains(&dist) {
        return None;
    }
    let clockwise_from_top = (90.0 - dy.atan2(dx).to_degrees()).rem_euclid(360.0);
    Some(((clockwise_from_top / (360.0 / n as f64)) as usize).min(n - 1))
}

fn centre(view: &NSView) -> NSPoint {
    let b = view.bounds();
    NSPoint::new(b.size.width / 2.0, b.size.height / 2.0)
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
            let c = centre(self);
            let (n, m) = (data.choices.len(), data.ring.len());
            let dark = is_dark(self);
            let p = Palette::new(dark, data.glass);
            let (pulp_out, pulp_in) = (OUTER - RIND - PITH, INNER + 1.0);
            if data.scale < 1.0 {
                let t = NSAffineTransform::transform();
                t.translateXBy_yBy(c.x, c.y);
                t.scaleBy(data.scale.max(0.01));
                t.translateXBy_yBy(-c.x, -c.y);
                t.concat();
            }
            if m > 0 {
                let r = data.radius();
                if let Some(disc) = &p.disc {
                    let band = circle(c, r);
                    band.appendBezierPath(&circle(c, DISC));
                    band.setWindingRule(objc2_app_kit::NSWindingRule::EvenOdd);
                    disc.setFill();
                    band.fill();
                }
                let edge = circle(c, r - 0.5);
                edge.setLineWidth(1.0);
                NSColor::colorWithWhite_alpha(1.0, if dark { 0.14 } else { 0.6 }).setStroke();
                edge.stroke();
            }
            if let Some(disc) = &p.disc {
                disc.setFill();
                circle(c, DISC).fill();
            }
            // A fine light edge catches the "light" around the disc.
            let edge = circle(c, DISC - 0.5);
            edge.setLineWidth(1.0);
            NSColor::colorWithWhite_alpha(1.0, if dark { 0.14 } else { 0.6 }).setStroke();
            edge.stroke();

            // Inner radius, outer radius, segment count and position within its band, for wheel and ring alike.
            let band = |i: usize| if i < n { (pulp_in, pulp_out, n, i) } else { (DISC + 4.0, RING_DISC - RIND - PITH, m, i - n) };
            let angles = |k: usize, j: usize| {
                let step = 360.0 / k.max(1) as f64;
                (90.0 - j as f64 * step, 90.0 - (j + 1) as f64 * step)
            };
            // The ring's buttons appear once it has grown.
            let count = if data.ring_scale >= 1.0 { n + m } else { n };
            let shapes: Vec<_> = (0..count)
                .map(|i| {
                    let (ri, ro, k, j) = band(i);
                    let (start, end) = angles(k, j);
                    if k > 1 {
                        segment(c, ri, ro, start, end)
                    } else {
                        let ring = circle(c, pulp_out);
                        ring.appendBezierPath(&circle(c, pulp_in));
                        ring.setWindingRule(objc2_app_kit::NSWindingRule::EvenOdd);
                        Some(ring)
                    }
                })
                .collect();
            let color = |i: usize| if data.hovered == Some(i) { &p.pulp_hover } else { &p.pulp };
            // Shadows first, so no button's shadow falls across a neighbour.
            for (i, shape) in shapes.iter().enumerate() {
                if let Some(shape) = shape {
                    drop_shadow(shape, color(i), if data.hovered == Some(i) { 3.5 } else { 1.2 }, dark);
                }
            }
            let label_attrs = |k: usize| attrs(if k > 9 { 11.0 } else { 13.0 }, &p.text, true);
            // Light mode engraves the labels with a white line below; dark mode lifts them with a black one.
            let emboss = shadow(0.0, -1.0, 0.0, &NSColor::colorWithWhite_alpha(if dark { 0.0 } else { 1.0 }, if dark { 0.5 } else { 0.45 }));
            for (i, choice) in data.choices.iter().chain(&data.ring).enumerate().take(count) {
                let (ri, ro, k, j) = band(i);
                let (start, end) = angles(k, j);
                if let Some(shape) = &shapes[i] {
                    face(shape, color(i), dark);
                    if let Some(busy) = data.busy.as_ref().filter(|b| b.index == i && b.shown > 0.0) {
                        NSGraphicsContext::saveGraphicsState_class();
                        shape.addClip();
                        face(&circle(c, ri + (ro - ri) * busy.shown), &p.pulp_hover, dark);
                        NSGraphicsContext::restoreGraphicsState_class();
                    }
                }
                let at = polar(c, (if i < n { INNER } else { ri } + ro) / 2.0, (start + end) / 2.0);
                let gap = if k > 9 { 8.0 } else { 10.0 };
                NSGraphicsContext::saveGraphicsState_class();
                emboss.set();
                crate::icons::draw(choice, NSPoint::new(at.x, at.y + gap), if k > 9 { 13.0 } else { 16.0 }, &p.text);
                draw_centered(&choice.label(), NSPoint::new(at.x, at.y - gap), &label_attrs(k));
                NSGraphicsContext::restoreGraphicsState_class();
            }

            let hub = circle(c, INNER - 4.0);
            p.hub.setFill();
            hub.fill();
            inset(&hub, c, INNER - 4.0, dark);
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
            self.ivars().borrow_mut().hold(None, Instant::now());
            self.set_hovered(None);
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> Bool {
            let picked = {
                let data = self.ivars().borrow();
                data.hovered.and_then(|i| Some((i, data.choice(i)?.clone())))
            };
            let Some((index, choice)) = picked else {
                hide();
                return Bool::NO;
            };
            self.begin_busy(index);
            crate::queue(choice, crate::files_on(&info.draggingPasteboard()));
            Bool::YES
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let (hovered, n, busy) = {
                let data = self.ivars().borrow();
                (data.hovered, data.choices.len(), data.busy.is_some())
            };
            if busy {
                return;
            }
            match event.keyCode() {
                KEY_RIGHT | KEY_DOWN => self.set_hovered(step(hovered, n, 1)),
                KEY_LEFT | KEY_UP => self.set_hovered(step(hovered, n, -1)),
                KEY_RETURN | KEY_ENTER => {
                    let picked = {
                        let data = self.ivars().borrow();
                        data.hovered.and_then(|i| Some((i, data.choices.get(i)?.clone(), data.files.clone()?)))
                    };
                    match picked {
                        Some((index, choice, files)) => {
                            self.begin_busy(index);
                            crate::queue(choice, files);
                        }
                        None => hide(),
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
                (data.kind, data.files.clone().filter(|_| data.busy.is_none()))
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
        let (p, c) = (self.convertPoint_fromView(info.draggingLocation(), None), centre(self));
        let (n, m, busy) = {
            let data = self.ivars().borrow();
            (data.choices.len(), data.ring.len(), data.busy.is_some())
        };
        if busy {
            return NSDragOperation::None;
        }
        let hit = segment_at(p, c, INNER, DISC, n).or_else(|| segment_at(p, c, DISC, RING_DISC, m).map(|i| n + i));
        self.ivars().borrow_mut().hold(hit, Instant::now());
        self.set_hovered(hit);
        if hit.is_some() {
            NSDragOperation::Copy
        } else {
            NSDragOperation::None
        }
    }

    fn begin_busy(&self, index: usize) {
        {
            let mut data = self.ivars().borrow_mut();
            data.hovered = None;
            data.busy = Some(Busy { index, shown: 0.0, target: 0.0, done: None });
        }
        self.setNeedsDisplay(true);
        crate::animate();
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

/// The wheel's choices, and the ring's.
fn choices(kind: WheelKind, files: &[PathBuf]) -> (Vec<Choice>, Vec<Choice>) {
    let ops = |ops: Vec<lime_core::Op>| ops.into_iter().map(Choice::Op).collect();
    match kind {
        WheelKind::Convert => (ops(lime_core::ops(lime_core::Wheel::Convert, files)), vec![]),
        WheelKind::Tools => {
            // Zip and Compress share one button: dropping zips, resting on it opens the ring with
            // the file's own Compress and every archive format.
            let mut tools = lime_core::ops(lime_core::Wheel::Tools, files);
            let mut ring: Vec<lime_core::Op> = tools.iter().position(|o| o.id == "compress").map(|i| tools.remove(i)).into_iter().collect();
            for o in ring.iter_mut().chain(&mut tools) {
                o.label = match o.id.as_str() {
                    "zip" => "Compress",
                    "compress" => "Shrink",
                    _ => continue,
                }
                .into();
            }
            ring.extend(lime_core::compress_ops());
            (ops(tools), ops(ring))
        }
        WheelKind::Actions => (Action::ALL.iter().copied().map(Choice::Action).collect(), vec![]),
    }
}

/// Liquid Glass (`NSGlassEffectView`) exists from macOS 26; older systems get the grey wheel.
pub fn has_glass() -> bool {
    objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some()
}

pub fn shown() -> Option<WheelKind> {
    WHEEL.with_borrow(|w| {
        // A wheel showing progress counts as closed, so a new drag gets a fresh one.
        w.as_ref().filter(|(panel, ..)| panel.isVisible()).map(|(_, view, _)| view.ivars().borrow()).filter(|d| d.busy.is_none()).and_then(|d| d.kind)
    })
}

/// Opens the wheel centred on `at`. With `keyboard` it takes key focus and remembers `files`,
/// since there is no drag pasteboard to read them from.
pub fn show(mtm: MainThreadMarker, kind: WheelKind, files: &[PathBuf], at: NSPoint, keyboard: bool) {
    let (choices, ring_choices) = choices(kind, files);
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
                // Regular, not Clear: Clear adapts to what is behind it and fades to almost nothing over light windows.
                glass.setStyle(NSGlassEffectViewStyle::Regular);
                root.addSubview(&glass);
                glass
            });
            let view = WheelView::new(mtm);
            root.addSubview(&view);
            panel.setContentView(Some(&root));
            (panel, view, glass)
        });
        // Keep the wheel where it first appeared when switching kinds mid-drag; grow it in when it is new.
        let fresh = !panel.isVisible() || view.ivars().borrow().busy.is_some();
        resize(panel, view, if fresh { at } else { centre_of(panel) }, SIZE);
        let (opened, scale) = if fresh { (Some(Instant::now()), 0.0) } else { (view.ivars().borrow().opened, view.ivars().borrow().scale) };
        // The view paints a solid disc over the glass that fades out as glassiness rises.
        let g = glass.as_ref().map(|_| crate::settings::glassiness());
        let hovered = if keyboard && !choices.is_empty() { Some(0) } else { None };
        let files = keyboard.then(|| files.to_vec());
        *view.ivars().borrow_mut() = WheelData { kind: Some(kind), choices, hovered, glass: g, files, opened, scale, ring_choices, ..Default::default() };
        place(panel, glass.as_deref(), scale, DISC * scale);
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
    crate::animate();
}

/// Fades the panel for grow-in step `scale` (0..1) and sizes the glass disc to radius `r`.
fn place(panel: &NSPanel, glass: Option<&NSGlassEffectView>, scale: f64, r: f64) {
    panel.setAlphaValue(scale);
    if let Some(glass) = glass {
        let c = panel.frame().size.width / 2.0;
        glass.setFrame(NSRect::new(NSPoint::new(c - r, c - r), NSSize::new(r * 2.0, r * 2.0)));
        glass.setCornerRadius(r);
    } else {
        panel.invalidateShadow(); // follows the growing ring
    }
}

fn centre_of(panel: &NSPanel) -> NSPoint {
    let f = panel.frame();
    NSPoint::new(f.origin.x + f.size.width / 2.0, f.origin.y + f.size.height / 2.0)
}

/// Makes the panel and the wheel view a `size` square around `centre` (screen coordinates).
fn resize(panel: &NSPanel, view: &WheelView, centre: NSPoint, size: f64) {
    panel.setFrame_display(NSRect::new(NSPoint::new(centre.x - size / 2.0, centre.y - size / 2.0), NSSize::new(size, size)), false);
    view.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size, size)));
}

/// Opens the compress ring once the pointer has rested on Compress for `OPEN_HOLD`, and closes it once the
/// pointer has been away from it for `CLOSE_HOLD`, so crossing the wheel on the way to the other side keeps it.
pub fn tick() {
    let changed = WHEEL.with_borrow(|w| {
        let Some((panel, view, glass)) = w else { return false };
        let due = view.ivars().borrow().ring_due(Instant::now());
        let Some(open) = due.filter(|_| panel.isVisible()) else { return false };
        {
            let mut d = view.ivars().borrow_mut();
            d.ring = if open { d.ring_choices.clone() } else { vec![] };
            d.ring_opened = open.then(Instant::now);
            d.ring_scale = 0.0;
            (d.on_zip, d.away) = (None, None);
            if d.hovered.is_some_and(|i| i >= d.choices.len()) {
                d.hovered = None;
            }
        }
        resize(panel, view, centre_of(panel), if open { RING_SIZE } else { SIZE });
        place(panel, glass.as_deref(), 1.0, view.ivars().borrow().radius());
        view.setNeedsDisplay(true);
        true
    });
    if changed {
        crate::animate();
    }
}

/// Runs `f` on the wheel's progress state, if a button is filling.
fn with_busy(f: impl FnOnce(&mut Busy)) {
    WHEEL.with_borrow(|w| {
        if let Some((_, view, _)) = w {
            if let Some(busy) = view.ivars().borrow_mut().busy.as_mut() {
                f(busy);
            }
        }
    });
}

pub fn is_busy() -> bool {
    WHEEL.with(|w| w.try_borrow().ok().and_then(|w| w.as_ref().map(|(_, view, _)| view.ivars().borrow().busy.is_some())).unwrap_or(false))
}

/// Moves the filling button towards `f` (0..1); it never shrinks.
pub fn progress(f: f64) {
    with_busy(|b| b.target = b.target.max(f.min(1.0)));
}

/// Fills the button completely, then closes the wheel.
pub fn finish() {
    with_busy(|b| {
        b.target = 1.0;
        b.done.get_or_insert_with(Instant::now);
    });
}

/// One animation frame: grow-in and progress fill. False once nothing moves any more.
pub fn animate() -> bool {
    WHEEL.with_borrow(|w| {
        let Some((panel, view, glass)) = w else { return false };
        if !panel.isVisible() {
            return false;
        }
        let (scale, radius, opening, busy, close) = {
            let mut d = view.ivars().borrow_mut();
            let grown = |since: Option<Instant>| since.map_or(1.0, |o| (o.elapsed().as_secs_f64() / OPEN_SECS).min(1.0));
            let (t, rt) = (grown(d.opened), grown(d.ring_opened));
            d.scale = 1.0 - (1.0 - t).powi(3); // ease out
            d.ring_scale = 1.0 - (1.0 - rt).powi(3);
            let close = d.busy.as_mut().is_some_and(|b| {
                b.shown += (b.target - b.shown) * 0.25;
                if b.target - b.shown < 0.003 {
                    b.shown = b.target;
                }
                b.shown >= 1.0 && b.done.is_some_and(|t| t.elapsed() > Duration::from_millis(250))
            });
            if close {
                d.busy = None;
            }
            (d.scale, d.radius(), t < 1.0 || rt < 1.0, d.busy.is_some(), close)
        };
        place(panel, glass.as_deref(), scale, radius);
        view.setNeedsDisplay(true);
        if close {
            panel.orderOut(None);
        }
        opening || busy
    })
}

pub fn hide() {
    // try_borrow: ordering out resigns key, and resignKeyWindow calls back in here.
    WHEEL.with(|w| {
        if let Ok(w) = w.try_borrow() {
            if let Some((panel, view, _)) = &*w {
                if let Ok(mut data) = view.ivars().try_borrow_mut() {
                    data.busy = None;
                }
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
        let at = |p: NSPoint, n| segment_at(p, NSPoint::new(c, c), INNER, DISC, n);
        assert_eq!(at(NSPoint::new(c, c + r), 4), Some(0)); // top
        assert_eq!(at(NSPoint::new(c + r, c - 1.0), 4), Some(1)); // right, just below horizontal
        assert_eq!(at(NSPoint::new(c - 1.0, c - r), 4), Some(2)); // bottom
        assert_eq!(at(NSPoint::new(c - r, c + 1.0), 4), Some(3)); // left, just above horizontal
        assert_eq!(at(NSPoint::new(c, c), 4), None); // centre cancels
        assert_eq!(at(NSPoint::new(c + r, c), 0), None);
        assert_eq!(at(NSPoint::new(c + DISC + 1.0, c), 4), None); // just outside the disc
    }

    #[test]
    fn ring_opens_after_resting_on_zip_and_survives_crossing() {
        let zip = Choice::Op(lime_core::Op { id: "zip".into(), label: "Compress".into(), param: None });
        let other = Choice::Op(lime_core::Op { id: "trim".into(), label: "Trim".into(), param: None });
        let mut d = WheelData { choices: vec![other.clone(), zip], ring_choices: vec![other.clone(); 4], ..Default::default() };
        let t0 = Instant::now();
        let at = |s: f64| t0 + Duration::from_secs_f64(s);
        d.hold(Some(1), at(0.0));
        assert_eq!(d.ring_due(at(0.4)), None);
        d.hold(Some(0), at(0.4)); // left early: the hold restarts
        d.hold(Some(1), at(1.0));
        assert_eq!(d.ring_due(at(1.4)), None);
        assert_eq!(d.ring_due(at(1.5)), Some(true));
        d.ring = d.ring_choices.clone();
        (d.on_zip, d.away) = (None, None);
        d.hold(Some(3), at(2.0)); // on the ring
        d.hold(Some(0), at(2.0)); // crossing the middle
        d.hold(None, at(2.5));
        d.hold(Some(4), at(3.0)); // back on the ring in time
        assert_eq!(d.ring_due(at(9.0)), None);
        d.hold(None, at(9.0));
        assert_eq!(d.ring_due(at(10.4)), None);
        assert_eq!(d.ring_due(at(10.5)), Some(false));
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
