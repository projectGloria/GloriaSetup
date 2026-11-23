//! Path guardrails shared by gloria-pack (pack time) and the stub (run time). See docs/design.md §5 and §9.

use std::path::{Component, Path, PathBuf, Prefix};

/// The three roots a data folder may start with.
pub const DATA_ROOTS: [&str; 3] = ["APPDATA", "LOCALAPPDATA", "USERPROFILE"];

/// Shared folders that may never be declared as (or be the parent chain of) app data.
const DENY: &[(&str, &[&str])] = &[
    ("APPDATA", &["microsoft", "temp", "programs"]),
    ("LOCALAPPDATA", &["programs", "temp", "microsoft", "packages", "low", "virtualstore", "d3dscache", "crashdumps"]),
    (
        "USERPROFILE",
        &[
            "documents", "desktop", "downloads", "pictures", "music", "videos", "appdata", "onedrive", "favorites",
            "links", "contacts", "searches", "saved games", "3d objects", "source", "repos", ".ssh",
        ],
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataFolder {
    /// e.g. "LOCALAPPDATA"
    pub root: String,
    /// Concrete components after the root, e.g. ["com.omniconvert.app"]
    pub rest: Vec<String>,
}

impl DataFolder {
    /// The declared form, `%ROOT%\a\b`.
    pub fn declared(&self) -> String {
        format!("%{}%\\{}", self.root, self.rest.join("\\"))
    }

    /// Expands with a lookup function (process env at run time, a fake map in tests).
    pub fn expand(&self, env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
        let base = env(&self.root)?;
        let base = PathBuf::from(base);
        // The root itself must be an absolute local path.
        if !is_local_absolute(&base) {
            return None;
        }
        let mut p = base;
        for c in &self.rest {
            p.push(c);
        }
        Some(p)
    }
}

/// Validates a declared data folder like `%APPDATA%\com.foo.app`.
pub fn parse_data_folder(s: &str) -> Result<DataFolder, String> {
    let bad = |why: &str| Err(format!("data folder {s:?}: {why}"));
    let t = s.trim();
    let Some(after) = t.strip_prefix('%') else {
        return bad("must start with %APPDATA%, %LOCALAPPDATA% or %USERPROFILE%");
    };
    let Some((root, rest)) = after.split_once('%') else {
        return bad("unterminated %VAR%");
    };
    let root_up = root.to_ascii_uppercase();
    if !DATA_ROOTS.contains(&root_up.as_str()) {
        return bad("must start with %APPDATA%, %LOCALAPPDATA% or %USERPROFILE%");
    }
    let Some(rest) = rest.strip_prefix(['\\', '/']) else {
        return bad("needs a folder after the root (a bare root is never allowed)");
    };
    let comps: Vec<&str> = rest.split(['\\', '/']).collect();
    if comps.iter().all(|c| c.is_empty()) {
        return bad("needs a folder after the root (a bare root is never allowed)");
    }
    let mut out = Vec::new();
    for (i, c) in comps.iter().enumerate() {
        if c.is_empty() {
            // Allow one trailing separator only.
            if i == comps.len() - 1 && i > 0 {
                continue;
            }
            return bad("empty path component");
        }
        if *c == "." || *c == ".." {
            return bad("'.' and '..' are not allowed");
        }
        if c.contains(['*', '?', ':', '%', '<', '>', '|', '"']) {
            return bad("wildcards, drive letters, variables and reserved characters are not allowed");
        }
        if c.ends_with(['.', ' ']) || c.starts_with(' ') {
            return bad("components may not start with a space or end with a dot or space");
        }
        out.push(c.to_string());
    }
    // Deny-list: the first component may not be a shared folder.
    if let Some((_, deny)) = DENY.iter().find(|(r, _)| *r == root_up) {
        if deny.contains(&out[0].to_ascii_lowercase().as_str()) && out.len() == 1 {
            return bad("is a shared Windows folder");
        }
        // Even deeper, Documents\X etc. is user content, not app data.
        if root_up == "USERPROFILE" && deny.contains(&out[0].to_ascii_lowercase().as_str()) {
            return bad("is inside a shared user folder");
        }
        if root_up == "LOCALAPPDATA" && ["programs", "temp"].contains(&out[0].to_ascii_lowercase().as_str()) {
            return bad("is inside a shared folder");
        }
    }
    Ok(DataFolder { root: root_up, rest: out })
}

pub fn is_local_absolute(p: &Path) -> bool {
    let mut comps = p.components();
    matches!(
        (comps.next(), comps.next()),
        (Some(Component::Prefix(px)), Some(Component::RootDir))
            if matches!(px.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    )
}

/// Lowercased, `/`→`\`, no trailing separator. Used for prefix comparisons on Windows paths.
pub fn norm(p: &Path) -> String {
    let mut s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    while s.ends_with('\\') && s.len() > 3 {
        s.pop();
    }
    s
}

/// `child` is `parent` or inside it.
pub fn is_within(child: &Path, parent: &Path) -> bool {
    let c = norm(child);
    let p = norm(parent);
    c == p || c.starts_with(&format!("{}\\", p.trim_end_matches('\\')))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathProblem {
    NotAbsolute,
    DriveRoot,
    Unc,
    /// Program Files, Windows, ProgramData, Temp: needs admin or is shared.
    SystemFolder,
    /// Overlaps a declared data folder.
    DataFolder,
    BadCharacters,
}

/// Environment the install-folder check needs. Built from the real environment at run time.
#[derive(Debug, Clone, Default)]
pub struct SystemDirs {
    pub blocked: Vec<PathBuf>,
}

impl SystemDirs {
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> SystemDirs {
        let mut blocked = Vec::new();
        for v in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "WINDIR", "SystemRoot", "ProgramData", "TEMP", "TMP"]
        {
            if let Some(x) = env(v).filter(|x| !x.is_empty()) {
                blocked.push(PathBuf::from(x));
            }
        }
        SystemDirs { blocked }
    }
}

/// Checks a chosen install folder (before the "append \<Name>" rule). Write probes happen in the stub.
pub fn check_install_dir(p: &Path, sys: &SystemDirs, data: &[PathBuf]) -> Result<(), PathProblem> {
    let s = p.to_string_lossy();
    if s.starts_with("\\\\") || s.starts_with("//") {
        return Err(PathProblem::Unc);
    }
    if !is_local_absolute(p) {
        return Err(PathProblem::NotAbsolute);
    }
    if p.components().count() <= 2 {
        return Err(PathProblem::DriveRoot);
    }
    for c in p.components().skip(2) {
        let c = c.as_os_str().to_string_lossy();
        if c == "." || c == ".." || c.contains(['*', '?', ':', '<', '>', '|', '"']) {
            return Err(PathProblem::BadCharacters);
        }
    }
    if sys.blocked.iter().any(|b| is_within(p, b)) {
        return Err(PathProblem::SystemFolder);
    }
    if data.iter().any(|d| is_within(p, d) || is_within(d, p)) {
        return Err(PathProblem::DataFolder);
    }
    Ok(())
}

/// The shared parent every projectGloria app installs under by default.
pub const SUITE_DIR: &str = "Project Gloria Softwares";

/// Default install folder: `%LOCALAPPDATA%\Programs\Project Gloria Softwares\<Name>`.
pub fn default_install_dir(local_app_data: &Path, name: &str) -> PathBuf {
    local_app_data.join("Programs").join(SUITE_DIR).join(sanitize_file_name(name))
}

/// `dir`'s parent when it is the shared [`SUITE_DIR`] folder (removed once the last app in it is gone).
pub fn suite_parent(dir: &Path) -> Option<&Path> {
    dir.parent().filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(SUITE_DIR)))
}

/// Is `file_name` (directly in `%TEMP%`) something GloriaSetup left there for app `id`: a run log
/// (`GloriaSetup-<id>-YYYYMMDD-HHMMSS.log`), an uninstall trampoline copy (`GloriaSetup-<id>-<8 hex>.exe`) or the
/// migration data backup (`GloriaSetup-backup-<id>`)? Strict, so app `foo` never matches app `foo-bar`'s files.
pub fn is_temp_leftover(file_name: &str, id: &str) -> bool {
    let f = file_name.to_ascii_lowercase();
    let id = id.to_ascii_lowercase();
    if f == format!("gloriasetup-backup-{id}") {
        return true;
    }
    let Some(rest) = f.strip_prefix(&format!("gloriasetup-{id}-")) else { return false };
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    if let Some(stamp) = rest.strip_suffix(".log") {
        return matches!(stamp.split_once('-'), Some((d, t)) if digits(d, 8) && digits(t, 6));
    }
    if let Some(hex) = rest.strip_suffix(".exe") {
        return hex.len() == 8 && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    false
}

/// Makes a display name safe as a single path component (used for the folder and .lnk names).
pub fn sanitize_file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    let s = s.trim().trim_end_matches('.').trim().to_string();
    if s.is_empty() {
        "App".into()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_data_folders() {
        let d = parse_data_folder(r"%APPDATA%\com.omniconvert.app").unwrap();
        assert_eq!(d.root, "APPDATA");
        assert_eq!(d.rest, vec!["com.omniconvert.app"]);
        assert_eq!(d.declared(), r"%APPDATA%\com.omniconvert.app");
        assert!(parse_data_folder(r"%localappdata%\Gloria\Dummy\").is_ok());
        assert!(parse_data_folder(r"%USERPROFILE%\.gloriadummy").is_ok());
    }

    #[test]
    fn rejects_bad_data_folders() {
        for s in [
            r"%APPDATA%",
            r"%APPDATA%\",
            r"%LOCALAPPDATA%",
            r"%USERPROFILE%",
            r"%USERPROFILE%\",
            r"%APPDATA%\..\x",
            r"%APPDATA%\a\..\..",
            r"%APPDATA%\.",
            r"%APPDATA%\a\\b",
            r"%APPDATA%\*",
            r"%APPDATA%\C:\x",
            r"C:\",
            r"C:\Users\you\AppData",
            r"\\server\share\x",
            r"%TEMP%\x",
            r"%WINDIR%\x",
            r"%APPDATA%x",
            r"%LOCALAPPDATA%\Programs",
            r"%LOCALAPPDATA%\Programs\Foo",
            r"%LOCALAPPDATA%\Temp",
            r"%APPDATA%\Microsoft",
            r"%USERPROFILE%\Documents",
            r"%USERPROFILE%\Documents\Foo",
            r"%USERPROFILE%\AppData\Roaming",
            r"%USERPROFILE%\Desktop",
            r"%APPDATA%\%OTHER%",
        ] {
            assert!(parse_data_folder(s).is_err(), "{s:?} should be rejected");
        }
    }

    #[test]
    fn expand_uses_env() {
        let d = parse_data_folder(r"%APPDATA%\com.x").unwrap();
        let env = |k: &str| (k == "APPDATA").then(|| r"C:\Users\you\AppData\Roaming".to_string());
        assert_eq!(d.expand(env), Some(PathBuf::from(r"C:\Users\you\AppData\Roaming\com.x")));
        // A relative or missing root expands to nothing.
        assert_eq!(d.expand(|_| Some("rel".into())), None);
        assert_eq!(d.expand(|_| None), None);
    }

    fn sys() -> SystemDirs {
        SystemDirs::from_env(|k| {
            Some(
                match k {
                    "ProgramFiles" | "ProgramW6432" => r"C:\Program Files",
                    "ProgramFiles(x86)" => r"C:\Program Files (x86)",
                    "WINDIR" | "SystemRoot" => r"C:\Windows",
                    "ProgramData" => r"C:\ProgramData",
                    "TEMP" | "TMP" => r"C:\Users\you\AppData\Local\Temp",
                    _ => return None,
                }
                .to_string(),
            )
        })
    }

    #[test]
    fn install_dir_rules() {
        let s = sys();
        let data = vec![PathBuf::from(r"C:\Users\you\AppData\Roaming\com.x")];
        let ok = |p: &str| check_install_dir(Path::new(p), &s, &data);
        assert_eq!(ok(r"C:\Users\you\AppData\Local\Programs\Foo"), Ok(()));
        assert_eq!(ok(r"D:\Apps\Foo"), Ok(()));
        assert_eq!(ok(r"C:\"), Err(PathProblem::DriveRoot));
        assert_eq!(ok(r"D:"), Err(PathProblem::NotAbsolute));
        assert_eq!(ok(r"relative\x"), Err(PathProblem::NotAbsolute));
        assert_eq!(ok(r"\\srv\share\x"), Err(PathProblem::Unc));
        assert_eq!(ok(r"C:\Program Files\Foo"), Err(PathProblem::SystemFolder));
        assert_eq!(ok(r"c:\program files (x86)\Foo"), Err(PathProblem::SystemFolder));
        assert_eq!(ok(r"C:\Windows\Foo"), Err(PathProblem::SystemFolder));
        assert_eq!(ok(r"C:\ProgramData\Foo"), Err(PathProblem::SystemFolder));
        assert_eq!(ok(r"C:\Users\you\AppData\Local\Temp\Foo"), Err(PathProblem::SystemFolder));
        assert_eq!(ok(r"C:\Users\you\AppData\Roaming\com.x\bin"), Err(PathProblem::DataFolder));
        assert_eq!(ok(r"C:\Users\you\AppData\Roaming"), Err(PathProblem::DataFolder));
        // "Program Files Extra" is not inside "Program Files".
        assert_eq!(ok(r"C:\Program Files Extra\Foo"), Ok(()));
    }

    #[test]
    fn names() {
        assert_eq!(sanitize_file_name("Omni: Convert?"), "Omni_ Convert_");
        assert_eq!(sanitize_file_name(" .. "), "App");
        assert_eq!(
            default_install_dir(Path::new(r"C:\Users\you\AppData\Local"), "GloriaDummy"),
            PathBuf::from(r"C:\Users\you\AppData\Local\Programs\Project Gloria Softwares\GloriaDummy")
        );
        let d = default_install_dir(Path::new(r"C:\Users\you\AppData\Local"), "X");
        assert_eq!(suite_parent(&d), Some(Path::new(r"C:\Users\you\AppData\Local\Programs\Project Gloria Softwares")));
        assert_eq!(suite_parent(Path::new(r"D:\Apps\X")), None);
    }

    #[test]
    fn temp_leftovers() {
        for ok in ["GloriaSetup-Foo-20261123-101500.log", "gloriasetup-foo-0a1b2c3d.exe", "GloriaSetup-backup-Foo"] {
            assert!(is_temp_leftover(ok, "Foo"), "{ok}");
        }
        for no in [
            "GloriaSetup-Foo-Bar-20261123-101500.log",
            "GloriaSetup-Foo-bar.exe",
            "GloriaSetup-Foo-0a1b2c3.exe",
            "GloriaSetup-Foo-2026112-101500.log",
            "GloriaSetup-FooBar-20261123-101500.log",
            "GloriaSetup-backup-Foo-old",
            "GloriaSetup-Foo-20261123-101500.txt",
            "other.log",
        ] {
            assert!(!is_temp_leftover(no, "Foo"), "{no}");
        }
    }
}
