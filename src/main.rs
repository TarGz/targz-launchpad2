//! Targz Launchpad — menu-bar launcher for local dev projects.
//! `--screenshot <file.png>`: draw ~120 frames, save the window, exit.

mod core;
#[cfg(target_os = "macos")]
mod tray;
mod theme;
mod ui;
mod update;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let shot = args.iter().position(|a| a == "--screenshot").and_then(|i| args.get(i + 1)).map(PathBuf::from);

    let shared: core::Shared = Arc::new(Mutex::new(core::Core::new()));

    // Terminal-like PATH so npm/npx/jekyll are found when opened from Finder.
    {
        let s = shared.clone();
        std::thread::spawn(move || {
            let path = core::login_path();
            s.lock().unwrap().path_env = path;
        });
    }

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Targz Launchpad")
        .with_inner_size([1200.0, 800.0])
        .with_min_inner_size([420.0, 420.0]);
    if shot.is_some() {
        viewport = viewport.with_active(false);
        #[cfg(target_os = "windows")]
        {
            viewport = viewport.with_window_level(eframe::egui::WindowLevel::AlwaysOnBottom);
        }
    }
    let mut options = eframe::NativeOptions { viewport, run_and_return: false, ..Default::default() };
    #[cfg(target_os = "macos")]
    if shot.is_some() {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            b.with_activate_ignoring_other_apps(false);
        }));
    }

    eframe::run_native(
        "Targz Launchpad",
        options,
        Box::new(move |cc| {
            if shot.is_none() {
                #[cfg(target_os = "macos")]
                tray::setup(shared.clone(), cc.egui_ctx.clone());
                #[cfg(not(target_os = "macos"))]
                {
                    let s = shared.clone();
                    std::thread::spawn(move || loop {
                        s.lock().unwrap().check_alive();
                        std::thread::sleep(Duration::from_secs(5));
                    });
                }
                // Look for updates shortly after start, then every 30 minutes.
                let s = shared.clone();
                let ctx = cc.egui_ctx.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(8));
                    loop {
                        update::check(&s);
                        ctx.request_repaint();
                        std::thread::sleep(update::CHECK_EVERY);
                    }
                });
            }
            Ok(Box::new(ui::App::new(cc, shared, shot)))
        }),
    )
}
