//! Lime tray app for Windows and Linux: pick files from the tray menu, then run a conversion or tool on them.
//! Results are saved next to the originals, as in Lime.app.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(target_os = "macos")]
fn main() {
    eprintln!("lime-tray is for Windows and Linux; on macOS use Lime.app");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    tray::run();
}

#[cfg(not(target_os = "macos"))]
mod tray {
    use lime_core::{Op, Wheel};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tao::event::{Event, StartCause};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    enum UserEvent {
        Menu(MenuEvent),
        Picked(Vec<PathBuf>),
    }

    const CHOOSE: &str = "choose";
    const QUIT: &str = "quit";

    /// The tray menu for `files`, and the op behind each op item's id.
    fn menu(files: &[PathBuf]) -> (Menu, HashMap<MenuId, Op>) {
        let menu = Menu::new();
        let mut ops = HashMap::new();
        let _ = menu.append(&MenuItem::with_id(CHOOSE, "Choose Files…", true, None));
        if !files.is_empty() {
            let names: Vec<String> = files.iter().filter_map(|f| f.file_name()).map(|n| n.to_string_lossy().into()).collect();
            let _ = menu.append(&MenuItem::new(names.join(", "), false, None));
            for (wheel, title) in [(Wheel::Convert, "Convert"), (Wheel::Tools, "Tools")] {
                let wheel_ops = lime_core::ops(wheel, files);
                let sub = Submenu::new(title, !wheel_ops.is_empty());
                for op in wheel_ops {
                    let item = MenuItem::new(&op.label, true, None);
                    ops.insert(item.id().clone(), op);
                    let _ = sub.append(&item);
                }
                let _ = menu.append(&sub);
            }
        }
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&MenuItem::with_id(QUIT, "Quit Lime", true, None));
        (menu, ops)
    }

    fn icon() -> Icon {
        let img = image::load_from_memory(include_bytes!("../../../assets/icon-1024.png")).expect("bundled icon").thumbnail(32, 32).into_rgba8();
        let (w, h) = img.dimensions();
        Icon::from_rgba(img.into_raw(), w, h).expect("valid icon")
    }

    fn job(op: Op, files: Vec<PathBuf>) {
        std::thread::spawn(move || {
            // shortcut: tools that need a value run with its default; add an input dialog if users need other values.
            let arg = op.param.as_ref().map(|p| p.default);
            let (level, text) = match lime_core::run(&op.id, &files, arg) {
                Ok(out) => (rfd::MessageLevel::Info, out.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n")),
                Err(e) => (rfd::MessageLevel::Error, format!("{e:#}")),
            };
            rfd::MessageDialog::new().set_title(format!("Lime: {}", op.label)).set_description(text).set_level(level).show();
        });
    }

    pub fn run() {
        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        let proxy = event_loop.create_proxy();
        let menu_proxy = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = menu_proxy.send_event(UserEvent::Menu(e));
        }));

        let mut tray: Option<TrayIcon> = None;
        let mut files: Vec<PathBuf> = vec![];
        let mut ops: HashMap<MenuId, Op> = HashMap::new();

        event_loop.run(move |event, _, control_flow| {
            *control_flow = ControlFlow::Wait;
            match event {
                // The tray has to be created once the event loop runs (GTK on Linux).
                Event::NewEvents(StartCause::Init) => {
                    let (m, _) = menu(&files);
                    tray = Some(TrayIconBuilder::new().with_menu(Box::new(m)).with_icon(icon()).with_tooltip("Lime").build().expect("tray icon"));
                }
                Event::UserEvent(UserEvent::Menu(e)) => match e.id.as_ref() {
                    CHOOSE => {
                        let proxy = proxy.clone();
                        std::thread::spawn(move || {
                            if let Some(picked) = rfd::FileDialog::new().set_title("Lime: Choose Files").pick_files() {
                                let _ = proxy.send_event(UserEvent::Picked(picked));
                            }
                        });
                    }
                    QUIT => {
                        tray.take();
                        *control_flow = ControlFlow::Exit;
                    }
                    _ => {
                        if let Some(op) = ops.get(&e.id) {
                            job(op.clone(), files.clone());
                        }
                    }
                },
                Event::UserEvent(UserEvent::Picked(picked)) => {
                    files = picked;
                    let (m, new_ops) = menu(&files);
                    ops = new_ops;
                    if let Some(t) = &tray {
                        t.set_menu(Some(Box::new(m)));
                    }
                }
                _ => {}
            }
        });
    }
}
