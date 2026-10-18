//! The radial wheel: a transparent panel under the pointer that accepts the dragged files.

use crate::actions::Action;
use crate::WheelKind;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSDragOperation, NSDraggingInfo, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSPanel, NSPasteboardTypeFileURL, NSPopUpMenuWindowLevel, NSStringDrawing, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::path::PathBuf;

const SIZE: f64 = 340.0;
const OUTER: f64 = 160.0;
const INNER: f64 = 52.0;

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
}

/// Segment under `p` (view coordinates), counted clockwise from 12 o'clock.
fn segment_at(p: NSPoint, n: usize) -> Option<usize> {
    let (dx, dy) = (p.x - SIZE / 2.0, p.y - SIZE / 2.0);
    let dist = dx.hypot(dy);
    if n == 0 || !(INNER..=OUTER).contains(&dist) {
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

            NSColor::colorWithWhite_alpha(0.08, 0.88).setFill();
            NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(c.x - OUTER, c.y - OUTER), NSSize::new(OUTER * 2.0, OUTER * 2.0))).fill();

            let step = 360.0 / n.max(1) as f64;
            let white = NSColor::whiteColor();
            let label_attrs = attrs(if n > 9 { 11.0 } else { 13.0 }, &white, true);
            for (i, choice) in data.choices.iter().enumerate() {
                let (start, end) = (90.0 - i as f64 * step, 90.0 - (i + 1) as f64 * step);
                if data.hovered == Some(i) {
                    let wedge = NSBezierPath::bezierPath();
                    wedge.appendBezierPathWithArcWithCenter_radius_startAngle_endAngle_clockwise(c, OUTER, start, end, true);
                    wedge.appendBezierPathWithArcWithCenter_radius_startAngle_endAngle_clockwise(c, INNER, end, start, false);
                    wedge.closePath();
                    NSColor::controlAccentColor().setFill();
                    wedge.fill();
                }
                if n > 1 {
                    let a = start.to_radians();
                    let line = NSBezierPath::bezierPath();
                    line.moveToPoint(NSPoint::new(c.x + INNER * a.cos(), c.y + INNER * a.sin()));
                    line.lineToPoint(NSPoint::new(c.x + OUTER * a.cos(), c.y + OUTER * a.sin()));
                    NSColor::colorWithWhite_alpha(1.0, 0.15).setStroke();
                    line.stroke();
                }
                let mid = ((start + end) / 2.0).to_radians();
                let r = (INNER + OUTER) / 2.0;
                draw_centered(&choice.label(), NSPoint::new(c.x + r * mid.cos(), c.y + r * mid.sin()), &label_attrs);
            }

            NSColor::colorWithWhite_alpha(0.0, 0.6).setFill();
            NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(c.x - INNER, c.y - INNER), NSSize::new(INNER * 2.0, INNER * 2.0))).fill();
            let title = data.kind.map(WheelKind::title).unwrap_or("");
            let dim = NSColor::colorWithWhite_alpha(1.0, 0.7);
            if n == 0 {
                draw_centered("Nothing to do", NSPoint::new(c.x, c.y + INNER + 40.0), &attrs(13.0, &white, true));
            }
            draw_centered(title, c, &attrs(12.0, &dim, false));
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
    static WHEEL: RefCell<Option<(Retained<NSPanel>, Retained<WheelView>)>> = const { RefCell::new(None) };
}

fn choices(kind: WheelKind, files: &[PathBuf]) -> Vec<Choice> {
    match kind {
        WheelKind::Convert => lime_core::ops(lime_core::Wheel::Convert, files).into_iter().map(Choice::Op).collect(),
        WheelKind::Tools => lime_core::ops(lime_core::Wheel::Tools, files).into_iter().map(Choice::Op).collect(),
        WheelKind::Actions => Action::ALL.iter().copied().map(Choice::Action).collect(),
    }
}

pub fn shown() -> Option<WheelKind> {
    WHEEL.with_borrow(|w| {
        w.as_ref().filter(|(panel, _)| panel.isVisible()).and_then(|(_, view)| view.ivars().borrow().kind)
    })
}

pub fn show(mtm: MainThreadMarker, kind: WheelKind, files: &[PathBuf], at: NSPoint) {
    let choices = choices(kind, files);
    WHEEL.with_borrow_mut(|w| {
        let (panel, view) = w.get_or_insert_with(|| {
            let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
                NSPanel::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(SIZE, SIZE)),
                NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                NSBackingStoreType::Buffered,
                false,
            );
            panel.setOpaque(false);
            panel.setBackgroundColor(Some(&NSColor::clearColor()));
            panel.setHasShadow(false);
            panel.setLevel(NSPopUpMenuWindowLevel);
            panel.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary);
            unsafe { panel.setReleasedWhenClosed(false) };
            let view = WheelView::new(mtm);
            panel.setContentView(Some(&view));
            (panel, view)
        });
        // Keep the wheel where it first appeared when switching kinds mid-drag.
        if !panel.isVisible() {
            panel.setFrameOrigin(NSPoint::new(at.x - SIZE / 2.0, at.y - SIZE / 2.0));
        }
        *view.ivars().borrow_mut() = WheelData { kind: Some(kind), choices, hovered: None };
        view.setNeedsDisplay(true);
        panel.orderFrontRegardless();
    });
}

pub fn hide() {
    WHEEL.with_borrow(|w| {
        if let Some((panel, _)) = w {
            panel.orderOut(None);
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
    }
}
