//! Menu-bar icon (macOS): running count as the title, a menu with each
//! running project, Open Dashboard and Quit. All AppKit work stays on the
//! main thread; the 5 s poll hops there through the main dispatch queue.

use crate::core::{open_url, Shared};
use std::cell::RefCell;
use std::time::Duration;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// Tray poll period: `tokio::time::sleep(5 s)` at 0x1000c45cc in setup_tray's loop.
pub const TRAY_POLL: Duration = Duration::from_secs(5);

thread_local! {
    static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
    static LAST_SIG: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The menu-bar glyph: `tray.png`, loaded.
///
/// IT IS NOT THE APP ICON SCALED DOWN, AND IT IS NOT A DIFFERENT DRAWING
/// EITHER. The icon's three sheets have gaps that land under a pixel at this
/// size — reduced straight to 36 they merge into one blob and the thin
/// trailing edges break off it entirely. `tray.svg` is the same idea drawn for
/// a size where only three or four strokes fit: three sheets leaning the same
/// way, fat enough to survive and far enough apart that the gaps are gaps.
///
/// It was a launch arrow over a pad line, which was a second mark for the same
/// app — the one the user sees in the Dock and the one they see in the bar
/// said different things about it.
///
/// BLACK WITH ALPHA AND NOTHING ELSE. macOS treats a menu-bar image as a
/// template and inverts it for the dark bar, so any colour in the file is
/// thrown away; what carries the shape is the alpha channel.
fn icon() -> Icon {
    const BYTES: &[u8] = include_bytes!("../tray.png");
    let img = image::load_from_memory_with_format(BYTES, image::ImageFormat::Png)
        .expect("tray.png")
        .to_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).expect("tray icon")
}

/// The update line shown at the top of the menu: (label, clickable).
type UpdateLine = Option<(String, bool)>;

fn update_line(u: &crate::core::UpdateState) -> UpdateLine {
    if u.busy {
        Some((u.message.clone(), false))
    } else if u.behind > 0 {
        Some((format!("Update available: {} — Install", u.message), true))
    } else {
        None
    }
}

fn menu(running: &[(String, u16)], update: &UpdateLine) -> Menu {
    let menu = Menu::new();
    if let Some((label, enabled)) = update {
        let _ = menu.append(&MenuItem::with_id("install_update", label, *enabled, None));
        let _ = menu.append(&PredefinedMenuItem::separator());
    }
    if !running.is_empty() {
        for (name, port) in running {
            let _ = menu.append(&MenuItem::with_id(format!("project_{port}"), format!("{name} :{port}"), true, None));
        }
        let _ = menu.append(&PredefinedMenuItem::separator());
    }
    let _ = menu.append(&MenuItem::with_id("open_dashboard", "Open Dashboard", true, None));
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id("quit", "Quit", true, None));
    menu
}

/// Must run on the main thread, after the event loop started.
pub fn setup(shared: Shared, ctx: eframe::egui::Context) {
    let tray = TrayIconBuilder::new()
        .with_icon(icon())
        .with_icon_as_template(true)
        .with_menu(Box::new(menu(&[], &None)))
        .with_menu_on_left_click(true)
        .with_tooltip("Targz Launchpad")
        .with_title("0")
        .build();
    let Ok(tray) = tray else { return };
    TRAY.with(|t| *t.borrow_mut() = Some(tray));

    let s = shared.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let id = event.id.0.as_str();
        if id == "open_dashboard" {
            show_dashboard(&ctx);
        } else if id == "quit" {
            s.lock().unwrap().stop_all();
            std::process::exit(0);
        } else if id == "install_update" {
            let s = s.clone();
            let busy = s.lock().unwrap().update.busy;
            if !busy {
                std::thread::spawn(move || crate::update::apply(&s));
            }
        } else if let Some(port) = id.strip_prefix("project_").and_then(|p| p.parse().ok()) {
            open_url(port);
        }
    }));

    std::thread::spawn(move || loop {
        let (running, update) = {
            let mut c = shared.lock().unwrap();
            c.check_alive();
            (c.running_list(), update_line(&c.update))
        };
        dispatch::Queue::main().exec_async(move || refresh(&running, &update));
        std::thread::sleep(TRAY_POLL);
    });
}

fn refresh(running: &[(String, u16)], update: &UpdateLine) {
    let mut sig = running.iter().map(|(n, p)| format!("{n}:{p}")).collect::<Vec<_>>().join(";");
    if let Some((label, _)) = update {
        sig.push_str("|");
        sig.push_str(label);
    }
    let changed = LAST_SIG.with(|l| {
        let mut l = l.borrow_mut();
        let changed = l.as_deref() != Some(sig.as_str());
        *l = Some(sig);
        changed
    });
    if !changed {
        return;
    }
    TRAY.with(|t| {
        if let Some(tray) = t.borrow().as_ref() {
            let count = running.len();
            tray.set_title(Some(count.to_string()));
            let tip = if count > 0 { format!("Targz Launchpad - {count} running") } else { "Targz Launchpad - idle".into() };
            let _ = tray.set_tooltip(Some(tip));
            tray.set_menu(Some(Box::new(menu(running, update))));
        }
    });
}

/// Hide from the Dock and Cmd+Tab while only the menu-bar icon is shown.
pub fn set_accessory(accessory: bool) {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        let policy = if accessory { NSApplicationActivationPolicy::Accessory } else { NSApplicationActivationPolicy::Regular };
        app.setActivationPolicy(policy);
    }
}

/// Bring the dashboard window back, straight through AppKit: a hidden
/// window does not redraw, so a viewport command alone may never land.
#[allow(deprecated)]
pub fn show_dashboard(ctx: &eframe::egui::Context) {
    use objc2_app_kit::NSApplication;
    set_accessory(false);
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        // Only the dashboard. app.windows() also holds the status-item window
        // and the tray menu's window; ordering those front leaves a transparent
        // window on screen that eats clicks meant for whatever is underneath.
        for w in app.windows().iter() {
            if w.canBecomeMainWindow() {
                w.makeKeyAndOrderFront(None);
            }
        }
        app.activateIgnoringOtherApps(true);
    }
    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Focus);
    ctx.request_repaint();
}
