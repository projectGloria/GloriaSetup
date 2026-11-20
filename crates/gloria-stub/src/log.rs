//! `%TEMP%\GloriaSetup-<id>-<timestamp>.log`: every run writes one (docs/design.md §11). The error screen links to it.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use windows::Win32::System::SystemInformation::GetLocalTime;

static LOG: Mutex<Option<(File, PathBuf)>> = Mutex::new(None);

fn stamp(with_seconds: bool) -> String {
    let t = unsafe { GetLocalTime() };
    if with_seconds {
        format!("{:02}:{:02}:{:02}.{:03}", t.wHour, t.wMinute, t.wSecond, t.wMilliseconds)
    } else {
        format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
    }
}

/// Opens the log for app `id`. Failing to open it never stops an install.
pub fn open(id: &str) -> Option<PathBuf> {
    let p = std::env::temp_dir().join(format!("GloriaSetup-{id}-{}.log", stamp(false)));
    let f = File::create(&p).ok()?;
    *LOG.lock().unwrap() = Some((f, p.clone()));
    Some(p)
}

pub fn path() -> Option<PathBuf> {
    LOG.lock().unwrap().as_ref().map(|(_, p)| p.clone())
}

/// The log path as the error screen shows it (`%TEMP%\…`).
pub fn display_path() -> String {
    let Some(p) = path() else { return String::new() };
    let tmp = std::env::temp_dir();
    match p.strip_prefix(&tmp) {
        Ok(rest) => format!(r"%TEMP%\{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

pub fn line(msg: &str) {
    if let Some((f, _)) = LOG.lock().unwrap().as_mut() {
        let _ = writeln!(f, "{} {msg}", stamp(true));
        let _ = f.flush();
    }
}

#[macro_export]
macro_rules! log {
    ($($t:tt)*) => { $crate::log::line(&format!($($t)*)) };
}
