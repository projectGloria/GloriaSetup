//! Path guardrails shared by gloria-pack (pack time) and the stub (run time). See docs/design.md §5 and §9.

use std::path::{Component, Path, PathBuf, Prefix};

/// The three roots a data folder may start with.
pub const DATA_ROOTS: [&str; 3] = ["APPDATA", "LOCALAPPDATA", "USERPROFILE"];

/// Shared folders that may never be declared as (or be the parent chain of) app data.
const DENY: &[(&str, &[&str])] = &[
    ("APPDATA", &["microsoft", "temp", "programs"]),
    (
        "LOCALAPPDATA",
        &[
            "programs", "temp", "microsoft", "packages", "low", "virtualstore", "d3dscache", "crashdumps", "history",
            "temporary internet files", "application data", "comms", "connecteddevicesplatform", "publishers",
        ],
    ),
    (
        "USERPROFILE",
        &[
            "documents", "desktop", "downloads", "pictures", "music", "videos", "appdata", "onedrive", "favorites",
            "links", "contacts", "searches", "saved games", "3d objects", "source", "repos", ".ssh", "application data",
            "local settings", "my documents", "cookies", "recent", "sendto", "start menu", "templates", "printhood",
            "nethood", ".config", ".gnupg", ".aws", ".docker", ".vscode",
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
        if c.contains(['*', '?', ':', '%', '<', '>', '|', '"']) || c.chars().any(char::is_control) {
            return bad("wildcards, drive letters, variables and reserved characters are not allowed");
        }
        if c.ends_with(['.', ' ']) || c.starts_with(' ') {
            return bad("components may not start with a space or end with a dot or space");
        }
        if is_reserved_name(c) {
            return bad("reserved device names (CON, NUL, COM1, ...) are not allowed");
        }
        if is_short_name(c) {
            return bad("8.3 short names (like PROGRA~1) are not allowed; use the long name");
        }
        out.push(c.to_string());
    }
    // Deny-list: neither a shared folder nor anything inside one (`%APPDATA%\Microsoft\X` is as shared as
    // `%APPDATA%\Microsoft`, and `Documents\X` is user content, not app data).
    if let Some((_, deny)) = DENY.iter().find(|(r, _)| *r == root_up) {
        if deny.contains(&out[0].to_ascii_lowercase().as_str()) {
            return bad(if out.len() == 1 { "is a shared Windows folder" } else { "is inside a shared folder" });
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

/// `CON`, `NUL`, `COM1`, `lpt9.txt`, ...: names Windows maps to devices, with or without an extension.
pub fn is_reserved_name(c: &str) -> bool {
    let stem = c.split('.').next().unwrap_or(c).trim_end().to_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") {
        return true;
    }
    let digit = |s: &str| matches!(s, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³");
    ["COM", "LPT"].iter().any(|p| stem.strip_prefix(p).is_some_and(digit))
}

/// Looks like an 8.3 short name (`PROGRA~1`): it could alias a blocked folder, so the long name is required.
pub fn is_short_name(c: &str) -> bool {
    c.as_bytes().windows(2).any(|w| w[0] == b'~' && w[1].is_ascii_digit())
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
    /// A shared root like `%LOCALAPPDATA%` or the profile folder itself: fine as a parent, never as the folder.
    SharedRoot,
    BadCharacters,
}

/// Environment the install-folder check needs. Built from the real environment at run time.
#[derive(Debug, Clone, Default)]
pub struct SystemDirs {
    /// Never install in or under these.
    pub blocked: Vec<PathBuf>,
    /// Never install directly into these (shared roots whose subfolders are fine, like `%LOCALAPPDATA%`).
    pub roots: Vec<PathBuf>,
}

impl SystemDirs {
    /// `env` should give long (not 8.3) paths; the stub expands them before calling this.
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> SystemDirs {
        let get = |v: &str| env(v).filter(|x| !x.is_empty()).map(PathBuf::from);
        let mut blocked = Vec::new();
        for v in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "WINDIR", "SystemRoot", "ProgramData", "TEMP", "TMP"]
        {
            blocked.extend(get(v));
        }
        let mut roots = Vec::new();
        if let Some(home) = get("USERPROFILE") {
            roots.extend(home.parent().map(Path::to_path_buf));
            for sub in ["Desktop", "Documents", "Downloads", "AppData", "OneDrive"] {
                roots.push(home.join(sub));
            }
            roots.push(home);
        }
        roots.extend(get("APPDATA"));
        if let Some(local) = get("LOCALAPPDATA") {
            roots.push(local.join("Programs"));
            roots.push(local.join("Programs").join(SUITE_DIR));
            roots.push(local);
        }
        SystemDirs { blocked, roots }
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
        // Windows drops trailing dots/spaces (`Program Files.` is `Program Files`) and maps 8.3 names to long ones,
        // so either could slip past the folder checks below.
        if c == "."
            || c == ".."
            || c.contains(['*', '?', ':', '<', '>', '|', '"'])
            || c.chars().any(char::is_control)
            || c.ends_with(['.', ' '])
            || is_reserved_name(&c)
            || is_short_name(&c)
        {
            return Err(PathProblem::BadCharacters);
        }
    }
    if sys.blocked.iter().any(|b| is_within(p, b)) {
        return Err(PathProblem::SystemFolder);
    }
    if data.iter().any(|d| is_within(p, d) || is_within(d, p)) {
        return Err(PathProblem::DataFolder);
    }
    if sys.roots.iter().any(|r| norm(p) == norm(r)) {
        return Err(PathProblem::SharedRoot);
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
/// (`GloriaSetup-<id>-YYYYMMDD-HHMMSS[-n].log`), an uninstall trampoline copy (`GloriaSetup-<id>-<8 hex>.exe`) or the
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
        // `YYYYMMDD-HHMMSS`, or `YYYYMMDD-HHMMSS-<n>` when another run took that second's name.
        let mut parts = stamp.split('-');
        let (d, t, n, extra) = (parts.next(), parts.next(), parts.next(), parts.next());
        let n_ok = n.map_or(true, |n| (1..=2).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit()));
        return matches!((d, t), (Some(d), Some(t)) if digits(d, 8) && digits(t, 6)) && n_ok && extra.is_none();
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
            r"%APPDATA%\Microsoft\Windows\Foo",
            r"%LOCALAPPDATA%\Packages\Foo",
            r"%LOCALAPPDATA%\Microsoft\Edge",
            r"%APPDATA%\CON",
            r"%APPDATA%\Foo\nul.txt",
            r"%APPDATA%\com1",
            r"%APPDATA%\FOOBAR~1",
            r"%APPDATA%\Foo.",
            r"%USERPROFILE%\.config\foo",
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
                    "USERPROFILE" => r"C:\Users\you",
                    "APPDATA" => r"C:\Users\you\AppData\Roaming",
                    "LOCALAPPDATA" => r"C:\Users\you\AppData\Local",
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
        // Windows strips trailing dots/spaces and resolves 8.3 names, so both could alias a blocked folder.
        assert_eq!(ok(r"C:\Program Files.\Foo"), Err(PathProblem::BadCharacters));
        assert_eq!(ok(r"C:\Program Files \Foo"), Err(PathProblem::BadCharacters));
        assert_eq!(ok(r"C:\PROGRA~1\Foo"), Err(PathProblem::BadCharacters));
        assert_eq!(ok(r"D:\Apps\CON"), Err(PathProblem::BadCharacters));
        assert_eq!(ok(r"D:\Apps\lpt1.log\x"), Err(PathProblem::BadCharacters));
        // Shared roots are parents only.
        for r in [
            r"C:\Users",
            r"C:\Users\you",
            r"C:\Users\you\Desktop",
            r"C:\Users\you\Documents\",
            r"C:\Users\you\AppData\Local",
            r"C:\Users\you\AppData\Local\Programs",
            r"C:\Users\you\AppData\Local\Programs\Project Gloria Softwares",
        ] {
            assert_eq!(check_install_dir(Path::new(r), &s, &[]), Err(PathProblem::SharedRoot), "{r}");
        }
        assert_eq!(ok(r"C:\Users\you\Desktop\Foo"), Ok(()));
    }

    #[test]
    fn reserved_and_short_names() {
        for n in ["CON", "con", "Nul.txt", "COM1", "lpt9", "AUX ", "com\u{b9}", "CONIN$"] {
            assert!(is_reserved_name(n), "{n}");
        }
        for n in ["CONSOLE", "COM10", "nullx", "Comet", "lpt", "auxiliary.txt"] {
            assert!(!is_reserved_name(n), "{n}");
        }
        assert!(is_short_name("PROGRA~1") && is_short_name("a~9b"));
        assert!(!is_short_name("~tmp") && !is_short_name("x~") && !is_short_name("Foo"));
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
        for ok in [
            "GloriaSetup-Foo-20261123-101500.log",
            "GloriaSetup-Foo-20261123-101500-2.log",
            "gloriasetup-foo-0a1b2c3d.exe",
            "GloriaSetup-backup-Foo",
        ] {
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
            "GloriaSetup-Foo-20261123-101500-x.log",
            "GloriaSetup-Foo-20261123-101500-2-3.log",
            "other.log",
        ] {
            assert!(!is_temp_leftover(no, "Foo"), "{no}");
        }
    }
}
