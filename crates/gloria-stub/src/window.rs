//! The real window: a borderless 640×440 popup with a Direct2D target, hit-testing over the regions `ui::draw`
//! returns, keyboard focus, and a 16 ms animation timer.

use std::time::Instant;

use windows::core::{w, Interface, Result, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{Act, App, Effect, Page};
use crate::gfx::{Gfx, Image, Painter, Res};
use crate::ui::{self, Frame, Input, H, MIN_BTN, W};

/// Handles effects the window cannot do itself (engine jobs, launching, closing the app). Returns an exit code to
/// close the window with.
pub type Handler = Box<dyn FnMut(&mut App, Effect, HWND) -> Option<i32>>;

const TIMER: usize = 1;
const WM_MOUSELEAVE: u32 = 0x02a3;
/// Posted by engine threads to wake the window.
pub const WM_JOB: u32 = WM_APP + 1;

struct State {
    g: Gfx,
    app: App,
    handler: Handler,
    software: bool,
    hwnd: HWND,
    rt: Option<ID2D1HwndRenderTarget>,
    res: Res,
    inp: Input,
    frame: Frame,
    t0: Instant,
    scale: f32,
    tracking: bool,
    timer: bool,
    /// Footer link hover animation: from `.0` to `.1`, started at `.2`.
    link: (f32, f32, f64),
    exit: Option<i32>,
    icons: (HICON, HICON),
    /// The last render error logged, so a failing target is logged once, not every frame.
    last_render_err: Option<i32>,
}

impl State {
    fn now(&self) -> f64 {
        self.t0.elapsed().as_secs_f64()
    }
    /// How far the footer link's hover effect is in (0..1), eased.
    fn link_k(&self, now: f64) -> f32 {
        let (from, to, t0) = self.link;
        let t = ((now - t0) as f32 / LINK_ANIM).clamp(0.0, 1.0);
        let t = 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t);
        from + (to - from) * t
    }
    /// Retargets the link hover animation after the hovered control changed.
    fn hover_changed(&mut self) {
        let now = self.now();
        let to = if self.inp.hover == Some(Act::OpenGithub) { 1.0 } else { 0.0 };
        if to != self.link.1 {
            self.link = (self.link_k(now), to, now);
            start_timer(self);
        }
    }
}

const LINK_ANIM: f32 = 0.22;

pub fn run(g: Gfx, mut app: App, software: bool, handler: Handler) -> Result<i32> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let hinst = GetModuleHandleW(None)?;
        let class = w!("GloriaSetupWindow");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);

        // Center on the monitor under the cursor, sized for its DPI.
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let scale = dx as f32 / 96.0;
        let (pw, ph) = ((W * scale).round() as i32, (H * scale).round() as i32);
        let wa = mi.rcWork;
        let x = wa.left + (wa.right - wa.left - pw) / 2;
        let y = wa.top + (wa.bottom - wa.top - ph) / 2;

        app.page_t0 = 0.0;
        let title = HSTRING::from(app.title());
        let state = Box::new(State {
            g,
            app,
            handler,
            software,
            hwnd: HWND::default(),
            rt: None,
            res: Res::default(),
            inp: Input::default(),
            frame: Frame::default(),
            t0: Instant::now(),
            scale,
            tracking: false,
            timer: false,
            link: (0.0, 0.0, 0.0),
            exit: None,
            icons: (HICON::default(), HICON::default()),
            last_render_err: None,
        });
        let ptr = Box::into_raw(state);
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class,
            &title,
            WS_POPUP | WS_MINIMIZEBOX | WS_SYSMENU | WS_CLIPCHILDREN,
            x,
            y,
            pw,
            ph,
            None,
            None,
            Some(hinst.into()),
            Some(ptr as *const _),
        )?;
        let st = &mut *ptr;
        st.hwnd = hwnd;
        let pref = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as _, 4);
        let dark = windows_core::BOOL(1);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
        // Fix the size for the window's actual DPI (it may differ from the monitor guess).
        let dpi = GetDpiForWindow(hwnd);
        if dpi != dx {
            st.scale = dpi as f32 / 96.0;
            let (pw, ph) = ((W * st.scale).round() as i32, (H * st.scale).round() as i32);
            let _ = SetWindowPos(hwnd, None, 0, 0, pw, ph, SWP_NOMOVE | SWP_NOZORDER);
        }
        set_icons(st);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        start_timer(st);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let code = st.exit.unwrap_or(1);
        let _st = Box::from_raw(ptr);
        Ok(code)
    }
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut State> {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    p.as_mut()
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let Some(st) = state(hwnd) else { return DefWindowProcW(hwnd, msg, wp, lp) };
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                render(st);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SIZE => {
                if let Some(rt) = &st.rt {
                    let (w, h) = (lp.0 as u32 & 0xffff, (lp.0 as u32 >> 16) & 0xffff);
                    let _ = rt.Resize(&D2D_SIZE_U { width: w, height: h });
                }
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xffff) as u32;
                st.scale = dpi as f32 / 96.0;
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                st.rt = None;
                st.res = Res::default();
                set_icons(st);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_NCHITTEST => {
                let mut pt = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
                let _ = ScreenToClient(hwnd, &mut pt);
                let (x, y) = (pt.x as f32 / st.scale, pt.y as f32 / st.scale);
                if y < 36.0 && x < MIN_BTN.x && x >= 1.0 && y >= 1.0 {
                    return LRESULT(HTCAPTION as isize);
                }
                LRESULT(HTCLIENT as isize)
            }
            WM_SETCURSOR => {
                if (lp.0 & 0xffff) as u32 == HTCLIENT {
                    let hand = st.inp.hover.is_some();
                    let _ = SetCursor(LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }).ok());
                    return LRESULT(1);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_MOUSEMOVE => {
                if !st.tracking {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut t);
                    st.tracking = true;
                }
                let (x, y) = pos(st, lp);
                let h = hit(&st.frame, x, y);
                if h != st.inp.hover {
                    st.inp.hover = h;
                    st.hover_changed();
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                st.tracking = false;
                if st.inp.hover.is_some() {
                    st.inp.hover = None;
                    st.hover_changed();
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = pos(st, lp);
                st.inp.pressed = hit(&st.frame, x, y);
                st.inp.focus = None;
                let _ = SetFocus(Some(hwnd));
                SetCapture(hwnd);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                let (x, y) = pos(st, lp);
                let h = hit(&st.frame, x, y);
                let p = st.inp.pressed.take();
                if let (Some(a), Some(b)) = (h, p) {
                    if a == b {
                        act(st, a);
                    }
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                if matches!(vk, VK_TAB | VK_RETURN | VK_SPACE | VK_ESCAPE) {
                    key(st, vk);
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_TIMER => {
                let now = st.now();
                let mut moving = st.app.tick(now) || now - st.link.2 < LINK_ANIM as f64;
                if let Some(eff) = st.app.pending.take() {
                    if let Some(code) = (st.handler)(&mut st.app, eff, hwnd) {
                        st.exit = Some(code);
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                    moving = true;
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
                if !moving {
                    let _ = KillTimer(Some(hwnd), TIMER);
                    st.timer = false;
                }
                LRESULT(0)
            }
            WM_JOB => {
                start_timer(st);
                LRESULT(0)
            }
            WM_CLOSE => {
                act(st, Act::Close);
                LRESULT(0)
            }
            WM_DESTROY => {
                for i in [st.icons.0, st.icons.1] {
                    if !i.is_invalid() {
                        let _ = DestroyIcon(i);
                    }
                }
                st.icons = (HICON::default(), HICON::default());
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn pos(st: &State, lp: LPARAM) -> (f32, f32) {
    let x = (lp.0 & 0xffff) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
    (x / st.scale, y / st.scale)
}

fn hit(f: &Frame, x: f32, y: f32) -> Option<Act> {
    f.hits.iter().rev().find(|h| h.enabled && h.r.contains(x, y)).map(|h| h.act)
}

fn start_timer(st: &mut State) {
    if !st.timer {
        unsafe {
            SetTimer(Some(st.hwnd), TIMER, 16, None);
        }
        st.timer = true;
    }
}

fn act(st: &mut State, a: Act) {
    let now = st.now();
    let before = st.app.page;
    let eff = st.app.act(a, now);
    match eff {
        Effect::None => {}
        Effect::Exit(code) => {
            st.exit = Some(code);
            unsafe {
                let _ = DestroyWindow(st.hwnd);
            }
            return;
        }
        Effect::Minimize => unsafe {
            let _ = ShowWindow(st.hwnd, SW_MINIMIZE);
        },
        Effect::OpenUrl(url) => crate::sys::open_url(url),
        other => {
            if let Some(code) = (st.handler)(&mut st.app, other, st.hwnd) {
                st.exit = Some(code);
                unsafe {
                    let _ = DestroyWindow(st.hwnd);
                }
                return;
            }
        }
    }
    if st.app.page != before {
        st.inp.focus = None;
        st.inp.hover = None;
        st.link = (0.0, 0.0, 0.0);
    }
    start_timer(st);
    unsafe {
        let _ = InvalidateRect(Some(st.hwnd), None, false);
    }
}

fn key(st: &mut State, vk: VIRTUAL_KEY) {
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    match vk {
        VK_TAB => {
            // Tab order: page controls in drawing order, then the window buttons.
            let mut order: Vec<Act> = st
                .frame
                .hits
                .iter()
                .filter(|h| h.enabled && !matches!(h.act, Act::Noop | Act::Minimize | Act::Close))
                .map(|h| h.act)
                .collect();
            order.dedup();
            order.push(Act::Minimize);
            order.push(Act::Close);
            let cur = st.inp.focus;
            let i = cur.and_then(|c| order.iter().position(|a| *a == c));
            let n = order.len() as isize;
            let next = match i {
                None => {
                    if shift {
                        n - 1
                    } else {
                        0
                    }
                }
                Some(i) => (i as isize + if shift { -1 } else { 1 }).rem_euclid(n),
            };
            let a = order[next as usize];
            unsafe {
                st.inp.focus = Some(a);
                let _ = SetFocus(Some(st.hwnd));
            }
        }
        VK_RETURN | VK_SPACE => {
            if let Some(a) = st.inp.focus {
                act(st, a);
            } else if vk == VK_RETURN {
                // Enter without focus presses the page's primary button.
                if let Some(h) = st.frame.hits.iter().rev().find(|h| matches!(h.act, Act::Start | Act::LaunchApp) && h.enabled) {
                    let a = h.act;
                    act(st, a);
                } else if matches!(st.app.page, Page::UninstallDone | Page::Done) {
                    act(st, Act::Close);
                }
            }
        }
        VK_ESCAPE => {
            if st.app.modal.is_some() {
                act(st, Act::Modal(None));
            } else if !matches!(st.app.page, Page::Progress | Page::UninstallProgress) {
                act(st, Act::Close);
            }
        }
        _ => {}
    }
    unsafe {
        let _ = InvalidateRect(Some(st.hwnd), None, false);
    }
}

fn render(st: &mut State) {
    let now = st.now();
    let r: Result<()> = (|| unsafe {
        if st.rt.is_none() {
            let mut rc = RECT::default();
            GetClientRect(st.hwnd, &mut rc)?;
            let dpi = 96.0 * st.scale;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: if st.software { D2D1_RENDER_TARGET_TYPE_SOFTWARE } else { D2D1_RENDER_TARGET_TYPE_DEFAULT },
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: dpi,
                dpiY: dpi,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: st.hwnd,
                pixelSize: D2D_SIZE_U { width: (rc.right - rc.left) as u32, height: (rc.bottom - rc.top) as u32 },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            st.rt = Some(st.g.d2d.CreateHwndRenderTarget(&props, &hp)?);
            st.res = Res::default();
        }
        let rt = st.rt.clone().unwrap();
        let dc: ID2D1DeviceContext = rt.cast()?;
        rt.BeginDraw();
        let mut inp = st.inp;
        inp.link = st.link_k(now);
        {
            let mut p = Painter::new(&st.g, dc, &mut st.res, st.scale)?;
            st.frame = ui::draw(&mut p, &st.app, inp, now);
        }
        if let Err(e) = rt.EndDraw(None, None) {
            if e.code() == D2DERR_RECREATE_TARGET {
                st.rt = None;
                st.res = Res::default();
                let _ = InvalidateRect(Some(st.hwnd), None, false);
            } else {
                return Err(e);
            }
        }
        Ok(())
    })();
    if let Err(e) = r {
        // Once per error code, so a broken target doesn't fill the log at 60 frames a second.
        if st.last_render_err != Some(e.code().0) {
            st.last_render_err = Some(e.code().0);
            crate::log!("render: {e}");
        }
    }
}

// ───────────── window icon ─────────────

fn set_icons(st: &mut State) {
    unsafe {
        let big = (GetSystemMetricsForDpi(SM_CXICON, (96.0 * st.scale) as u32)).max(32) as u32;
        let small = (GetSystemMetricsForDpi(SM_CXSMICON, (96.0 * st.scale) as u32)).max(16) as u32;
        let wic = &st.g.wic;
        let mk = |sz: u32| st.app.info.icon.resized(wic, sz).ok().and_then(|i| hicon(&i).ok());
        let (b, s) = (mk(big), mk(small));
        if let Some(b) = b {
            SendMessageW(st.hwnd, WM_SETICON, Some(WPARAM(ICON_BIG as usize)), Some(LPARAM(b.0 as isize)));
        }
        if let Some(s) = s {
            SendMessageW(st.hwnd, WM_SETICON, Some(WPARAM(ICON_SMALL as usize)), Some(LPARAM(s.0 as isize)));
        }
        for old in [st.icons.0, st.icons.1] {
            if !old.is_invalid() {
                let _ = DestroyIcon(old);
            }
        }
        st.icons = (b.unwrap_or_default(), s.unwrap_or_default());
    }
}

pub fn hicon(img: &Image) -> Result<HICON> {
    unsafe {
        let bi = BITMAPV5HEADER {
            bV5Size: std::mem::size_of::<BITMAPV5HEADER>() as u32,
            bV5Width: img.w as i32,
            bV5Height: -(img.h as i32),
            bV5Planes: 1,
            bV5BitCount: 32,
            bV5Compression: BI_BITFIELDS,
            bV5RedMask: 0x00ff0000,
            bV5GreenMask: 0x0000ff00,
            bV5BlueMask: 0x000000ff,
            bV5AlphaMask: 0xff000000,
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let dc = GetDC(None);
        let color = CreateDIBSection(Some(dc), &bi as *const _ as *const BITMAPINFO, DIB_RGB_COLORS, &mut bits, None, 0);
        ReleaseDC(None, dc);
        let color = color?;
        std::ptr::copy_nonoverlapping(img.px.as_ptr(), bits as *mut u8, img.px.len());
        let mask = CreateBitmap(img.w as i32, img.h as i32, 1, 1, None);
        let ii = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}
