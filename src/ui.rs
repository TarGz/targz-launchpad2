//! The dashboard window, built from the design system's launcher parts
//! (`skew-egui`, `docs/skew-panels.html` § 06 Launchpad, and the running
//! `cargo run --example gallery` § 06 that is its port): the console, one
//! strip of boxes, the running switch, and one module card per project.
//! Nothing here paints a key or a well by hand — the drawings are the
//! crate's, and the pixels move only when its pinned `rev` moves.
//!
//! THE LAUNCHER IS A WINDOW, NOT A PANEL. The ruled desk is INSIDE the
//! `console` and the ground around it stays plain, because the desk belongs
//! to the application and not to the document it is scrolling.

use crate::core::{self, Busy, Report, Shared, UpdateState};
use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, Context, Event, Id, Key, LayerId, Layout, Order, Rect, Sense, Stroke, Ui, Vec2,
    ViewportCommand,
};
use skew_egui::{
    act, channel, con_say, console, empty_rack, mono, module_card, sel, sel_list, sel_list_size, slide, slot_field,
    version_readout_h, ActStyle, Icon, Lens, ModuleHit, ModuleView, Part, PullHit, PullState, SlideKind, Theme, VerState,
    MODULE_MIN_W,
};
use std::path::PathBuf;
use std::time::Duration;
use std::collections::HashMap;

const MAX_W: f32 = 1200.0;
const PAD: f32 = 24.0;
const GAP: f32 = 14.0;
/// One control height for the whole strip: `.con-top` cuts every box in that
/// row — the selector, the field, the keys, the readout — to one height.
const CAP: f32 = 32.0;
/// How long a finished report stays on a card before it leaves on its own.
const REPORT_LINGER: f64 = 5.0;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The two values that are not folders, and they lead the list. `ALL FOLDERS`
/// and `NO FOLDER` are filters; everything after them is a folder you can
/// rename and delete. The folder commands split on exactly that line.
const ALL: &str = "ALL FOLDERS";
const NONE: &str = "NO FOLDER";

/// Which inline edit the strip is in, if any. `None` is the resting row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    /// Naming a folder that does not exist yet — so there is no bin.
    New,
    /// Renaming the one that is picked — bin, field, tick.
    Rename,
}

struct Card {
    name: String,
    port: Option<u16>,
    running: bool,
    busy: Option<Busy>,
    /// Every folder this project is filed in. The card stamps the first and
    /// carries the rest as tags; the drawer writes exactly one.
    folders: Vec<String>,
    path: PathBuf,
    report: Option<Report>,
}

impl Card {
    /// The line the card shows under its keys: what it is doing now, or the
    /// last thing it said. Busy wins — it is live, a report is history.
    fn line(&self) -> Option<PullState> {
        match (self.busy, &self.report) {
            (Some(Busy::Starting), _) => Some(PullState::Ok("starting…".into())),
            (Some(Busy::Restarting), _) => Some(PullState::Ok("restarting…".into())),
            (Some(Busy::Stopping), _) => Some(PullState::Ok("stopping…".into())),
            (None, Some(Report::Run(p))) => Some(PullState::Run(*p)),
            (None, Some(Report::Ok(m))) => Some(PullState::Ok(m.clone())),
            (None, Some(Report::Err(m))) | (None, Some(Report::Fail(m))) => Some(PullState::Err(m.clone())),
            (None, None) => None,
        }
    }

    /// The folder stamped on the card. Empty prints `NO FOLDER`, which is a
    /// value and not a missing control.
    fn folder(&self) -> &str {
        self.folders.first().map(String::as_str).unwrap_or("")
    }
}

enum Action {
    Launch(String),
    Stop(String),
    Restart(String),
    OpenUrl(u16),
    Editor(PathBuf),
    Finder(PathBuf),
    Terminal(PathBuf),
    Pull(String, PathBuf),
    Dismiss(String),
    StopAll,
    Refresh,
    Update,
    CheckUpdate,
    /// The card drawer writes a project's folder: exactly one, or none.
    SetFolder(String, Option<String>),
}

pub struct App {
    shared: Shared,
    th: Theme,
    search: String,
    /// `ALL FOLDERS`, `NO FOLDER`, or a folder's name.
    folder: String,
    only_up: bool,
    /// the strip's inline edit, and the name being typed into it
    edit: Option<Edit>,
    buf: String,
    focus_buf: bool,
    /// the folder the bin has armed — the row becomes the question
    del: Option<String>,
    /// the folder selector's plate is open
    list: bool,
    sel_rect: Rect,
    /// the card whose folder drawer is open, and whether its plate is up
    card: Option<String>,
    card_list: bool,
    card_rect: Rect,
    toast: Option<(String, f64)>,
    shot: Option<PathBuf>,
    frames: u64,
    focus_search: bool,
    /// The filter field's own id, as egui minted it. `/` and Escape act on the
    /// field that is actually there, not on a name we hoped it would take.
    filter_id: Option<Id>,
    /// when a finished report should leave, by project name
    linger: HashMap<String, f64>,
    ver: VerState,
    ver_checking: bool,
    ver_t0: f64,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, shared: Shared, shot: Option<PathBuf>) -> App {
        // The light skin is the tuned one: every pixel in the crate's keys and
        // wells was approved against it (skew-egui/README.md).
        let th = Theme::light();
        th.apply(&cc.egui_ctx);
        App {
            shared,
            th,
            search: String::new(),
            folder: ALL.to_string(),
            only_up: false,
            edit: None,
            buf: String::new(),
            focus_buf: false,
            del: None,
            list: false,
            sel_rect: Rect::NOTHING,
            card: None,
            card_list: false,
            card_rect: Rect::NOTHING,
            toast: None,
            shot,
            frames: 0,
            focus_search: false,
            filter_id: None,
            linger: HashMap::new(),
            ver: VerState::Idle(stamp()),
            ver_checking: false,
            ver_t0: 0.0,
        }
    }

    /// One read of the shared state per frame — and the one write: a finished
    /// report that has had its time on screen is dropped here.
    fn snapshot(&mut self, now: f64) -> (Vec<Card>, Vec<String>, usize, String, UpdateState) {
        let mut c = self.shared.lock().unwrap();
        if let Some(t) = c.toasts.pop() {
            self.toast = Some((t, now));
            c.toasts.clear();
        }
        // a done line leaves on its own; a failed one stays until it is answered
        let done: Vec<String> = c
            .reports
            .iter()
            .filter(|(_, r)| matches!(r, Report::Ok(_)))
            .map(|(n, _)| n.clone())
            .collect();
        for n in done {
            if c.busy.contains_key(&n) {
                continue;
            }
            match self.linger.get(&n).copied() {
                None => {
                    self.linger.insert(n, now + REPORT_LINGER);
                }
                Some(t) if now > t => {
                    c.reports.remove(&n);
                    self.linger.remove(&n);
                }
                Some(_) => {}
            }
        }
        self.linger.retain(|n, _| matches!(c.reports.get(n), Some(Report::Ok(_))));
        let cards: Vec<Card> = c
            .projects
            .iter()
            .map(|p| Card {
                name: p.name.clone(),
                port: c.port_of(p),
                running: c.running.contains_key(&p.name),
                busy: c.busy.get(&p.name).copied(),
                folders: c.cfg.folders_for(&p.name),
                path: p.path.clone(),
                report: c.reports.get(&p.name).cloned(),
            })
            .collect();
        let folders = c.cfg.folders.clone();
        // a folder deleted under the selector leaves it pointing at nothing
        if self.folder != ALL && self.folder != NONE && !folders.contains(&self.folder) {
            self.folder = ALL.to_string();
        }
        if let Some(n) = &self.card {
            if !cards.iter().any(|p| &p.name == n) {
                self.card = None;
                self.card_list = false;
            }
        }
        (cards, folders, c.running.len(), c.cfg.git_dir_label.clone(), c.update.clone())
    }

    /// Which projects survive the three filters. Each one is applied to the
    /// result of the last: folder, then running, then text.
    fn in_filter(&self, p: &Card) -> bool {
        let by_folder = self.folder == ALL
            || (self.folder == NONE && p.folders.is_empty())
            || p.folders.iter().any(|f| *f == self.folder);
        by_folder && (!self.only_up || p.running)
    }

    fn matches(&self, p: &Card) -> bool {
        let q = self.search.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        let hay = format!("{} {}", p.name, p.folders.join(" ")).to_lowercase();
        hay.contains(&q)
    }

    fn filtering(&self) -> bool {
        !self.search.trim().is_empty() || self.folder != ALL || self.only_up
    }

    /// A NAMED folder is picked — so RENAME and DELETE have an object. They
    /// are PRESENT OR ABSENT rather than enabled or greyed, because a key that
    /// is there and dead still has to be read before it can be dismissed.
    fn named(&self) -> bool {
        self.folder != ALL && self.folder != NONE
    }

    /// `[ALL FOLDERS, NO FOLDER, …folders]` — the selector's values.
    fn opts(&self, folders: &[String]) -> Vec<String> {
        let mut v = vec![ALL.to_string(), NONE.to_string()];
        v.extend(folders.iter().cloned());
        v
    }

    /// Enter commits. An empty name is a cancel, because a folder with no
    /// name is not a folder. A name core refuses — a duplicate — leaves the
    /// field open with what you typed still in it, and says why in a toast.
    fn commit(&mut self) {
        let Some(mode) = self.edit else { return };
        let v = self.buf.trim().to_string();
        if v.is_empty() {
            self.edit = None;
            return;
        }
        let was = self.folder.clone();
        let res = {
            let mut c = self.shared.lock().unwrap();
            match mode {
                Edit::New => c.cfg.create_folder(&v),
                Edit::Rename => c.cfg.rename_folder(&was, &v),
            }
        };
        match res {
            Ok(()) => {
                self.folder = v;
                self.edit = None;
            }
            // the field stays open with what you typed — and focused, because
            // Enter has already blurred it and a field you cannot type in is
            // not an open field
            Err(e) => {
                self.shared.lock().unwrap().toasts.push(e);
                self.focus_buf = true;
            }
        }
    }

    /// The bin arms; the row states the consequence and offers CANCEL and
    /// DELETE. Two presses, which is the whole of the protection — what went
    /// is the room you had to be sent to, not the stop.
    fn do_delete(&mut self) {
        let Some(was) = self.del.take() else { return };
        self.shared.lock().unwrap().cfg.delete_folder(&was);
        if self.folder == was {
            self.folder = ALL.to_string();
        }
    }

    /// `UpdateState` as the readout's own state machine. UP TO DATE and CHECK
    /// FAILED are flashes: they show what the check found, then the readout
    /// goes back to being a version.
    fn ver_state(&mut self, u: &UpdateState, now: f64) -> VerState {
        if u.busy {
            // no download reports bytes, so the percentage is the phase it is
            // in — the words in `update.message` are the real report
            let pct = match u.message.as_str() {
                m if m.starts_with("Downloading") => 25,
                m if m.starts_with("Building") => 65,
                m if m.starts_with("Unzip") => 80,
                _ => 5,
            };
            self.ver_checking = false;
            return VerState::Updating(pct);
        }
        if u.checking {
            self.ver_checking = true;
            return VerState::Checking;
        }
        if self.ver_checking {
            // the check just landed
            self.ver_checking = false;
            self.ver_t0 = now;
            self.ver = if u.failed {
                VerState::Failed
            } else if u.behind > 0 {
                VerState::Available(u.latest.clone().unwrap_or_else(|| "AVAILABLE".into()))
            } else {
                VerState::Latest
            };
        }
        if u.behind > 0 {
            return VerState::Available(u.latest.clone().unwrap_or_else(|| "AVAILABLE".into()));
        }
        match &self.ver {
            VerState::Latest | VerState::Failed if now - self.ver_t0 < 2.6 => self.ver.clone(),
            _ => VerState::Idle(stamp()),
        }
    }
}

/// What this copy calls itself: the crate version, and the build number when
/// the GitHub workflow baked one in — the only way to see an auto-update land.
fn stamp() -> String {
    match crate::update::build_number() {
        Some(b) => format!("{VERSION} B{b}"),
        None => VERSION.to_string(),
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.th.paper.to_normalized_gamma_f32()
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
        let th = self.th;

        // WHETHER ESCAPE BELONGS TO THE FILTER IS READ BEFORE THE STRIP RUNS.
        // The inline flows answer Escape themselves and clear their own state
        // as they do it, so a gate read afterwards would already be open and
        // one press would both cancel the rename and empty the filter.
        let inline = self.edit.is_some() || self.del.is_some() || self.list || self.card_list;

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(th.paper))
            .show(ctx, |ui| {
                page(ui.painter(), ui.max_rect());
                egui::ScrollArea::vertical().id_salt("page").auto_shrink([false, false]).show(ui, |ui| {
                    let col = (ui.available_width() - 2.0 * PAD).min(MAX_W - 2.0 * PAD).max(220.0);
                    ui.vertical_centered(|ui| {
                        ui.set_max_width(col);
                        ui.add_space(PAD);
                        self.window(ui, &cards, &folders, running_count, &git_label, &update, now, &mut actions);
                        ui.add_space(PAD);
                    });
                });
            });

        self.toast_ui(ctx, now);

        // keyboard: "/" focuses the filter, Esc clears it — but only when no
        // inline flow is up and nothing else is taking the keys.
        if !inline && !ctx.wants_keyboard_input() {
            let slash = ctx.input(|i| i.events.iter().any(|e| matches!(e, Event::Text(t) if t == "/")));
            if slash {
                self.focus_search = true;
            }
        }
        if !inline && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.search.clear();
            if let Some(id) = self.filter_id {
                ctx.memory_mut(|m| m.surrender_focus(id));
            }
        }

        let live = cards.iter().any(|c| c.busy.is_some() || matches!(c.report, Some(Report::Run(_))))
            || matches!(self.ver, VerState::Checking | VerState::Updating(_))
            || update.busy
            || update.checking;
        ctx.request_repaint_after(Duration::from_millis(if live { 33 } else { 500 }));
        self.run(ctx, actions);
    }
}

impl App {
    /// The whole launcher: the strip, the running switch, then the rack, all
    /// of it standing on the console's own ruled desk.
    #[allow(clippy::too_many_arguments)]
    fn window(
        &mut self,
        ui: &mut Ui,
        cards: &[Card],
        folders: &[String],
        running: usize,
        git_label: &str,
        update: &UpdateState,
        now: f64,
        actions: &mut Vec<Action>,
    ) {
        let th = self.th;
        console(ui, &th, 18.0, |ui| {
            ui.set_width(ui.available_width());
            self.strip(ui, cards, running, git_label, update, now, actions);
            // TEN UNDER ROW ONE, because the switch belongs to it: a folder, a
            // field and then only-what-is-running are three narrowings of one
            // list, and a bigger break would cut the third off from the two it
            // follows.
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 9.0;
                pipe(ui, &th, self.only_up);
                let mut on = self.only_up;
                if slide(ui, &th, "lp-running", &mut on, SlideKind::Grip).clicked() {
                    self.only_up = on;
                }
                ui.label(mono("RUNNING", 9.5).color(if self.only_up { th.ink } else { th.ink_2 }));
            });
            self.folder_plate(ui, folders);
            // THIRTY-TWO UNDER THE WHOLE STRIP: the break between the window's
            // chrome and its contents. It is the only gap here bigger than the
            // 14 between two cards, because a band of controls one card-gap
            // above the rack reads as the rack's first row.
            ui.add_space(32.0);
            self.rack(ui, cards, folders, actions);
        });
    }

    /// ══ ROW ONE: A LINE OF BOXES ════════════════════════════════════════
    /// Everything cut as a box of one height — the selector, the field, the
    /// keys — reading left to right as a sentence: which folder, then what
    /// matches, then, past the channel, what the program is.
    ///
    /// IT IS LAID FROM THE RIGHT, and that is not a preference. The app group
    /// is all fixed widths and the field is the only part that gives, so
    /// taking the program's corner out of the row first leaves exactly the
    /// room the field is allowed to claim.
    #[allow(clippy::too_many_arguments)]
    fn strip(
        &mut self,
        ui: &mut Ui,
        cards: &[Card],
        running: usize,
        git_label: &str,
        update: &UpdateState,
        now: f64,
        actions: &mut Vec<Action>,
    ) {
        let th = self.th;
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                // ── what the program IS, right of the channel
                let ver = self.ver_state(update, now);
                self.ver = ver.clone();
                if version_readout_h(ui, &th, &ver, CAP).clicked() {
                    match ver {
                        VerState::Available(_) => actions.push(Action::Update),
                        VerState::Idle(_) | VerState::Latest | VerState::Failed | VerState::Done(_) => {
                            actions.push(Action::CheckUpdate)
                        }
                        _ => {}
                    }
                }
                // THE PERCENTAGE IS A GUESS AND THE WORDS ARE THE REPORT, so
                // the words stay on the strip: a check that failed has a
                // reason, and the readout only has room to say that it did.
                if (update.failed || update.busy) && !update.message.is_empty() {
                    ui.label(mono(&update.message, 9.0).color(if update.failed { th.no_ink } else { th.ink_3 }));
                }
                channel(ui, &th);
                // ── and what you do to the whole rack
                if running > 0
                    && act(
                        ui,
                        &th,
                        &[Part::Lens(Lens::Stop), Part::Text("STOP ALL"), Part::Count(running)],
                        ActStyle::Aux,
                        CAP,
                        true,
                    )
                    .on_hover_text("Stop every running server")
                    .clicked()
                {
                    actions.push(Action::StopAll);
                }
                // THE GIT DIRECTORY RIDES ON RESCAN. It is the answer to "look
                // for projects WHERE", so it belongs on the key that looks.
                if act(ui, &th, &[Part::Icon(Icon::Restart), Part::Text("RESCAN")], ActStyle::Aux, CAP, true)
                    .on_hover_text(format!("Look for projects again — {git_label}"))
                    .clicked()
                {
                    actions.push(Action::Refresh);
                }
                ui.add_space(10.0);

                // ── and your projects, in the room that is left
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if let Some(name) = self.del.clone() {
                        self.armed_row(ui, cards, &name);
                    } else if let Some(mode) = self.edit {
                        self.edit_row(ui, mode);
                    } else {
                        self.resting_row(ui);
                    }
                });
            });
        });
    }

    /// THE WHOLE ROW, not a group in it: the sentence needs the width the
    /// selector and the field were using, and leaving them there would let you
    /// change the folder you are deleting while the warning for the old one is
    /// still up.
    fn armed_row(&mut self, ui: &mut Ui, cards: &[Card], name: &str) {
        let th = self.th;
        let n = cards.iter().filter(|p| p.folders.iter().any(|f| f == name)).count();
        let tail = if n > 0 {
            format!(
                " — {n} project{} go back to no folder. Nothing on disk is touched.",
                if n == 1 { "" } else { "s" }
            )
        } else {
            " — it is empty. Nothing on disk is touched.".to_string()
        };
        con_say(ui, &th, "Delete ", name, &tail);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if act(ui, &th, &[Part::Icon(Icon::Trash), Part::Text("DELETE")], ActStyle::Halt, CAP, true).clicked() {
                self.do_delete();
            }
            if act(ui, &th, &[Part::Text("CANCEL")], ActStyle::Aux, CAP, true).clicked() {
                self.del = None;
            }
        });
        // ESCAPE LEAVES, which is what every other field here already does —
        // and the armed row is the one place a stray Return could empty a
        // folder, so nothing in it is focused by default.
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            self.del = None;
        }
    }

    /// THE SELECTOR BECOMES THE FIELD. Same place, same 186px, same row: the
    /// control that SHOWS the name is the control that TAKES it, which is how
    /// a file is renamed in every file browser there has ever been.
    fn edit_row(&mut self, ui: &mut Ui, mode: Edit) {
        let th = self.th;
        // THE BIN IS ON THE FAR SIDE OF THE FIELD FROM THE TICK. They are the
        // two ways out and they do opposite things; side by side they are a
        // pair of caps differing only by the mark on the face, and the one you
        // hit by mistake empties a folder. A folder being named for the first
        // time does not exist yet, so there is nothing to bin.
        let bin = (mode == Edit::Rename).then(|| {
            act(ui, &th, &[Part::Icon(Icon::Trash)], ActStyle::Aux, CAP, true).on_hover_text("Delete this folder")
        });
        let hint = if mode == Edit::New { "NEW FOLDER" } else { "FOLDER NAME" };
        let f = slot_field(ui, &th, 186.0, CAP, None, hint, &mut self.buf);
        if self.focus_buf {
            f.request_focus();
            self.focus_buf = false;
        }
        let ok = act(ui, &th, &[Part::Icon(Icon::Check)], ActStyle::Go, CAP, true).on_hover_text("Save this name");

        // THE ORDER THESE ARE RESOLVED IN IS THE WHOLE CORRECTNESS OF THE ROW.
        // ESCAPE FIRST: a `TextEdit` also loses focus on Escape, so a commit
        // keyed off `lost_focus` would save the name you just cancelled.
        // THE BIN NEXT: the press that arms it is the press that blurs the
        // field, so arming has to happen before the blur is read as a commit —
        // otherwise the bin arrives to delete a folder that has just been
        // renamed out from under it.
        let esc = ui.input(|i| i.key_pressed(Key::Escape));
        let binning = bin.as_ref().is_some_and(|b| b.clicked() || b.is_pointer_button_down_on());
        if esc {
            self.edit = None;
        } else if binning {
            // `was` is the folder as it stands, never the half-typed name:
            // that is the only thing there is to remove.
            self.del = Some(self.folder.clone());
            self.edit = None;
        } else if ok.clicked() || (f.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
            self.commit();
        }
    }

    /// THE VALUE LEADS AND ITS COMMAND FOLLOWS IT. The selector says which
    /// folder; the pencil acts on THAT folder, so it comes after it and
    /// vanishes with it. First would be a verb with no object for as long as
    /// ALL FOLDERS is up.
    fn resting_row(&mut self, ui: &mut Ui) {
        let th = self.th;
        let cur = self.folder.clone();
        let sr = sel(ui, &th, &cur, 186.0, CAP, self.list);
        self.sel_rect = sr.rect;
        if sr.on_hover_text("Which projects").clicked() {
            self.list = !self.list;
        }
        if self.named()
            && act(ui, &th, &[Part::Icon(Icon::Pencil)], ActStyle::Aux, CAP, true)
                .on_hover_text("Rename or delete this folder")
                .clicked()
        {
            self.edit = Some(Edit::Rename);
            self.buf = cur;
            self.focus_buf = true;
        }
        ui.add_space(10.0);
        // THE FIELD IS THE ONLY THING THAT GIVES, and it never gives
        // everything: at 200 the placeholder still reads, and a field narrower
        // than its own label cannot be told from a button.
        let w = ui.available_width().clamp(200.0, 420.0);
        let f = ui.push_id("lp-filter", |ui| slot_field(ui, &th, w, CAP, Some(Icon::Search), "FILTER", &mut self.search)).inner;
        self.filter_id = Some(f.id);
        if self.focus_search {
            f.request_focus();
            self.focus_search = false;
        }
    }

    /// THE OPEN LIST, IN AN OVERLAY AND NOT IN THE ROW. A plate that takes
    /// layout space is not a plate: laid out in the strip it pushed the rack
    /// half a window down every time the list opened, which is the one thing a
    /// popover exists to avoid.
    ///
    /// IT CLOSES THE WAY EVERY PLATE IN THIS SYSTEM CLOSES: the face, Escape,
    /// or a press anywhere else. Not the chevron, which lies down and stays
    /// down because it is a signpost.
    fn folder_plate(&mut self, ui: &mut Ui, folders: &[String]) {
        if !self.list {
            return;
        }
        let th = self.th;
        let opts = self.opts(folders);
        let refs: Vec<&str> = opts.iter().map(|s| s.as_str()).collect();
        let cur = refs.iter().position(|v| *v == self.folder).unwrap_or(0);
        let at = pos2(self.sel_rect.left(), self.sel_rect.bottom() + 6.0);
        let picked = egui::Area::new(Id::new("lp-sellist"))
            .order(Order::Foreground)
            .fixed_pos(at)
            .show(ui.ctx(), |ui| sel_list(ui, &th, 186.0, &refs, cur, &["NEW FOLDER…"]))
            .inner;
        let plate = Rect::from_min_size(at, sel_list_size(refs.len(), 1, 186.0));
        if let Some(i) = picked {
            self.list = false;
            if i < opts.len() {
                self.folder = opts[i].clone();
            } else {
                // NEW NEEDS NOTHING, so it lives in the list — that is where
                // you already are when you are thinking about folders, and the
                // only place the answer could be.
                self.edit = Some(Edit::New);
                self.buf.clear();
                self.focus_buf = true;
            }
        } else if ui.input(|i| i.key_pressed(Key::Escape)) {
            self.list = false;
        } else if ui.ctx().input(|i| i.pointer.any_click()) {
            if let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                if !plate.contains(pos) && !self.sel_rect.contains(pos) {
                    self.list = false;
                }
            }
        }
    }

    /// ══ THE RACK ════════════════════════════════════════════════════════
    fn rack(&mut self, ui: &mut Ui, cards: &[Card], folders: &[String], actions: &mut Vec<Action>) {
        let th = self.th;
        let shown: Vec<&Card> = cards.iter().filter(|c| self.in_filter(c)).filter(|c| self.matches(c)).collect();
        if shown.is_empty() {
            let (txt, out) = if self.filtering() {
                ("NOTHING MATCHES", Some("CLEAR FILTERS"))
            } else {
                ("NO PROJECTS FOUND", None)
            };
            if let Some(r) = empty_rack(ui, &th, txt, out) {
                if r.clicked() {
                    self.search.clear();
                    self.folder = ALL.to_string();
                    self.only_up = false;
                }
            }
            return;
        }
        let open = self.card.clone();
        let cols = ((ui.available_width() + GAP) / (MODULE_MIN_W + GAP)).floor().max(1.0) as usize;
        let mut hits: Vec<(String, ModuleHit)> = Vec::new();
        let mut open_rect = None;
        for row in shown.chunks(cols) {
            ui.columns(cols, |c| {
                for (k, card) in row.iter().enumerate() {
                    // THE FOLDER IS THE STAMP AND THE REST ARE TAGS. Core lets
                    // a project sit in several folders; the card names the one
                    // it leads with and carries the others beside it, so
                    // nothing the config holds goes missing off the face.
                    let tags: Vec<&str> = card.folders.iter().skip(1).map(String::as_str).collect();
                    let line = card.line();
                    let v = ModuleView {
                        name: &card.name,
                        port: card.port.unwrap_or(0),
                        up: card.running,
                        folder: card.folder(),
                        tags: &tags,
                        pull: line.as_ref(),
                        editing: open.as_deref() == Some(card.name.as_str()),
                    };
                    let out = c[k].scope(|ui| module_card(ui, &th, &v));
                    if open.as_deref() == Some(card.name.as_str()) {
                        open_rect = Some(out.response.rect);
                    }
                    if let Some(h) = out.inner {
                        hits.push((card.name.clone(), h));
                    }
                }
            });
            ui.add_space(GAP);
        }
        if let Some(r) = open_rect {
            self.card_rect = r;
        }
        for (name, h) in hits {
            let Some(card) = cards.iter().find(|c| c.name == name) else { continue };
            match h {
                // THE DRAWER SURVIVES THE RACK BEING REDRAWN UNDER IT: filing
                // a project repaints everything, and a drawer that shut every
                // time you used it is one you open twice.
                ModuleHit::Edit => {
                    self.card = if self.card.as_deref() == Some(name.as_str()) { None } else { Some(name.clone()) };
                    self.card_list = false;
                }
                ModuleHit::Drawer => self.card_list = !self.card_list,
                ModuleHit::Stop => actions.push(Action::Stop(name)),
                ModuleHit::Restart => actions.push(Action::Restart(name)),
                ModuleHit::Launch => actions.push(Action::Launch(name)),
                ModuleHit::Open => actions.push(Action::OpenUrl(card.port.unwrap_or(0))),
                ModuleHit::Editor => actions.push(Action::Editor(card.path.clone())),
                ModuleHit::Finder => actions.push(Action::Finder(card.path.clone())),
                ModuleHit::Terminal => actions.push(Action::Terminal(card.path.clone())),
                ModuleHit::Pull => actions.push(Action::Pull(name, card.path.clone())),
                // RETRY answers whatever failed: the launcher or the pull
                ModuleHit::Pull2(PullHit::Retry) => actions.push(match &card.report {
                    Some(Report::Fail(_)) if card.running => Action::Restart(name),
                    Some(Report::Fail(_)) => Action::Launch(name),
                    _ => Action::Pull(name, card.path.clone()),
                }),
                ModuleHit::Pull2(PullHit::Terminal) => actions.push(Action::Terminal(card.path.clone())),
                ModuleHit::Pull2(PullHit::Dismiss) => actions.push(Action::Dismiss(name)),
            }
        }
        self.drawer_plate(ui, cards, folders, actions);
    }

    /// The drawer's own plate. It hangs off the CARD and not off the selector
    /// inside it: `module_card` draws that selector itself and returns a hit,
    /// not a rect, and the card is the thing that stays put while the rack
    /// scrolls under it.
    fn drawer_plate(&mut self, ui: &mut Ui, cards: &[Card], folders: &[String], actions: &mut Vec<Action>) {
        if !self.card_list {
            return;
        }
        let Some(name) = self.card.clone() else { return };
        let Some(card) = cards.iter().find(|c| c.name == name) else { return };
        let th = self.th;
        let mut opts = vec![NONE.to_string()];
        opts.extend(folders.iter().cloned());
        let refs: Vec<&str> = opts.iter().map(|s| s.as_str()).collect();
        let cur = refs.iter().position(|v| *v == card.folder()).unwrap_or(0);
        let at = pos2(self.card_rect.left() + 16.0, self.card_rect.bottom() + 6.0);
        let picked = egui::Area::new(Id::new("lp-drawerlist"))
            .order(Order::Foreground)
            .fixed_pos(at)
            .show(ui.ctx(), |ui| sel_list(ui, &th, 176.0, &refs, cur, &[]))
            .inner;
        let plate = Rect::from_min_size(at, sel_list_size(refs.len(), 0, 176.0));
        if let Some(i) = picked {
            self.card_list = false;
            // ONE FOLDER, WRITTEN WHOLE. The drawer is a single selector, so
            // what it commits is a single answer: the card leaves every folder
            // it was in and joins the one that was picked.
            let to = (i > 0).then(|| opts[i].clone());
            actions.push(Action::SetFolder(name, to));
        } else if ui.input(|i| i.key_pressed(Key::Escape)) {
            self.card_list = false;
        } else if ui.ctx().input(|i| i.pointer.any_click()) {
            if let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                // the whole card is excluded, because the press that opened
                // this plate landed on the selector inside it
                if !plate.contains(pos) && !self.card_rect.contains(pos) {
                    self.card_list = false;
                }
            }
        }
    }

    /// What has no slot on a card: an editor that would not open, a folder
    /// name core refused.
    fn toast_ui(&mut self, ctx: &Context, now: f64) {
        let Some((msg, at)) = &self.toast else { return };
        let th = self.th;
        let age = now - at;
        if age > 3.2 {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(50));
        let alpha = (age / 0.18).min(1.0).min((3.2 - age) / 0.18) as f32;
        let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("toast")));
        let screen = ctx.screen_rect();
        let font = egui::FontId::monospace(11.0);
        let w = p.layout_no_wrap(msg.clone(), font.clone(), th.ink).size().x + 32.0;
        let rect = Rect::from_center_size(
            pos2(screen.center().x, screen.bottom() - 42.0 + 8.0 * (1.0 - alpha)),
            vec2(w.min(screen.width() - 40.0), 36.0),
        );
        let mut p = p;
        p.set_opacity(alpha);
        p.rect_filled(rect, th.r_key, th.card);
        p.rect_stroke(rect.shrink(0.5), th.r_key, Stroke::new(1.0, th.edge_ctl));
        p.text(rect.center(), Align2::CENTER_CENTER, msg, font, th.ink);
    }

    fn run(&mut self, ctx: &Context, actions: Vec<Action>) {
        for a in actions {
            let repaint = {
                let ctx = ctx.clone();
                move || ctx.request_repaint()
            };
            match a {
                Action::Launch(n) => {
                    self.shared.lock().unwrap().reports.remove(&n);
                    core::start_async(&self.shared, n, repaint)
                }
                Action::Restart(n) => {
                    self.shared.lock().unwrap().reports.remove(&n);
                    core::restart_async(&self.shared, n, repaint)
                }
                Action::Stop(n) => self.shared.lock().unwrap().stop(&n),
                Action::StopAll => self.shared.lock().unwrap().stop_all(),
                Action::OpenUrl(port) => core::open_url(port),
                Action::Finder(p) => core::open_finder(&p),
                Action::Dismiss(n) => {
                    self.shared.lock().unwrap().reports.remove(&n);
                }
                Action::Editor(p) => {
                    let mut c = self.shared.lock().unwrap();
                    if let Err(e) = core::open_editor(&c.cfg, &p) {
                        c.toasts.push(format!("Edit failed: {e}"));
                    }
                }
                Action::Terminal(p) => {
                    let mut c = self.shared.lock().unwrap();
                    if let Err(e) = core::open_terminal(&c.cfg, &p) {
                        c.toasts.push(format!("Terminal failed: {e}"));
                    }
                }
                Action::Pull(name, p) => {
                    let s = self.shared.clone();
                    std::thread::spawn(move || {
                        crate::update::pull_reporting(&s, &name, &p, repaint);
                    });
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
                        s.lock().unwrap().update.checking = false;
                        repaint();
                    });
                }
                Action::SetFolder(project, to) => {
                    let mut c = self.shared.lock().unwrap();
                    let all = c.cfg.folders.clone();
                    for f in &all {
                        let keep = to.as_deref() == Some(f.as_str());
                        c.cfg.assign(f, &project, keep);
                    }
                }
            }
        }
    }
}

/// `.pipe` — the layer lamp at the head of a `.tog` row: 4px of lit colour,
/// the height of the row, with its own small bloom. Unlit it is a milled bar.
/// A round lamp between the switch and its name would be a third object in a
/// part that has two.
fn pipe(ui: &mut Ui, th: &Theme, on: bool) {
    let (r, _) = ui.allocate_exact_size(vec2(4.0, 22.0), Sense::hover());
    if !ui.is_rect_visible(r) {
        return;
    }
    let p = ui.painter();
    if on {
        p.rect_filled(r.expand(3.0), egui::Rounding::same(5.0), th.primary.gamma_multiply(0.16));
        p.rect_filled(r, egui::Rounding::same(2.0), th.primary);
    } else {
        p.rect_filled(r, egui::Rounding::same(2.0), th.edge_ctl);
    }
}

/// `--page` — the light skin's own page, which is not flat paper:
/// `linear-gradient(135deg, #FAEFE4 0%, #F1EEE9 42%, #EBEDF1 100%)`
/// (`src/skew.css`, "THE PAGE KEEPS THE OLD DOCUMENT'S WARMTH"), warm cream at
/// the top left and cool grey at the bottom right, with the window standing on
/// it. egui has no gradient and the crate's `vgrad` is vertical only, so the
/// diagonal is a mesh: a grid whose vertices are coloured by where they fall
/// along the 135° line. Painted on the window, not on the scrolled content —
/// the CSS says `background-attachment: fixed`.
///
/// THE RULING IS NOT HERE ANY MORE. `console` draws the desk INSIDE the
/// launcher's window and the ground around it stays plain, because a desk that
/// ran out past the application is a desk belonging to the document.
fn page(p: &egui::Painter, rect: Rect) {
    const STOPS: [(f32, Color32); 3] = [
        (0.0, Color32::from_rgb(0xFA, 0xEF, 0xE4)),
        (0.42, Color32::from_rgb(0xF1, 0xEE, 0xE9)),
        (1.0, Color32::from_rgb(0xEB, 0xED, 0xF1)),
    ];
    /// How finely the quad is cut. The colour is linear in x and y between the
    /// stops, so the only error a grid makes is at the 42% knee, across one
    /// cell — invisible between two creams three levels apart.
    const N: usize = 24;
    fn at(t: f32) -> Color32 {
        let t = t.clamp(0.0, 1.0);
        for w in STOPS.windows(2) {
            let ((t0, a), (t1, b)) = (w[0], w[1]);
            if t <= t1 {
                let k = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
                let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k) as u8;
                return Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()));
            }
        }
        STOPS[STOPS.len() - 1].1
    }
    // CSS 0deg points up and the angle runs clockwise, so 135deg is
    // (sin a, -cos a) in screen coordinates: down and to the right. The line
    // is centred on the box and its length is |w·sin a| + |h·cos a|.
    let d = Vec2::splat(std::f32::consts::FRAC_1_SQRT_2);
    let len = ((rect.width() + rect.height()) * std::f32::consts::FRAC_1_SQRT_2).max(1.0);
    let c = rect.center();
    let mut m = egui::epaint::Mesh::default();
    for iy in 0..=N {
        for ix in 0..=N {
            let q = pos2(
                rect.left() + rect.width() * ix as f32 / N as f32,
                rect.top() + rect.height() * iy as f32 / N as f32,
            );
            m.colored_vertex(q, at(0.5 + (q - c).dot(d) / len));
        }
    }
    let row = N as u32 + 1;
    for iy in 0..N as u32 {
        for ix in 0..N as u32 {
            let i = iy * row + ix;
            m.add_triangle(i, i + 1, i + row + 1);
            m.add_triangle(i, i + row + 1, i + row);
        }
    }
    p.add(egui::Shape::mesh(m));
}
