#![windows_subsystem = "windows"]

mod app;
mod engine;
mod gfx;
mod i18n;
#[macro_use]
mod log;
mod real;
mod shot;
mod sys;
mod ui;
mod window;

use std::path::Path;

use gloria_core::cli::{self, Args};
use gloria_core::exit::Exit;
use gloria_core::manifest::{AppManifest, RuntimeManifest};
use gloria_core::res;
use windows::core::PCWSTR;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows::Win32::System::LibraryLoader::{FindResourceW, LoadResource, LockResource, SizeofResource};
use windows::Win32::UI::WindowsAndMessaging::{RT_GROUP_ICON, RT_ICON};

use app::{App, AppInfo};
use gfx::{Gfx, Image};
use i18n::Strings;

fn main() {
    let line = unsafe { windows::Win32::System::Environment::GetCommandLineW().to_string().unwrap_or_default() };
    let args = match cli::parse_command_line(&line) {
        Ok(a) => a,
        Err(_) => std::process::exit(Exit::InvalidArgs.code()),
    };
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    std::process::exit(run(&args));
}

fn run(args: &Args) -> i32 {
    if args.preview || args.screenshot.is_some() {
        return preview(args).unwrap_or(Exit::Error.code());
    }
    let src = match engine::open_self() {
        Ok(Some(s)) => s,
        // A bare stub (no payload) has nothing to install: show the preview.
        Ok(None) => return preview(args).unwrap_or(Exit::Error.code()),
        Err(_) => return Exit::Error.code(),
    };
    log::open(&src.m.app.id);
    log!("GloriaSetup {} ({}), {} {}", gloria_core::GLORIA_SETUP_VERSION, src.exe.display(), src.m.app.id, src.m.app.version);
    log!("args: {args:?}");
    // One engine per app at a time. The plain `uninstall.exe` only hands off to its trampoline (which takes the
    // lock), so it must not hold it.
    if args.cleanup.is_some() || !src.payload.is_uninstaller() {
        let wait = if args.silent { std::time::Duration::from_secs(120) } else { std::time::Duration::from_secs(3) };
        if !sys::instance_lock(&src.m.app.id, wait) {
            log!("another GloriaSetup for {} is still running", src.m.app.id);
            return Exit::FilesLocked.code();
        }
    }
    if let Some(dir) = args.cleanup.clone() {
        return real::cleanup(args, src, &dir);
    }
    if src.payload.is_uninstaller() {
        return real::uninstaller(args, src);
    }
    real::setup(args, src)
}

/// `--preview` / `--screenshot`: the UI with a sample (or `--manifest`) app and simulated jobs. Nothing is installed.
fn preview(args: &Args) -> windows::core::Result<i32> {
    // Preview never touches the disk, so it shows a neutral profile instead of this machine's user name (screenshots
    // end up in docs).
    for (k, v) in [
        ("USERPROFILE", r"C:\Users\you"),
        ("APPDATA", r"C:\Users\you\AppData\Roaming"),
        ("LOCALAPPDATA", r"C:\Users\you\AppData\Local"),
        ("TEMP", r"C:\Users\you\AppData\Local\Temp"),
        ("TMP", r"C:\Users\you\AppData\Local\Temp"),
    ] {
        std::env::set_var(k, v);
    }
    let m = match &args.manifest {
        Some(p) => {
            let src = std::fs::read_to_string(p).map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?;
            let app = AppManifest::from_toml(&src).map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e))?;
            RuntimeManifest { app, ..RuntimeManifest::sample() }
        }
        None => RuntimeManifest::sample(),
    };
    let wic_g = Gfx::new("en")?;
    let icon = load_icon(&wic_g, args.icon.as_deref())?;
    let make = |lang: &str| -> App {
        let info = preview_info(m.clone(), icon.clone());
        let s = strings(lang, &info);
        let mut a = App::new(info, s, true);
        a.credit = credit(&wic_g);
        a.gloria = gloria_red(&wic_g);
        a
    };
    if let Some(dir) = &args.screenshot {
        let n = shot::all(dir, 1.0, make)?;
        let _ = n;
        return Ok(0);
    }
    let lang = i18n::detect(args.lang.as_deref());
    let mut a = make(lang);
    a.preview_page(args.page.as_deref().unwrap_or("welcome"));
    let g = Gfx::new(lang)?;
    window::run(g, a, args.software, Box::new(|_, _, _| None))
}

pub(crate) fn strings(lang: &str, info: &AppInfo) -> Strings {
    Strings::new(lang, vec![("name", info.name().to_string()), ("version", info.version().to_string()), ("exe", info.exe())])
}

/// Sample data folder sizes and file names for the simulated progress.
fn preview_info(m: RuntimeManifest, icon: Image) -> AppInfo {
    let exe = m.app.main_exe_rel();
    let tmp = AppInfo::new(m.clone(), icon.clone(), vec![], vec![]);
    let sizes = [13_002_342u64, 3_250_585, 1_048_576];
    let data = tmp.data_paths().iter().enumerate().map(|(i, p)| (p.to_string_lossy().into_owned(), Some(sizes[i % 3]))).collect();
    let files = [exe.as_str(), r"resources\app.pak", r"resources\icudtl.dat", r"locales\en-US.pak", r"locales\fr.pak", r"uninstall.exe"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    AppInfo::new(m, icon, data, files)
}

/// `--icon <file>`, else this exe's own icon group (the app's icon in a packed setup).
pub(crate) fn load_icon(g: &Gfx, path: Option<&Path>) -> windows::core::Result<Image> {
    let bytes = match path {
        Some(p) => std::fs::read(p).map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?,
        None => own_icon().ok_or_else(|| windows::core::Error::from_hresult(windows::core::HRESULT(-1)))?,
    };
    let img = Image::decode(&g.wic, &bytes)?;
    if img.w > 256 {
        img.resized(&g.wic, 256)
    } else {
        Ok(img)
    }
}

fn resource(kind: PCWSTR, id: u16) -> Option<&'static [u8]> {
    unsafe {
        let h = FindResourceW(None, PCWSTR(id as usize as *const u16), kind);
        if h.is_invalid() {
            return None;
        }
        let size = SizeofResource(None, h);
        let data = LoadResource(None, h).ok()?;
        let p = LockResource(data) as *const u8;
        (!p.is_null()).then(|| std::slice::from_raw_parts(p, size as usize))
    }
}

/// Rebuilds an .ico from the largest image of RT_GROUP_ICON 1.
fn own_icon() -> Option<Vec<u8>> {
    let grp = resource(RT_GROUP_ICON, 1)?;
    let count = u16::from_le_bytes(grp.get(4..6)?.try_into().ok()?) as usize;
    let mut best: Option<res::IconImage> = None;
    for i in 0..count {
        let e = grp.get(6 + i * 14..6 + i * 14 + 14)?;
        let w = if e[0] == 0 { 256 } else { e[0] as u32 };
        let h = if e[1] == 0 { 256 } else { e[1] as u32 };
        let bpp = u16::from_le_bytes([e[6], e[7]]);
        let id = u16::from_le_bytes([e[12], e[13]]);
        if best.as_ref().is_some_and(|b| b.width >= w) {
            continue;
        }
        if let Some(data) = resource(RT_ICON, id) {
            best = Some(res::IconImage { width: w, height: h, bit_count: bpp, data: data.to_vec() });
        }
    }
    best.map(|b| res::build_ico(&[b]))
}

const CREDIT_ICO: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/credit.ico"));

pub(crate) fn credit(g: &Gfx) -> Option<Image> {
    Image::decode(&g.wic, CREDIT_ICO).ok()
}

const GLORIA_RED_ICO: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/gloria_red.ico"));

/// The projectGloria mark shown next to the website link in every footer.
pub(crate) fn gloria_red(g: &Gfx) -> Option<Image> {
    Image::decode(&g.wic, GLORIA_RED_ICO).ok()
}
