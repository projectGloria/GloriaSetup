//! `%TEMP%\GloriaSetup-<id>-<timestamp>.log`: every run writes one (docs/design.md §11). The error screen links to it.

use std::fs::{File, OpenOptions};
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

/// Opens a new log for app `id`. Never reuses a file: two runs in the same second (the `uninstall.exe` and its
/// trampoline, say) get `-2`, `-3`, ... instead of truncating each other. Failing to open it never stops an install.
pub fn open(id: &str) -> Option<PathBuf> {
    let base = format!("GloriaSetup-{id}-{}", stamp(false));
    for n in 1..100 {
        let name = if n == 1 { format!("{base}.log") } else { format!("{base}-{n}.log") };
        let p = std::env::temp_dir().join(name);
        match OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(f) => {
                *LOG.lock().unwrap() = Some((f, p.clone()));
                return Some(p);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
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
