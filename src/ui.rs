//! The dashboard window: header, folder chips + filter, the card rack.

use crate::core::{self, Busy, Kind, Shared, UpdateState};
use crate::theme::*;
use eframe::egui::{
    self, text::LayoutJob, text::TextWrapping, Align2, Color32, Context, DragAndDrop, Event, Id, Key, LayerId, Order, Pos2, Rect, Response,
    RichText, Sense, Stroke, TextFormat, Ui, UiBuilder, Vec2, ViewportCommand,
};
use std::path::PathBuf;
use std::time::Duration;

const CARD_MIN_W: f32 = 340.0;
const GAP: f32 = 16.0;
const CARD_H: f32 = 228.0;
const MAX_W: f32 = 1200.0;
const PAD_X: f32 = 40.0;
const PAD_Y: f32 = 32.0;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Card {
    name: String,
    group: Option<String>,
    short: String,
    desc: String,
    kind: Kind,
    port: Option<u16>,
    running: bool,
    busy: Option<Busy>,
    folders: Vec<String>,
    path: PathBuf,
}

enum Action {
    Launch(String),
    Stop(String),
    Restart(String),
    OpenUrl(u16),
    Editor(PathBuf),
    Finder(PathBuf),
    StopAll,
    Refresh,
    Update,
    CheckUpdate,
    Assign(String, String, bool),
    NewFolder,
    EditFolder(String),
}

enum ModalMode {
    New,
    Edit(String),
}

struct Modal {
    mode: ModalMode,
    text: String,
    error: String,
    focused: bool,
}

pub struct App {
    shared: Shared,
    search: String,
    folder: Option<String>,
    modal: Option<Modal>,
    toast: Option<(String, f64)>,
    shot: Option<PathBuf>,
    frames: u64,
    focus_search: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, shared: Shared, shot: Option<PathBuf>) -> App {
        install_fonts(&cc.egui_ctx);
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BOARD;
        visuals.extreme_bg_color = BAY;
        visuals.selection.bg_fill = rgba(255, 106, 0, 0.35);
        visuals.text_cursor.stroke = Stroke::new(1.5, LED);
        cc.egui_ctx.set_visuals(visuals);
        App { shared, search: String::new(), folder: None, modal: None, toast: None, shot, frames: 0, focus_search: false }
    }

    fn snapshot(&mut self, now: f64) -> (Vec<Card>, Vec<(String, usize)>, usize, String, UpdateState) {
        let mut c = self.shared.lock().unwrap();
        if let Some(t) = c.toasts.pop() {
            self.toast = Some((t, now));
            c.toasts.clear();
        }
        let cards: Vec<Card> = c
            .projects
            .iter()
            .map(|p| Card {
                name: p.name.clone(),
                group: p.group.clone(),
                short: p.name.rsplit('/').next().unwrap_or(&p.name).to_string(),
                desc: p.description.clone(),
                kind: p.kind,
                port: c.port_of(p),
                running: c.running.contains_key(&p.name),
                busy: c.busy.get(&p.name).copied(),
                folders: c.cfg.folders_for(&p.name),
                path: p.path.clone(),
            })
            .collect();
        let folders = c.cfg.folders.iter().map(|f| (f.clone(), cards.iter().filter(|p| p.folders.contains(f)).count())).collect();
        if let Some(f) = &self.folder {
            if !c.cfg.folders.contains(f) {
                self.folder = None;
            }
        }
        (cards, folders, c.running.len(), c.cfg.git_dir_label.clone(), c.update.clone())
    }

    fn matches(&self, p: &Card) -> bool {
        let q = self.search.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        let mut hay = vec![p.name.as_str(), p.desc.as_str(), p.kind.label()];
        hay.extend(p.folders.iter().map(String::as_str));
        hay.iter().any(|s| s.to_lowercase().contains(&q))
    }
}

fn text_w(ui: &Ui, s: &str, font: egui::FontId) -> f32 {
    ui.fonts(|f| f.layout_no_wrap(s.to_string(), font, Color32::WHITE).size().x)
}

/// A clickable key with painted contents.
fn key(ui: &mut Ui, rect: Rect, id: Id, kind: KeyKind, r: f32, enabled: bool, draw: impl FnOnce(&egui::Painter, Rect, Color32)) -> Response {
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    let hover = enabled && resp.hovered();
    let pressed = enabled && resp.is_pointer_button_down_on();
    let look = key_look(kind, hover);
    let p = ui.painter();
    paint_key(p, rect, r, &look, pressed);
    let face = if pressed { rect.translate(Vec2::new(0.0, 1.0)) } else { rect };
    draw(p, face, if enabled { look.text } else { fade(look.text, 0.5) });
    if hover {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Key with an icon then a label, sized to fit. Returns (response, width).
fn label_key(ui: &mut Ui, x_right: Option<f32>, x_left: f32, y: f32, h: f32, id: Id, kind: KeyKind, label: &str, icon: fn(&egui::Painter, Pos2, f32, Color32)) -> (Response, Rect) {
    let font = bold(12.0);
    let w = 14.0 + 14.0 + 5.0 + text_w(ui, label, font.clone()) + 14.0;
    let rect = match x_right {
        Some(xr) => Rect::from_min_size(Pos2::new(xr - w, y), Vec2::new(w, h)),
        None => Rect::from_min_size(Pos2::new(x_left, y), Vec2::new(w, h)),
    };
    let label = label.to_string();
    let resp = key(ui, rect, id, kind, R_SM, true, |p, r, col| {
        icon(p, Pos2::new(r.left() + 14.0 + 7.0, r.center().y), 14.0, col);
        p.text(Pos2::new(r.left() + 14.0 + 14.0 + 5.0, r.center().y), Align2::LEFT_CENTER, label, font, col);
    });
    (resp, rect)
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }

    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        if let Some(path) = self.shot.clone() {
            self.frames += 1;
            ctx.request_repaint();
            if self.frames == 120 {
                ctx.send_viewport_cmd(ViewportCommand::Screenshot);
            }
            let events = ctx.input(|i| i.raw.events.clone());
            for e in events {
                if let Event::Screenshot { image, .. } = e {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Some(img) = image::RgbaImage::from_raw(image.size[0] as u32, image.size[1] as u32, image.as_raw().to_vec()) {
                        let _ = img.save(&path);
                    }
                    std::process::exit(0);
                }
            }
        } else if ctx.input(|i| i.viewport().close_requested()) {
            // Closing the window keeps the menu-bar icon alive.
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            #[cfg(target_os = "macos")]
            crate::tray::set_accessory(true);
        }

        let (cards, folders, running_count, git_label, update) = self.snapshot(now);
        let mut actions: Vec<Action> = vec![];
        let focused = ctx.input(|i| i.focused);

        egui::CentralPanel::default().frame(egui::Frame::none().fill(BOARD)).show(ctx, |ui| {
            let full = ui.max_rect();
            paint_board(ui.painter(), full);
            let col_w = (full.width().min(MAX_W) - 2.0 * PAD_X).max(200.0);
            let x0 = full.center().x - col_w / 2.0;
            let x1 = x0 + col_w;
            let mut y = full.top() + PAD_Y;

            // ── header ──
            {
                let p = ui.painter().clone();
                let title = p.text(Pos2::new(x0, y + 17.0), Align2::LEFT_CENTER, "Targz Launchpad", bold(26.0), TEXT);
                // The version badge doubles as "check for updates now".
                let vtxt = if update.checking { "checking…".to_string() } else { match crate::update::build_number() { Some(b) => format!("v{VERSION} · build {b}"), None => format!("v{VERSION}") } };
                let vw = text_w(ui, &vtxt, ui_font(11.0)) + 16.0;
                let vr = Rect::from_min_size(Pos2::new(title.right() + 12.0, y + 10.0), Vec2::new(vw, 17.0));
                let vresp = ui.interact(vr, Id::new("version"), Sense::click());
                p.rect_filled(vr, 100.0, if vresp.hovered() { hex(0x34373d) } else { hex(0x2e3137) });
                p.text(vr.center(), Align2::CENTER_CENTER, vtxt, ui_font(11.0), if vresp.hovered() { LED } else { TEXT_3 });
                if vresp.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let tip = if update.message.is_empty() { "Check for updates".to_string() } else { format!("{} — click to check again", update.message) };
                if vresp.on_hover_text(tip).clicked() && !update.checking && !update.busy {
                    actions.push(Action::CheckUpdate);
                }

                let mut xr = x1;
                if running_count > 0 {
                    let (r, rect) = label_key(ui, Some(xr), 0.0, y + 2.0, 32.0, Id::new("stopall"), KeyKind::Stop, "Stop All", icon_power);
                    if r.clicked() {
                        actions.push(Action::StopAll);
                    }
                    xr = rect.left() - 10.0;
                }
                let (r, rect) = label_key(ui, Some(xr), 0.0, y + 2.0, 32.0, Id::new("refresh"), KeyKind::Metal, "Refresh", icon_restart);
                if r.on_hover_text("Rescan projects").clicked() {
                    actions.push(Action::Refresh);
                }
                xr = rect.left() - 10.0;
                if update.behind > 0 || update.busy {
                    let label = if update.busy { update.message.clone() } else { "Install update".to_string() };
                    let (r, _) = label_key(ui, Some(xr), 0.0, y + 2.0, 32.0, Id::new("update"), KeyKind::Primary, &label, icon_download);
                    if r.on_hover_text(&update.message).clicked() && !update.busy {
                        actions.push(Action::Update);
                    }
                }
                y += 40.0;
                let sub = ui.painter().text(Pos2::new(x0, y), Align2::LEFT_TOP, &git_label, ui_font(13.0), TEXT_3);
                if !update.message.is_empty() {
                    let resp = ui.interact(sub, Id::new("sub"), Sense::hover());
                    resp.on_hover_text(&update.message);
                }
                y += 13.0 + 24.0 + 4.0;
            }

            // ── toolbar: filter, folder chips, counts ──
            let in_folder: Vec<&Card> = cards.iter().filter(|c| self.folder.as_ref().map_or(true, |f| c.folders.contains(f))).collect();
            let shown: Vec<&Card> = in_folder.iter().copied().filter(|c| self.matches(c)).collect();
            {
                let row = Rect::from_min_max(Pos2::new(x0, y), Pos2::new(x1, y + 34.0));
                self.toolbar(ui, row, &folders, cards.len(), running_count, shown.len(), in_folder.len(), &mut actions);
                y += 34.0 + 12.0;
                ui.painter().line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], Stroke::new(1.0, hex(0x232529)));
                y += 20.0;
            }

            // ── the rack ──
            let grid_rect = Rect::from_min_max(Pos2::new(full.left(), y), full.max);
            ui.allocate_new_ui(UiBuilder::new().max_rect(grid_rect), |ui| {
                egui::ScrollArea::vertical().id_salt("rack").auto_shrink([false, false]).show(ui, |ui| {
                    let top = ui.cursor().top();
                    if shown.is_empty() {
                        let msg = if !self.search.trim().is_empty() {
                            format!("No projects match \"{}\"{}.", self.search.trim(), self.folder.as_ref().map(|f| format!(" in {f}")).unwrap_or_default())
                        } else if let Some(f) = &self.folder {
                            format!("\"{f}\" is empty. Drag a project card onto the folder chip to add it.")
                        } else {
                            "No projects found.".to_string()
                        };
                        let r = Rect::from_min_size(Pos2::new(x0, top), Vec2::new(col_w, 120.0));
                        ui.allocate_rect(Rect::from_min_size(Pos2::new(full.left(), top), Vec2::new(full.width(), 120.0 + PAD_Y)), Sense::hover());
                        well(ui.painter(), r, R_CARD, BAY);
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, msg, ui_font(14.0), hex(0x4a4e56));
                        return;
                    }
                    let cols = (((col_w + GAP) / (CARD_MIN_W + GAP)).floor() as usize).max(1);
                    let cw = (col_w - GAP * (cols as f32 - 1.0)) / cols as f32;
                    let rows = shown.len().div_ceil(cols);
                    let total_h = rows as f32 * CARD_H + (rows as f32 - 1.0) * GAP + PAD_Y;
                    ui.allocate_rect(Rect::from_min_size(Pos2::new(full.left(), top), Vec2::new(full.width(), total_h)), Sense::hover());
                    for (i, c) in shown.iter().enumerate() {
                        let (r, k) = (i / cols, i % cols);
                        let rect = Rect::from_min_size(Pos2::new(x0 + k as f32 * (cw + GAP), top + r as f32 * (CARD_H + GAP)), Vec2::new(cw, CARD_H));
                        if ui.is_rect_visible(rect.expand(12.0)) {
                            card(ui, rect, c, now, focused, &mut actions);
                        }
                    }
                });
            });
        });

        self.modal_ui(ctx, &mut actions);
        self.toast_ui(ctx, now);
        self.drag_ghost(ctx);

        // keyboard: "/" focuses the filter, Esc clears it
        if self.modal.is_none() {
            let slash = ctx.input(|i| i.events.iter().any(|e| matches!(e, Event::Text(t) if t == "/")));
            if slash && !ctx.memory(|m| m.has_focus(Id::new("search"))) {
                self.focus_search = true;
            }
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.search.clear();
                ctx.memory_mut(|m| m.surrender_focus(Id::new("search")));
            }
        }

        let animate = cards.iter().any(|c| c.running || c.busy.is_some()) && focused;
        ctx.request_repaint_after(Duration::from_millis(if animate { 33 } else { 500 }));
        self.run(ctx, actions);
    }
}

impl App {
    #[allow(clippy::too_many_arguments)]
    fn toolbar(&mut self, ui: &mut Ui, row: Rect, folders: &[(String, usize)], all: usize, running: usize, shown: usize, total: usize, actions: &mut Vec<Action>) {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let cy = row.center().y;

        // search well — grows on focus
        let has_focus = ctx.memory(|m| m.has_focus(Id::new("search")));
        let sw = ctx.animate_value_with_time(Id::new("search_w"), if has_focus { 300.0 } else { 200.0 }, 0.2);
        let srect = Rect::from_min_size(Pos2::new(row.left(), cy - 15.0), Vec2::new(sw, 30.0));
        if has_focus {
            p.rect_stroke(srect.expand(1.5), 100.0, Stroke::new(3.0, rgba(255, 106, 0, 0.2)));
        }
        well(&p, srect, 15.0, BAY);
        icon_search(&p, Pos2::new(srect.left() + 18.0, cy), 14.0, if has_focus { LED } else { hex(0x5b6069) });
        let placeholder = match &self.folder {
            None => "Filter all projects…".to_string(),
            Some(f) => format!("Filter in {f}…"),
        };
        let edit_rect = Rect::from_min_max(Pos2::new(srect.left() + 31.0, cy - 9.0), Pos2::new(srect.right() - 28.0, cy + 9.0));
        let resp = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut self.search)
                .id(Id::new("search"))
                .frame(false)
                .margin(Vec2::ZERO)
                .font(ui_font(12.5))
                .text_color(TEAL)
                .hint_text(RichText::new(placeholder).color(hex(0x3f444c)).font(ui_font(12.5))),
        );
        if self.focus_search {
            resp.request_focus();
            self.focus_search = false;
        }
        if !self.search.is_empty() {
            let xr = Rect::from_center_size(Pos2::new(srect.right() - 15.0, cy), Vec2::splat(18.0));
            let r = ui.interact(xr, Id::new("search_clear"), Sense::click());
            icon_x(&p, xr.center(), 13.0, if r.hovered() { TEXT } else { TEXT_3 });
            if r.on_hover_text("Clear filter").clicked() {
                self.search.clear();
            }
        }

        // meta on the right: running pill, count, "/" hint
        let mut xr = row.right();
        let kbd = Rect::from_min_size(Pos2::new(xr - 18.0, cy - 9.0), Vec2::new(18.0, 18.0));
        if self.search.is_empty() && !has_focus {
            p.rect_filled(kbd, 5.0, rgba(0, 0, 0, 0.45));
            p.rect_stroke(kbd, 5.0, Stroke::new(1.0, hex(0x2e3137)));
            p.text(kbd.center(), Align2::CENTER_CENTER, "/", bold(10.0), TEXT_3);
        }
        xr = kbd.left() - 10.0;
        let count = if shown == total { format!("{total} project{}", if total == 1 { "" } else { "s" }) } else { format!("{shown} of {total}") };
        let cr = p.text(Pos2::new(xr, cy), Align2::RIGHT_CENTER, count, ui_font(11.5), TEXT_3);
        xr = cr.left() - 10.0;
        if running > 0 {
            let label = format!("{running} running");
            let w = text_w(ui, &label, bold(11.0)) + 9.0 + 5.0 + 5.0 + 9.0;
            let pr = Rect::from_min_size(Pos2::new(xr - w, cy - 10.0), Vec2::new(w, 20.0));
            p.rect_filled(pr, 100.0, rgba(0, 0, 0, 0.45));
            let pulse = if ctx.input(|i| i.focused) { (((ctx.input(|i| i.time) * std::f64::consts::PI).sin() * 0.5 + 0.5) as f32) * 0.6 + 0.4 } else { 1.0 };
            p.circle_filled(Pos2::new(pr.left() + 11.5, cy), 2.5, fade(TEAL_2, pulse));
            p.text(Pos2::new(pr.left() + 19.0, cy), Align2::LEFT_CENTER, label, bold(11.0), TEAL);
            xr = pr.left() - 10.0;
        }

        // folder chips, scrolling sideways when they do not fit
        let bar = Rect::from_min_max(Pos2::new(srect.right() + 14.0, row.top()), Pos2::new(xr - 4.0, row.bottom()));
        if bar.width() < 40.0 {
            return;
        }
        let dragging = DragAndDrop::payload::<String>(&ctx);
        let released = ctx.input(|i| i.pointer.any_released());
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        let mut entries: Vec<(Option<String>, String, usize)> = vec![(None, "All".into(), all)];
        entries.extend(folders.iter().map(|(f, n)| (Some(f.clone()), f.clone(), *n)));
        ui.allocate_new_ui(UiBuilder::new().max_rect(bar), |ui| {
            egui::ScrollArea::horizontal().id_salt("chips").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for (folder, label, n) in &entries {
                        let active = &self.folder == folder;
                        let count = n.to_string();
                        let cw = text_w(ui, &count, bold(10.0)) + 12.0;
                        let edit_w = if active && folder.is_some() { 16.0 } else { 0.0 };
                        let w = 14.0 + text_w(ui, label, ui_font(12.5)) + 6.0 + cw + edit_w + 14.0;
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 30.0), Sense::click());
                        let p = ui.painter();
                        let droppable = dragging.is_some() && folder.is_some();
                        let over = droppable && pointer.is_some_and(|q| rect.contains(q));
                        if over {
                            shadow(p, rect, 100.0, Vec2::new(0.0, 2.0), 10.0, 0.0, rgba(255, 106, 0, 0.35));
                            p.rect_filled(rect.expand(1.0), 100.0, LED);
                        } else {
                            paint_key(p, rect, 100.0, &key_look(KeyKind::Metal, resp.hovered()), false);
                            if droppable {
                                p.rect_stroke(rect, 100.0, Stroke::new(1.0, LED));
                            }
                        }
                        let col = if over {
                            hex(0x0b0b0d)
                        } else if active {
                            LED
                        } else if resp.hovered() {
                            hex(0xdfe2e7)
                        } else {
                            hex(0x8d929b)
                        };
                        let tr = p.text(Pos2::new(rect.left() + 14.0, rect.center().y), Align2::LEFT_CENTER, label, ui_font(12.5), col);
                        if active && !over {
                            shadow(p, tr, 3.0, Vec2::ZERO, 8.0, 0.0, rgba(255, 130, 30, 0.12));
                        }
                        let cr = Rect::from_min_size(Pos2::new(tr.right() + 6.0, rect.center().y - 8.0), Vec2::new(cw, 16.0));
                        p.rect_filled(cr, 100.0, rgba(0, 0, 0, 0.45));
                        p.text(cr.center(), Align2::CENTER_CENTER, count, bold(10.0), if active { hex(0xFFD9BC) } else { TEAL });
                        let mut edit_hit = false;
                        if edit_w > 0.0 {
                            let er = Rect::from_center_size(Pos2::new(cr.right() + 10.0, rect.center().y), Vec2::splat(14.0));
                            let hov = pointer.is_some_and(|q| er.contains(q));
                            icon_pencil(p, er.center(), 12.0, fade(col, if hov { 1.0 } else { 0.45 }));
                            edit_hit = hov;
                        }
                        if over && released {
                            if let (Some(f), Some(name)) = (folder, DragAndDrop::take_payload::<String>(ui.ctx())) {
                                actions.push(Action::Assign(f.clone(), (*name).clone(), true));
                            }
                        }
                        if resp.clicked() {
                            if edit_hit {
                                if let Some(f) = folder {
                                    actions.push(Action::EditFolder(f.clone()));
                                }
                            } else {
                                self.folder = folder.clone();
                            }
                        }
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                    // "+ New folder": an outline, not a key
                    let label = "+ New folder";
                    let w = 28.0 + text_w(ui, label, ui_font(12.5));
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 30.0), Sense::click());
                    let p = ui.painter();
                    p.rect_stroke(rect.shrink(0.5), 100.0, Stroke::new(1.0, rgba(255, 255, 255, if resp.hovered() { 0.18 } else { 0.09 })));
                    p.text(rect.center(), Align2::CENTER_CENTER, label, ui_font(12.5), if resp.hovered() { hex(0xc6cad1) } else { hex(0x6d727c) });
                    if resp.clicked() {
                        actions.push(Action::NewFolder);
                    }
                });
            });
        });
    }

    fn modal_ui(&mut self, ctx: &Context, actions: &mut Vec<Action>) {
        let Some(m) = self.modal.as_mut() else { return };
        let screen = ctx.screen_rect();
        let mut close = false;
        let mut confirm = false;
        let mut delete = false;
        egui::Area::new(Id::new("modal_bg")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let r = ui.allocate_rect(screen, Sense::click());
            ui.painter().rect_filled(screen, 0.0, rgba(0, 0, 0, 0.55));
            if r.clicked() {
                close = true;
            }
        });
        let w = 420.0f32.min(screen.width() - 40.0);
        let h = 236.0;
        let rect = Rect::from_center_size(screen.center(), Vec2::new(w, h));
        egui::Area::new(Id::new("modal")).order(Order::Tooltip).fixed_pos(rect.min).show(ctx, |ui| {
            ui.allocate_rect(rect, Sense::click());
            let p = ui.painter().clone();
            shadow(&p, rect, R_CARD, Vec2::new(0.0, 16.0), 48.0, 0.0, rgba(0, 0, 0, 0.5));
            p.rect_filled(rect, R_CARD, hex(0x17181b));
            rim(&p, rect, R_CARD, rgba(255, 255, 255, 0.08));
            let x = rect.left() + 24.0;
            let mut y = rect.top() + 24.0;
            let (title, hint, ok) = match &m.mode {
                ModalMode::New => ("New folder", "Group projects you use together. Drag a card's grip onto the folder to add it.", "Create"),
                ModalMode::Edit(_) => ("Edit folder", "Rename it, or delete the folder. Projects themselves are never touched.", "Save"),
            };
            p.text(Pos2::new(x, y), Align2::LEFT_TOP, title, bold(16.0), TEXT);
            y += 26.0;
            let mut job = LayoutJob::single_section(hint.to_string(), TextFormat { font_id: ui_font(12.5), color: TEXT_2, ..Default::default() });
            job.wrap.max_width = w - 48.0;
            let g = p.layout_job(job);
            let gh = g.size().y;
            p.galley(Pos2::new(x, y), g, TEXT_2);
            y += gh + 16.0;
            p.text(Pos2::new(x, y), Align2::LEFT_TOP, "NAME", bold(11.0), TEXT_3);
            y += 18.0;
            let field = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w - 48.0, 38.0));
            well(&p, field, R_SM, BAY);
            let resp = ui.put(
                field.shrink2(Vec2::new(12.0, 10.0)),
                egui::TextEdit::singleline(&mut m.text).id(Id::new("modal_text")).frame(false).margin(Vec2::ZERO).font(ui_font(14.0)).text_color(TEAL),
            );
            if !m.focused {
                resp.request_focus();
                m.focused = true;
            }
            if resp.has_focus() {
                p.rect_stroke(field.expand(1.5), R_SM + 1.5, Stroke::new(3.0, rgba(255, 106, 0, 0.2)));
            }
            y += 38.0 + 8.0;
            if !m.error.is_empty() {
                p.text(Pos2::new(x, y), Align2::LEFT_TOP, &m.error, ui_font(12.0), hex(0xFF9B9B));
            }
            let by = rect.bottom() - 24.0 - 36.0;
            let (r_ok, ok_rect) = label_key_plain(ui, Some(rect.right() - 24.0), by, Id::new("m_ok"), KeyKind::Primary, ok);
            let (r_cancel, _) = label_key_plain(ui, Some(ok_rect.left() - 8.0), by, Id::new("m_cancel"), KeyKind::Metal, "Cancel");
            if matches!(m.mode, ModalMode::Edit(_)) {
                let (r_del, _) = label_key_plain(ui, None, by, Id::new("m_del"), KeyKind::Danger, "Delete folder");
                if r_del.clicked() {
                    delete = true;
                }
                let _ = x;
            }
            if r_ok.clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                confirm = true;
            }
            if r_cancel.clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                close = true;
            }
        });
        let _ = actions;
        if delete {
            if let ModalMode::Edit(name) = &m.mode {
                let name = name.clone();
                self.shared.lock().unwrap().cfg.delete_folder(&name);
                if self.folder.as_deref() == Some(name.as_str()) {
                    self.folder = None;
                }
            }
            self.modal = None;
            return;
        }
        if confirm {
            let res = {
                let mut c = self.shared.lock().unwrap();
                match &m.mode {
                    ModalMode::New => c.cfg.create_folder(&m.text),
                    ModalMode::Edit(from) => c.cfg.rename_folder(from, &m.text),
                }
            };
            match res {
                Ok(()) => {
                    let name = m.text.trim().to_string();
                    if let ModalMode::Edit(from) = &m.mode {
                        if self.folder.as_deref() == Some(from.as_str()) {
                            self.folder = Some(name);
                        }
                    } else {
                        self.folder = Some(name);
                    }
                    self.modal = None;
                }
                Err(e) => m.error = e,
            }
            return;
        }
        if close {
            self.modal = None;
        }
    }

    fn toast_ui(&mut self, ctx: &Context, now: f64) {
        let Some((msg, at)) = &self.toast else { return };
        let age = now - at;
        if age > 2.8 {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(50));
        let alpha = (age / 0.18).min(1.0).min((2.8 - age) / 0.18) as f32;
        let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("toast")));
        let screen = ctx.screen_rect();
        let w = p.layout_no_wrap(msg.clone(), ui_font(13.0), TEXT).size().x + 36.0;
        let rect = Rect::from_center_size(Pos2::new(screen.center().x, screen.bottom() - 24.0 - 20.0 + 8.0 * (1.0 - alpha)), Vec2::new(w, 40.0));
        shadow(&p, rect, 100.0, Vec2::new(0.0, 8.0), 24.0, 0.0, rgba(0, 0, 0, 0.4 * alpha));
        p.rect_filled(rect, 100.0, fade(hex(0x2c2f34), alpha));
        p.text(rect.center(), Align2::CENTER_CENTER, msg, ui_font(13.0), fade(TEXT, alpha));
    }

    fn drag_ghost(&self, ctx: &Context) {
        let Some(name) = DragAndDrop::payload::<String>(ctx) else { return };
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("ghost")));
        let w = p.layout_no_wrap(name.to_string(), bold(12.5), TEXT).size().x + 24.0;
        let rect = Rect::from_min_size(pos + Vec2::new(12.0, 8.0), Vec2::new(w, 28.0));
        shadow(&p, rect, R_SM, Vec2::new(0.0, 5.0), 9.0, 0.0, rgba(0, 0, 0, 0.65));
        p.rect_filled(rect, R_SM, CARD_LIT);
        p.text(rect.center(), Align2::CENTER_CENTER, name.as_str(), bold(12.5), TEXT);
    }

    fn run(&mut self, ctx: &Context, actions: Vec<Action>) {
        for a in actions {
            let repaint = {
                let ctx = ctx.clone();
                move || ctx.request_repaint()
            };
            match a {
                Action::Launch(n) => core::start_async(&self.shared, n, repaint),
                Action::Restart(n) => core::restart_async(&self.shared, n, repaint),
                Action::Stop(n) => self.shared.lock().unwrap().stop(&n),
                Action::StopAll => self.shared.lock().unwrap().stop_all(),
                Action::OpenUrl(port) => core::open_url(port),
                Action::Finder(p) => core::open_finder(&p),
                Action::Editor(p) => {
                    let mut c = self.shared.lock().unwrap();
                    if let Err(e) = core::open_editor(&c.cfg, &p) {
                        c.toasts.push(format!("Edit failed: {e}"));
                    }
                }
                Action::Refresh => {
                    let s = self.shared.clone();
                    std::thread::spawn(move || {
                        let mut c = s.lock().unwrap();
                        c.check_alive();
                        c.rescan();
                        drop(c);
                        repaint();
                    });
                }
                Action::Update => {
                    let s = self.shared.clone();
                    std::thread::spawn(move || crate::update::apply(&s));
                }
                Action::CheckUpdate => {
                    let s = self.shared.clone();
                    s.lock().unwrap().update.checking = true;
                    std::thread::spawn(move || {
                        crate::update::check(&s);
                        let mut c = s.lock().unwrap();
                        c.update.checking = false;
                        let msg = c.update.message.clone();
                        c.toasts.push(msg);
                        drop(c);
                        repaint();
                    });
                }
                Action::Assign(f, p, add) => self.shared.lock().unwrap().cfg.assign(&f, &p, add),
                Action::NewFolder => self.modal = Some(Modal { mode: ModalMode::New, text: String::new(), error: String::new(), focused: false }),
                Action::EditFolder(f) => self.modal = Some(Modal { mode: ModalMode::Edit(f.clone()), text: f, error: String::new(), focused: false }),
            }
        }
    }
}

fn ui_font(s: f32) -> egui::FontId {
    crate::theme::ui(s)
}

/// Dialog key: label only.
fn label_key_plain(ui: &mut Ui, x_right: Option<f32>, y: f32, id: Id, kind: KeyKind, label: &str) -> (Response, Rect) {
    let font = bold(13.0);
    let w = text_w(ui, label, font.clone()) + 32.0;
    let x = match x_right {
        Some(xr) => xr - w,
        None => ui.max_rect().left() + 24.0,
    };
    let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 36.0));
    let label = label.to_string();
    let r = key(ui, rect, id, kind, R_SM, true, |p, r, col| {
        p.text(r.center(), Align2::CENTER_CENTER, label, font, col);
    });
    (r, rect)
}

/// The board: black, with a faint 120/24 px rule grid.
fn paint_board(p: &egui::Painter, rect: Rect) {
    p.rect_filled(rect, 0.0, BOARD);
    let mut x = rect.left();
    let mut i = 0;
    while x < rect.right() {
        let c = if i % 5 == 0 { rgba(255, 255, 255, 0.055) } else { rgba(255, 255, 255, 0.025) };
        p.line_segment([Pos2::new(x + 23.5, rect.top()), Pos2::new(x + 23.5, rect.bottom())], Stroke::new(1.0, c));
        x += 24.0;
        i += 1;
    }
    let mut y = rect.top();
    i = 0;
    while y < rect.bottom() {
        let c = if i % 5 == 4 { rgba(255, 255, 255, 0.055) } else { rgba(255, 255, 255, 0.025) };
        p.line_segment([Pos2::new(rect.left(), y + 23.5), Pos2::new(rect.right(), y + 23.5)], Stroke::new(1.0, c));
        y += 24.0;
        i += 1;
    }
}

fn elided(p: &egui::Painter, text: &str, font: egui::FontId, color: Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(text.to_string(), TextFormat { font_id: font, color, ..Default::default() });
    job.wrap = TextWrapping { max_width: max_w, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    p.layout_job(job)
}

/// One project module on the rack.
fn card(ui: &mut Ui, rect: Rect, c: &Card, t: f64, focused: bool, actions: &mut Vec<Action>) {
    let ctx = ui.ctx().clone();
    let id = Id::new(("card", &c.name));
    let starting = c.busy == Some(Busy::Starting);
    let restarting = c.busy == Some(Busy::Restarting);
    let lit = c.running && !starting;
    let hover = ui.rect_contains_pointer(rect);
    let dragged_me = DragAndDrop::payload::<String>(&ctx).is_some_and(|n| *n == c.name);
    let rect = if hover && !dragged_me { rect.translate(Vec2::new(0.0, -1.0)) } else { rect };
    let p = ui.painter().clone();

    // body
    if lit {
        shadow(&p, rect, R_CARD, Vec2::new(0.0, if hover { 5.0 } else { 3.0 }), if hover { 9.0 } else { 6.0 }, 0.0, rgba(0, 0, 0, 0.62));
        p.rect_filled(rect, R_CARD, CARD_LIT);
        rim(&p, rect, R_CARD, rgba(255, 255, 255, 0.11));
    } else {
        shadow(&p, rect, R_CARD, Vec2::new(0.0, if hover { 4.0 } else { 2.0 }), if hover { 8.0 } else { 4.0 }, 0.0, rgba(0, 0, 0, 0.5));
        p.rect_filled(rect, R_CARD, CARD_OFF);
        rim(&p, rect, R_CARD, rgba(255, 255, 255, 0.04));
    }
    // the running rail: a light pipe down the left edge, nothing when idle
    if lit && !restarting {
        let rail = Rect::from_min_size(rect.min, Vec2::new(10.0, rect.height()));
        shadow(&p, rail, egui::Rounding { nw: R_CARD, sw: R_CARD, ne: 0.0, se: 0.0 }, Vec2::ZERO, 7.0, 0.0, rgba(45, 212, 191, 0.55));
        vgrad(&p, rail, egui::Rounding { nw: R_CARD, sw: R_CARD, ne: 0.0, se: 0.0 }, &[(0.0, TEAL), (1.0, TEAL_2)]);
        p.line_segment([Pos2::new(rail.right() - 3.5, rail.top() + 8.0), Pos2::new(rail.right() - 3.5, rail.bottom() - 8.0)], Stroke::new(1.0, rgba(255, 255, 255, 0.35)));
    }

    let x0 = rect.left() + 10.0 + 20.0;
    let x1 = rect.right() - 20.0;
    let y = rect.top() + 18.0;

    // header: lamp + name
    let pulse = if lit && focused { ((t * std::f64::consts::PI).sin() * 0.5 + 0.5) as f32 } else { 0.0 };
    led(&p, Pos2::new(x0 + 4.0, y + 11.0), lit && !restarting, pulse);
    let mut nx = x0 + 16.0;
    if let Some(g) = &c.group {
        let r = p.text(Pos2::new(nx, y + 11.0), Align2::LEFT_CENTER, format!("{g}/"), ui_font(15.0), TEXT_3);
        nx = r.right();
    }
    let ng = elided(&p, &c.short, bold(15.0), TEXT, x1 - nx);
    p.galley(Pos2::new(nx, y + 11.0 - ng.size().y / 2.0), ng, TEXT);

    // description: one line, full text on hover
    let dg = elided(&p, &c.desc, ui_font(12.5), TEXT_3, x1 - x0);
    let drect = Rect::from_min_size(Pos2::new(x0, y + 28.0), Vec2::new(x1 - x0, 18.0));
    p.galley(Pos2::new(x0, drect.center().y - dg.size().y / 2.0), dg, TEXT_3);
    if !c.desc.is_empty() {
        ui.interact(drect, id.with("desc"), Sense::hover()).on_hover_text(&c.desc);
    }

    // the port strip: a lit link when running, a sleeping slot when not
    let ur = Rect::from_min_size(Pos2::new(x0, y + 60.0), Vec2::new(x1 - x0, 36.0));
    if lit {
        let resp = ui.interact(ur, id.with("url"), if restarting { Sense::hover() } else { Sense::click() });
        let k = if restarting { 0.4 } else { 1.0 };
        vgrad(&p, ur, R_SM, &[(0.0, hex(0x0b0b0d)), (1.0, hex(0x042F2E))]);
        p.rect_stroke(ur.shrink(1.0), R_SM, Stroke::new(2.0, rgba(0, 0, 0, 0.5)));
        let glow = if resp.hovered() { 0.2 } else { 0.1 };
        p.rect_stroke(ur.shrink(3.0), R_SM - 2.0, Stroke::new(4.0, rgba(45, 212, 191, glow * 0.5)));
        if resp.hovered() {
            shadow(&p, ur, R_SM, Vec2::ZERO, 8.0, 0.0, rgba(45, 212, 191, 0.12));
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        icon_external(&p, Pos2::new(ur.left() + 12.0 + 8.0, ur.center().y), 16.0, fade(TEAL, k));
        let port = c.port.unwrap_or(0);
        p.text(Pos2::new(ur.left() + 12.0 + 16.0 + 8.0, ur.center().y), Align2::LEFT_CENTER, format!("localhost:{port}"), bold(13.0), fade(TEAL, k));
        let face = if resp.hovered() { "( O.O )" } else { "( o.o )" };
        p.text(Pos2::new(ur.right() - 12.0, ur.center().y), Align2::RIGHT_CENTER, face, mono(11.0), fade(TEAL, 0.45 * k));
        if resp.on_hover_text("Open in browser").clicked() && !restarting {
            actions.push(Action::OpenUrl(port));
        }
    } else {
        let k = if hover { 0.85 } else { 0.5 };
        p.rect_filled(ur, R_SM, rgba(0, 0, 0, 0.22));
        let port = c.port.map(|p| p.to_string()).unwrap_or_else(|| "????".into());
        p.text(Pos2::new(ur.left() + 12.0, ur.center().y), Align2::LEFT_CENTER, format!("localhost:{port}"), mono(11.0), fade(hex(0x585e68), k));
        let face = if hover { "( o.o )" } else { "( -.- )" };
        p.text(Pos2::new(ur.right() - 12.0, ur.center().y), Align2::RIGHT_CENTER, face, mono(11.0), fade(hex(0x7e838d), k * 0.8));
    }

    // actions: the state key on the left, tools on the right
    let ay = y + 108.0;
    if lit {
        // Stop + Restart: one key with a groove
        let font = bold(12.0);
        let sw = 14.0 + 14.0 + 5.0 + text_w(ui, "Stop", font.clone()) + 14.0;
        let group = Rect::from_min_size(Pos2::new(x0, ay + 2.0), Vec2::new(sw + 36.0, 32.0));
        let stop_r = Rect::from_min_size(group.min, Vec2::new(sw, 32.0));
        let rs_r = Rect::from_min_max(Pos2::new(stop_r.right(), group.top()), group.max);
        let kind = if restarting { KeyKind::Metal } else { KeyKind::Stop };
        let s_resp = ui.interact(stop_r, id.with("stop"), if restarting { Sense::hover() } else { Sense::click() });
        let r_resp = ui.interact(rs_r, id.with("restart"), if c.busy.is_some() { Sense::hover() } else { Sense::click() });
        let look = key_look(kind, false);
        if let Some(g) = look.glow {
            shadow(&p, group, R_SM, Vec2::ZERO, 12.0, 0.0, g);
        }
        shadow(&p, group, R_SM, Vec2::new(0.0, 3.0), 6.0, 0.0, rgba(0, 0, 0, 0.6));
        p.rect_filled(group.translate(Vec2::new(0.0, 2.0)), R_SM, look.ledge);
        let left_r = egui::Rounding { nw: R_SM, sw: R_SM, ne: 0.0, se: 0.0 };
        let right_r = egui::Rounding { nw: 0.0, sw: 0.0, ne: R_SM, se: R_SM };
        let sl = key_look(kind, s_resp.hovered() && !restarting);
        let rl = key_look(kind, r_resp.hovered() && !restarting);
        vgrad(&p, stop_r, left_r, &sl.stops);
        vgrad(&p, rs_r, right_r, &rl.stops);
        rim(&p, group, R_SM, look.rim);
        p.line_segment([Pos2::new(rs_r.left(), rs_r.top() + 1.0), Pos2::new(rs_r.left(), rs_r.bottom() - 1.0)], Stroke::new(1.0, rgba(0, 0, 0, 0.55)));
        p.line_segment([Pos2::new(rs_r.left() + 1.0, rs_r.top() + 1.0), Pos2::new(rs_r.left() + 1.0, rs_r.bottom() - 1.0)], Stroke::new(1.0, rgba(255, 237, 213, 0.13)));
        let tcol = if restarting { TEXT_3 } else { sl.text };
        icon_stop(&p, Pos2::new(stop_r.left() + 21.0, stop_r.center().y), 14.0, tcol);
        p.text(Pos2::new(stop_r.left() + 33.0, stop_r.center().y), Align2::LEFT_CENTER, "Stop", font, tcol);
        if restarting {
            spinner(&p, rs_r.center(), 6.0, t, TEAL);
        } else {
            icon_restart(&p, rs_r.center(), 15.0, rl.text);
        }
        if s_resp.hovered() || r_resp.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if s_resp.clicked() {
            actions.push(Action::Stop(c.name.clone()));
        }
        if r_resp.on_hover_text("Restart").clicked() {
            actions.push(Action::Restart(c.name.clone()));
        }
    } else if starting {
        let font = bold(12.0);
        let w = 14.0 + 14.0 + 5.0 + text_w(ui, "Starting", font.clone()) + 14.0;
        let r = Rect::from_min_size(Pos2::new(x0, ay + 2.0), Vec2::new(w, 32.0));
        key(ui, r, id.with("launch"), KeyKind::Launch, R_SM, false, |p, r, col| {
            spinner(p, Pos2::new(r.left() + 21.0, r.center().y), 5.5, t, TEAL);
            p.text(Pos2::new(r.left() + 33.0, r.center().y), Align2::LEFT_CENTER, "Starting", font, col);
        });
    } else {
        let (resp, _) = label_key(ui, None, x0, ay + 2.0, 32.0, id.with("launch"), KeyKind::Launch, "Launch", icon_play);
        if resp.clicked() {
            actions.push(Action::Launch(c.name.clone()));
        }
    }
    // tools: editor + Finder, one quiet segmented metal control
    let tools = Rect::from_min_size(Pos2::new(x1 - 72.0, ay), Vec2::new(72.0, 36.0));
    paint_key(&p, tools, R_SM, &key_look(KeyKind::Metal, false), false);
    p.line_segment([Pos2::new(tools.center().x, tools.top() + 1.0), Pos2::new(tools.center().x, tools.bottom() - 1.0)], Stroke::new(1.0, rgba(0, 0, 0, 0.5)));
    for (i, (tip, which)) in [("Open in editor", 0), ("Reveal in Finder", 1)].iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(tools.left() + 36.0 * i as f32, tools.top()), Vec2::new(36.0, 36.0));
        let resp = ui.interact(r, id.with(("tool", i)), Sense::click());
        if resp.hovered() {
            p.rect_filled(r.shrink(1.0), if i == 0 { egui::Rounding { nw: R_SM, sw: R_SM, ne: 0.0, se: 0.0 } } else { egui::Rounding { nw: 0.0, sw: 0.0, ne: R_SM, se: R_SM } }, rgba(255, 255, 255, 0.06));
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let col = if resp.hovered() { hex(0xdfe2e7) } else { TEXT_2 };
        if *which == 0 {
            icon_code(&p, r.center(), 15.0, col);
        } else {
            icon_folder(&p, r.center(), 15.0, col, 1.6);
        }
        if resp.on_hover_text(*tip).clicked() {
            actions.push(if *which == 0 { Action::Editor(c.path.clone()) } else { Action::Finder(c.path.clone()) });
        }
    }

    // footer: what it is, then where it is filed
    let fy = y + 156.0 + 9.0;
    let mut fx = x0;
    let kind = c.kind.label();
    let tw = text_w(ui, kind, ui_font(10.0)) + 16.0;
    let tr = Rect::from_min_size(Pos2::new(fx, fy - 8.0), Vec2::new(tw, 16.0));
    engraved(&p, tr);
    p.text(tr.center(), Align2::CENTER_CENTER, kind, ui_font(10.0), TAG_TEXT);
    fx = tr.right() + 4.0;
    for f in &c.folders {
        let w = 8.0 + 9.0 + 4.0 + text_w(ui, f, ui_font(10.0)) + 4.0 + 12.0 + 4.0;
        if fx + w > x1 - 30.0 {
            break;
        }
        let r = Rect::from_min_size(Pos2::new(fx, fy - 8.0), Vec2::new(w, 16.0));
        engraved(&p, r);
        icon_folder(&p, Pos2::new(r.left() + 8.0 + 4.5, fy), 10.0, FOLDER_TEXT, 1.3);
        let lr = p.text(Pos2::new(r.left() + 8.0 + 9.0 + 4.0, fy), Align2::LEFT_CENTER, f, ui_font(10.0), FOLDER_TEXT);
        let xr = Rect::from_center_size(Pos2::new(lr.right() + 4.0 + 6.0, fy), Vec2::splat(12.0));
        let xresp = ui.interact(xr, id.with(("untag", f)), Sense::click());
        icon_x(&p, xr.center(), 9.0, fade(FOLDER_TEXT, if xresp.hovered() { 1.0 } else { 0.5 }));
        if xresp.on_hover_text(format!("Remove from {f}")).clicked() {
            actions.push(Action::Assign(f.clone(), c.name.clone(), false));
        }
        fx = r.right() + 4.0;
    }

    // drag grip: drop it on a folder chip to file the project there
    let hr = Rect::from_min_size(Pos2::new(rect.right() - 8.0 - 26.0, rect.bottom() - 8.0 - 26.0), Vec2::splat(26.0));
    let hresp = ui.interact(hr, id.with("grip"), Sense::drag());
    let hot = hresp.hovered() || hresp.dragged();
    if hot {
        vgrad(&p, hr, 8.0, &[(0.0, hex(0x2c2f34)), (1.0, hex(0x1e2024))]);
        ctx.set_cursor_icon(if hresp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
    } else {
        p.rect_filled(hr, 8.0, PLATE);
    }
    rim(&p, hr, 8.0, rgba(255, 255, 255, 0.08));
    let dot = if hot { LED } else { rgba(255, 255, 255, 0.4) };
    for (dx, dy) in [(-2.5, -2.5), (2.5, -2.5), (-2.5, 2.5), (2.5, 2.5)] {
        p.circle_filled(hr.center() + Vec2::new(dx, dy), 1.15, dot);
    }
    if hresp.drag_started() {
        DragAndDrop::set_payload(&ctx, c.name.clone());
    }
    if !hresp.dragged() {
        hresp.on_hover_text("Drag onto a folder chip to add it there");
    }
    if dragged_me {
        p.rect_filled(rect, R_CARD, rgba(0, 0, 0, 0.5));
    }
}

/// An engraved mark cut into the face: not a control.
fn engraved(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 100.0, TAG);
    let top = p.with_clip_rect(Rect::from_min_max(r.min, Pos2::new(r.max.x, r.min.y + 3.0)));
    top.rect_stroke(r.shrink(0.5), 100.0, Stroke::new(1.0, rgba(0, 0, 0, 0.85)));
    p.line_segment([Pos2::new(r.left() + 6.0, r.bottom() + 0.5), Pos2::new(r.right() - 6.0, r.bottom() + 0.5)], Stroke::new(1.0, rgba(255, 255, 255, 0.06)));
}
