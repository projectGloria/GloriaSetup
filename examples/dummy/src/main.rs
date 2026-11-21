//! The GloriaSetup test app: shows its version (from `version.txt` beside it, written by `pack.ps1`) and writes a
//! small settings file into its data folder, so install, update and the uninstall data question have something real
//! to work on.
//!
//! Test modes (used by `gloria-pack/tests/e2e.rs`):
//! - `--quiet` or `GLORIA_DUMMY_QUIET=1`: write the settings file, show nothing, exit.
//! - `GLORIA_DUMMY_DATA=<folder>`: use that data folder instead of `%APPDATA%\com.gloria.dummy`.
//! - `--hold`: show the message box only (no settings file). Closes when asked, like a normal app.
//! - `--stubborn`: no window, ignores every request to close, runs for two minutes. Only force-closing ends it.

#![windows_subsystem = "windows"]

use std::path::PathBuf;

use windows::core::HSTRING;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};

fn main() {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
    let version = std::fs::read_to_string(dir.join("version.txt")).unwrap_or_else(|_| "unknown".into());
    let version = version.trim();
    let title = HSTRING::from(format!("GloriaSetup Dummy {version}"));
    let arg = |a: &str| std::env::args().any(|x| x == a);

    if arg("--stubborn") {
        std::thread::sleep(std::time::Duration::from_secs(120));
        return;
    }
    if arg("--hold") {
        unsafe {
            MessageBoxW(None, &HSTRING::from(format!("GloriaSetup Dummy {version} (hold)")), &title, MB_OK);
        }
        return;
    }

    let data = std::env::var_os("GLORIA_DUMMY_DATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("com.gloria.dummy")));
    let saved = data.as_ref().and_then(|d| {
        std::fs::create_dir_all(d).ok()?;
        let p = d.join("settings.json");
        let runs = std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| s.split("\"runs\":").nth(1)?.trim().trim_end_matches('}').trim().parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        std::fs::write(&p, format!("{{\"version\": \"{version}\", \"runs\": {runs}}}\n")).ok()?;
        Some((p, runs))
    });

    if arg("--quiet") || std::env::var("GLORIA_DUMMY_QUIET").is_ok_and(|v| v == "1") {
        return;
    }
    let mut text = format!("GloriaSetup Dummy {version}\n\nRunning from:\n{}", dir.display());
    if let Some((p, runs)) = saved {
        text += &format!("\n\nSettings ({runs} runs):\n{}", p.display());
    }
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), &title, MB_OK | MB_ICONINFORMATION);
    }
}
