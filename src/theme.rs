//! Skew (targz-design-system) dark palette and the painted parts: keys,
//! wells, lamps, icons. Every colour comes from skew.css or the original's
//! dark rules lifted from it.

use eframe::egui::{self, epaint, Color32, FontFamily, FontId, Painter, Pos2, Rect, Rounding, Shape, Stroke, Vec2};

pub const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}
pub fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, (a * 255.0) as u8)
}

pub const BOARD: Color32 = hex(0x000000);
pub const CARD_LIT: Color32 = hex(0x1e2023);
pub const CARD_OFF: Color32 = hex(0x0e0f11);
pub const BAY: Color32 = hex(0x0e0f11);
pub const PLATE: Color32 = hex(0x232529);
pub const TEXT: Color32 = hex(0xe3e6eb);
pub const TEXT_2: Color32 = hex(0x9aa0aa);
pub const TEXT_3: Color32 = hex(0x7e838d);
pub const TEAL: Color32 = hex(0x5EEAD4);
pub const TEAL_2: Color32 = hex(0x2DD4BF);
pub const LED: Color32 = hex(0xFF6A00);
pub const LED_HOT: Color32 = hex(0xFFB273);
pub const TAG: Color32 = hex(0x1a1c1f);
pub const TAG_TEXT: Color32 = hex(0x797e88);
pub const FOLDER_TEXT: Color32 = hex(0xC99A5B);

pub const R_CARD: f32 = 16.0;
pub const R_SM: f32 = 10.0;

pub fn ui(size: f32) -> FontId {
    FontId::proportional(size)
}
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// System fonts (Skew: -apple-system for UI, SF Mono/Menlo for readouts).
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let load = |path: &str, index: u32| -> Option<egui::FontData> {
        let bytes = std::fs::read(path).ok()?;
        let mut d = egui::FontData::from_owned(bytes);
        d.index = index;
        Some(d)
    };
    let mut bold_family = vec![];
    if let Some(d) = load("/System/Library/Fonts/HelveticaNeue.ttc", 0) {
        fonts.font_data.insert("ui".into(), d);
        fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "ui".into());
    }
    if let Some(d) = load("/System/Library/Fonts/HelveticaNeue.ttc", 1) {
        fonts.font_data.insert("ui-bold".into(), d);
        bold_family.push("ui-bold".to_string());
    }
    if let Some(d) = load("/System/Library/Fonts/Menlo.ttc", 0) {
        fonts.font_data.insert("mono".into(), d);
        fonts.families.get_mut(&FontFamily::Monospace).unwrap().insert(0, "mono".into());
    }
    bold_family.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts.families.insert(FontFamily::Name("bold".into()), bold_family);
    ctx.set_fonts(fonts);
}

/// Outline of a rounded rect as a convex polygon.
fn rounded_path(rect: Rect, r: Rounding) -> Vec<Pos2> {
    let max = rect.width().min(rect.height()) / 2.0;
    let r = Rounding { nw: r.nw.min(max), ne: r.ne.min(max), sw: r.sw.min(max), se: r.se.min(max) };
    let mut pts = vec![];
    let corner = |pts: &mut Vec<Pos2>, c: Pos2, rad: f32, a0: f32| {
        let steps = if rad < 0.5 { 0 } else { 6 };
        if steps == 0 {
            pts.push(c);
            return;
        }
        for i in 0..=steps {
            let a = a0 + std::f32::consts::FRAC_PI_2 * i as f32 / steps as f32;
            pts.push(c + Vec2::new(a.cos(), a.sin()) * rad);
        }
    };
    use std::f32::consts::PI;
    corner(&mut pts, Pos2::new(rect.max.x - r.se, rect.max.y - r.se), r.se, 0.0);
    corner(&mut pts, Pos2::new(rect.min.x + r.sw, rect.max.y - r.sw), r.sw, PI * 0.5);
    corner(&mut pts, Pos2::new(rect.min.x + r.nw, rect.min.y + r.nw), r.nw, PI);
    corner(&mut pts, Pos2::new(rect.max.x - r.ne, rect.min.y + r.ne), r.ne, PI * 1.5);
    pts
}

/// Vertical gradient through any number of (t, colour) stops.
pub fn vgrad(p: &Painter, rect: Rect, r: impl Into<Rounding>, stops: &[(f32, Color32)]) {
    let r = r.into();
    // anti-aliased base in the middle colour hides the mesh's hard edge
    p.rect_filled(rect, r, at(stops, 0.5));
    let pts = rounded_path(rect.shrink(0.4), r);
    let mut m = epaint::Mesh::default();
    m.colored_vertex(rect.center(), at(stops, 0.5));
    for q in &pts {
        let t = ((q.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        m.colored_vertex(*q, at(stops, t));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        m.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(Shape::mesh(m));
}

fn at(stops: &[(f32, Color32)], t: f32) -> Color32 {
    if t <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.0 {
            let k = (t - a.0) / (b.0 - a.0).max(1e-4);
            return lerp(a.1, b.1, k);
        }
    }
    stops[stops.len() - 1].1
}

pub fn lerp(a: Color32, b: Color32, k: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
    Color32::from_rgba_premultiplied(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()), f(a.a(), b.a()))
}

pub fn fade(c: Color32, k: f32) -> Color32 {
    c.gamma_multiply(k)
}

pub fn shadow(p: &Painter, rect: Rect, r: impl Into<Rounding>, offset: Vec2, blur: f32, spread: f32, color: Color32) {
    let s = epaint::Shadow { offset, blur, spread, color };
    p.add(s.as_shape(rect, r));
}

/// The lit 1px top edge every raised part has.
pub fn rim(p: &Painter, rect: Rect, radius: f32, color: Color32) {
    let y = rect.top() + 0.5;
    p.line_segment([Pos2::new(rect.left() + radius * 0.7, y), Pos2::new(rect.right() - radius * 0.7, y)], Stroke::new(1.0, color));
}

/// Skew's recess: a well cut into the face (inset shadows, lit lip below).
pub fn well(p: &Painter, rect: Rect, r: f32, fill: Color32) {
    p.rect_filled(rect, r, fill);
    let top = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + 5.0));
    let clip = p.with_clip_rect(top.intersect(p.clip_rect()));
    clip.rect_stroke(rect.shrink(0.5), r, Stroke::new(2.0, rgba(0, 0, 0, 0.85)));
    p.rect_stroke(rect.shrink(0.5), r, Stroke::new(1.0, rgba(0, 0, 0, 0.6)));
    let y = rect.bottom() + 0.5;
    p.line_segment([Pos2::new(rect.left() + r * 0.6, y), Pos2::new(rect.right() - r * 0.6, y)], Stroke::new(1.0, rgba(255, 255, 255, 0.05)));
}

#[derive(Clone, Copy, PartialEq)]
pub enum KeyKind {
    /// unlit plain metal — opens something elsewhere
    Metal,
    /// teal, lit: start the project
    Launch,
    /// orange ramp, lit because the thing it stops is running
    Stop,
    /// the one primary action in a dialog
    Primary,
    /// the destructive key: metal with a red rim
    Danger,
}

pub struct KeyLook {
    pub stops: [(f32, Color32); 3],
    pub text: Color32,
    pub ledge: Color32,
    pub glow: Option<Color32>,
    pub rim: Color32,
    pub border: Option<Color32>,
}

pub fn key_look(kind: KeyKind, hover: bool) -> KeyLook {
    match kind {
        KeyKind::Metal => KeyLook {
            stops: if hover { [(0.0, hex(0x343841)), (0.5, hex(0x2c2f36)), (1.0, hex(0x24262b))] } else { [(0.0, hex(0x2c2f34)), (0.5, hex(0x25272c)), (1.0, hex(0x1e2024))] },
            text: if hover { hex(0xdfe2e7) } else { TEXT_2 },
            ledge: hex(0x0e0f11),
            glow: None,
            rim: rgba(255, 255, 255, 0.13),
            border: None,
        },
        KeyKind::Launch => KeyLook {
            stops: if hover { [(0.0, hex(0x14B8A6)), (0.55, hex(0x0D9488)), (1.0, hex(0x0F766E))] } else { [(0.0, hex(0x0F766E)), (0.55, hex(0x115E59)), (1.0, hex(0x134E4A))] },
            text: hex(0xCCFBF1),
            ledge: hex(0x042F2E),
            glow: Some(rgba(45, 212, 191, if hover { 0.4 } else { 0.22 })),
            rim: rgba(204, 251, 241, if hover { 0.45 } else { 0.35 }),
            border: None,
        },
        KeyKind::Stop => KeyLook {
            stops: if hover { [(0.0, hex(0xC2410C)), (0.55, hex(0x9A3412)), (1.0, hex(0x7C2D12))] } else { [(0.0, hex(0x9A3412)), (0.55, hex(0x7C2D12)), (1.0, hex(0x431407))] },
            text: if hover { Color32::WHITE } else { hex(0xFFEDD5) },
            ledge: hex(0x431407),
            glow: Some(rgba(255, 106, 0, if hover { 0.45 } else { 0.25 })),
            rim: rgba(255, 237, 213, if hover { 0.45 } else { 0.35 }),
            border: None,
        },
        KeyKind::Primary => KeyLook {
            stops: [(0.0, LED_HOT), (0.5, hex(0xFF8E3A)), (1.0, LED)],
            text: hex(0x0b0b0d),
            ledge: hex(0x7C2D12),
            glow: hover.then(|| rgba(255, 106, 0, 0.3)),
            rim: rgba(255, 255, 255, 0.35),
            border: None,
        },
        KeyKind::Danger => KeyLook {
            stops: if hover { [(0.0, hex(0x8E1F1F)), (0.55, hex(0x661616)), (1.0, hex(0x490F0F))] } else { [(0.0, hex(0x2c2f34)), (0.5, hex(0x25272c)), (1.0, hex(0x1e2024))] },
            text: if hover { Color32::WHITE } else { hex(0xFF9B9B) },
            ledge: hex(0x0e0f11),
            glow: None,
            rim: rgba(255, 255, 255, 0.13),
            border: Some(hex(0x490F0F)),
        },
    }
}

/// A raised key: drop shadow, ledge, gradient face, lit rim, optional glow.
pub fn paint_key(p: &Painter, rect: Rect, r: impl Into<Rounding> + Copy, look: &KeyLook, pressed: bool) {
    let face = if pressed { rect.translate(Vec2::new(0.0, 1.0)) } else { rect };
    if let Some(g) = look.glow {
        shadow(p, rect, r, Vec2::ZERO, 12.0, 0.0, g);
    }
    shadow(p, rect, r, Vec2::new(0.0, if pressed { 2.0 } else { 3.0 }), 6.0, 0.0, rgba(0, 0, 0, 0.6));
    p.rect_filled(rect.translate(Vec2::new(0.0, if pressed { 1.0 } else { 2.0 })), r, look.ledge);
    vgrad(p, face, r, &look.stops);
    rim(p, face, r.into().nw, look.rim);
    if let Some(b) = look.border {
        p.rect_stroke(face.shrink(0.5), r, Stroke::new(1.0, b));
    }
}

// ── Icons, drawn in a box of side `s` centred on `c` ───────────────────

pub fn icon_play(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let h = s * 0.5;
    p.add(Shape::convex_polygon(
        vec![Pos2::new(c.x - h * 0.6, c.y - h * 0.75), Pos2::new(c.x + h * 0.75, c.y), Pos2::new(c.x - h * 0.6, c.y + h * 0.75)],
        col,
        Stroke::NONE,
    ));
}

pub fn icon_stop(p: &Painter, c: Pos2, s: f32, col: Color32) {
    p.rect_filled(Rect::from_center_size(c, Vec2::splat(s * 0.55)), 2.0, col);
}

pub fn icon_restart(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let r = s * 0.36;
    let st = Stroke::new(1.7, col);
    let pts: Vec<Pos2> = (0..=20)
        .map(|i| {
            let a = -0.35 + (i as f32 / 20.0) * 5.3;
            c + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    let end = pts[0];
    p.add(Shape::line(pts, st));
    p.line_segment([end, end + Vec2::new(0.0, -r * 0.75)], st);
    p.line_segment([end, end + Vec2::new(-r * 0.75, 0.0)], st);
}

pub fn icon_code(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let st = Stroke::new(1.7, col);
    let w = s * 0.42;
    let h = s * 0.3;
    p.add(Shape::line(vec![c + Vec2::new(-w * 0.45, -h), c + Vec2::new(-w, 0.0), c + Vec2::new(-w * 0.45, h)], st));
    p.add(Shape::line(vec![c + Vec2::new(w * 0.45, -h), c + Vec2::new(w, 0.0), c + Vec2::new(w * 0.45, h)], st));
}

pub fn icon_folder(p: &Painter, c: Pos2, s: f32, col: Color32, width: f32) {
    let w = s * 0.42;
    let h = s * 0.32;
    let st = Stroke::new(width, col);
    let pts = vec![
        c + Vec2::new(-w, -h),
        c + Vec2::new(-w * 0.25, -h),
        c + Vec2::new(0.0, -h * 0.55),
        c + Vec2::new(w, -h * 0.55),
        c + Vec2::new(w, h),
        c + Vec2::new(-w, h),
    ];
    p.add(Shape::closed_line(pts, st));
}

pub fn icon_external(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let st = Stroke::new(1.7, col);
    let h = s * 0.4;
    p.add(Shape::line(vec![c + Vec2::new(-0.1 * h, -h), c + Vec2::new(-h, -h), c + Vec2::new(-h, h), c + Vec2::new(h, h), c + Vec2::new(h, 0.1 * h)], st));
    p.line_segment([c + Vec2::new(-0.1 * h, 0.1 * h), c + Vec2::new(h, -h)], st);
    p.add(Shape::line(vec![c + Vec2::new(0.35 * h, -h), c + Vec2::new(h, -h), c + Vec2::new(h, -0.35 * h)], st));
}

pub fn icon_search(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let r = s * 0.3;
    let o = c + Vec2::new(-s * 0.06, -s * 0.06);
    p.circle_stroke(o, r, Stroke::new(1.7, col));
    p.line_segment([o + Vec2::splat(r * 0.72), c + Vec2::splat(s * 0.42)], Stroke::new(1.7, col));
}

pub fn icon_x(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let h = s * 0.28;
    let st = Stroke::new(1.6, col);
    p.line_segment([c + Vec2::new(-h, -h), c + Vec2::new(h, h)], st);
    p.line_segment([c + Vec2::new(h, -h), c + Vec2::new(-h, h)], st);
}

pub fn icon_pencil(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let h = s * 0.36;
    let st = Stroke::new(1.5, col);
    p.line_segment([c + Vec2::new(-h, h), c + Vec2::new(h * 0.7, -h * 0.7)], st);
    p.line_segment([c + Vec2::new(-h, h), c + Vec2::new(-h * 0.55, h * 0.1)], st);
    p.line_segment([c + Vec2::new(-h, h), c + Vec2::new(-h * 0.1, h * 0.55)], st);
}

pub fn icon_power(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let r = s * 0.36;
    let st = Stroke::new(1.7, col);
    let pts: Vec<Pos2> = (0..=20)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + 0.75 + (i as f32 / 20.0) * (std::f32::consts::TAU - 1.5);
            c + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(Shape::line(pts, st));
    p.line_segment([c + Vec2::new(0.0, -r * 1.15), c + Vec2::new(0.0, -r * 0.1)], st);
}

pub fn icon_download(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let h = s * 0.36;
    let st = Stroke::new(1.7, col);
    p.line_segment([c + Vec2::new(0.0, -h), c + Vec2::new(0.0, h * 0.4)], st);
    p.add(Shape::line(vec![c + Vec2::new(-h * 0.5, -h * 0.1), c + Vec2::new(0.0, h * 0.4), c + Vec2::new(h * 0.5, -h * 0.1)], st));
    p.line_segment([c + Vec2::new(-h, h), c + Vec2::new(h, h)], st);
}

pub fn spinner(p: &Painter, c: Pos2, r: f32, t: f64, col: Color32) {
    p.circle_stroke(c, r, Stroke::new(2.0, rgba(255, 255, 255, 0.15)));
    let a0 = (t * 10.0) as f32;
    let pts: Vec<Pos2> = (0..=10).map(|i| c + Vec2::angled(a0 + i as f32 * 0.16) * r).collect();
    p.add(Shape::line(pts, Stroke::new(2.0, col)));
}

/// The status jewel: dark until the process is alive, then lit teal.
pub fn led(p: &Painter, c: Pos2, on: bool, pulse: f32) {
    if on {
        let k = pulse;
        p.circle_stroke(c, 5.5 + 2.5 * k, Stroke::new(3.0, rgba(45, 212, 191, 0.22 * (1.0 - 0.5 * k))));
        shadow(p, Rect::from_center_size(c, Vec2::splat(8.0)), 4.0, Vec2::ZERO, 6.0, 0.0, rgba(45, 212, 191, 0.85));
        p.circle_filled(c, 4.0, hex(0x0F766E));
        p.circle_filled(c + Vec2::new(-0.4, -0.5), 3.2, TEAL_2);
        p.circle_filled(c + Vec2::new(-1.0, -1.2), 1.5, hex(0x99F6E4));
    } else {
        p.circle_filled(c, 4.5, rgba(0, 0, 0, 0.6));
        p.circle_filled(c, 4.0, hex(0x26282c));
        p.circle_filled(c + Vec2::new(-0.9, -1.0), 2.0, hex(0x3a3e45));
    }
}
