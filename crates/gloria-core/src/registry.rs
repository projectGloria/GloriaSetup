//! The Uninstall registry values (GloriaLauncher contract). See docs/design.md §6.
//! The stub writes exactly what [`uninstall_values`] returns, so the contract is tested here without a registry.

use std::path::Path;

use crate::manifest::{AppManifest, PUBLISHER, UNINSTALLER_NAME};

pub const UNINSTALL_ROOT: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegValue {
    Sz(String),
    Dword(u32),
}

/// EstimatedSize is a DWORD in KB (rounded up), saturating.
pub fn estimated_size_kb(bytes: u64) -> u32 {
    bytes.div_ceil(1024).min(u32::MAX as u64) as u32
}

/// Values for `HKCU\<UNINSTALL_ROOT>\projectGloria.<id>`, in write order.
pub fn uninstall_values(
    app: &AppManifest,
    install_dir: &Path,
    total_bytes: u64,
    stub_version: &str,
) -> Vec<(&'static str, RegValue)> {
    let dir = install_dir.to_string_lossy().trim_end_matches('\\').to_string();
    let exe = format!("{dir}\\{}", app.main_exe_rel());
    let unins = format!("\"{dir}\\{UNINSTALLER_NAME}\"");
    vec![
        ("DisplayName", RegValue::Sz(app.name.clone())),
        ("DisplayVersion", RegValue::Sz(app.parsed_version().to_string())),
        ("Publisher", RegValue::Sz(PUBLISHER.into())),
        ("InstallLocation", RegValue::Sz(dir)),
        ("DisplayIcon", RegValue::Sz(exe)),
        ("UninstallString", RegValue::Sz(unins.clone())),
        ("QuietUninstallString", RegValue::Sz(format!("{unins} /S"))),
        ("EstimatedSize", RegValue::Dword(estimated_size_kb(total_bytes))),
        ("NoModify", RegValue::Dword(1)),
        ("NoRepair", RegValue::Dword(1)),
        ("GloriaSetup", RegValue::Sz(stub_version.into())),
    ]
}

/// Copies of the GloriaLauncher detection helpers (`src-tauri/src/lib.rs`, `mod system_installed`), so the tests
/// check our values against the launcher's real parsing rules. Keep in sync if the launcher changes.
pub mod launcher {
    use std::path::PathBuf;

    pub fn normalize(s: &str) -> String {
        s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
    }

    pub fn strip_version(name: &str) -> &str {
        let trimmed = name.trim();
        if let Some((head, last)) = trimmed.rsplit_once(' ') {
            let v = last.trim_start_matches(['v', 'V']);
            if v.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                return head.trim_end();
            }
        }
        trimmed
    }

    pub fn is_helper_exe(file_name: &str) -> bool {
        let n = file_name.to_ascii_lowercase();
        n.starts_with("unins")
            || n.starts_with("uninstall")
            || n == "update.exe"
            || n.contains("squirrel")
            || n.contains("crashpad")
            || n.contains("elevate")
    }

    pub fn parse_icon_path(icon: &str) -> PathBuf {
        let s = icon.trim();
        let s = match s.rsplit_once(',') {
            Some((p, idx)) if idx.trim().trim_start_matches('-').parse::<i32>().is_ok() => p,
            _ => s,
        };
        PathBuf::from(s.trim().trim_matches('"'))
    }
}

#[cfg(test)]
mod tests {
    use super::launcher::*;
    use super::*;
    use std::path::PathBuf;

    fn app() -> AppManifest {
        AppManifest::from_toml(
            r##"
id = "GloriaDummy"
name = "GloriaDummy"
version = "v0.2"
main_exe = "gloria-dummy.exe"
accent = "#3b82f6"
"##,
        )
        .unwrap()
    }

    fn get<'a>(v: &'a [(&str, RegValue)], k: &str) -> &'a RegValue {
        &v.iter().find(|(n, _)| *n == k).unwrap_or_else(|| panic!("missing {k}")).1
    }
    fn sz<'a>(v: &'a [(&str, RegValue)], k: &str) -> &'a str {
        match get(v, k) {
            RegValue::Sz(s) => s,
            other => panic!("{k} is {other:?}, expected REG_SZ"),
        }
    }

    #[test]
    fn contract_values() {
        let dir = PathBuf::from(r"C:\Users\you\AppData\Local\Programs\Gloria Dummy\");
        let v = uninstall_values(&app(), &dir, 10 * 1024 * 1024 + 1, "0.1.0");
        assert_eq!(app().uninstall_key(), "projectGloria.GloriaDummy");
        assert_eq!(sz(&v, "DisplayName"), "GloriaDummy");
        assert_eq!(sz(&v, "DisplayVersion"), "0.2.0", "no leading v, full core");
        assert_eq!(sz(&v, "Publisher"), "projectGloria");
        assert_eq!(sz(&v, "InstallLocation"), r"C:\Users\you\AppData\Local\Programs\Gloria Dummy");
        assert_eq!(sz(&v, "DisplayIcon"), r"C:\Users\you\AppData\Local\Programs\Gloria Dummy\gloria-dummy.exe");
        assert_eq!(sz(&v, "UninstallString"), r#""C:\Users\you\AppData\Local\Programs\Gloria Dummy\uninstall.exe""#);
        assert_eq!(
            sz(&v, "QuietUninstallString"),
            r#""C:\Users\you\AppData\Local\Programs\Gloria Dummy\uninstall.exe" /S"#
        );
        assert_eq!(get(&v, "EstimatedSize"), &RegValue::Dword(10 * 1024 + 1), "KB, rounded up");
        assert_eq!(get(&v, "NoModify"), &RegValue::Dword(1));
        assert_eq!(get(&v, "NoRepair"), &RegValue::Dword(1));
        assert_eq!(sz(&v, "GloriaSetup"), "0.1.0");
    }

    #[test]
    fn launcher_parses_our_values() {
        let dir = PathBuf::from(r"C:\Users\you\AppData\Local\Programs\GloriaDummy");
        let v = uninstall_values(&app(), &dir, 1, "0.1.0");
        let icon = parse_icon_path(sz(&v, "DisplayIcon"));
        assert_eq!(icon, dir.join("gloria-dummy.exe"));
        assert!(!is_helper_exe(icon.file_name().unwrap().to_str().unwrap()));
        // The uninstaller is a helper, so the launcher never mistakes it for the app.
        assert!(is_helper_exe(UNINSTALLER_NAME));
        assert_eq!(normalize(strip_version(sz(&v, "DisplayName"))), "gloriadummy");
    }

    #[test]
    fn kb_rounding() {
        assert_eq!(estimated_size_kb(0), 0);
        assert_eq!(estimated_size_kb(1), 1);
        assert_eq!(estimated_size_kb(1024), 1);
        assert_eq!(estimated_size_kb(1025), 2);
        assert_eq!(estimated_size_kb(u64::MAX), u32::MAX);
    }
}
