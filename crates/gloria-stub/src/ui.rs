//! The installer window, drawn immediate-mode from `App` each frame. Coordinates are DIPs of the 640×440 window,
//! including its 1 px border; numbers follow design/mockups/mockups.css.

use crate::app::{Act, App, Flow, Modal, Page};
use crate::gfx::{font, hex, rect, Color, Family, Font, Painter, Rect, Stop, Wrap};

pub const W: f32 = 640.0;
pub const H: f32 = 440.0;

const BG: Color = hex(0x0b0b0c);
const PANEL: Color = hex(0x0d0d0f);
const PANEL2: Color = hex(0x121215);
const RAISE: Color = hex(0x141418);
const LINE: Color = hex(0x1f1f24);
const LINE2: Color = hex(0x2a2a31);
const TEXT_HI: Color = hex(0xf4f4f6);
const MUTED: Color = hex(0xa1a1aa);
const DIM: Color = hex(0x8a8a96);
const DIM2: Color = hex(0x71717a);
const FAINT: Color = hex(0x5a5a66);
const SOFT: Color = hex(0xd4d4dc);
const DANGER: Color = hex(0xef4444);
const DANGER_BG: Color = hex(0x1a0d0f);
const DANGER_LINE: Color = hex(0x4a1d24);
const DANGER_SOFT: Color = hex(0xf0a3a3);
const WARN: Color = hex(0xf5c542);
const WARN_BG: Color = hex(0x1d1a0e);
const WARN_LINE: Color = hex(0x4a3f17);
const OK: Color = hex(0x86c49a);
const WHITE: Color = hex(0xffffff);
const CREAM: Color = hex(0xf2f1ee);
const WIN_BORDER: Color = hex(0x26262d);
/// projectGloria red (GloriaRed.ico) for the footer link's hover.
const GLORIA_RED: Color = hex(0xd0141a);
const GLORIA_RED_HI: Color = hex(0xff5c5c);

const MONO: Font = font(Family::Mono, 400, 10.5);
const KICKER: Font = font(Family::Mono, 600, 10.0).track(0.2);
const H1: Font = font(Family::Display, 800, 32.0).line(1.05).track(-0.01);
const H2: Font = font(Family::Display, 800, 22.0).line(1.15);
const P: Font = font(Family::Sans, 400, 13.0).line(1.55);
const LBL: Font = font(Family::Mono, 600, 9.5).track(0.16);
const BTN: Font = font(Family::Mono, 700, 10.5).track(0.16);
const CB: Font = font(Family::Sans, 400, 12.5).line(1.4);

pub const CONTENT_X: f32 = 37.0;
pub const CONTENT_R: f32 = 603.0;
pub const CONTENT_Y: f32 = 65.0;
pub const BODY_Y: f32 = 37.0;
pub const FOOT_Y: f32 = 375.0;
/// Window buttons (minimize, close).
pub const MIN_BTN: Rect = rect(551.0, 1.0, 44.0, 35.0);
pub const CLOSE_BTN: Rect = rect(595.0, 1.0, 44.0, 35.0);

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub r: Rect,
    pub act: Act,
    pub enabled: bool,
}

/// Pointer and keyboard state the drawing reacts to.
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub hover: Option<Act>,
    pub pressed: Option<Act>,
    pub focus: Option<Act>,
    /// Footer GitHub link hover effect, 0 (rest) to 1 (fully in). The window animates it.
    pub link: f32,
}

/// Result of a frame: clickable regions in tab order.
#[derive(Default)]
pub struct Frame {
    pub hits: Vec<Hit>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Normal,
    Ghost,
    Primary,
    Danger,
}

#[derive(Clone, Copy, PartialEq)]
enum Badge {
    None,
    Ok,
    Err,
    Gray,
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a + (b.a - a.a) * t }
}

fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t)
}

struct Ui<'u, 'a> {
    p: &'u mut Painter<'a>,
    app: &'u App,
    inp: Input,
    hits: Vec<Hit>,
    a: Color,
    now: f32,
}

pub fn draw(p: &mut Painter, app: &App, inp: Input, now: f64) -> Frame {
    let a = if app.page.danger() { DANGER } else { app.info.accent };
    let mut ui = Ui { p, app, inp, hits: Vec::new(), a, now: now as f32 };
    ui.window(now);
    Frame { hits: ui.hits }
}

impl<'u, 'a> Ui<'u, 'a> {
    fn t(&self, k: &str) -> String {
        self.app.s.t(k)
    }
    fn up(&self, k: &str) -> String {
        self.app.s.upper(&self.app.s.t(k))
    }
    fn lh(&self, f: Font) -> f32 {
        self.p.g.line_height(f)
    }

    fn window(&mut self, now: f64) {
        let app = self.app;
        self.p.clear(BG);
        let modal = app.modal;
        let page_t = ((now - app.page_t0) as f32 / 0.18).clamp(0.0, 1.0);
        let k = ease_out(page_t);

        // Page (and its wash), blurred behind a modal.
        if modal.is_some() {
            let body = rect(1.0, BODY_Y, W - 2.0, H - BODY_Y - 1.0);
            self.p.clip(body);
            let saved = std::mem::take(&mut self.hits);
            {
                let (app, a, nowf) = (self.app, self.a, self.now);
                self.p.blurred(W, H, 5.0, |sub| {
                    let mut u = Ui { p: sub, app, inp: Input::default(), hits: Vec::new(), a, now: nowf };
                    u.wash();
                    u.page();
                });
            }
            self.hits = saved;
            let mk = ((now - app.modal_t0) as f32 / 0.15).clamp(0.0, 1.0);
            self.p.fill(body, BG.alpha(0.62 * mk));
            self.p.unclip();
        } else {
            self.wash();
            let anim = page_t < 1.0;
            if anim {
                self.p.push_opacity(k);
                self.p.translate(0.0, 8.0 * (1.0 - k));
            }
            self.page();
            if anim {
                self.p.reset_transform();
                self.p.pop_opacity();
            }
        }

        self.titlebar();
        self.accent_line();
        if let Some(m) = modal {
            let mt = ease_out(((now - app.modal_t0) as f32 / 0.18).clamp(0.0, 1.0));
            self.p.push_opacity(mt);
            self.p.translate(0.0, 8.0 * (1.0 - mt));
            self.modal(m);
            self.p.reset_transform();
            self.p.pop_opacity();
        }
        self.p.border(rect(0.0, 0.0, W, H), WIN_BORDER);
    }

    fn wash(&mut self) {
        let k = if self.app.page.danger() { 0.10 } else { 0.13 };
        let a = self.a;
        self.p.radial(rect(1.0, 1.0, W - 2.0, H - 2.0), 93.0, 151.0, 420.0, 300.0, &[Stop(0.0, a.alpha(k)), Stop(0.7, a.alpha(0.0))]);
    }

    fn accent_line(&mut self) {
        let a = self.a;
        self.p.glow(rect(1.0, 1.0, 300.0, 2.0), 6.0, a.alpha(0.5));
        self.p.glow(rect(300.0, 1.0, 200.0, 2.0), 6.0, a.alpha(0.22));
        self.p.linear(rect(1.0, 1.0, W - 2.0, 2.0), &[Stop(0.0, a), Stop(0.45, a.alpha(0.55)), Stop(1.0, a.alpha(0.0))]);
    }

    fn titlebar(&mut self) {
        let app = self.app;
        self.p.hline(1.0, W - 1.0, 36.0, LINE);
        self.p.image(&app.info.icon, rect(13.0, 11.0, 16.0, 16.0), 3.0, 1.0);
        let f = font(Family::Mono, 400, 10.0).track(0.14);
        let title = app.s.upper(&app.title());
        let ty = 1.0 + (35.0 - self.lh(f)) / 2.0;
        let max = MIN_BTN.x - 38.0 - 90.0;
        let t = self.p.g.text(f, &title, max, Wrap::Ellipsis);
        self.p.text(&t, 38.0, ty, DIM);
        if app.m_test_build() {
            let tf = font(Family::Mono, 400, 8.5).track(0.14);
            let tw = self.p.g.measure(tf, "TEST BUILD");
            let th = self.lh(tf) + 4.0;
            let r = rect(38.0 + t.w + 9.0, 1.0 + (35.0 - th) / 2.0, tw + 12.0, th);
            self.p.fill(r, WARN_BG);
            self.p.border(r, WARN_LINE);
            self.p.label(tf, "TEST BUILD", r.x + 6.0, r.y + 2.0, WARN);
        }
        for (r, act, icon, hover_bg) in [(MIN_BTN, Act::Minimize, "minus", hex(0x18181d)), (CLOSE_BTN, Act::Close, "x", hex(0xdc143c))] {
            let hov = self.inp.hover == Some(act) && self.app.modal.is_none() || (self.inp.hover == Some(act) && act == Act::Close);
            if hov {
                self.p.fill(r, hover_bg);
            }
            self.p.icon(icon, r.cx() - 6.5, r.cy() - 6.5, 13.0, 2.0, if hov { WHITE } else { DIM });
            if self.inp.focus == Some(act) {
                self.p.border(r.inset(2.0), self.a.alpha(0.8));
            }
        }
    }

    // ───────────── page ─────────────

    fn page(&mut self) {
        let app = self.app;
        let x = CONTENT_X;
        // Window-control hits first so they are always reachable.
        self.hits.push(Hit { r: MIN_BTN, act: Act::Minimize, enabled: true });
        self.hits.push(Hit { r: CLOSE_BTN, act: Act::Close, enabled: true });
        match app.page {
            Page::Welcome => self.welcome(x),
            Page::Progress | Page::UninstallProgress => self.progress(x),
            Page::Done => self.done(x),
            Page::Update => self.update(x),
            Page::Reinstall => self.reinstall(x),
            Page::Downgrade => self.downgrade(x),
            Page::Migrate => self.migrate(x),
            Page::Uninstall => self.uninstall(x),
            Page::UninstallDone => self.uninstall_done(x),
            Page::Error => self.error(x),
        }
    }

    /// Draws the hero icon and returns the text column (x, width).
    fn hero(&mut self, size: f32, badge: Badge) -> (f32, f32) {
        let (ix, iy) = (CONTENT_X, CONTENT_Y);
        let a = self.a;
        let app = self.app;
        if badge != Badge::Gray {
            let blur = if size < 80.0 { 12.0 } else { 22.0 };
            self.p.glow(rect(ix + size * 0.1, iy + size * 0.1, size * 0.8, size * 0.8), blur, a.alpha(0.9 * 0.45));
        }
        let img = if badge == Badge::Gray { &app.info.icon_gray } else { &app.info.icon };
        self.p.image(img, rect(ix, iy, size, size), size * 0.18, 1.0);
        if matches!(badge, Badge::Ok | Badge::Err) {
            let (c, fg, icon) = if badge == Badge::Ok { (a, a.on(), "check") } else { (DANGER, WHITE, "x") };
            let (cx, cy) = (ix + size + 6.0 - 15.0, iy + size + 6.0 - 15.0);
            self.p.glow(rect(cx - 15.0, cy - 15.0, 30.0, 30.0), 7.0, c.alpha(0.8));
            self.p.circle(cx, cy, 15.0, BG);
            self.p.circle(cx, cy, 12.0, c);
            self.p.icon(icon, cx - 7.5, cy - 7.5, 15.0, 3.0, fg);
        }
        let tx = ix + size + 24.0;
        (tx, CONTENT_R - tx)
    }

    fn kicker(&mut self, x: f32, y: f32, s: &str, c: Color) -> f32 {
        let lh = self.lh(KICKER);
        let sy = y + (lh - 6.0) / 2.0;
        self.p.glow(rect(x, sy, 6.0, 6.0), 4.0, c.alpha(0.85));
        self.p.fill(rect(x, sy, 6.0, 6.0), c);
        let s = self.app.s.upper(s);
        self.p.label(KICKER, &s, x + 14.0, y, c);
        y + lh
    }

    fn heading(&mut self, f: Font, x: f32, y: f32, w: f32, s: &str) -> f32 {
        let t = self.p.g.text(f, s, w, Wrap::Wrap);
        self.p.text(&t, x, y + 8.0, TEXT_HI);
        y + 8.0 + t.h
    }

    fn para(&mut self, f: Font, x: f32, y: f32, w: f32, s: &str, c: Color) -> f32 {
        let t = self.p.g.text(f, s, w.min(520.0), Wrap::Wrap);
        self.p.text(&t, x, y, c);
        y + t.h
    }

    /// Two-column fact rows (label 92 px + value). `ok` rows are green with a shield.
    fn facts(&mut self, x: f32, y: f32, w: f32, rows: &[(&str, String, bool)]) -> f32 {
        self.p.hline(x, x + w, y, LINE);
        let mut y = y + 1.0;
        let vf = font(Family::Mono, 400, 11.0);
        let okf = font(Family::Sans, 400, 12.0);
        for (label, value, ok) in rows {
            let h = 18.0 + self.lh(vf).max(self.lh(LBL)).max(if *ok { self.lh(okf).max(14.0) } else { 0.0 });
            let lt = self.app.s.upper(&self.t(label));
            self.p.label(LBL, &lt, x, y + 9.0, DIM2);
            let vx = x + 92.0;
            if *ok {
                let rh = h - 18.0;
                self.p.icon("shield", vx, y + 9.0 + (rh - 14.0) / 2.0, 14.0, 2.0, OK);
                let t = self.p.g.text(okf, value, w - 92.0 - 21.0, Wrap::Ellipsis);
                self.p.text(&t, vx + 21.0, y + 9.0 + (rh - self.lh(okf)) / 2.0, OK);
            } else {
                let t = self.p.g.text(vf, value, w - 92.0, Wrap::Ellipsis);
                self.p.text(&t, vx, y + 9.0, SOFT);
            }
            y += h;
            self.p.hline(x, x + w, y - 1.0, LINE);
        }
        y
    }

    fn chip(&mut self, x: f32, y: f32, s: &str, fg: Color, bg: Color, border: Color, f: Font, pad: (f32, f32), row_h: f32) -> f32 {
        let tw = self.p.g.measure(f, s);
        let h = self.lh(f) + 2.0 * pad.0 + 2.0;
        let r = rect(x, y + (row_h - h) / 2.0, tw + 2.0 * pad.1 + 2.0, h);
        self.p.fill(r, bg);
        self.p.border(r, border);
        self.p.label(f, s, r.x + 1.0 + pad.1, r.y + 1.0 + pad.0, fg);
        r.w
    }

    // ───────────── controls ─────────────

    fn btn_w(&self, label: &str, kind: Kind, icon: Option<&str>) -> f32 {
        let pad = if kind == Kind::Ghost { 10.0 } else { 16.0 };
        let tw = self.p.g.measure(BTN, label);
        let w = tw + 2.0 * pad + 2.0 + if icon.is_some() { 21.0 } else { 0.0 };
        if matches!(kind, Kind::Primary | Kind::Danger) {
            w.max(132.0)
        } else {
            w
        }
    }

    fn button(&mut self, x: f32, y: f32, label: &str, kind: Kind, icon: Option<&'static str>, act: Act, enabled: bool, spinner: bool) -> f32 {
        let label = self.app.s.upper(label);
        let w = self.btn_w(&label, kind, if spinner { Some("x") } else { icon });
        let r = rect(x, y, w, 36.0);
        self.hits.push(Hit { r, act, enabled });
        let hov = enabled && self.inp.hover == Some(act);
        let down = hov && self.inp.pressed == Some(act);
        let r = if down { r.offset(0.0, 1.0) } else { r };
        if !enabled {
            self.p.push_opacity(0.35);
        }
        let (bg, border, fg) = match kind {
            Kind::Normal => (if hov { RAISE } else { Color { a: 0.0, ..BG } }, LINE2, if hov { WHITE } else { SOFT }),
            Kind::Ghost => (if hov { RAISE } else { Color { a: 0.0, ..BG } }, Color { a: 0.0, ..BG }, if hov { WHITE } else { DIM }),
            Kind::Primary => (if hov { WHITE } else { CREAM }, if hov { WHITE } else { CREAM }, BG),
            Kind::Danger => (if hov { hex(0xf05252) } else { DANGER }, if hov { hex(0xf05252) } else { DANGER }, WHITE),
        };
        if hov && kind == Kind::Primary {
            self.p.glow(r, 9.0, self.a.alpha(0.35));
        }
        if hov && kind == Kind::Danger {
            self.p.glow(r, 9.0, DANGER.alpha(0.45));
        }
        self.p.fill(r, bg);
        self.p.border(r, border);
        let pad = if kind == Kind::Ghost { 10.0 } else { 16.0 };
        let tw = self.p.g.measure(BTN, &label);
        let content = tw + if icon.is_some() || spinner { 21.0 } else { 0.0 };
        let mut cx = if matches!(kind, Kind::Primary | Kind::Danger) { r.x + (r.w - content) / 2.0 } else { r.x + 1.0 + pad };
        let cy = r.cy();
        if spinner {
            self.p.spinner(cx + 6.5, cy, 6.5, 2.0, self.now, fg);
            cx += 21.0;
        } else if let Some(ic) = icon {
            self.p.icon(ic, cx, cy - 6.5, 13.0, 2.4, fg);
            cx += 21.0;
        }
        self.p.label(BTN, &label, cx, cy - self.lh(BTN) / 2.0, fg);
        if !enabled {
            self.p.pop_opacity();
        }
        if self.inp.focus == Some(act) && enabled {
            self.p.border(r.inset(-3.0), self.a.alpha(0.8));
        }
        w
    }

    /// Lays out the footer: the GitHub link at the left edge on every page, then `left` buttons after it, and `right`
    /// buttons from the right edge (last item rightmost). `credit` puts the GloriaSetup credit just above the footer.
    fn footer(&mut self, credit: bool, left: &[(&str, Kind, Option<&'static str>, Act, bool)], right: &[(&str, Kind, Option<&'static str>, Act, bool, bool)]) {
        let y = FOOT_Y;
        if credit {
            if let Some(img) = &self.app.credit {
                let f = font(Family::Mono, 400, 8.5).track(0.16);
                let s = self.up("credit");
                let cy = y - 22.0;
                let img = img.clone();
                self.p.image(&img, rect(CONTENT_X, cy - 6.5, 13.0, 13.0), 2.0, 0.8);
                self.p.label(f, &s, CONTENT_X + 20.0, cy - self.lh(f) / 2.0, FAINT);
            }
        }
        self.p.fill(rect(1.0, y, W - 2.0, H - 1.0 - y), PANEL.alpha(0.85));
        self.p.hline(1.0, W - 1.0, y, LINE);
        let by = y + 1.0 + (63.0 - 36.0) / 2.0;
        let mut x = CONTENT_X;
        x += self.github_link(x, y + 32.0) + 18.0;
        for (label, kind, icon, act, en) in left {
            let w = self.button(x, by, label, *kind, *icon, *act, *en, false);
            x += w + 10.0;
        }
        let mut rx = 619.0;
        let widths: Vec<f32> =
            right.iter().map(|(l, k, i, _, _, sp)| self.btn_w(&self.app.s.upper(l), *k, if *sp { Some("x") } else { *i })).collect();
        let total: f32 = widths.iter().sum::<f32>() + 10.0 * (right.len().saturating_sub(1)) as f32;
        rx -= total;
        for ((label, kind, icon, act, en, sp), w) in right.iter().zip(widths) {
            self.button(rx, by, label, *kind, *icon, *act, *en, *sp);
            rx += w + 10.0;
        }
    }

    /// GloriaRed mark + "github.com/ProjectGloria", vertically centred on `cy`. On hover the mark lights up with a red
    /// glow and ring, the text brightens, a red underline sweeps in and an arrow slides out. Returns the width.
    fn github_link(&mut self, x: f32, cy: f32) -> f32 {
        let act = Act::OpenGithub;
        let k = self.inp.link.clamp(0.0, 1.0);
        let down = self.inp.hover == Some(act) && self.inp.pressed == Some(act);
        let cy = if down { cy + 1.0 } else { cy };
        let f = font(Family::Mono, 500, 10.0).track(0.02);
        let (host, org) = ("github.com/", "ProjectGloria");
        let hw = self.p.g.measure(f, host);
        let ow = self.p.g.measure(f, org);
        let size = 20.0;
        let ir = rect(x, cy - size / 2.0, size, size);
        let tx = ir.r() + 9.0;
        let arrow = 10.0;
        let w = tx - x + hw + ow + 5.0 + arrow;
        self.hits.push(Hit { r: rect(x - 5.0, cy - 15.0, w + 10.0, 30.0), act, enabled: true });

        // Mark: soft red bloom and a thin ring that fade in.
        if k > 0.0 {
            self.p.glow(ir.inset(1.0), 5.0 + 5.0 * k, GLORIA_RED.alpha(0.75 * k));
        }
        match &self.app.gloria {
            Some(img) => {
                let img = img.clone();
                self.p.image(&img, ir, 4.5, 0.82 + 0.18 * k);
            }
            None => self.p.fill(ir, GLORIA_RED),
        }
        if k > 0.0 {
            self.p.border(ir.inset(-2.0), GLORIA_RED_HI.alpha(0.55 * k));
        }

        // Text: host dims, org brightens to white.
        let ty = cy - self.lh(f) / 2.0;
        self.p.label(f, host, tx, ty, mix(FAINT, MUTED, k));
        self.p.label(f, org, tx + hw, ty, mix(DIM, WHITE, k));

        // Underline sweeping in from the left, with a glow.
        let uy = (cy + self.lh(f) / 2.0 + 1.0).round();
        let uw = (hw + ow) * k;
        if uw > 0.5 {
            let ur = rect(tx, uy, uw, 1.0);
            self.p.glow(ur, 3.0, GLORIA_RED.alpha(0.9 * k));
            self.p.linear(ur, &[Stop(0.0, GLORIA_RED_HI.alpha(k)), Stop(1.0, GLORIA_RED.alpha(k))]);
        }

        // Arrow sliding out to the upper right.
        if k > 0.0 {
            let ax = tx + hw + ow + 5.0 - 4.0 * (1.0 - k);
            let ay = cy - arrow / 2.0 + 2.0 * (1.0 - k);
            self.p.icon("external", ax, ay, arrow, 2.4, GLORIA_RED_HI.alpha(k));
        }

        if self.inp.focus == Some(act) {
            self.p.border(rect(x - 5.0, cy - 15.0, w + 10.0, 30.0), self.a.alpha(0.8));
        }
        w
    }

    /// Checkbox with a label that wraps in `w`. Returns the bottom.
    fn checkbox(&mut self, x: f32, y: f32, w: f32, label: &str, on: bool, act: Act, danger: bool, locked_tag: Option<&str>) -> f32 {
        let t = self.p.g.text(CB, label, w - 26.0, Wrap::Wrap);
        let h = t.h.max(17.0);
        let mut hit_w = t.w + 26.0;
        let locked = locked_tag.is_some();
        let hov = !locked && self.inp.hover == Some(act);
        let b = rect(x, y + 1.0, 16.0, 16.0);
        let c = if danger { DANGER } else { self.a };
        if locked {
            self.p.fill(b, hex(0x24242b));
            self.p.icon("check", b.x + 2.5, b.y + 2.5, 11.0, 3.2, DIM);
        } else if on {
            self.p.glow(b, 5.0, c.alpha(if danger { 0.5 } else { 0.45 }));
            self.p.fill(b, c);
            self.p.icon("check", b.x + 2.5, b.y + 2.5, 11.0, 3.2, if danger { WHITE } else { c.on() });
        } else {
            self.p.fill(b, PANEL);
            self.p.border(b, if hov { hex(0x5a5a68) } else { hex(0x3a3a44) });
        }
        let color = if locked { DIM } else if danger && on { DANGER_SOFT } else { SOFT };
        self.p.text(&t, x + 26.0, y, color);
        if let Some(tag) = locked_tag {
            let tf = font(Family::Mono, 400, 8.5).track(0.14);
            let tw = self.p.g.measure(tf, tag);
            let th = self.lh(tf) + 4.0;
            let r = rect(x + 26.0 + t.w + 8.0, y + (17.5 - th) / 2.0, tw + 12.0, th);
            self.p.border(r, LINE2);
            self.p.label(tf, tag, r.x + 6.0, r.y + 2.0, DIM2);
            hit_w += 8.0 + r.w;
        }
        if !locked {
            self.hits.push(Hit { r: rect(x - 2.0, y - 2.0, hit_w + 4.0, h + 4.0), act, enabled: true });
            if self.inp.focus == Some(act) {
                self.p.border(rect(x - 4.0, y - 3.0, hit_w + 8.0, h + 6.0), self.a.alpha(0.8));
            }
        }
        y + h
    }

    fn data_block(&mut self, x: f32, y: f32, w: f32, key: &str, tight: bool) -> f32 {
        let app = self.app;
        let on = app.purge;
        let (py, px) = if tight { (9.0, 12.0) } else { (12.0, 14.0) };
        // Measure first: height depends on rows.
        let rowf = MONO;
        let row_h = self.lh(rowf) + 4.0;
        let totf = font(Family::Mono, 400, 9.5).track(0.12);
        let label = self.t(key);
        let cbh = self.p.g.text(CB, &label, w - 2.0 * px - 2.0 - 26.0, Wrap::Wrap).h.max(17.0);
        let ul_top = if tight { 6.0 } else { 9.0 };
        let h = 2.0 + 2.0 * py + cbh + ul_top + row_h * app.info.data.len() as f32 + 6.0 + self.lh(totf);
        let r = rect(x, y, w, h);
        self.p.fill(r, if on { DANGER_BG } else { PANEL.alpha(0.7) });
        self.p.border(r, if on { DANGER_LINE } else { LINE });
        let ix = x + 1.0 + px;
        let iw = w - 2.0 - 2.0 * px;
        let mut cy = self.checkbox(ix, y + 1.0 + py, iw, &label, on, Act::Purge, true, None);
        cy += ul_top;
        for (path, size) in &app.info.data {
            let c = if on { DANGER_SOFT } else { DIM };
            let sz = size.map(|s| app.s.size(s)).unwrap_or_default();
            let sf = font(Family::Mono, 500, 10.5);
            let sw = self.p.g.measure(sf, &sz);
            let t = self.p.g.text(rowf, path, iw - 26.0 - sw - 12.0, Wrap::Ellipsis);
            self.p.text(&t, ix + 26.0, cy + 2.0, c);
            self.p.label(sf, &sz, ix + iw - sw, cy + 2.0, if on { DANGER_SOFT } else { DIM2 });
            cy += row_h;
        }
        let total = app.s.size(app.info.data_total());
        let s = app.s.upper(&app.s.tv(if on { "data.total" } else { "data.totalKeep" }, &[("size", &total)]));
        self.p.label(totf, &s, ix + 26.0, cy + 6.0, if on { DANGER } else { DIM2 });
        y + h
    }

    /// Installed → new version box. Returns the bottom.
    fn versions(&mut self, x: f32, y: f32, from: &str, to: &str, to_label: &str, to_color: Color, glow: bool, tight: bool) -> f32 {
        let (py, px, nsz) = if tight { (6.0, 14.0, 17.0) } else { (10.0, 18.0, 20.0) };
        let nf = font(Family::Mono, 600, nsz);
        let l1 = self.up("update.installed");
        let l2 = self.app.s.upper(to_label);
        let c1 = self.p.g.measure(LBL, &l1).max(self.p.g.measure(nf, from)) + 2.0 * px;
        let c3 = self.p.g.measure(LBL, &l2).max(self.p.g.measure(nf, to)) + 2.0 * px;
        let c2 = 14.0 + 8.0 + 2.0;
        let h = 2.0 + 2.0 * py + self.lh(LBL) + 4.0 + self.lh(nf);
        let r = rect(x, y, c1 + c2 + c3 + 2.0, h);
        self.p.border(r, LINE);
        let ax = x + 1.0 + c1;
        self.p.fill(rect(ax, y + 1.0, c2, h - 2.0), PANEL);
        self.p.vline(ax, y + 1.0, y + h - 1.0, LINE);
        self.p.vline(ax + c2 - 1.0, y + 1.0, y + h - 1.0, LINE);
        self.p.icon("arrow", ax + 5.0, y + h / 2.0 - 7.0, 14.0, 2.0, DIM2);
        let ty = y + 1.0 + py;
        let ny = ty + self.lh(LBL) + 4.0;
        self.p.label(LBL, &l1, x + 1.0 + px, ty, DIM2);
        self.p.label(nf, from, x + 1.0 + px, ny, MUTED);
        let x3 = ax + c2 + px;
        self.p.label(LBL, &l2, x3, ty, DIM2);
        if glow {
            let w = self.p.g.measure(nf, to);
            self.p.glow(rect(x3, ny + 4.0, w, self.lh(nf) - 8.0), 7.0, to_color.alpha(0.3));
        }
        self.p.label(nf, to, x3, ny, to_color);
        y + h
    }

    fn hint(&mut self, x: f32, y: f32, w: f32, icon: &'static str, icon_c: Color, text_c: Color, lines: &[(&str, Option<&str>)]) -> f32 {
        let f = font(Family::Sans, 400, 11.5).line(1.45);
        self.p.icon(icon, x, y + 1.0, 14.0, 2.0, icon_c);
        let mut cy = y;
        for (s, mono) in lines {
            let t = self.p.g.text(f, s, w - 21.0, Wrap::Wrap);
            self.p.text(&t, x + 21.0, cy, text_c);
            if let Some(m) = mono {
                let mf = font(Family::Mono, 400, 10.5);
                let mx = x + 21.0 + t.w + 4.0;
                let mt = self.p.g.text(mf, m, (x + w - mx).max(40.0), Wrap::Ellipsis);
                let base = self.baseline(f) - self.baseline(mf);
                self.p.text(&mt, mx, cy + base, SOFT);
            }
            cy += t.h;
        }
        cy
    }

    /// Distance from the top of a line box to the baseline.
    fn baseline(&self, f: Font) -> f32 {
        let t = self.p.g.text(f, "x", 0.0, Wrap::None);
        let mut lm = [windows::Win32::Graphics::DirectWrite::DWRITE_LINE_METRICS::default(); 1];
        let mut n = 0;
        unsafe {
            let _ = t.layout.GetLineMetrics(Some(&mut lm), &mut n);
        }
        lm[0].baseline
    }

    // ───────────── screens ─────────────

    fn welcome(&mut self, _x: f32) {
        let app = self.app;
        let (x, w) = self.hero(96.0, Badge::None);
        let mut y = self.kicker(x, CONTENT_Y, &self.t("welcome.kicker"), self.a);
        y = self.heading(H1, x, y, w, app.info.name());
        // chips
        y += 12.0;
        let cf = font(Family::Mono, 600, 10.5).track(0.12);
        let row = self.lh(cf) + 8.0;
        let a = self.a;
        let cw = self.chip(x, y, &format!("V{}", app.info.version()), a, a.alpha(0.07), a.alpha(0.35), cf, (3.0, 7.0), row);
        let bf = font(Family::Mono, 600, 8.5).track(0.14);
        self.chip(x + cw + 6.0, y, "BY GLORIA", hex(0x93b4ff), hex(0x141824), hex(0x252d44), bf, (2.0, 6.0), row);
        y += row;
        if let Some(d) = &app.info.m.app.description {
            y = self.para(P, x, y + 10.0, w, d, MUTED);
        }
        let folder = app.check.final_dir.to_string_lossy().into_owned();
        self.facts(x, y + 26.0, w, &[("location", folder, false), ("size", app.s.size(app.info.m.unpacked_size), false)]);
        self.footer(
            false,
            &[],
            &[(&self.t("welcome.install"), Kind::Primary, None, Act::Start, true, false)],
        );
    }

    fn progress(&mut self, _x: f32) {
        let app = self.app;
        let un = app.page == Page::UninstallProgress;
        let (x, w) = self.hero(64.0, Badge::None);
        let (kk, tk) = if un {
            ("unp.kicker", "unp.title")
        } else if app.flow == Flow::Update {
            ("update.kicker", "update.title")
        } else {
            ("progress.kicker", "progress.title")
        };
        let kc = if un { DANGER } else { self.a };
        let mut y = self.kicker(x, CONTENT_Y, &self.t(kk), kc);
        y = self.heading(H2, x, y, w, &self.t(tk));
        y += 26.0;
        let pct = app.status.pct.clamp(0.0, 100.0);
        let pf = font(Family::Mono, 500, 44.0).line(1.0).track(-0.02);
        let sf = font(Family::Mono, 400, 18.0);
        let num = format!("{}", pct.floor() as u32);
        let nw = self.p.label(pf, &num, x, y, TEXT_HI);
        let by = self.baseline(pf) - self.baseline(sf);
        self.p.label(sf, "%", x + nw + 3.0, y + by, DIM2);
        y += 44.0 + 18.0;
        // bar
        let a = self.a;
        self.p.fill(rect(x, y, w, 4.0), LINE);
        let fw = w * pct / 100.0;
        if fw > 0.5 {
            let fill = rect(x, y, fw, 4.0);
            self.p.glow(fill, 6.0, a.alpha(0.9));
            self.p.fill(fill, a);
            self.p.clip(fill);
            let sx = x - 60.0 + ((self.now / 1.1).fract()) * 660.0;
            self.p.linear(rect(sx, y, 60.0, 4.0), &[Stop(0.0, WHITE.alpha(0.0)), Stop(0.5, WHITE.alpha(0.55)), Stop(1.0, WHITE.alpha(0.0))]);
            self.p.unclip();
        }
        y += 4.0 + 10.0;
        let t = self.p.g.text(MONO, &app.status.file, w, Wrap::Ellipsis);
        self.p.text(&t, x, y, DIM2);
        y += self.lh(MONO) + 22.0;
        let keys: [&str; 4] =
            if un { ["step.close", "step.files", "step.data", "step.entry"] } else { ["step.prepare", "step.copy", "step.shortcuts", "step.register"] };
        let idx = if pct >= 100.0 {
            4
        } else if pct < 8.0 {
            0
        } else if pct < 78.0 {
            1
        } else if pct < 90.0 {
            2
        } else {
            3
        };
        let cw = (w - 24.0) / 2.0;
        let sf = font(Family::Sans, 400, 12.5);
        for (i, k) in keys.iter().enumerate() {
            let skip = un && !app.purge && *k == "step.data";
            let sx = x + (i % 2) as f32 * (cw + 24.0);
            let sy = y + (i / 2) as f32 * (16.0 + 9.0);
            let (done, act) = (!skip && i < idx, !skip && i == idx);
            if skip {
                self.p.push_opacity(0.5);
            }
            let (cx, cy) = (sx + 8.0, sy + 8.0);
            if done {
                self.p.circle(cx, cy, 8.0, a.alpha(0.15));
                self.p.ring(cx, cy, 8.0, 1.5, a);
                self.p.icon("check", cx - 5.0, cy - 5.0, 10.0, 3.5, a);
            } else if act {
                self.p.ring(cx, cy, 8.0, 1.5, a);
                let pulse = 0.675 + 0.325 * (self.now * std::f32::consts::TAU).cos();
                self.p.glow(rect(cx - 3.0, cy - 3.0, 6.0, 6.0), 4.0, a.alpha(pulse));
                self.p.circle(cx, cy, 3.0, a.alpha(pulse));
            } else {
                self.p.ring(cx, cy, 8.0, 1.5, hex(0x3a3a44));
            }
            let c = if done { MUTED } else if act { TEXT_HI } else { FAINT };
            let s = self.t(k);
            let t = self.p.g.text(sf, &s, cw - 26.0, Wrap::Ellipsis);
            let ty = sy + (16.0 - self.lh(sf)) / 2.0;
            self.p.text(&t, sx + 26.0, ty, c);
            if skip {
                self.p.fill(rect(sx + 26.0, ty + self.lh(sf) * 0.55, t.w, 1.0), c);
                self.p.pop_opacity();
            }
        }
        if un {
            self.footer(false, &[], &[]);
        } else {
            self.footer(false, &[], &[(&self.t("cancel"), Kind::Normal, None, Act::Cancel, true, false)]);
        }
    }

    fn done(&mut self, _x: f32) {
        let app = self.app;
        let (x, w) = self.hero(96.0, Badge::Ok);
        let mut y = self.kicker(x, CONTENT_Y, &self.t("done.kicker"), self.a);
        y = self.heading(H1, x, y, w, &self.t("done.title"));
        let body = self.t(if app.flow == Flow::Update { "done.updated" } else { "done.body" });
        y = self.para(P, x, y + 10.0, w, &body, MUTED);
        let folder = app.check.final_dir.to_string_lossy().into_owned();
        let y = self.facts(x, y + 26.0, w, &[("version", app.info.version().into(), false), ("location", folder, false)]);
        let y1 = self.checkbox(x, y + 18.0, w, &self.t("done.startMenu"), app.start_menu, Act::StartMenu, false, None);
        self.checkbox(x, y1 + 10.0, w, &self.t("done.desktop"), app.desktop, Act::Desktop, false, None);
        self.footer(
            true,
            &[],
            &[
                (&self.t("close"), Kind::Normal, None, Act::Close, true, false),
                (&self.t("done.launch"), Kind::Primary, Some("play"), Act::LaunchApp, true, false),
            ],
        );
    }

    fn update(&mut self, _x: f32) {
        let app = self.app;
        let (x, w) = self.hero(64.0, Badge::None);
        let mut y = self.kicker(x, CONTENT_Y, &self.t("update.kicker"), self.a);
        y = self.heading(H2, x, y, w, &self.t("update.title"));
        let from = app.installed.clone().unwrap_or_default();
        y = self.versions(x, y + 22.0, &from, app.info.version(), &self.t("update.new"), self.a, true, false);
        let folder = app.check.final_dir.to_string_lossy().into_owned();
        y = self.facts(x, y + 20.0, w, &[("data", self.t("update.kept"), true), ("location", folder, false)]);
        if app.running.is_some() {
            self.note(x, y + 14.0, w, &self.t("update.running"));
        }
        self.footer(
            false,
            &[],
            &[
                (&self.t("cancel"), Kind::Normal, None, Act::Cancel, true, false),
                (&self.t("update.btn"), Kind::Primary, Some("refresh"), Act::Start, true, false),
            ],
        );
    }

    fn note(&mut self, x: f32, y: f32, w: f32, s: &str) -> f32 {
        let f = font(Family::Sans, 400, 12.0).line(1.5);
        let t = self.p.g.text(f, s, w - 24.0 - 2.0 - 24.0, Wrap::Wrap);
        let h = t.h + 20.0 + 2.0;
        let r = rect(x, y, w, h);
        self.p.fill(r, WARN_BG);
        self.p.border(r, WARN_LINE);
        self.p.icon("alert", x + 13.0, y + 13.0, 14.0, 2.0, WARN);
        self.p.text(&t, x + 13.0 + 24.0, y + 11.0, hex(0xe9d8a0));
        y + h
    }

    fn reinstall(&mut self, _x: f32) {
        let app = self.app;
        let (x, w) = self.hero(64.0, Badge::None);
        let mut y = self.kicker(x, CONTENT_Y, &self.t("reinstall.kicker"), self.a);
        y = self.heading(H2, x, y, w, &self.t("reinstall.title"));
        y += 20.0;
        let cw = (w - 10.0) / 2.0;
        let bf = font(Family::Sans, 700, 13.0);
        let df = font(Family::Sans, 400, 11.5).line(1.4);
        let cards = [
            (true, self.t("reinstall.repair"), self.t("reinstall.repairDesc")),
            (false, self.t("reinstall.remove"), self.t("reinstall.removeDesc")),
        ];
        let inner = cw - 2.0 - 28.0 - 16.0 - 11.0;
        let h = cards
            .iter()
            .map(|(_, b, d)| self.p.g.text(bf, b, inner, Wrap::Wrap).h + 3.0 + self.p.g.text(df, d, inner, Wrap::Wrap).h)
            .fold(0.0f32, f32::max)
            + 26.0
            + 2.0;
        let a = self.a;
        for (i, (repair, b, d)) in cards.iter().enumerate() {
            let r = rect(x + i as f32 * (cw + 10.0), y, cw, h);
            let on = app.repair == *repair;
            let act = Act::Choice(*repair);
            let hov = self.inp.hover == Some(act);
            self.hits.push(Hit { r, act, enabled: true });
            self.p.fill(r, if on { a.alpha(0.06).over(PANEL) } else if hov { RAISE } else { PANEL });
            self.p.border(r, if on { a.alpha(0.6) } else { LINE });
            if on {
                self.p.fill(rect(r.x + 1.0, r.y + 1.0, r.w - 2.0, 2.0), a);
            }
            let (cx, cy) = (r.x + 15.0 + 8.0, r.y + 14.0 + 8.0);
            self.p.ring(cx, cy, 8.0, 1.5, if on { a } else { hex(0x3a3a44) });
            if on {
                self.p.glow(rect(cx - 4.0, cy - 4.0, 8.0, 8.0), 3.0, a);
                self.p.circle(cx, cy, 4.0, a);
            }
            let tx = r.x + 15.0 + 16.0 + 11.0;
            let t = self.p.g.text(bf, b, inner, Wrap::Wrap);
            self.p.text(&t, tx, r.y + 14.0, TEXT_HI);
            let dt = self.p.g.text(df, d, inner, Wrap::Wrap);
            self.p.text(&dt, tx, r.y + 14.0 + t.h + 3.0, DIM);
            if self.inp.focus == Some(act) {
                self.p.border(r.inset(-3.0), a.alpha(0.8));
            }
        }
        y += h;
        if app.repair {
            self.data_block(x, y + 14.0, w, "reinstall.clean", false);
        }
        self.footer(
            false,
            &[],
            &[
                (&self.t("cancel"), Kind::Normal, None, Act::Cancel, true, false),
                (&self.t("continue"), Kind::Primary, None, Act::Start, true, false),
            ],
        );
    }

    fn downgrade(&mut self, _x: f32) {
        let app = self.app;
        let (x, w) = self.hero(64.0, Badge::None);
        let mut y = self.kicker(x, CONTENT_Y, &self.t("down.kicker"), WARN);
        y = self.heading(H2, x, y, w, &self.t("down.title"));
        let from = app.installed.clone().unwrap_or_default();
        y = self.versions(x, y + 12.0, &from, app.info.version(), &self.t("version"), WARN, false, true);
        y = self.para(P.size(12.0), x, y + 8.0, w, &self.t("down.body"), MUTED);
        y = self.data_block(x, y + 10.0, w, "data.remove", true);
        self.checkbox(x, y + 10.0, w, &self.t("down.confirm"), app.confirm_down, Act::ConfirmDown, false, None);
        self.footer(
            false,
            &[],
            &[
                (&self.t("cancel"), Kind::Normal, None, Act::Cancel, true, false),
                (&self.t("down.btn"), Kind::Primary, None, Act::Start, app.confirm_down, false),
            ],
        );
    }

    fn migrate(&mut self, _x: f32) {
        let app = self.app;
        let (x, w) = self.hero(64.0, Badge::None);
        let mut y = self.kicker(x, CONTENT_Y, &self.t("mig.kicker"), self.a);
        y = self.heading(H2, x, y, w, &self.t("mig.title"));
        y += 12.0;
        let legacy = app.legacy.clone().unwrap_or(crate::app::Legacy {
            kind: "?".into(),
            scope: String::new(),
            path: String::new(),
            admin: false,
            version: String::new(),
        });
        let folder = app.check.final_dir.to_string_lossy().into_owned();
        let tf = font(Family::Sans, 400, 12.5);
        let sf = font(Family::Mono, 400, 10.0);
        let rows = [
            (self.t("mig.step1"), format!("{} · {} · {}", legacy.version, legacy.kind, self.t(&legacy.scope)), legacy.admin),
            (self.t("mig.step2"), folder, false),
        ];
        let rh = 16.0 + self.lh(tf) + 2.0 + self.lh(sf);
        let total = 2.0 + rows.len() as f32 * rh + (rows.len() - 1) as f32;
        self.p.border(rect(x, y, w, total), LINE);
        let mut ry = y + 1.0;
        for (i, (t, small, admin)) in rows.iter().enumerate() {
            if i > 0 {
                self.p.hline(x + 1.0, x + w - 1.0, ry, LINE);
                ry += 1.0;
            }
            let nb = rect(x + 13.0, ry + (rh - 20.0) / 2.0, 20.0, 20.0);
            self.p.border(nb, LINE2);
            let n = format!("{}", i + 1);
            let nf = font(Family::Mono, 400, 10.0);
            let nw = self.p.g.measure(nf, &n);
            self.p.label(nf, &n, nb.cx() - nw / 2.0, nb.cy() - self.lh(nf) / 2.0, DIM);
            let tx = nb.r() + 12.0;
            let mut chip_w = 0.0;
            if *admin {
                let cf = font(Family::Mono, 600, 10.5).track(0.12);
                let s = self.up("mig.admin");
                let tw = self.p.g.measure(cf, &s);
                chip_w = tw + 14.0 + 2.0 + 14.0 + 5.0;
                let ch = self.lh(cf) + 8.0;
                let cr = rect(x + w - 13.0 - chip_w, ry + (rh - ch) / 2.0, chip_w, ch);
                self.p.fill(cr, WARN_BG);
                self.p.border(cr, WARN_LINE);
                self.p.icon("upshield", cr.x + 8.0, cr.cy() - 7.0, 14.0, 2.0, WARN);
                self.p.label(cf, &s, cr.x + 8.0 + 19.0, cr.y + 4.0, WARN);
                chip_w += 12.0;
            }
            let tw = x + w - 13.0 - chip_w - tx;
            let tt = self.p.g.text(tf, t, tw, Wrap::Ellipsis);
            self.p.text(&tt, tx, ry + 8.0, SOFT);
            let st = self.p.g.text(sf, small, tw, Wrap::Ellipsis);
            self.p.text(&st, tx, ry + 8.0 + self.lh(tf) + 2.0, DIM2);
            ry += rh;
        }
        y += total;
        if legacy.admin {
            y = self.hint(x, y + 7.0, w, "info", DIM, DIM, &[(&self.t("mig.adminNote"), None)]);
        }
        self.data_block(x, y + 10.0, w, "data.remove", true);
        self.footer(
            false,
            &[],
            &[
                (&self.t("cancel"), Kind::Normal, None, Act::Cancel, true, false),
                (&self.t("mig.btn"), Kind::Primary, if legacy.admin { Some("upshield") } else { None }, Act::Start, true, false),
            ],
        );
    }

    fn uninstall(&mut self, _x: f32) {
        let app = self.app;
        let (x, w) = self.hero(64.0, Badge::None);
        let mut y = self.kicker(x, CONTENT_Y, &self.t("un.kicker"), DANGER);
        y = self.heading(H2, x, y, w, &self.t("un.title"));
        y = self.para(P, x, y + 10.0, w, &self.t("un.body"), MUTED);
        let folder = app.check.final_dir.to_string_lossy().into_owned();
        y = self.facts(x, y + 16.0, w, &[("location", folder, false)]);
        y = self.data_block(x, y + 12.0, w, "data.remove", true);
        // "Remove everything": lined up with the data checkbox above, with what it adds in a dim line under it.
        let (cx, cw) = (x + 13.0, w - 13.0);
        y = self.checkbox(cx, y + 10.0, cw, &self.t("un.wipe"), app.wipe, Act::Wipe, true, None);
        let hf = font(Family::Sans, 400, 11.5).line(1.4);
        let t = self.p.g.text(hf, &self.t("un.wipeHint"), cw - 26.0, Wrap::Wrap);
        self.p.text(&t, cx + 26.0, y + 2.0, if app.wipe { DANGER_SOFT } else { DIM });
        self.footer(
            false,
            &[],
            &[
                (&self.t("cancel"), Kind::Normal, None, Act::Cancel, true, false),
                (&self.t("un.btn"), Kind::Danger, Some("trash"), Act::Start, true, false),
            ],
        );
    }

    fn uninstall_done(&mut self, _x: f32) {
        let app = self.app;
        let (x, w) = self.hero(96.0, Badge::Gray);
        let mut y = self.kicker(x, CONTENT_Y, &self.t("und.kicker"), self.a);
        y = self.heading(H1.size(28.0), x, y, w, &self.t("und.title"));
        y = self.para(P, x, y + 10.0, w, &self.t(if app.wipe { "und.wiped" } else if app.purge { "und.purged" } else { "und.kept" }), MUTED);
        if !app.purge && !app.info.data.is_empty() {
            let d = &app.info.data;
            let mut s = d[0].0.clone();
            if d.len() > 1 {
                s += &format!(" +{}", d.len() - 1);
            }
            self.facts(x, y + 26.0, w, &[("data", s, true)]);
        }
        self.footer(true, &[], &[(&self.t("close"), Kind::Primary, None, Act::Close, true, false)]);
    }

    fn error(&mut self, _x: f32) {
        let app = self.app;
        let (x, w) = self.hero(64.0, Badge::Err);
        let un = app.flow == crate::app::Flow::Uninstall;
        let mut y = self.kicker(x, CONTENT_Y, &self.t(if un { "unerr.kicker" } else { "err.kicker" }), DANGER);
        y = self.heading(H2, x, y, w, &self.t(if un { "unerr.title" } else { "err.title" }));
        let e = app.error.clone().unwrap_or(crate::app::ErrorInfo { code: 1, reason: String::new(), log: String::new() });
        let body = match (e.code, un) {
            (3, _) => "err.body",
            (_, true) => "err.bodyUninstall",
            _ => "err.bodyGeneric",
        };
        y = self.para(P, x, y + 10.0, w, &self.t(body), MUTED);
        // Code box.
        y += 16.0;
        let reason = if e.reason.contains('.') && !e.reason.contains(' ') { self.t(&e.reason) } else { e.reason.clone() };
        let rows = [("err.code", e.code.to_string(), false), ("err.reason", reason, false), ("err.log", e.log.clone(), true)];
        let kf = font(Family::Mono, 400, 9.5).track(0.12);
        let vf = MONO;
        let rh = 14.0 + self.lh(vf);
        let total = 2.0 + 3.0 * rh + 2.0;
        let r = rect(x, y, w, total);
        self.p.fill(r, PANEL);
        self.p.border(r, LINE);
        let mut ry = y + 1.0;
        for (i, (k, v, link)) in rows.iter().enumerate() {
            if i > 0 {
                self.p.hline(x + 1.0, x + w - 1.0, ry, LINE);
                ry += 1.0;
            }
            let ky = ry + 7.0 + (self.lh(vf) - self.lh(kf)) / 2.0;
            self.p.label(kf, &self.up(k), x + 13.0, ky, DIM2);
            let vx = x + 13.0 + 92.0;
            let vw = x + w - 13.0 - vx;
            if *link {
                let open = self.t("err.open");
                let sep = " · ";
                let ow = self.p.g.measure(vf, &open) + self.p.g.measure(vf, sep);
                let t = self.p.g.text(vf, v, vw - ow, Wrap::Ellipsis);
                self.p.text(&t, vx, ry + 7.0, SOFT);
                let sx = vx + t.w;
                let sw = self.p.label(vf, sep, sx, ry + 7.0, SOFT);
                let hov = self.inp.hover == Some(Act::OpenLog);
                let lw = self.p.label(vf, &open, sx + sw, ry + 7.0, if hov { self.a.brighten(1.2) } else { self.a });
                self.hits.push(Hit { r: rect(sx + sw, ry + 4.0, lw, self.lh(vf) + 6.0), act: Act::OpenLog, enabled: true });
            } else {
                let t = self.p.g.text(vf, v, vw, Wrap::Ellipsis);
                self.p.text(&t, vx, ry + 7.0, SOFT);
            }
            ry += rh;
        }
        self.footer(
            false,
            &[(&self.t("err.log"), Kind::Ghost, Some("file"), Act::OpenLog, true)],
            &[
                (&self.t("close"), Kind::Normal, None, Act::Close, true, false),
                // A refused folder (6) fails the same way again, so there is nothing to retry.
                (&self.t("err.retry"), Kind::Primary, Some("refresh"), Act::Start, e.code != gloria_core::exit::Exit::InvalidArgs.code(), false),
            ],
        );
    }

    // ───────────── modal ─────────────

    fn modal(&mut self, m: Modal) {
        let app = self.app;
        let w = 470.0;
        let iw = w - 2.0 - 44.0;
        let h2 = H2.size(19.0);
        let body_key = match m {
            Modal::Ask => "running.body",
            Modal::Waiting => "running.waiting",
            Modal::Stuck => "running.stuck",
        };
        let body = self.t(body_key);
        let title = self.t("running.title");
        let tt = self.p.g.text(h2, &title, iw, Wrap::Wrap);
        let pt = self.p.g.text(P, &body, iw, Wrap::Wrap);
        let proc_h = 2.0 + 16.0 + self.lh(MONO);
        let h = 2.0 + 20.0 + self.lh(KICKER) + 8.0 + tt.h + 8.0 + pt.h + 12.0 + proc_h + 4.0 + 16.0 + 36.0 + 18.0;
        let bx = 1.0 + (W - 2.0 - w) / 2.0;
        let by = BODY_Y + (H - 1.0 - BODY_Y - h) / 2.0;
        let r = rect(bx.round(), by.round(), w, h.round());
        self.p.glow(r.offset(0.0, 20.0), 25.0, hex(0).alpha(0.6));
        self.p.fill(r, PANEL2);
        self.p.border(r, LINE2);
        self.p.glow(rect(r.x, r.y - 1.0, r.w, 2.0), 5.0, WARN.alpha(0.6));
        self.p.fill(rect(r.x, r.y - 1.0, r.w, 2.0), WARN);
        let x = r.x + 1.0 + 22.0;
        let mut y = r.y + 1.0 + 20.0;
        y = self.kicker(x, y, app.info.name(), WARN);
        self.p.text(&tt, x, y + 8.0, TEXT_HI);
        y += 8.0 + tt.h + 8.0;
        self.p.text(&pt, x, y, MUTED);
        y += pt.h + 12.0;
        let pr = rect(x, y, iw, proc_h);
        self.p.fill(pr, BG);
        self.p.border(pr, LINE);
        let (dc, pulse) = match m {
            Modal::Ask => (hex(0x4ade80), 1.0),
            _ => (WARN, 0.675 + 0.325 * (self.now * std::f32::consts::TAU).cos()),
        };
        let dy = pr.cy();
        self.p.glow(rect(x + 11.0, dy - 3.0, 6.0, 6.0), 3.0, dc.alpha(pulse));
        self.p.circle(x + 14.0, dy, 3.0, dc.alpha(pulse));
        let pid = app.running.as_ref().map(|r| format!("{} · PID {}", r.path, r.pid)).unwrap_or_default();
        let t = self.p.g.text(MONO, &pid, iw - 2.0 - 20.0 - 14.0, Wrap::Ellipsis);
        self.p.text(&t, x + 11.0 + 14.0, dy - self.lh(MONO) / 2.0, DIM);
        y += proc_h + 4.0 + 16.0;

        // Footer buttons, right-aligned.
        let cancel = self.t("cancel");
        let (label2, kind2, act2, en2, spin) = match m {
            Modal::Ask => (self.t("running.closeBtn"), Kind::Primary, Act::Modal(Some(Modal::Waiting)), true, false),
            Modal::Waiting => (self.t("running.closeBtn"), Kind::Primary, Act::Noop, false, true),
            Modal::Stuck => (self.t("running.force"), Kind::Danger, Act::Force, true, false),
        };
        let w2 = self.btn_w(&app.s.upper(&label2), kind2, if spin { Some("x") } else { None });
        let w1 = self.btn_w(&app.s.upper(&cancel), Kind::Normal, None);
        let rx = r.r() - 1.0 - 22.0;
        self.hits.clear();
        self.hits.push(Hit { r: CLOSE_BTN, act: Act::Close, enabled: true });
        self.button(rx - w2 - 8.0 - w1, y, &cancel, Kind::Normal, None, Act::Modal(None), true, false);
        if spin {
            // Disabled but drawn with the spinner at full strength inside the dimmed button.
            self.button(rx - w2, y, &label2, kind2, None, act2, en2, true);
        } else {
            self.button(rx - w2, y, &label2, kind2, None, act2, en2, false);
        }
    }
}

impl App {
    pub fn m_test_build(&self) -> bool {
        self.info.m.test_build
    }
}
