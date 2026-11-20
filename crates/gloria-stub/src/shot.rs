//! `--screenshot <dir>`: renders every screen in every language to `<dir>/<lang>/<page>.png` on a WIC bitmap, with no
//! window, so CI and reviewers get the same pixels.

use std::path::Path;

use windows::core::{Interface, Result};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::*;

use crate::app::{App, Page, PAGE_IDS};
use crate::gfx::{save_png, Gfx, Painter, Res};
use crate::i18n::LANGS;
use crate::ui::{self, Input, H, W};

/// Extra states beyond the mockup pages.
pub const EXTRA_IDS: [&str; 2] = ["update-waiting", "update-stuck"];

/// Renders one screen to a PNG at `scale` (1 = 96 DPI).
pub fn render(g: &Gfx, app: &App, scale: f32, path: &Path) -> Result<()> {
    unsafe {
        let (pw, ph) = ((W * scale).round() as u32, (H * scale).round() as u32);
        let bmp = g.wic.CreateBitmap(pw, ph, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0 * scale,
            dpiY: 96.0 * scale,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let rt = g.d2d.CreateWicBitmapRenderTarget(&bmp, &props)?;
        let dc: ID2D1DeviceContext = rt.cast()?;
        let mut res = Res::default();
        rt.BeginDraw();
        {
            let mut p = Painter::new(g, dc, &mut res, scale)?;
            // `now` is far past every animation start, so pages and modals are fully in.
            ui::draw(&mut p, app, Input { field_text: true, ..Default::default() }, 10.0);
        }
        rt.EndDraw(None, None)?;
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        save_png(&g.wic, &bmp, path)
    }
}

/// Writes every page in every language. `make` builds a fresh preview `App` for a language.
pub fn all(dir: &Path, scale: f32, mut make: impl FnMut(&str) -> App) -> Result<usize> {
    let mut n = 0;
    for lang in LANGS {
        let g = Gfx::new(lang)?;
        for id in PAGE_IDS.iter().chain(EXTRA_IDS.iter()) {
            let mut app = make(lang);
            app.preview_page(id);
            app.page_t0 = -10.0;
            app.modal_t0 = -10.0;
            if matches!(app.page, Page::Progress | Page::UninstallProgress) {
                app.freeze_progress(62.0);
            }
            render(&g, &app, scale, &dir.join(lang).join(format!("{id}.png")))?;
            n += 1;
        }
    }
    Ok(n)
}
