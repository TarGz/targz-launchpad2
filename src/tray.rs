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

/// A 36×36 template glyph: a launch arrow over a pad line.
fn icon() -> Icon {
    let n = 36usize;
    let mut rgba = vec![0u8; n * n * 4];
    let mut put = |x: usize, y: usize| {
        let i = (y * n + x) * 4;
        rgba[i + 3] = 255;
    };
    for y in 4..24 {
        // triangle widening downwards, apex at y=4
        let half = (y - 4) as f32 * 0.55 + 1.0;
        let x0 = (18.0 - half).max(0.0) as usize;
        let x1 = (18.0 + half).min(35.0) as usize;
        for x in x0..x1 {
            put(x, y);
        }
    }
    for y in 24..27 {
        for x in 15..21 {
            put(x, y);
        }
    }
    for y in 29..32 {
        for x in 5..31 {
            put(x, y);
        }
    }
    Icon::from_rgba(rgba, n as u32, n as u32).expect("tray icon")
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
