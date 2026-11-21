//! Drawing layer: Direct2D + DirectWrite + WIC.
//!
//! Everything draws on an `ID2D1DeviceContext` in DIPs, either a window target (hardware, or software with
//! `--software`) or a WIC bitmap for `--screenshot`. Glows are CPU-made Gaussian-blurred rectangles cached as bitmaps,
//! so they look the same on every target. Fonts are embedded and loaded from memory.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::{w, Interface, Result, HSTRING, PCWSTR};
use windows::Win32::Foundation::GENERIC_WRITE;
use windows_core::BOOL;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows_numerics::{Matrix3x2, Vector2};

// ───────────────────────────── colors & geometry ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

pub const fn hex(v: u32) -> Color {
    Color {
        r: ((v >> 16) & 0xff) as f32 / 255.0,
        g: ((v >> 8) & 0xff) as f32 / 255.0,
        b: (v & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

impl Color {
    pub fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
    }
    /// Same color with alpha multiplied by `a`.
    pub fn alpha(self, a: f32) -> Color {
        Color { a: self.a * a, ..self }
    }
    pub fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.r, g: self.g, b: self.b, a: self.a }
    }
    /// Composites `self` over an opaque `bg` and returns an opaque color.
    pub fn over(self, bg: Color) -> Color {
        let a = self.a;
        Color { r: self.r * a + bg.r * (1.0 - a), g: self.g * a + bg.g * (1.0 - a), b: self.b * a + bg.b * (1.0 - a), a: 1.0 }
    }
    pub fn brighten(self, k: f32) -> Color {
        Color { r: (self.r * k).min(1.0), g: (self.g * k).min(1.0), b: (self.b * k).min(1.0), a: self.a }
    }
    fn key(self) -> u32 {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        (q(self.r) << 24) | (q(self.g) << 16) | (q(self.b) << 8) | q(self.a)
    }
    /// WCAG relative luminance.
    pub fn luminance(self) -> f32 {
        let f = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * f(self.r) + 0.7152 * f(self.g) + 0.0722 * f(self.b)
    }
    /// Text color for something drawn on this color: white or near-black, whichever contrasts more.
    pub fn on(self) -> Color {
        let l = self.luminance();
        if 1.05 / (l + 0.05) > (l + 0.05) / 0.05 {
            hex(0xffffff)
        } else {
            hex(0x0b0b0c)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub const fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

impl Rect {
    pub fn r(&self) -> f32 {
        self.x + self.w
    }
    pub fn b(&self) -> f32 {
        self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    pub fn inset(&self, d: f32) -> Rect {
        Rect { x: self.x + d, y: self.y + d, w: self.w - 2.0 * d, h: self.h - 2.0 * d }
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.r() && y >= self.y && y < self.b()
    }
    pub fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.r(), bottom: self.b() }
    }
    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, ..*self }
    }
}

fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

// ───────────────────────────── fonts ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Display,
    Sans,
    Mono,
}

impl Family {
    fn name(self) -> PCWSTR {
        match self {
            Family::Display => w!("Archivo"),
            Family::Sans => w!("Inter"),
            Family::Mono => w!("JetBrains Mono"),
        }
    }
}

/// A text style: family, weight, size in DIPs, letter spacing in em, and line height as a multiple of the size
/// (None = the font's "normal" line height, like CSS).
#[derive(Clone, Copy, Debug)]
pub struct Font {
    pub family: Family,
    pub weight: u16,
    pub size: f32,
    pub track: f32,
    pub line: Option<f32>,
}

pub const fn font(family: Family, weight: u16, size: f32) -> Font {
    Font { family, weight, size, track: 0.0, line: None }
}

impl Font {
    pub const fn track(self, em: f32) -> Font {
        Font { track: em, ..self }
    }
    pub const fn line(self, k: f32) -> Font {
        Font { line: Some(k), ..self }
    }
    pub const fn size(self, size: f32) -> Font {
        Font { size, ..self }
    }
}

const FONT_FILES: [&[u8]; 11] = [
    include_bytes!("../../../assets/fonts/Archivo-600.ttf"),
    include_bytes!("../../../assets/fonts/Archivo-700.ttf"),
    include_bytes!("../../../assets/fonts/Archivo-800.ttf"),
    include_bytes!("../../../assets/fonts/Inter-400.ttf"),
    include_bytes!("../../../assets/fonts/Inter-500.ttf"),
    include_bytes!("../../../assets/fonts/Inter-600.ttf"),
    include_bytes!("../../../assets/fonts/Inter-700.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-400.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-500.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-600.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-700.ttf"),
];

/// The mono font file, also loaded into GDI for the native path field.
pub const MONO_TTF: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-400.ttf");

#[derive(Clone, Copy, Debug, Default)]
struct Metrics {
    asc: f32,
    desc: f32,
    gap: f32,
}

pub enum Wrap {
    /// One line, no clipping.
    None,
    /// Wraps at `max_w`.
    Wrap,
    /// One line, cut with "…" at `max_w`.
    Ellipsis,
}

pub struct Text {
    pub layout: IDWriteTextLayout,
    pub w: f32,
    pub h: f32,
}

// ───────────────────────────── Gfx ─────────────────────────────

pub struct Gfx {
    pub d2d: ID2D1Factory1,
    pub dw: IDWriteFactory5,
    pub wic: IWICImagingFactory,
    fonts: IDWriteFontCollection1,
    _loader: IDWriteInMemoryFontFileLoader,
    metrics: [Metrics; 3],
    locale: HSTRING,
    formats: RefCell<HashMap<(Family, u16, u32), IDWriteTextFormat>>,
    icons: RefCell<HashMap<&'static str, ID2D1Geometry>>,
    pub round: ID2D1StrokeStyle,
}

impl Gfx {
    pub fn new(lang: &str) -> Result<Gfx> {
        unsafe {
            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;

            let loader = dw.CreateInMemoryFontFileLoader()?;
            dw.RegisterFontFileLoader(&loader)?;
            let builder = dw.CreateFontSetBuilder()?;
            for data in FONT_FILES {
                let file = loader.CreateInMemoryFontFileReference(&dw, data.as_ptr() as _, data.len() as u32, None)?;
                builder.AddFontFile(&file)?;
            }
            let set = builder.CreateFontSet()?;
            // Typographic families group "Archivo ExtraBold" with "Archivo" (name ID 16), like CSS does.
            let fonts: IDWriteFontCollection1 = match dw.cast::<IDWriteFactory6>() {
                Ok(f6) => f6.CreateFontCollectionFromFontSet(&set, DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC)?.cast()?,
                Err(_) => dw.CreateFontCollectionFromFontSet(&set)?,
            };

            let mut metrics = [Metrics::default(); 3];
            for (i, fam) in [Family::Display, Family::Sans, Family::Mono].into_iter().enumerate() {
                let mut idx = 0;
                let mut exists = BOOL(0);
                fonts.FindFamilyName(fam.name(), &mut idx, &mut exists)?;
                if exists.as_bool() {
                    let f = fonts.GetFontFamily(idx)?.GetFirstMatchingFont(
                        DWRITE_FONT_WEIGHT_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        DWRITE_FONT_STYLE_NORMAL,
                    )?;
                    let mut m = DWRITE_FONT_METRICS::default();
                    f.GetMetrics(&mut m);
                    let u = m.designUnitsPerEm as f32;
                    metrics[i] = Metrics { asc: m.ascent as f32 / u, desc: m.descent as f32 / u, gap: m.lineGap as f32 / u };
                } else {
                    metrics[i] = Metrics { asc: 0.95, desc: 0.25, gap: 0.0 };
                }
            }

            let round = ID2D1Factory::CreateStrokeStyle(&d2d,
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?;

            let locale = HSTRING::from(match lang {
                "fr" => "fr-fr",
                "es" => "es-es",
                "tr" => "tr-tr",
                _ => "en-us",
            });
            Ok(Gfx {
                d2d: d2d.cast()?,
                dw,
                wic,
                fonts,
                _loader: loader,
                metrics,
                locale,
                formats: RefCell::new(HashMap::new()),
                icons: RefCell::new(HashMap::new()),
                round: round.cast()?,
            })
        }
    }

    /// True when all three embedded families resolved (checked by a test).
    #[cfg(test)]
    pub fn has_fonts(&self) -> bool {
        [Family::Display, Family::Sans, Family::Mono].iter().all(|f| unsafe {
            let mut i = 0;
            let mut e = BOOL(0);
            self.fonts.FindFamilyName(f.name(), &mut i, &mut e).is_ok() && e.as_bool()
        })
    }

    fn format(&self, f: Font) -> Result<IDWriteTextFormat> {
        let key = (f.family, f.weight, f.size.to_bits());
        if let Some(x) = self.formats.borrow().get(&key) {
            return Ok(x.clone());
        }
        let fmt = unsafe {
            self.dw.CreateTextFormat(
                f.family.name(),
                &self.fonts,
                DWRITE_FONT_WEIGHT(f.weight as i32),
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                f.size,
                &self.locale,
            )?
        };
        self.formats.borrow_mut().insert(key, fmt.clone());
        Ok(fmt)
    }

    /// CSS line height (DIPs) of a style.
    pub fn line_height(&self, f: Font) -> f32 {
        let m = self.metrics[f.family as usize];
        f.line.map(|k| k * f.size).unwrap_or((m.asc + m.desc + m.gap) * f.size)
    }

    pub fn text(&self, f: Font, s: &str, max_w: f32, wrap: Wrap) -> Text {
        self.try_text(f, s, max_w, wrap).expect("text layout")
    }

    fn try_text(&self, f: Font, s: &str, max_w: f32, wrap: Wrap) -> Result<Text> {
        unsafe {
            let fmt = self.format(f)?;
            let wide: Vec<u16> = s.encode_utf16().collect();
            let mw = match wrap {
                Wrap::None => 100_000.0,
                _ => max_w.max(1.0),
            };
            let layout = self.dw.CreateTextLayout(&wide, &fmt, mw, 10_000.0)?;
            let m = self.metrics[f.family as usize];
            let lh = self.line_height(f);
            let baseline = (lh - (m.asc + m.desc) * f.size) / 2.0 + m.asc * f.size;
            layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, lh, baseline)?;
            let all = DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 };
            if f.track != 0.0 {
                if let Ok(l1) = layout.cast::<IDWriteTextLayout1>() {
                    l1.SetCharacterSpacing(0.0, f.track * f.size, 0.0, all)?;
                }
            }
            match wrap {
                Wrap::Wrap => layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?,
                Wrap::None => layout.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?,
                Wrap::Ellipsis => {
                    layout.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                    let sign = self.dw.CreateEllipsisTrimmingSign(&layout)?;
                    let t = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                    layout.SetTrimming(&t, &sign)?;
                }
            }
            let mut tm = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut tm)?;
            let w = match wrap {
                Wrap::Ellipsis => tm.widthIncludingTrailingWhitespace.min(mw),
                _ => tm.widthIncludingTrailingWhitespace,
            };
            Ok(Text { layout, w, h: tm.height.max(lh) })
        }
    }

    /// Width of a single line.
    pub fn measure(&self, f: Font, s: &str) -> f32 {
        self.text(f, s, 0.0, Wrap::None).w
    }

    pub fn icon(&self, name: &'static str) -> ID2D1Geometry {
        if let Some(g) = self.icons.borrow().get(name) {
            return g.clone();
        }
        let d = ICONS.iter().find(|(n, _)| *n == name).map(|(_, d)| *d).unwrap_or(ICONS[0].1);
        let g = svg_path(&self.d2d, d).expect("icon path");
        self.icons.borrow_mut().insert(name, g.clone());
        g
    }
}

// ───────────────────────────── icons (Lucide, 24×24, stroke) ─────────────────────────────

const ICONS: &[(&str, &str)] = &[
    ("check", "M20 6 9 17l-5-5"),
    ("x", "M18 6 6 18M6 6l12 12"),
    ("minus", "M5 12h14"),
    ("sliders", "M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6"),
    ("arrow", "M5 12h14M13 6l6 6-6 6"),
    ("external", "M7 7h10v10M7 17 17 7"),
    (
        "folder",
        "M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z",
    ),
    (
        "shield",
        "M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1zM9 12l2 2 4-4",
    ),
    ("alert", "m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3M12 9v4M12 17h.01"),
    ("info", "M2 12a10 10 0 1 0 20 0a10 10 0 1 0-20 0M12 16v-4M12 8h.01"),
    ("play", "m6 3 14 9-14 9V3z"),
    ("trash", "M3 6h18M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"),
    (
        "refresh",
        "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8M21 3v5h-5M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16M8 16H3v5",
    ),
    ("file", "M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7ZM14 2v4a2 2 0 0 0 2 2h4"),
    (
        "upshield",
        "M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z",
    ),
];

/// Parses the SVG path subset Lucide uses (M L H V C A Z, absolute and relative) into a Direct2D path.
fn svg_path(f: &ID2D1Factory1, d: &str) -> Result<ID2D1Geometry> {
    let toks = tokenize(d);
    unsafe {
        let path = f.CreatePathGeometry()?;
        let sink = path.Open()?;
        let (mut cx, mut cy) = (0.0f32, 0.0f32);
        let (mut sx, mut sy) = (0.0f32, 0.0f32);
        let mut open = false;
        let mut i = 0;
        let mut cmd = 'M';
        let num = |i: &mut usize| -> f32 {
            let v = match toks.get(*i) {
                Some(Tok::N(v)) => *v,
                _ => 0.0,
            };
            *i += 1;
            v
        };
        while i < toks.len() {
            if let Tok::C(c) = toks[i] {
                cmd = c;
                i += 1;
                if c == 'z' || c == 'Z' {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                    cx = sx;
                    cy = sy;
                    continue;
                }
            }
            let rel = cmd.is_ascii_lowercase();
            let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
            match cmd.to_ascii_uppercase() {
                'M' => {
                    let (x, y) = (num(&mut i) + ox, num(&mut i) + oy);
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }
                    sink.BeginFigure(v2(x, y), D2D1_FIGURE_BEGIN_HOLLOW);
                    open = true;
                    (cx, cy, sx, sy) = (x, y, x, y);
                    cmd = if rel { 'l' } else { 'L' };
                }
                'L' => {
                    let (x, y) = (num(&mut i) + ox, num(&mut i) + oy);
                    sink.AddLine(v2(x, y));
                    (cx, cy) = (x, y);
                }
                'H' => {
                    let x = num(&mut i) + ox;
                    sink.AddLine(v2(x, cy));
                    cx = x;
                }
                'V' => {
                    let y = num(&mut i) + oy;
                    sink.AddLine(v2(cx, y));
                    cy = y;
                }
                'C' => {
                    let p1 = v2(num(&mut i) + ox, num(&mut i) + oy);
                    let p2 = v2(num(&mut i) + ox, num(&mut i) + oy);
                    let p3 = v2(num(&mut i) + ox, num(&mut i) + oy);
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: p1, point2: p2, point3: p3 });
                    (cx, cy) = (p3.X, p3.Y);
                }
                'A' => {
                    let rx = num(&mut i);
                    let ry = num(&mut i);
                    let rot = num(&mut i);
                    let large = num(&mut i) != 0.0;
                    let sweep = num(&mut i) != 0.0;
                    let (x, y) = (num(&mut i) + ox, num(&mut i) + oy);
                    sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: v2(x, y),
                        size: D2D_SIZE_F { width: rx, height: ry },
                        rotationAngle: rot,
                        sweepDirection: if sweep { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                        arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                    });
                    (cx, cy) = (x, y);
                }
                _ => i += 1,
            }
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
        sink.Close()?;
        path.cast()
    }
}

#[derive(Debug, PartialEq)]
enum Tok {
    C(char),
    N(f32),
}

fn tokenize(d: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let b = d.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_alphabetic() {
            out.push(Tok::C(c));
            i += 1;
        } else if c == '-' || c == '.' || c.is_ascii_digit() {
            let start = i;
            i += 1;
            let mut dot = c == '.';
            while i < b.len() {
                let ch = b[i] as char;
                if ch.is_ascii_digit() {
                    i += 1;
                } else if ch == '.' && !dot {
                    dot = true;
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Tok::N(d[start..i].parse().unwrap_or(0.0)));
        } else {
            i += 1;
        }
    }
    out
}

// ───────────────────────────── images ─────────────────────────────

/// Straight-alpha BGRA pixels.
#[derive(Clone)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
    pub id: u32,
}

static NEXT_IMAGE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

impl Image {
    pub fn new(w: u32, h: u32, px: Vec<u8>) -> Image {
        Image { w, h, px, id: NEXT_IMAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) }
    }

    /// Decodes PNG/ICO/JPEG/BMP bytes; for multi-frame files (ICO) takes the largest frame.
    pub fn decode(wic: &IWICImagingFactory, bytes: &[u8]) -> Result<Image> {
        unsafe {
            let stream = wic.CreateStream()?;
            stream.InitializeFromMemory(bytes)?;
            let dec = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
            let n = dec.GetFrameCount()?;
            let mut best = (0u32, 0u32);
            for i in 0..n {
                let f = dec.GetFrame(i)?;
                let (mut w, mut h) = (0, 0);
                f.GetSize(&mut w, &mut h)?;
                if w * h > best.1 {
                    best = (i, w * h);
                }
            }
            let frame = dec.GetFrame(best.0)?;
            Self::from_source(wic, &frame.cast()?)
        }
    }

    fn from_source(_wic: &IWICImagingFactory, src: &IWICBitmapSource) -> Result<Image> {
        unsafe {
            let conv = WICConvertBitmapSource(&GUID_WICPixelFormat32bppBGRA, src)?;
            let (mut w, mut h) = (0, 0);
            conv.GetSize(&mut w, &mut h)?;
            let mut px = vec![0u8; (w * h * 4) as usize];
            conv.CopyPixels(std::ptr::null(), w * 4, &mut px)?;
            Ok(Image::new(w, h, px))
        }
    }

    /// High-quality resize (for window icons).
    pub fn resized(&self, wic: &IWICImagingFactory, size: u32) -> Result<Image> {
        unsafe {
            let bmp = wic.CreateBitmapFromMemory(self.w, self.h, &GUID_WICPixelFormat32bppBGRA, self.w * 4, &self.px)?;
            let sc = wic.CreateBitmapScaler()?;
            sc.Initialize(&bmp, size, size, WICBitmapInterpolationModeHighQualityCubic)?;
            Self::from_source(wic, &sc.cast()?)
        }
    }

    /// Grayscale at 60% brightness (CSS `grayscale(1) brightness(.6)`).
    pub fn gray(&self) -> Image {
        let mut px = self.px.clone();
        for p in px.chunks_exact_mut(4) {
            let l = 0.0722 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.2126 * p[2] as f32;
            let v = (l * 0.6).round().min(255.0) as u8;
            p[0] = v;
            p[1] = v;
            p[2] = v;
        }
        Image::new(self.w, self.h, px)
    }

    fn premultiplied(&self) -> Vec<u8> {
        let mut px = self.px.clone();
        for p in px.chunks_exact_mut(4) {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
        px
    }
}

/// Saves a WIC bitmap as PNG.
pub fn save_png(wic: &IWICImagingFactory, bmp: &IWICBitmap, path: &std::path::Path) -> Result<()> {
    unsafe {
        let stream = wic.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0)?;
        let enc = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        enc.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        enc.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.unwrap();
        frame.Initialize(None)?;
        let (mut w, mut h) = (0, 0);
        bmp.GetSize(&mut w, &mut h)?;
        frame.SetSize(w, h)?;
        let mut fmt = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut fmt)?;
        frame.WriteSource(&bmp.cast::<IWICBitmapSource>()?, std::ptr::null())?;
        frame.Commit()?;
        enc.Commit()?;
        Ok(())
    }
}

// ───────────────────────────── glow ─────────────────────────────

fn erf(x: f32) -> f32 {
    // Abramowitz & Stegun 7.1.26, |error| < 1.5e-7.
    let s = x.signum();
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * (-x * x).exp();
    s * y
}

/// Coverage of a Gaussian-blurred [a, b] interval at sample `x` (all in pixels).
fn blur_1d(x: f32, a: f32, b: f32, sigma: f32) -> f32 {
    if sigma <= 0.01 {
        return if x >= a && x < b { 1.0 } else { 0.0 };
    }
    let k = 1.0 / (sigma * std::f32::consts::SQRT_2);
    0.5 * (erf((x - a) * k) - erf((x - b) * k))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlowKey {
    w: u32,
    h: u32,
    sigma: u32,
    scale: u32,
    color: u32,
}

/// Device-dependent resources of one render target.
#[derive(Default)]
pub struct Res {
    glows: HashMap<GlowKey, ID2D1Bitmap1>,
    images: HashMap<u32, ID2D1Bitmap1>,
    effect: Option<Option<ID2D1Effect>>,
}

// ───────────────────────────── Painter ─────────────────────────────

pub struct Painter<'a> {
    pub g: &'a Gfx,
    pub dc: ID2D1DeviceContext,
    pub res: &'a mut Res,
    pub scale: f32,
    brush: ID2D1SolidColorBrush,
}

pub struct Stop(pub f32, pub Color);

impl<'a> Painter<'a> {
    pub fn new(g: &'a Gfx, dc: ID2D1DeviceContext, res: &'a mut Res, scale: f32) -> Result<Painter<'a>> {
        let brush = unsafe { dc.CreateSolidColorBrush(&hex(0).d2d(), None)? };
        unsafe {
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        }
        Ok(Painter { g, dc, res, scale, brush })
    }

    fn brush(&self, c: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&c.d2d()) };
        &self.brush
    }

    /// Rounds a DIP coordinate to the device pixel grid.
    pub fn snap(&self, v: f32) -> f32 {
        (v * self.scale).round() / self.scale
    }

    fn snapped(&self, r: Rect) -> D2D_RECT_F {
        D2D_RECT_F { left: self.snap(r.x), top: self.snap(r.y), right: self.snap(r.r()), bottom: self.snap(r.b()) }
    }

    pub fn clear(&self, c: Color) {
        unsafe { self.dc.Clear(Some(&c.d2d())) }
    }

    pub fn fill(&self, r: Rect, c: Color) {
        if c.a <= 0.0 {
            return;
        }
        unsafe { self.dc.FillRectangle(&self.snapped(r), self.brush(c)) }
    }

    /// A 1-DIP border drawn inside `r` (CSS box-sizing: border-box).
    pub fn border(&self, r: Rect, c: Color) {
        if c.a <= 0.0 {
            return;
        }
        let t = (self.scale.round().max(1.0)) / self.scale;
        let (l, tp, rr, b) = (self.snap(r.x), self.snap(r.y), self.snap(r.r()), self.snap(r.b()));
        let br = self.brush(c);
        unsafe {
            self.dc.FillRectangle(&D2D_RECT_F { left: l, top: tp, right: rr, bottom: tp + t }, br);
            self.dc.FillRectangle(&D2D_RECT_F { left: l, top: b - t, right: rr, bottom: b }, br);
            self.dc.FillRectangle(&D2D_RECT_F { left: l, top: tp + t, right: l + t, bottom: b - t }, br);
            self.dc.FillRectangle(&D2D_RECT_F { left: rr - t, top: tp + t, right: rr, bottom: b - t }, br);
        }
    }

    /// Horizontal 1-DIP line from x0 to x1 whose top is at y.
    pub fn hline(&self, x0: f32, x1: f32, y: f32, c: Color) {
        let t = (self.scale.round().max(1.0)) / self.scale;
        let y = self.snap(y);
        unsafe {
            self.dc.FillRectangle(&D2D_RECT_F { left: self.snap(x0), top: y, right: self.snap(x1), bottom: y + t }, self.brush(c))
        }
    }

    pub fn vline(&self, x: f32, y0: f32, y1: f32, c: Color) {
        let t = (self.scale.round().max(1.0)) / self.scale;
        let x = self.snap(x);
        unsafe {
            self.dc.FillRectangle(&D2D_RECT_F { left: x, top: self.snap(y0), right: x + t, bottom: self.snap(y1) }, self.brush(c))
        }
    }

    pub fn circle(&self, cx: f32, cy: f32, r: f32, c: Color) {
        let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: r, radiusY: r };
        unsafe { self.dc.FillEllipse(&e, self.brush(c)) }
    }

    /// Circle outline; `r` is the outer radius, the stroke is drawn inside it.
    pub fn ring(&self, cx: f32, cy: f32, r: f32, width: f32, c: Color) {
        let rr = r - width / 2.0;
        let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: rr, radiusY: rr };
        unsafe { self.dc.DrawEllipse(&e, self.brush(c), width, None) }
    }

    pub fn text(&self, t: &Text, x: f32, y: f32, c: Color) {
        unsafe {
            self.dc.DrawTextLayout(v2(x, y), &t.layout, self.brush(c), D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
    }

    /// Lays out and draws one line; returns its width.
    pub fn label(&self, f: Font, s: &str, x: f32, y: f32, c: Color) -> f32 {
        let t = self.g.text(f, s, 0.0, Wrap::None);
        self.text(&t, x, y, c);
        t.w
    }

    /// Draws a Lucide icon whose 24×24 box is placed at (x, y) with the given size; `stroke` is in icon units.
    pub fn icon(&self, name: &'static str, x: f32, y: f32, size: f32, stroke: f32, c: Color) {
        let g = self.g.icon(name);
        let k = size / 24.0;
        unsafe {
            let mut old = Matrix3x2::default();
            self.dc.GetTransform(&mut old);
            let m = Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: x, M32: y } * old;
            self.dc.SetTransform(&m);
            self.dc.DrawGeometry(&g, self.brush(c), stroke, &self.g.round);
            self.dc.SetTransform(&old);
        }
    }

    /// Spinner: a 3/4 ring rotating with `t` seconds.
    pub fn spinner(&self, cx: f32, cy: f32, r: f32, width: f32, t: f32, c: Color) {
        let rr = r - width / 2.0;
        let a0 = (t / 0.8).fract() * std::f32::consts::TAU;
        let a1 = a0 + std::f32::consts::TAU * 0.75;
        unsafe {
            if let Ok(path) = self.g.d2d.CreatePathGeometry() {
                if let Ok(sink) = path.Open() {
                    sink.BeginFigure(v2(cx + rr * a0.cos(), cy + rr * a0.sin()), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: v2(cx + rr * a1.cos(), cy + rr * a1.sin()),
                        size: D2D_SIZE_F { width: rr, height: rr },
                        rotationAngle: 0.0,
                        sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                        arcSize: D2D1_ARC_SIZE_LARGE,
                    });
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    let _ = sink.Close();
                    self.dc.DrawGeometry(&path, self.brush(c), width, None);
                }
            }
        }
    }

    /// Soft glow: rectangle `r` blurred with a Gaussian of `sigma` DIPs (CSS box-shadow blur b → sigma b/2).
    pub fn glow(&mut self, r: Rect, sigma: f32, c: Color) {
        if c.a <= 0.0 {
            return;
        }
        let s = self.scale;
        let pad = (sigma * 3.0).ceil();
        let key = GlowKey {
            w: (r.w * s).round() as u32,
            h: (r.h * s).round() as u32,
            sigma: (sigma * s * 4.0).round() as u32,
            scale: (s * 100.0).round() as u32,
            color: c.key(),
        };
        if !self.res.glows.contains_key(&key) {
            if self.res.glows.len() > 64 {
                self.res.glows.clear();
            }
            let pw = ((r.w + 2.0 * pad) * s).ceil() as u32;
            let ph = ((r.h + 2.0 * pad) * s).ceil() as u32;
            let (x0, x1) = (pad * s, (pad + r.w) * s);
            let (y0, y1) = (pad * s, (pad + r.h) * s);
            let sp = sigma * s;
            let fx: Vec<f32> = (0..pw).map(|x| blur_1d(x as f32 + 0.5, x0, x1, sp)).collect();
            let fy: Vec<f32> = (0..ph).map(|y| blur_1d(y as f32 + 0.5, y0, y1, sp)).collect();
            let mut px = vec![0u8; (pw * ph * 4) as usize];
            for (y, wy) in fy.iter().enumerate() {
                for (x, wx) in fx.iter().enumerate() {
                    let a = (c.a * wx * wy).clamp(0.0, 1.0);
                    let i = (y * pw as usize + x) * 4;
                    px[i] = (c.b * a * 255.0).round() as u8;
                    px[i + 1] = (c.g * a * 255.0).round() as u8;
                    px[i + 2] = (c.r * a * 255.0).round() as u8;
                    px[i + 3] = (a * 255.0).round() as u8;
                }
            }
            match self.bitmap(pw, ph, &px) {
                Ok(b) => {
                    self.res.glows.insert(key, b);
                }
                Err(_) => return,
            }
        }
        let bmp = &self.res.glows[&key];
        let dest = D2D_RECT_F { left: r.x - pad, top: r.y - pad, right: r.r() + pad, bottom: r.b() + pad };
        unsafe {
            self.dc.DrawBitmap(bmp, Some(&dest), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
        }
    }

    fn bitmap(&self, w: u32, h: u32, premul: &[u8]) -> Result<ID2D1Bitmap1> {
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0 * self.scale,
            dpiY: 96.0 * self.scale,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        unsafe { self.dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(premul.as_ptr() as _), w * 4, &props) }
    }

    /// Draws an image into `r` with rounded corners of `radius` DIPs.
    pub fn image(&mut self, img: &Image, r: Rect, radius: f32, opacity: f32) {
        if !self.res.images.contains_key(&img.id) {
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let px = img.premultiplied();
            let b = unsafe {
                self.dc.CreateBitmap(D2D_SIZE_U { width: img.w, height: img.h }, Some(px.as_ptr() as _), img.w * 4, &props)
            };
            match b {
                Ok(b) => {
                    self.res.images.insert(img.id, b);
                }
                Err(_) => return,
            }
        }
        let bmp = self.res.images[&img.id].clone();
        // object-fit: cover
        let (iw, ih) = (img.w as f32, img.h as f32);
        let k = (r.w / iw).max(r.h / ih);
        let (sw, sh) = (r.w / k, r.h / k);
        let src = D2D_RECT_F { left: (iw - sw) / 2.0, top: (ih - sh) / 2.0, right: (iw + sw) / 2.0, bottom: (ih + sh) / 2.0 };
        unsafe {
            let clip = if radius > 0.0 {
                self.g
                    .d2d
                    .CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius })
                    .ok()
            } else {
                None
            };
            if let Some(clip) = &clip {
                self.push_layer(Some(clip.cast().unwrap()), 1.0, r);
            }
            self.dc.DrawBitmap(&bmp, Some(&r.d2d()), opacity, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, Some(&src), None);
            if clip.is_some() {
                self.pop_layer();
            }
        }
    }

    fn push_layer(&self, mask: Option<ID2D1Geometry>, opacity: f32, bounds: Rect) {
        let p = D2D1_LAYER_PARAMETERS1 {
            contentBounds: bounds.d2d(),
            geometricMask: std::mem::ManuallyDrop::new(mask),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity,
            opacityBrush: std::mem::ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS1_NONE,
        };
        unsafe {
            self.dc.PushLayer(&p, None);
            // PushLayer AddRef'd the mask; release our reference.
            let mut p = p;
            std::mem::ManuallyDrop::drop(&mut p.geometricMask);
        }
    }

    fn pop_layer(&self) {
        unsafe { self.dc.PopLayer() }
    }

    /// Group opacity for everything drawn until `pop_opacity`.
    pub fn push_opacity(&self, opacity: f32) {
        self.push_layer(None, opacity, rect(-10_000.0, -10_000.0, 20_000.0, 20_000.0));
    }

    pub fn pop_opacity(&self) {
        self.pop_layer();
    }

    pub fn clip(&self, r: Rect) {
        unsafe { self.dc.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_ALIASED) }
    }

    pub fn unclip(&self) {
        unsafe { self.dc.PopAxisAlignedClip() }
    }

    pub fn translate(&self, dx: f32, dy: f32) {
        unsafe {
            self.dc.SetTransform(&Matrix3x2::translation(dx, dy));
        }
    }

    pub fn reset_transform(&self) {
        unsafe { self.dc.SetTransform(&Matrix3x2::identity()) }
    }

    /// Elliptical radial gradient.
    pub fn radial(&self, r: Rect, cx: f32, cy: f32, rx: f32, ry: f32, stops: &[Stop]) {
        unsafe {
            let st: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|s| D2D1_GRADIENT_STOP { position: s.0, color: s.1.d2d() }).collect();
            let Ok(coll) = ID2D1RenderTarget::CreateGradientStopCollection(&self.dc, &st, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else { return };
            let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES { center: v2(cx, cy), gradientOriginOffset: v2(0.0, 0.0), radiusX: rx, radiusY: ry };
            if let Ok(b) = self.dc.CreateRadialGradientBrush(&props, None, &coll) {
                self.dc.FillRectangle(&r.d2d(), &b);
            }
        }
    }

    /// Left-to-right linear gradient across `r`.
    pub fn linear(&self, r: Rect, stops: &[Stop]) {
        unsafe {
            let st: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|s| D2D1_GRADIENT_STOP { position: s.0, color: s.1.d2d() }).collect();
            let Ok(coll) = ID2D1RenderTarget::CreateGradientStopCollection(&self.dc, &st, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else { return };
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: v2(r.x, 0.0), endPoint: v2(r.r(), 0.0) };
            if let Ok(b) = self.dc.CreateLinearGradientBrush(&props, None, &coll) {
                self.dc.FillRectangle(&r.d2d(), &b);
            }
        }
    }

    /// Draws `draw` into an offscreen bitmap of `size` DIPs, then draws that bitmap Gaussian-blurred with `sigma` (or
    /// sharp when the blur effect is unavailable).
    pub fn blurred(&mut self, w: f32, h: f32, sigma: f32, draw: impl FnOnce(&mut Painter)) {
        let r: Result<()> = (|| unsafe {
            let size = D2D_SIZE_F { width: w, height: h };
            let rt = self.dc.CreateCompatibleRenderTarget(Some(&size), None, None, D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE)?;
            let dc: ID2D1DeviceContext = rt.cast()?;
            rt.BeginDraw();
            {
                let mut sub = Painter::new(self.g, dc, self.res, self.scale)?;
                sub.clear(hex(0x0b0b0c));
                draw(&mut sub);
            }
            rt.EndDraw(None, None)?;
            let bmp = rt.GetBitmap()?;
            let effect = self.res.effect.get_or_insert_with(|| self.dc.CreateEffect(&CLSID_D2D1GaussianBlur).ok()).clone();
            match effect {
                Some(e) => {
                    e.SetInput(0, &bmp, true);
                    e.SetValue(D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &sigma.to_le_bytes())?;
                    e.SetValue(
                        D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32,
                        D2D1_PROPERTY_TYPE_ENUM,
                        &(D2D1_BORDER_MODE_HARD.0 as u32).to_le_bytes(),
                    )?;
                    let out = e.GetOutput()?;
                    self.dc.DrawImage(&out, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                }
                None => {
                    self.dc.DrawBitmap(&bmp, None, 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
                }
            }
            Ok(())
        })();
        let _ = r;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_compact_numbers() {
        assert_eq!(tokenize("m21.73 18-8-14a2 2 0 0 0-3.48 0"), {
            use Tok::*;
            vec![C('m'), N(21.73), N(18.0), N(-8.0), N(-14.0), C('a'), N(2.0), N(2.0), N(0.0), N(0.0), N(0.0), N(-3.48), N(0.0)]
        });
        assert_eq!(tokenize("M1.5.5"), vec![Tok::C('M'), Tok::N(1.5), Tok::N(0.5)]);
    }

    #[test]
    fn glow_profile() {
        // Far inside a wide rect the coverage is 1, at the edge 1/2.
        assert!((blur_1d(50.0, 0.0, 100.0, 4.0) - 1.0).abs() < 1e-4);
        assert!((blur_1d(0.0, 0.0, 100.0, 4.0) - 0.5).abs() < 1e-4);
        assert!(blur_1d(-12.0, 0.0, 100.0, 4.0) < 0.002);
    }

    #[test]
    fn embedded_fonts_resolve() {
        unsafe { let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED); }
        let g = Gfx::new("en").expect("gfx");
        assert!(g.has_fonts(), "Display, Sans and Mono must load from the embedded files");
    }

    #[test]
    fn contrast_text() {
        assert_eq!(hex(0xf59e0b).on(), hex(0x0b0b0c));
        assert_eq!(hex(0x4caf50).on(), hex(0x0b0b0c));
        assert_eq!(hex(0x1f3a8a).on(), hex(0xffffff));
    }
}
