//! The drag-and-drop wheel on Windows, as in Lime.app: drag files, hold a key, drop them on a wheel under the pointer.
//!
//! Windows has no drag pasteboard to watch, so `tick` polls the mouse button and keys. Once a drag is under way and a
//! wheel key is held, an almost invisible disc (alpha 2) opens under the pointer. It is a real OLE drop target, so the
//! first `DragEnter` names the dragged files; only then is the wheel drawn. A drag that carries no files never shows it.

use crate::tray::run_op;
use lime_core::{Op, Wheel};
use std::cell::RefCell;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use tiny_skia::{Color, FillRule, GradientStop, LineJoin, LinearGradient, Paint, Path, PathBuilder, Pixmap, Point, SpreadMode, Stroke, Transform};
use windows::core::{w, Ref, Result};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, POINTL, SIZE as WSIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, MonitorFromPoint, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop, ReleaseStgMedium, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_LBUTTON, VK_RBUTTON, VK_SHIFT};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetCursorPos, GetSystemMetrics, RegisterClassW, ShowWindow, UpdateLayeredWindow, MA_NOACTIVATE, SM_CXDRAG,
    SM_CYDRAG, SM_SWAPBUTTON, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_MOUSEACTIVATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

/// How long the wheel takes to grow out of its centre.
const OPEN_SECS: f32 = 0.2;

// Geometry in logical pixels, as in Lime.app.
const SIZE: f32 = 340.0;
const OUTER: f32 = 160.0;
/// Radius of the background disc: a thin border around the segments.
const DISC: f32 = OUTER - 6.0;
const INNER: f32 = 52.0;
const RIND: f32 = 7.0;
const PITH: f32 = 5.0;
/// Half the gap between segments, and the corner radius of each segment.
const MEMBRANE: f32 = 2.5;
const ROUND: f32 = 12.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Convert,
    Tools,
}

impl Kind {
    fn title(self) -> &'static str {
        match self {
            Kind::Convert => "Convert",
            Kind::Tools => "Tools",
        }
    }

    fn ops(self, files: &[PathBuf]) -> Vec<Op> {
        lime_core::ops(if self == Kind::Convert { Wheel::Convert } else { Wheel::Tools }, files)
    }

    /// Shift opens Convert, Ctrl opens Tools. Explorer shows a different cursor badge for them, but the drop is always a copy.
    fn from_keys(shift: bool, ctrl: bool) -> Option<Kind> {
        match (shift, ctrl) {
            (true, false) => Some(Kind::Convert),
            (false, true) => Some(Kind::Tools),
            _ => None,
        }
    }
}

/// A chosen button filling from its inner to its outer edge while the job runs.
struct Busy {
    index: usize,
    started: Instant,
    /// Fill drawn now, easing towards `target`.
    shown: f32,
    target: f32,
    done: Option<Instant>,
}

#[derive(Default)]
struct State {
    hwnd: Option<HWND>,
    visible: bool,
    /// Physical pixels per logical pixel on the wheel's monitor.
    scale: f32,
    /// Top-left corner of the wheel window, in physical screen pixels.
    origin: POINT,
    kind: Option<Kind>,
    /// The dragged files, known from the first `DragEnter`.
    files: Option<Vec<PathBuf>>,
    choices: Vec<Op>,
    hovered: Option<usize>,
    /// When the wheel was first drawn; `None` while it is the invisible drop target.
    opened: Option<Instant>,
    busy: Option<Busy>,
    dark: bool,
    /// Where the mouse button went down, and whether it has since moved far enough to be a drag.
    press: Option<POINT>,
    dragging: bool,
    /// Ticks since the mouse was released while the wheel was shown.
    released_ticks: u32,
    jobs: Option<(Sender<std::result::Result<(), String>>, Receiver<std::result::Result<(), String>>)>,
    fonts: Option<Fonts>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// Call before the event loop is created, so it does not pick a coarser DPI mode first.
pub fn init_dpi() {
    // Fails harmlessly when a manifest or the event loop already set it.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// Creates the (hidden) wheel window on the event loop's thread.
pub fn init() -> Result<()> {
    unsafe {
        // OLE drag and drop needs a single-threaded apartment; S_FALSE (already initialised) is fine.
        let _ = OleInitialize(None);
        let instance = GetModuleHandleW(None)?;
        let class = w!("LimeWheel");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class, ..Default::default() });
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("Lime"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let target: IDropTarget = DropTarget.into();
        RegisterDragDrop(hwnd, &target)?;
        STATE.with_borrow_mut(|s| {
            s.hwnd = Some(hwnd);
            s.jobs = Some(channel());
            s.fonts = Fonts::load();
        });
    }
    Ok(())
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn key_down(vk: u16) -> bool {
    (unsafe { GetAsyncKeyState(vk as i32) }) < 0
}

fn cursor() -> POINT {
    let mut p = POINT::default();
    let _ = unsafe { GetCursorPos(&mut p) };
    p
}

/// Polls for drags and moves animations on. Returns how long to wait before the next tick.
pub fn tick() -> Duration {
    STATE.with_borrow_mut(|s| {
        if s.hwnd.is_none() {
            return Duration::from_secs(3600);
        }
        if let Some(result) = s.jobs.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) {
            match result {
                Ok(()) => {
                    if let Some(b) = s.busy.as_mut() {
                        b.target = 1.0;
                        b.done = Some(Instant::now());
                    }
                }
                Err(e) => {
                    hide(s);
                    std::thread::spawn(move || {
                        rfd::MessageDialog::new().set_title("Lime").set_description(e).set_level(rfd::MessageLevel::Error).show();
                    });
                }
            }
        }

        let button = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON };
        if key_down(button.0) {
            s.released_ticks = 0;
            let p = cursor();
            let start = *s.press.get_or_insert(p);
            let (dx, dy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
            s.dragging |= (p.x - start.x).abs() > dx || (p.y - start.y).abs() > dy;
            if s.dragging && s.busy.is_none() {
                // The wheel stays open after the key is released; the other key switches it.
                if let Some(kind) = Kind::from_keys(key_down(VK_SHIFT.0), key_down(VK_CONTROL.0)) {
                    if !s.visible {
                        show(s, kind, p);
                    } else if s.kind != Some(kind) {
                        switch(s, kind);
                    }
                }
            }
        } else {
            s.press = None;
            s.dragging = false;
            s.released_ticks += 1;
            // Give Drop a moment before closing the wheel on a miss or cancel.
            if s.released_ticks == 10 && s.busy.is_none() {
                hide(s);
            }
        }

        if !s.visible {
            return Duration::from_millis(30);
        }
        let opening = s.opened.is_some_and(|o| o.elapsed().as_secs_f32() < OPEN_SECS);
        let mut close = false;
        if let Some(b) = s.busy.as_mut() {
            // Jobs report no progress here: creep towards 90 % until they finish.
            let creep = 0.9 * (1.0 - (-b.started.elapsed().as_secs_f32() / 4.0).exp());
            b.target = b.target.max(creep);
            b.shown += (b.target - b.shown) * 0.25;
            if b.target - b.shown < 0.003 {
                b.shown = b.target;
            }
            close = b.shown >= 1.0 && b.done.is_some_and(|t| t.elapsed() > Duration::from_millis(250));
        }
        if close {
            hide(s);
        } else if opening || s.busy.is_some() {
            render(s);
        }
        Duration::from_millis(16)
    })
}

fn show(s: &mut State, kind: Kind, at: POINT) {
    let mut dpi = (96, 96);
    let _ = unsafe { GetDpiForMonitor(MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST), MDT_EFFECTIVE_DPI, &mut dpi.0, &mut dpi.1) };
    s.scale = dpi.0 as f32 / 96.0;
    let half = (SIZE * s.scale / 2.0) as i32;
    s.origin = POINT { x: at.x - half, y: at.y - half };
    s.kind = Some(kind);
    s.files = None;
    s.choices.clear();
    s.hovered = None;
    s.opened = None;
    s.dark = dark_mode();
    s.visible = true;
    render(s);
    if let Some(hwnd) = s.hwnd {
        let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    }
}

fn switch(s: &mut State, kind: Kind) {
    s.kind = Some(kind);
    s.choices = s.files.as_deref().map(|f| kind.ops(f)).unwrap_or_default();
    s.hovered = None;
    render(s);
}

fn hide(s: &mut State) {
    s.visible = false;
    s.busy = None;
    s.opened = None;
    if let Some(hwnd) = s.hwnd {
        let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
    }
}

fn dark_mode() -> bool {
    let mut light: u32 = 1;
    let mut size = 4u32;
    unsafe {
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut light as *mut u32 as *mut _),
            Some(&mut size),
        );
    }
    light == 0
}

/// File paths in a shell drag (`CF_HDROP`); empty for anything else.
fn files_in(data: &IDataObject) -> Vec<PathBuf> {
    let format = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    let Ok(mut medium) = (unsafe { data.GetData(&format) }) else { return vec![] };
    let hdrop = HDROP(unsafe { medium.u.hGlobal.0 });
    let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
    let files = (0..count)
        .map(|i| {
            let mut buf = vec![0u16; unsafe { DragQueryFileW(hdrop, i, None) } as usize + 1];
            let len = unsafe { DragQueryFileW(hdrop, i, Some(&mut buf)) } as usize;
            PathBuf::from(OsString::from_wide(&buf[..len]))
        })
        .collect();
    unsafe { ReleaseStgMedium(&mut medium) };
    files
}

/// Segment under `(x, y)` (logical pixels from the wheel's centre, y down), counted clockwise from 12 o'clock.
fn segment_at(x: f32, y: f32, n: usize) -> Option<usize> {
    if n == 0 || !(INNER..=DISC).contains(&x.hypot(y)) {
        return None;
    }
    let clockwise_from_top = x.atan2(-y).to_degrees().rem_euclid(360.0);
    Some(((clockwise_from_top / (360.0 / n as f32)) as usize).min(n - 1))
}

/// The segment under the screen point `pt`, or `None` when the wheel is not drawn or busy.
fn hit(s: &State, pt: &POINTL) -> Option<usize> {
    if s.opened.is_none() || s.busy.is_some() {
        return None;
    }
    let c = SIZE * s.scale / 2.0;
    let (x, y) = ((pt.x - s.origin.x) as f32 - c, (pt.y - s.origin.y) as f32 - c);
    segment_at(x / s.scale, y / s.scale, s.choices.len())
}

fn effect(hit: Option<usize>) -> DROPEFFECT {
    if hit.is_some() {
        DROPEFFECT_COPY
    } else {
        DROPEFFECT_NONE
    }
}

#[windows::core::implement(IDropTarget)]
struct DropTarget;

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect_out: *mut DROPEFFECT) -> Result<()> {
        let files = data.as_ref().map(files_in).unwrap_or_default();
        STATE.with_borrow_mut(|s| {
            if s.busy.is_none() && !files.is_empty() {
                let kind = Kind::from_keys(keys.contains(MK_SHIFT), keys.contains(MK_CONTROL)).or(s.kind).unwrap_or(Kind::Convert);
                s.kind = Some(kind);
                s.choices = kind.ops(&files);
                s.files = Some(files);
                s.opened.get_or_insert_with(Instant::now);
            }
            s.hovered = hit(s, pt);
            unsafe { *effect_out = effect(s.hovered) };
            render(s);
        });
        Ok(())
    }

    fn DragOver(&self, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect_out: *mut DROPEFFECT) -> Result<()> {
        STATE.with_borrow_mut(|s| {
            if let Some(kind) = Kind::from_keys(keys.contains(MK_SHIFT), keys.contains(MK_CONTROL)).filter(|k| s.busy.is_none() && Some(*k) != s.kind) {
                switch(s, kind);
            }
            let h = hit(s, pt);
            unsafe { *effect_out = effect(h) };
            if h != s.hovered {
                s.hovered = h;
                render(s);
            }
        });
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        STATE.with_borrow_mut(|s| {
            if s.hovered.take().is_some() {
                render(s);
            }
        });
        Ok(())
    }

    fn Drop(&self, _data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect_out: *mut DROPEFFECT) -> Result<()> {
        STATE.with_borrow_mut(|s| {
            let picked = hit(s, pt).and_then(|i| Some((i, s.choices.get(i)?.clone(), s.files.clone()?)));
            unsafe { *effect_out = effect(picked.as_ref().map(|p| p.0)) };
            let Some((index, op, files)) = picked else {
                hide(s);
                return;
            };
            s.hovered = None;
            s.busy = Some(Busy { index, started: Instant::now(), shown: 0.0, target: 0.0, done: None });
            // Run off the UI thread; Explorer waits for Drop to return.
            if let Some((tx, _)) = &s.jobs {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(run_op(&op, &files).map(|_| ()));
                });
            }
            render(s);
        });
        Ok(())
    }
}

// Drawing.

struct Palette {
    disc: Color,
    pulp: Color,
    pulp_hover: Color,
    hub: Color,
    text: Color,
    dim: Color,
}

fn grey(w: f32, a: f32) -> Color {
    Color::from_rgba(w, w, w, a).unwrap_or(Color::BLACK)
}

impl Palette {
    /// Lime.app's solid (no Liquid Glass) palette, with its matte lime for the hovered button and the progress fill.
    fn new(dark: bool) -> Self {
        if dark {
            Palette {
                disc: grey(0.18, 0.97),
                pulp: grey(0.33, 1.0),
                pulp_hover: Color::from_rgba(0.45, 0.62, 0.18, 1.0).unwrap(),
                hub: grey(0.18, 1.0),
                text: grey(1.0, 0.95),
                dim: grey(1.0, 0.6),
            }
        } else {
            Palette {
                disc: grey(0.95, 0.97),
                pulp: grey(0.80, 1.0),
                pulp_hover: Color::from_rgba(0.69, 0.83, 0.33, 1.0).unwrap(),
                hub: grey(0.95, 1.0),
                text: grey(0.0, 0.85),
                dim: grey(0.0, 0.5),
            }
        }
    }
}

/// `c` mixed towards white (`by` > 0) or black (`by` < 0).
fn tint(c: Color, by: f32) -> Color {
    let to = if by > 0.0 { 1.0 } else { 0.0 };
    let m = |v: f32| v + (to - v) * by.abs();
    Color::from_rgba(m(c.red()), m(c.green()), m(c.blue()), c.alpha()).unwrap_or(c)
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

/// The point `r` from the centre `c`, `deg` clockwise from 12 o'clock (y down).
fn polar(c: f32, r: f32, deg: f32) -> Point {
    let a = deg.to_radians();
    Point::from_xy(c + r * a.sin(), c - r * a.cos())
}

/// Appends a circular arc from the current point (at angle `a0`) to angle `a1`, as cubic Béziers.
fn arc(pb: &mut PathBuilder, c: f32, r: f32, a0: f32, a1: f32) {
    let pieces = ((a1 - a0).abs() / 45.0).ceil().max(1.0) as usize;
    let step = (a1 - a0) / pieces as f32;
    for i in 0..pieces {
        let (t0, t1) = (a0 + step * i as f32, a0 + step * (i + 1) as f32);
        let k = 4.0 / 3.0 * (step.to_radians() / 4.0).tan() * r;
        let (p0, p1) = (polar(c, r, t0), polar(c, r, t1));
        let (d0, d1) = (t0.to_radians(), t1.to_radians());
        pb.cubic_to(p0.x + k * d0.cos(), p0.y + k * d0.sin(), p1.x - k * d1.cos(), p1.y - k * d1.sin(), p1.x, p1.y);
    }
}

/// A button: the path, and the corner radius to stroke it with (0 for the full ring of a one-button wheel).
struct Shape {
    path: Path,
    round: f32,
}

/// One segment from `a0` clockwise to `a1` between radii `ri` and `ro`, its sides kept `MEMBRANE` from the dividers.
/// The path is the segment shrunk by its corner radius; stroking it that wide with round joins gives the rounded corners.
fn segment(c: f32, ri: f32, ro: f32, a0: f32, a1: f32) -> Option<Shape> {
    let half = (a1 - a0) / 2.0;
    let round = ROUND.min(((half.to_radians().sin() * ri) - MEMBRANE) * 0.8).min((ro - ri) / 2.0 - 0.5);
    if round < 1.0 {
        return None;
    }
    let d = MEMBRANE + round;
    let (ro, ri) = (ro - round, ri + round);
    let (off_o, off_i) = ((d / ro).asin().to_degrees(), (d / ri).asin().to_degrees());
    if !(off_i < half) {
        return None;
    }
    let mut pb = PathBuilder::new();
    let start = polar(c, ro, a0 + off_o);
    pb.move_to(start.x, start.y);
    arc(&mut pb, c, ro, a0 + off_o, a1 - off_o);
    let p = polar(c, ri, a1 - off_i);
    pb.line_to(p.x, p.y);
    arc(&mut pb, c, ri, a1 - off_i, a0 + off_i);
    pb.close();
    Some(Shape { path: pb.finish()?, round })
}

fn ring(c: f32, ri: f32, ro: f32) -> Option<Shape> {
    let mut pb = PathBuilder::new();
    pb.push_circle(c, c, ro);
    pb.push_circle(c, c, ri);
    Some(Shape { path: pb.finish()?, round: 0.0 })
}

fn circle(c: f32, r: f32) -> Option<Path> {
    PathBuilder::from_circle(c, c, r)
}

fn fill(pix: &mut Pixmap, shape: &Shape, paint: &Paint, ts: Transform) {
    if shape.round > 0.0 {
        pix.fill_path(&shape.path, paint, FillRule::Winding, ts, None);
        let stroke = Stroke { width: shape.round * 2.0, line_join: LineJoin::Round, ..Default::default() };
        pix.stroke_path(&shape.path, paint, &stroke, ts, None);
    } else {
        pix.fill_path(&shape.path, paint, FillRule::EvenOdd, ts, None);
    }
}

/// Matte top-lit face: a gentle vertical gradient.
fn face(pix: &mut Pixmap, shape: &Shape, color: Color, ts: Transform) {
    let b = shape.path.bounds();
    let (top, bottom) = (b.top() - shape.round, b.bottom() + shape.round);
    let stops = vec![GradientStop::new(0.0, tint(color, 0.18)), GradientStop::new(1.0, tint(color, -0.08))];
    let mut p = paint(color);
    if let Some(shader) = LinearGradient::new(Point::from_xy(0.0, top), Point::from_xy(0.0, bottom), stops, SpreadMode::Pad, Transform::identity()) {
        p.shader = shader;
    }
    fill(pix, shape, &p, ts);
}

struct Fonts {
    bold: ab_glyph::FontVec,
    regular: ab_glyph::FontVec,
}

impl Fonts {
    /// Segoe UI from the Windows fonts folder; without it the wheel draws no text.
    fn load() -> Option<Fonts> {
        let dir = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts");
        let load = |names: &[&str]| names.iter().find_map(|n| ab_glyph::FontVec::try_from_vec(std::fs::read(dir.join(n)).ok()?).ok());
        Some(Fonts { bold: load(&["seguisb.ttf", "segoeuib.ttf", "arialbd.ttf"])?, regular: load(&["segoeui.ttf", "arial.ttf"])? })
    }
}

/// Draws `text` centred on `(cx, cy)` (physical pixels), `size` pixels high.
fn text(pix: &mut Pixmap, font: &ab_glyph::FontVec, s: &str, cx: f32, cy: f32, size: f32, color: Color) {
    use ab_glyph::{Font, ScaleFont};
    let scaled = font.as_scaled(size);
    let ids: Vec<_> = s.chars().map(|ch| scaled.glyph_id(ch)).collect();
    let width: f32 = ids.iter().enumerate().map(|(i, &g)| scaled.h_advance(g) + if i > 0 { scaled.kern(ids[i - 1], g) } else { 0.0 }).sum();
    let baseline = cy + (scaled.ascent() + scaled.descent()) / 2.0;
    let (w, h) = (pix.width() as i32, pix.height() as i32);
    let data = pix.data_mut();
    let mut x = cx - width / 2.0;
    for (i, &id) in ids.iter().enumerate() {
        if i > 0 {
            x += scaled.kern(ids[i - 1], id);
        }
        if let Some(outline) = font.outline_glyph(id.with_scale_and_position(size, ab_glyph::point(x, baseline))) {
            let b = outline.px_bounds();
            outline.draw(|gx, gy, cov| {
                let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if px < 0 || py < 0 || px >= w || py >= h {
                    return;
                }
                // Source-over onto premultiplied RGBA.
                let a = color.alpha() * cov.min(1.0);
                let i = ((py * w + px) * 4) as usize;
                for (k, v) in [color.red(), color.green(), color.blue(), 1.0].into_iter().enumerate() {
                    data[i + k] = (v * a * 255.0 + data[i + k] as f32 * (1.0 - a)).round() as u8;
                }
            });
        }
        x += scaled.h_advance(id);
    }
}

fn draw(s: &State) -> Option<Pixmap> {
    let k = s.scale;
    let px = (SIZE * k).ceil() as u32;
    let mut pix = Pixmap::new(px, px)?;
    let c = SIZE / 2.0;
    // The drop target: nearly invisible, but Windows only hit-tests layered pixels with some alpha.
    pix.fill_path(&circle(c, DISC)?, &paint(Color::from_rgba8(0, 0, 0, 2)), FillRule::Winding, Transform::from_scale(k, k), None);
    let Some(opened) = s.opened else { return Some(pix) };

    let t = (opened.elapsed().as_secs_f32() / OPEN_SECS).min(1.0);
    let a = (1.0 - (1.0 - t).powi(3)).max(0.01); // ease out
    // Logical point p maps to k * (c + a * (p - c)).
    let ts = Transform::from_row(k * a, 0.0, 0.0, k * a, k * c * (1.0 - a), k * c * (1.0 - a));
    let at = |p: Point| (k * (c + a * (p.x - c)), k * (c + a * (p.y - c)));
    let p = Palette::new(s.dark);
    let n = s.choices.len();
    let (pulp_out, pulp_in) = (OUTER - RIND - PITH, INNER + 1.0);

    let disc = circle(c, DISC)?;
    pix.fill_path(&disc, &paint(p.disc), FillRule::Winding, ts, None);
    let edge = circle(c, DISC - 0.5)?;
    pix.stroke_path(&edge, &paint(grey(1.0, if s.dark { 0.14 } else { 0.6 })), &Stroke { width: 1.0, ..Default::default() }, ts, None);

    let step = 360.0 / n.max(1) as f32;
    let span = |i: usize| (i as f32 * step, (i + 1) as f32 * step);
    let shapes: Vec<Option<Shape>> = (0..n)
        .map(|i| if n > 1 { segment(c, pulp_in, pulp_out, span(i).0, span(i).1) } else { ring(c, pulp_in, pulp_out) })
        .collect();
    let color = |i: usize| if s.hovered == Some(i) { p.pulp_hover } else { p.pulp };
    // Shadows first, so no button's shadow falls across a neighbour; a hovered button floats higher.
    for (i, shape) in shapes.iter().enumerate() {
        if let Some(shape) = shape {
            let lift = if s.hovered == Some(i) { 3.5 } else { 1.2 };
            fill(&mut pix, shape, &paint(grey(0.0, if s.dark { 0.35 } else { 0.18 })), ts.pre_translate(0.0, lift));
        }
    }
    for (i, op) in s.choices.iter().enumerate() {
        if let Some(shape) = &shapes[i] {
            face(&mut pix, shape, color(i), ts);
            if let Some(busy) = s.busy.as_ref().filter(|b| b.index == i && b.shown > 0.0) {
                let r = pulp_in + (pulp_out - pulp_in) * busy.shown;
                let filled = if n > 1 { segment(c, pulp_in, r, span(i).0, span(i).1) } else { ring(c, pulp_in, r) };
                if let Some(filled) = filled {
                    face(&mut pix, &filled, p.pulp_hover, ts);
                }
            }
        }
        if let Some(fonts) = &s.fonts {
            let (x, y) = at(polar(c, (INNER + pulp_out) / 2.0, (span(i).0 + span(i).1) / 2.0));
            text(&mut pix, &fonts.bold, &op.label, x, y, if n > 9 { 11.0 } else { 13.0 } * 1.33 * k * a, p.text);
        }
    }

    pix.fill_path(&circle(c, INNER - 4.0)?, &paint(p.hub), FillRule::Winding, ts, None);
    pix.stroke_path(&circle(c, INNER - 4.0)?, &paint(grey(0.0, if s.dark { 0.4 } else { 0.15 })), &Stroke { width: 1.0, ..Default::default() }, ts, None);
    if let Some(fonts) = &s.fonts {
        let (x, y) = at(Point::from_xy(c, c));
        text(&mut pix, &fonts.regular, s.kind.map(Kind::title).unwrap_or(""), x, y, 12.0 * 1.33 * k * a, p.dim);
        if n == 0 {
            let (x, y) = at(Point::from_xy(c, c - INNER - 40.0));
            text(&mut pix, &fonts.bold, "Nothing to do", x, y, 13.0 * 1.33 * k * a, p.text);
        }
    }
    Some(pix)
}

/// Draws the wheel and puts it on screen (premultiplied BGRA, as layered windows want it).
fn render(s: &State) {
    let (Some(hwnd), Some(pix)) = (s.hwnd, draw(s)) else { return };
    let (w, h) = (pix.width() as i32, pix.height() as i32);
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        if let Ok(bmp) = CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
            for (d, s) in dst.chunks_exact_mut(4).zip(pix.data().chunks_exact(4)) {
                d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
            }
            let old = SelectObject(mem, bmp.into());
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(
                hwnd,
                Some(screen),
                Some(&s.origin),
                Some(&WSIZE { cx: w, cy: h }),
                Some(mem),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            SelectObject(mem, old);
            let _ = DeleteObject(bmp.into());
        }
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_clockwise_from_top() {
        let r = (INNER + OUTER) / 2.0;
        assert_eq!(segment_at(0.0, -r, 4), Some(0)); // top
        assert_eq!(segment_at(r, 1.0, 4), Some(1)); // right, just below horizontal
        assert_eq!(segment_at(-1.0, r, 4), Some(2)); // bottom
        assert_eq!(segment_at(-r, -1.0, 4), Some(3)); // left, just above horizontal
        assert_eq!(segment_at(0.0, 0.0, 4), None); // centre cancels
        assert_eq!(segment_at(r, 0.0, 0), None);
        assert_eq!(segment_at(DISC + 1.0, 0.0, 4), None); // just outside the disc
    }

    #[test]
    fn segments_stay_inside_their_ring() {
        let c = SIZE / 2.0;
        let (ri, ro) = (INNER + 1.0, OUTER - RIND - PITH);
        for n in [2, 4, 8, 16] {
            let shape = segment(c, ri, ro, 0.0, 360.0 / n as f32).expect("segment fits");
            let b = shape.path.bounds();
            // The stroke adds `round` on every side.
            assert!(b.left() - shape.round >= c - 0.5, "n={n}: starts right of 12 o'clock");
            assert!(b.top() - shape.round >= c - ro - 0.5, "n={n}");
            assert!(b.right() + shape.round <= c + ro + 0.5, "n={n}");
        }
    }

    #[test]
    fn busy_fill_shrinks_its_corners() {
        let c = SIZE / 2.0;
        assert!(segment(c, INNER + 1.0, INNER + 3.0, 0.0, 90.0).is_none());
        assert!(segment(c, INNER + 1.0, INNER + 20.0, 0.0, 90.0).is_some());
    }
}
