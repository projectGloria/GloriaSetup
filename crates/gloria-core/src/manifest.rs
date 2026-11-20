//! App manifest: `gloria.toml` in the app repo (pack time) and the JSON copy inside the payload (run time).
//! See docs/design.md §1. `publisher` is fixed to "projectGloria" and is not configurable.

use serde::{Deserialize, Serialize};

use crate::paths::{parse_data_folder, DataFolder};
use crate::version::Version;

pub const PUBLISHER: &str = "projectGloria";
/// File name of the installed uninstaller. GloriaLauncher skips exes whose name starts with "unins".
pub const UNINSTALLER_NAME: &str = "uninstall.exe";
pub const INSTALL_MANIFEST_NAME: &str = "install-manifest.json";
/// Hidden work folder inside the install folder (staging, backup).
pub const WORK_DIR: &str = ".gloria";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub main_exe: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub needs_webview2: bool,
    #[serde(default)]
    pub data_folders: Vec<String>,
    #[serde(default)]
    pub legacy_names: Vec<String>,
    #[serde(default)]
    pub app_user_model_id: Option<String>,
}

fn default_accent() -> String {
    "#9a9aa6".into()
}

/// What the stub reads from its payload: the validated manifest plus pack metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub app: AppManifest,
    /// Packed with `--allow-default-icon`: the UI shows a TEST BUILD badge.
    #[serde(default)]
    pub test_build: bool,
    /// gloria-pack version that built this setup.
    #[serde(default)]
    pub packer: String,
    /// Total size of the archive's files in bytes (for the disk-space line and EstimatedSize before install).
    #[serde(default)]
    pub unpacked_size: u64,
    #[serde(default)]
    pub file_count: u64,
}

impl AppManifest {
    pub fn from_toml(s: &str) -> Result<AppManifest, String> {
        let m: AppManifest = toml::from_str(s).map_err(|e| format!("gloria.toml: {e}"))?;
        m.validate()?;
        Ok(m)
    }

    pub fn parsed_version(&self) -> Version {
        Version::parse(&self.version).expect("validated")
    }

    pub fn data(&self) -> Vec<DataFolder> {
        self.data_folders.iter().filter_map(|d| parse_data_folder(d).ok()).collect()
    }

    /// Accent as RGB.
    pub fn accent_rgb(&self) -> (u8, u8, u8) {
        parse_hex_color(&self.accent).unwrap_or((0x9a, 0x9a, 0xa6))
    }

    pub fn validate(&self) -> Result<(), String> {
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 64
            && self.id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            && !self.id.starts_with('.');
        if !id_ok {
            return Err(format!("id {:?}: use 1-64 letters, digits, '.', '_' or '-'", self.id));
        }
        if self.name.trim().is_empty() || self.name.len() > 128 || self.name.chars().any(|c| c.is_control()) {
            return Err(format!("name {:?}: must be a non-empty single line", self.name));
        }
        if self.name != self.name.trim() {
            return Err(format!("name {:?}: no leading or trailing spaces", self.name));
        }
        if Version::parse(&self.version).is_none() {
            return Err(format!("version {:?}: not a version (expected e.g. 1.2.3 or 1.2.3-beta.1)", self.version));
        }
        let exe = self.main_exe.replace('/', "\\");
        let exe_ok = exe.to_ascii_lowercase().ends_with(".exe")
            && !exe.starts_with('\\')
            && !exe.contains(':')
            && exe.split('\\').all(|c| !c.is_empty() && c != "." && c != "..");
        if !exe_ok {
            return Err(format!("main_exe {:?}: must be a relative path to an .exe inside the app folder", self.main_exe));
        }
        let file = exe.rsplit('\\').next().unwrap_or("").to_ascii_lowercase();
        if file.starts_with("unins") {
            return Err(format!("main_exe {:?}: names starting with 'unins' are reserved for uninstallers", self.main_exe));
        }
        if parse_hex_color(&self.accent).is_none() {
            return Err(format!("accent {:?}: expected #rrggbb", self.accent));
        }
        for d in &self.data_folders {
            parse_data_folder(d)?;
        }
        if let Some(a) = &self.app_user_model_id {
            if a.is_empty() || a.len() > 128 || a.contains(char::is_whitespace) {
                return Err(format!("app_user_model_id {a:?}: no spaces, at most 128 characters"));
            }
        }
        Ok(())
    }

    /// Registry subkey under `...\CurrentVersion\Uninstall`.
    pub fn uninstall_key(&self) -> String {
        format!("{PUBLISHER}.{}", self.id)
    }

    /// `main_exe` with Windows separators.
    pub fn main_exe_rel(&self) -> String {
        self.main_exe.replace('/', "\\")
    }
}

pub fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

impl RuntimeManifest {
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("serializable")
    }
    pub fn from_json(b: &[u8]) -> Result<RuntimeManifest, String> {
        let m: RuntimeManifest = serde_json::from_slice(b).map_err(|e| format!("payload manifest: {e}"))?;
        m.app.validate()?;
        Ok(m)
    }

    /// The sample app a bare `--preview` shows.
    pub fn sample() -> RuntimeManifest {
        RuntimeManifest {
            app: AppManifest {
                id: "SampleApp".into(),
                name: "Sample App".into(),
                version: "1.0.0".into(),
                main_exe: "sample.exe".into(),
                icon: None,
                accent: "#4caf50".into(),
                description: Some("A preview of the GloriaSetup installer".into()),
                needs_webview2: false,
                data_folders: vec![r"%APPDATA%\com.gloria.sample".into(), r"%LOCALAPPDATA%\com.gloria.sample".into()],
                legacy_names: vec![],
                app_user_model_id: None,
            },
            test_build: true,
            packer: String::new(),
            unpacked_size: 48_600_000,
            file_count: 42,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OMNI: &str = r##"
id            = "OmniConvert"
name          = "OmniConvert"
version       = "0.2.0"
main_exe      = "omniconvert.exe"
icon          = "src-tauri/icons/icon.ico"
accent        = "#f59e0b"
description   = "Universal offline media & document converter"
needs_webview2 = true
data_folders  = ["%APPDATA%\\com.omniconvert.app", "%LOCALAPPDATA%\\com.omniconvert.app"]
legacy_names  = ["OmniConvert"]
app_user_model_id = "com.omniconvert.app"
"##;

    #[test]
    fn parses_design_example() {
        let m = AppManifest::from_toml(OMNI).unwrap();
        assert_eq!(m.uninstall_key(), "projectGloria.OmniConvert");
        assert_eq!(m.accent_rgb(), (0xf5, 0x9e, 0x0b));
        assert_eq!(m.data().len(), 2);
        let rt = RuntimeManifest { app: m, test_build: false, packer: "0.1.0".into(), unpacked_size: 1, file_count: 1 };
        assert_eq!(RuntimeManifest::from_json(&rt.to_json()).unwrap(), rt);
    }

    #[test]
    fn publisher_is_not_configurable() {
        let s = format!("{OMNI}\npublisher = \"Someone\"\n");
        assert!(AppManifest::from_toml(&s).is_err());
    }

    #[test]
    fn validation_errors() {
        let base = AppManifest::from_toml(OMNI).unwrap();
        let bad = |f: &dyn Fn(&mut AppManifest)| {
            let mut m = base.clone();
            f(&mut m);
            m.validate().is_err()
        };
        assert!(bad(&|m| m.id = "has space".into()));
        assert!(bad(&|m| m.id = String::new()));
        assert!(bad(&|m| m.name = " Lead".into()));
        assert!(bad(&|m| m.version = "one".into()));
        assert!(bad(&|m| m.main_exe = "../x.exe".into()));
        assert!(bad(&|m| m.main_exe = r"C:\x.exe".into()));
        assert!(bad(&|m| m.main_exe = "x.dll".into()));
        assert!(bad(&|m| m.main_exe = "bin/uninstall.exe".into()));
        assert!(bad(&|m| m.accent = "orange".into()));
        assert!(bad(&|m| m.data_folders = vec!["%APPDATA%".into()]));
        assert!(!bad(&|m| m.main_exe = "bin/app.exe".into()));
        assert!(RuntimeManifest::sample().app.validate().is_ok());
    }
}
