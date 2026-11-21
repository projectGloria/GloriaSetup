//! The GloriaSetup test app: shows its version (from `version.txt` beside it, written by `pack.ps1`) and writes a
//! small settings file into its data folder, so install, update and the uninstall data question have something real
//! to work on.
//!
//! Test modes (used by `gloria-pack/tests/e2e.rs`):
//! - `--quiet` or `GLORIA_DUMMY_QUIET=1`: write the settings file, show nothing, exit.
//! - `GLORIA_DUMMY_DATA=<folder>`: use that data folder instead of `%APPDATA%\com.gloria.dummy`.
//! - `--hold`: show the message box only (no settings file). Closes when asked, like a normal app.
//! - `--stubborn`: no window, ignores every request to close, runs for two minutes. Only force-closing ends it.
//! - Fake old uninstaller: `/S` with `GLORIA_DUMMY_LEGACY_KEY=<HKCU\...>` deletes the other files in its folder and
//!   that registry key, like an NSIS/Inno uninstaller would; `GLORIA_DUMMY_LEGACY_WIPE=<folder>` also deletes that
//!   (data) folder, like an uninstaller that takes the user's settings with it.
//! - Fake WebView2 bootstrapper: `/install` with `GLORIA_TEST_WEBVIEW2=<file>` creates that file ("runtime
//!   installed"), unless `GLORIA_DUMMY_WEBVIEW2_FAIL=1`.

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

    if arg("/S") {
        if let Ok(key) = std::env::var("GLORIA_DUMMY_LEGACY_KEY") {
            fake_uninstall(&exe, &dir, &key);
            return;
        }
    }
    if arg("/install") {
        if let Some(marker) = std::env::var_os("GLORIA_TEST_WEBVIEW2") {
            if std::env::var("GLORIA_DUMMY_WEBVIEW2_FAIL").is_ok_and(|v| v == "1") {
                std::process::exit(1);
            }
            let _ = std::fs::write(marker, "fake WebView2 runtime");
            return;
        }
    }

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

/// Removes everything in `dir` but this exe, then the registry key (an uninstaller can't delete its running self;
/// NSIS run with `_?=` leaves it behind the same way).
fn fake_uninstall(exe: &std::path::Path, dir: &std::path::Path, key: &str) {
    std::thread::sleep(std::time::Duration::from_millis(500));
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p == exe {
            continue;
        }
        let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
    }
    if let Some(wipe) = std::env::var_os("GLORIA_DUMMY_LEGACY_WIPE") {
        let _ = std::fs::remove_dir_all(wipe);
    }
    let _ = std::process::Command::new("reg").args(["delete", key, "/f"]).output();
}
