//! Installs of the same app made by an older installer (NSIS / Tauri, MSI, Inno Setup), found in the Windows
//! Uninstall keys (docs/design.md §8). Pure logic: the stub reads the registry and hands the entries in here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::manifest::AppManifest;
use crate::paths::{is_local_absolute, is_within, norm};
use crate::registry::launcher::{normalize, strip_version};

/// Where an Uninstall entry was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    /// HKCU: per-user, no elevation needed.
    User,
    /// HKLM: installed for all users, the old uninstaller needs admin.
    Machine,
}

/// The registry view the entry came from (HKLM only; HKCU's Uninstall key is shared by both views).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Native,
    Wow32,
}

/// One raw Uninstall subkey. Value names are matched case-insensitively.
#[derive(Debug, Clone)]
pub struct Entry {
    pub hive: Hive,
    pub view: View,
    /// Subkey name under `...\CurrentVersion\Uninstall`.
    pub key: String,
    pub values: BTreeMap<String, String>,
}

impl Entry {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()).filter(|v| !v.is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Nsis,
    Msi,
    Inno,
    Other,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Nsis => "NSIS",
            Kind::Msi => "MSI",
            Kind::Inno => "Inno Setup",
            Kind::Other => "EXE",
        }
    }
}

/// How to run the old uninstaller without UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub exe: PathBuf,
    /// Raw argument string, passed as is.
    pub args: String,
}

#[derive(Debug, Clone)]
pub struct Found {
    pub entry: Entry,
    pub kind: Kind,
    pub version: String,
    /// The old install folder, when the entry tells us.
    pub dir: Option<PathBuf>,
    /// `None` when the entry has no silent way to uninstall (silent mode then refuses).
    pub quiet: Option<Command>,
    /// The NSIS uninstaller, which `_?=` leaves behind and we delete afterwards.
    pub leftover: Option<PathBuf>,
}

impl Found {
    pub fn needs_admin(&self) -> bool {
        self.entry.hive == Hive::Machine
    }
}

/// Splits `"C:\x\unins.exe" /a b` or `C:\x y\unins.exe /a` into the exe and the rest.
pub fn split_command(cmd: &str) -> Option<(PathBuf, String)> {
    let s = cmd.trim();
    if let Some(rest) = s.strip_prefix('"') {
        let end = rest.find('"')?;
        let exe = &rest[..end];
        (!exe.is_empty()).then(|| (PathBuf::from(exe), rest[end + 1..].trim().to_string()))
    } else {
        let lower = s.to_ascii_lowercase();
        let end = lower.find(".exe").map(|i| i + 4).unwrap_or_else(|| s.find(' ').unwrap_or(s.len()));
        let exe = &s[..end];
        (!exe.is_empty()).then(|| (PathBuf::from(exe), s[end..].trim().to_string()))
    }
}

/// The `{GUID}` of an MSI product: the key name itself, or the one in `MsiExec.exe /X{...}`.
pub fn msi_guid(e: &Entry) -> Option<String> {
    let is_guid = |s: &str| s.len() == 38 && s.starts_with('{') && s.ends_with('}') && s[1..37].chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    if is_guid(&e.key) {
        return Some(e.key.clone());
    }
    let u = e.get("UninstallString")?;
    let i = u.find('{')?;
    let g = u.get(i..i + 38)?;
    is_guid(g).then(|| g.to_string())
}

/// Which folders may be taken as an old install's own folder. The migration closes apps running from that folder and
/// removes it once empty, so it must be the app's own folder: never a drive root, never inside Windows, and never
/// a shared folder itself (`Program Files`, `%LOCALAPPDATA%`, the user profile...), only a folder inside one. An
/// entry with no `InstallLocation` and an uninstaller like `C:\Windows\System32\msiexec.exe` or `rundll32.exe` would
/// otherwise make System32 "the app's folder".
#[derive(Debug, Clone, Default)]
pub struct Guard {
    /// May contain an app folder but never be one (nor be inside one).
    pub shared: Vec<PathBuf>,
    /// Nothing inside these is an app folder.
    pub system: Vec<PathBuf>,
}

impl Guard {
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Guard {
        let get = |v: &str| env(v).filter(|x| !x.is_empty()).map(PathBuf::from);
        let mut shared: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP", "PUBLIC"]
            .iter()
            .filter_map(|v| get(v))
            .collect();
        if let Some(l) = get("LOCALAPPDATA") {
            shared.push(l.join("Programs"));
        }
        if let Some(u) = get("USERPROFILE") {
            shared.extend(["Desktop", "Documents", "Downloads", r"AppData\Local\Temp"].iter().map(|x| u.join(x)));
            if let Some(users) = u.parent() {
                shared.push(users.to_path_buf());
            }
        }
        let system = ["WINDIR", "SystemRoot"].iter().filter_map(|v| get(v)).collect();
        Guard { shared, system }
    }

    pub fn is_app_folder(&self, p: &Path) -> bool {
        is_local_absolute(p)
            && p.components().count() > 2
            && !self.system.iter().any(|s| is_within(p, s))
            && !self.shared.iter().any(|s| is_within(s, p))
    }

    fn keep(&self, p: Option<PathBuf>) -> Option<PathBuf> {
        p.filter(|p| self.is_app_folder(p) && !norm(p).contains("\\..\\"))
    }
}

fn unquote(s: &str) -> PathBuf {
    PathBuf::from(s.trim().trim_matches('"').trim_end_matches('\\'))
}

/// Does this entry belong to `app`? Matched like GloriaLauncher does: normalized display name without a trailing
/// version, against the app name and its `legacy_names`. Our own `projectGloria.*` keys never match.
pub fn matches(app: &AppManifest, e: &Entry) -> bool {
    if e.key.to_ascii_lowercase().starts_with("projectgloria.") || e.get("GloriaSetup").is_some() {
        return false;
    }
    if e.get("SystemComponent") == Some("1") || e.get("ParentKeyName").is_some() {
        return false;
    }
    let Some(name) = e.get("DisplayName") else { return false };
    let n = normalize(strip_version(name));
    !n.is_empty() && std::iter::once(&app.name).chain(&app.legacy_names).any(|w| normalize(w) == n)
}

/// Classifies an entry and builds its silent uninstall command. `is_nsis` checks the uninstaller exe (the stub
/// looks for the `NullsoftInst` marker in its bytes). A folder `guard` rejects is dropped (`dir: None`).
pub fn classify(e: Entry, is_nsis: &dyn Fn(&Path) -> bool, guard: &Guard) -> Found {
    let version = e.get("DisplayVersion").unwrap_or("").trim_start_matches(['v', 'V']).to_string();
    let uninstall = e.get("UninstallString").and_then(split_command);
    // A rejected InstallLocation isn't replaced by a guess from the uninstaller's folder.
    let raw_location = e.get("InstallLocation").map(unquote);
    let has_location = raw_location.is_some();
    let location = guard.keep(raw_location);
    let exe_dir = |x: &PathBuf| if has_location { None } else { guard.keep(x.parent().map(Path::to_path_buf)) };
    let quiet_str = e.get("QuietUninstallString").and_then(split_command).filter(|(exe, _)| exe.is_absolute());

    let msi = e.get("WindowsInstaller") == Some("1") || uninstall.as_ref().is_some_and(|(x, _)| x.file_name().is_some_and(|f| f.eq_ignore_ascii_case("msiexec.exe")));
    if msi {
        if let Some(g) = msi_guid(&e) {
            let quiet = Some(Command { exe: PathBuf::from("msiexec.exe"), args: format!("/X{g} /qn /norestart") });
            return Found { entry: e, kind: Kind::Msi, version, dir: location, quiet, leftover: None };
        }
    }

    let inno = e.key.to_ascii_lowercase().ends_with("_is1") || e.get("Inno Setup: App Path").is_some();
    if inno {
        let dir = guard.keep(e.get("Inno Setup: App Path").map(unquote)).or(location);
        let quiet = uninstall.as_ref().filter(|(x, _)| x.is_absolute()).map(|(x, _)| Command { exe: x.clone(), args: "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART".into() });
        return Found { entry: e, kind: Kind::Inno, version, dir, quiet, leftover: None };
    }

    if let Some((x, _)) = uninstall.as_ref().filter(|(x, _)| x.is_absolute() && is_nsis(x)) {
        // `_?=` makes NSIS run in place and wait instead of copying itself to %TEMP% and returning at once. It must be
        // the last argument and unquoted.
        let dir = location.clone().or_else(|| exe_dir(x));
        if let Some(d) = &dir {
            let quiet = Some(Command { exe: x.clone(), args: format!("/S _?={}", d.display()) });
            return Found { entry: e, kind: Kind::Nsis, version, dir, quiet, leftover: Some(x.clone()) };
        }
    }

    let dir = location.or_else(|| uninstall.as_ref().and_then(|(x, _)| exe_dir(x)));
    let quiet = quiet_str.map(|(exe, args)| Command { exe, args });
    Found { entry: e, kind: Kind::Other, version, dir, quiet, leftover: None }
}

/// Every legacy install of `app` in `entries`, per-user ones first, with duplicates (same key seen in both views)
/// dropped.
pub fn find(app: &AppManifest, entries: Vec<Entry>, is_nsis: &dyn Fn(&Path) -> bool, guard: &Guard) -> Vec<Found> {
    let mut seen: Vec<(Hive, String)> = Vec::new();
    let mut out: Vec<Found> = Vec::new();
    let mut entries: Vec<Entry> = entries.into_iter().filter(|e| matches(app, e)).collect();
    entries.sort_by_key(|e| (e.hive == Hive::Machine, e.view == View::Wow32));
    for e in entries {
        let k = (e.hive, e.key.to_ascii_lowercase());
        if seen.contains(&k) {
            continue;
        }
        seen.push(k);
        out.push(classify(e, is_nsis, guard));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> AppManifest {
        AppManifest::from_toml(
            r##"
id = "OmniConvert"
name = "Omni Convert"
version = "2.0.0"
main_exe = "omni.exe"
accent = "#3b82f6"
legacy_names = ["OmniConverter"]
"##,
        )
        .unwrap()
    }

    fn entry(hive: Hive, key: &str, vals: &[(&str, &str)]) -> Entry {
        Entry { hive, view: View::Native, key: key.into(), values: vals.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    #[test]
    fn splits_commands() {
        assert_eq!(split_command(r#""C:\a b\unins000.exe" /x"#), Some((PathBuf::from(r"C:\a b\unins000.exe"), "/x".into())));
        assert_eq!(split_command(r"C:\a b\Uninstall.EXE /S"), Some((PathBuf::from(r"C:\a b\Uninstall.EXE"), "/S".into())));
        assert_eq!(split_command(r"MsiExec.exe /X{0}"), Some((PathBuf::from("MsiExec.exe"), "/X{0}".into())));
        assert_eq!(split_command(r#""""#), None);
    }

    #[test]
    fn matches_names_and_skips_ours() {
        let a = app();
        let m = |k: &str, n: &str| matches(&a, &entry(Hive::User, k, &[("DisplayName", n)]));
        assert!(m("Omni Convert", "Omni Convert"));
        assert!(m("x", "omni-convert 1.4.2"));
        assert!(m("x", "OmniConverter v1.0"));
        assert!(!m("x", "Omni Convert Helper"));
        assert!(!m("projectGloria.OmniConvert", "Omni Convert"));
        assert!(!matches(&a, &entry(Hive::User, "x", &[("DisplayName", "Omni Convert"), ("GloriaSetup", "0.1.0")])));
        assert!(!matches(&a, &entry(Hive::User, "x", &[("DisplayName", "Omni Convert"), ("SystemComponent", "1")])));
        assert!(!matches(&a, &entry(Hive::User, "x", &[])));
    }

    #[test]
    fn nsis_runs_in_place() {
        let e = entry(
            Hive::User,
            "Omni Convert",
            &[
                ("DisplayName", "Omni Convert"),
                ("DisplayVersion", "1.4.2"),
                ("UninstallString", r#""C:\Users\you\AppData\Local\Omni Convert\uninstall.exe""#),
                ("QuietUninstallString", r#""C:\Users\you\AppData\Local\Omni Convert\uninstall.exe" /S"#),
                ("InstallLocation", r#""C:\Users\you\AppData\Local\Omni Convert""#),
            ],
        );
        let f = classify(e, &|_| true, &Guard::default());
        assert_eq!(f.kind, Kind::Nsis);
        assert_eq!(f.version, "1.4.2");
        let q = f.quiet.unwrap();
        assert_eq!(q.args, r"/S _?=C:\Users\you\AppData\Local\Omni Convert");
        assert_eq!(f.leftover, Some(PathBuf::from(r"C:\Users\you\AppData\Local\Omni Convert\uninstall.exe")));
        assert!(!f.entry.hive.eq(&Hive::Machine));
    }

    #[test]
    fn not_nsis_uses_quiet_string() {
        let e = entry(
            Hive::User,
            "x",
            &[("DisplayName", "Omni Convert"), ("UninstallString", r"C:\o\un.exe"), ("QuietUninstallString", r#""C:\o\un.exe" --silent"#)],
        );
        let f = classify(e, &|_| false, &Guard::default());
        assert_eq!(f.kind, Kind::Other);
        assert_eq!(f.quiet, Some(Command { exe: PathBuf::from(r"C:\o\un.exe"), args: "--silent".into() }));
        assert_eq!(f.dir, Some(PathBuf::from(r"C:\o")));
        let none = classify(entry(Hive::User, "x", &[("DisplayName", "Omni Convert"), ("UninstallString", r"C:\o\un.exe")]), &|_| false, &Guard::default());
        assert!(none.quiet.is_none());
    }

    #[test]
    fn msi_and_inno() {
        let g = "{12345678-1234-1234-1234-123456789ABC}";
        let f = classify(entry(Hive::Machine, g, &[("DisplayName", "Omni Convert"), ("WindowsInstaller", "1")]), &|_| false, &Guard::default());
        assert_eq!(f.kind, Kind::Msi);
        assert_eq!(f.quiet.as_ref().unwrap().args, format!("/X{g} /qn /norestart"));
        assert!(f.needs_admin());
        let f = classify(entry(Hive::User, "x", &[("UninstallString", &format!("MsiExec.exe /I{g}"))]), &|_| false, &Guard::default());
        assert_eq!(f.kind, Kind::Msi);

        let f = classify(
            entry(
                Hive::User,
                "{ABC}_is1",
                &[("UninstallString", r#""C:\o\unins000.exe""#), ("Inno Setup: App Path", r"C:\o"), ("QuietUninstallString", r#""C:\o\unins000.exe" /SILENT"#)],
            ),
            &|_| true,
            &Guard::default(),
        );
        assert_eq!(f.kind, Kind::Inno);
        assert_eq!(f.quiet, Some(Command { exe: PathBuf::from(r"C:\o\unins000.exe"), args: "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART".into() }));
        assert_eq!(f.dir, Some(PathBuf::from(r"C:\o")));
    }

    #[test]
    fn never_takes_a_shared_or_system_folder_as_the_app_folder() {
        let g = Guard::from_env(|v| {
            Some(match v {
                "ProgramFiles" => r"C:\Program Files",
                "LOCALAPPDATA" => r"C:\Users\you\AppData\Local",
                "USERPROFILE" => r"C:\Users\you",
                "WINDIR" => r"C:\Windows",
                _ => return None,
            })
            .map(String::from)
        });
        for bad in [r"C:\", r"C:\Windows\System32", r"C:\Program Files", r"C:\Users\you", r"C:\Users", r"C:\Users\you\AppData\Local\Programs", r"C:\Users\you\Desktop", r"relative\dir"] {
            assert!(!g.is_app_folder(Path::new(bad)), "{bad}");
        }
        for ok in [r"C:\Program Files\Omni Convert", r"C:\Users\you\AppData\Local\Omni Convert", r"D:\Apps\Omni"] {
            assert!(g.is_app_folder(Path::new(ok)), "{ok}");
        }
        // No InstallLocation and a system uninstaller: no folder, so nothing is closed or removed there.
        let e = entry(Hive::Machine, "x", &[("DisplayName", "Omni Convert"), ("QuietUninstallString", r#""C:\Windows\System32\reg.exe" delete X /f"#), ("UninstallString", r"C:\Windows\System32\reg.exe delete X /f")]);
        let f = classify(e, &|_| true, &g);
        assert_eq!(f.kind, Kind::Other, "no folder for _?=, so not run as NSIS");
        assert_eq!(f.dir, None);
        assert!(f.quiet.is_some());
        let e = entry(Hive::User, "x", &[("DisplayName", "Omni Convert"), ("InstallLocation", r"C:\Users\you\AppData\Local"), ("UninstallString", r"C:\Users\you\AppData\Local\Omni\un.exe")]);
        assert_eq!(classify(e, &|_| false, &g).dir, None, "InstallLocation rejected; the exe folder isn't guessed instead");
    }

    #[test]
    fn finds_user_first_without_duplicates() {
        let a = app();
        let mut w = entry(Hive::Machine, "Omni", &[("DisplayName", "Omni Convert")]);
        w.view = View::Wow32;
        let list = vec![
            w,
            entry(Hive::Machine, "Omni", &[("DisplayName", "Omni Convert")]),
            entry(Hive::User, "Omni", &[("DisplayName", "Omni Convert")]),
            entry(Hive::User, "omni", &[("DisplayName", "Omni Convert")]),
            entry(Hive::User, "Other", &[("DisplayName", "Something else")]),
        ];
        let f = find(&a, list, &|_| false, &Guard::default());
        assert_eq!(f.iter().map(|f| (f.entry.hive, f.entry.view)).collect::<Vec<_>>(), vec![(Hive::User, View::Native), (Hive::Machine, View::Native)]);
    }
}
