//! `<install>\install-manifest.json`: everything an install created, so update and uninstall touch only that.
//! See docs/design.md §5. `remove_dir_all(InstallLocation)` is never used.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::archive::{safe_rel_path, FileEntry};
use crate::manifest::{INSTALL_MANIFEST_NAME, WORK_DIR};
use crate::paths::is_within;

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    /// Missing in manifests written before the Start Menu shortcut became optional: those always had one.
    #[serde(default = "yes")]
    pub start_menu_shortcut: bool,
    pub desktop_shortcut: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { start_menu_shortcut: true, desktop_shortcut: false }
    }
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallManifest {
    pub schema: u32,
    pub app_id: String,
    pub name: String,
    pub version: String,
    /// Unix seconds.
    pub installed_at: u64,
    /// GloriaSetup version that wrote this file.
    pub gloria_setup: String,
    pub files: Vec<FileEntry>,
    /// Directories (relative) the install created, parents before children.
    pub dirs: Vec<String>,
    /// The install folder itself was created by us (so it may be removed once empty).
    pub created_install_dir: bool,
    /// Absolute paths of .lnk files we created.
    pub shortcuts: Vec<String>,
    pub options: Options,
    /// Declared data folders (`%APPDATA%\...`), as packed.
    pub data_folders: Vec<String>,
}

impl InstallManifest {
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn load(install_dir: &Path) -> io::Result<InstallManifest> {
        let b = fs::read(install_dir.join(INSTALL_MANIFEST_NAME))?;
        let m: InstallManifest = serde_json::from_slice(&b).map_err(io::Error::other)?;
        if m.schema != SCHEMA {
            return Err(io::Error::other(format!("install-manifest schema {} is not supported", m.schema)));
        }
        Ok(m)
    }

    pub fn save(&self, install_dir: &Path) -> io::Result<()> {
        let tmp = install_dir.join(format!("{INSTALL_MANIFEST_NAME}.tmp"));
        fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(io::Error::other)?)?;
        fs::rename(&tmp, install_dir.join(INSTALL_MANIFEST_NAME))
    }

    /// Directories (relative) that would be needed to hold `files`, parents first.
    pub fn dirs_for(files: &[FileEntry]) -> Vec<String> {
        let mut dirs: Vec<String> = Vec::new();
        for f in files {
            let parts: Vec<&str> = f.path.split('\\').collect();
            for i in 1..parts.len() {
                let d = parts[..i].join("\\");
                if !dirs.iter().any(|x| x.eq_ignore_ascii_case(&d)) {
                    dirs.push(d);
                }
            }
        }
        dirs.sort_by_key(|d| d.matches('\\').count());
        dirs
    }
}

/// One step of a removal, for logging and dry runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    File(PathBuf),
    Shortcut(PathBuf),
    Dir(PathBuf),
}

#[derive(Debug, Default)]
pub struct RemoveReport {
    pub done: Vec<Removal>,
    /// Paths that could not be removed (locked, or a directory that still has foreign files).
    pub skipped: Vec<(PathBuf, String)>,
}

/// Plans the removal of everything `m` lists. Entries that would leave `install_dir` are refused, so a tampered
/// manifest cannot delete outside the folder. Directories come deepest first and are removed only if empty.
pub fn plan_removal(install_dir: &Path, m: &InstallManifest) -> Vec<Removal> {
    let mut out = Vec::new();
    for s in &m.shortcuts {
        let p = PathBuf::from(s);
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) && crate::paths::is_local_absolute(&p) {
            out.push(Removal::Shortcut(p));
        }
    }
    for f in &m.files {
        if let Some(rel) = safe_rel_path(&f.path) {
            let p = install_dir.join(rel);
            if is_within(&p, install_dir) {
                out.push(Removal::File(p));
            }
        }
    }
    out.push(Removal::File(install_dir.join(INSTALL_MANIFEST_NAME)));
    let mut dirs: Vec<PathBuf> = m
        .dirs
        .iter()
        .filter_map(|d| safe_rel_path(d))
        .map(|r| install_dir.join(r))
        .chain(std::iter::once(install_dir.join(WORK_DIR)))
        .collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    out.extend(dirs.into_iter().map(Removal::Dir));
    if m.created_install_dir {
        out.push(Removal::Dir(install_dir.to_path_buf()));
    }
    out
}

/// Executes a removal plan. Never follows links: a reparse point is unlinked, not traversed. With `dry_run`, only
/// reports. `skip` lets the caller keep a file (e.g. the running uninstaller on a dry run).
pub fn execute_removal(plan: &[Removal], dry_run: bool) -> RemoveReport {
    let mut r = RemoveReport::default();
    for step in plan {
        let (p, is_dir) = match step {
            Removal::File(p) | Removal::Shortcut(p) => (p, false),
            Removal::Dir(p) => (p, true),
        };
        let Ok(md) = fs::symlink_metadata(p) else { continue };
        if dry_run {
            let empty = !is_dir || fs::read_dir(p).map(|mut d| d.next().is_none()).unwrap_or(false);
            if empty || !is_dir {
                r.done.push(step.clone());
            }
            continue;
        }
        let res = if md.file_type().is_symlink() {
            // Unlink the link itself (file or directory symlink / junction).
            fs::remove_file(p).or_else(|_| fs::remove_dir(p))
        } else if is_dir {
            fs::remove_dir(p) // fails if not empty: foreign files stay
        } else {
            remove_file_force(p)
        };
        match res {
            Ok(()) => r.done.push(step.clone()),
            Err(e) => {
                if !(is_dir && dir_not_empty(&e)) {
                    r.skipped.push((p.clone(), e.to_string()));
                }
            }
        }
    }
    r
}

fn dir_not_empty(e: &io::Error) -> bool {
    // ERROR_DIR_NOT_EMPTY = 145
    e.raw_os_error() == Some(145) || e.kind() == io::ErrorKind::DirectoryNotEmpty
}

/// Removes a file, clearing the read-only attribute first if needed.
pub fn remove_file_force(p: &Path) -> io::Result<()> {
    match fs::remove_file(p) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            let mut perm = fs::metadata(p)?.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            fs::set_permissions(p, perm)?;
            fs::remove_file(p)
        }
        r => r,
    }
}

/// Deletes a user-data folder tree without following links: reparse points (symlinks, junctions) are unlinked, never
/// traversed. Used only when the user chose to remove their data.
pub fn remove_tree_no_follow(p: &Path, dry_run: bool, log: &mut dyn FnMut(&Path)) -> io::Result<()> {
    let md = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if md.file_type().is_symlink() || is_reparse_point(&md) {
        log(p);
        if !dry_run {
            fs::remove_dir(p).or_else(|_| fs::remove_file(p))?;
        }
        return Ok(());
    }
    if md.is_dir() {
        for e in fs::read_dir(p)? {
            remove_tree_no_follow(&e?.path(), dry_run, log)?;
        }
        log(p);
        if !dry_run {
            fs::remove_dir(p)?;
        }
    } else {
        log(p);
        if !dry_run {
            remove_file_force(p)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn is_reparse_point(md: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
}
#[cfg(not(windows))]
pub fn is_reparse_point(_: &fs::Metadata) -> bool {
    false
}

/// Total size of a folder tree in bytes (links not followed). Missing folder → 0.
pub fn tree_size(p: &Path) -> u64 {
    let Ok(md) = fs::symlink_metadata(p) else { return 0 };
    if md.file_type().is_symlink() || is_reparse_point(&md) {
        return 0;
    }
    if md.is_file() {
        return md.len();
    }
    fs::read_dir(p).map(|rd| rd.flatten().map(|e| tree_size(&e.path())).sum()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::sha256_hex;

    fn entry(p: &str, data: &[u8]) -> FileEntry {
        FileEntry { path: p.into(), size: data.len() as u64, sha256: sha256_hex(data) }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gloria-core-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn manifest(files: Vec<FileEntry>, created: bool) -> InstallManifest {
        InstallManifest {
            schema: SCHEMA,
            app_id: "GloriaDummy".into(),
            name: "GloriaDummy".into(),
            version: "0.1.0".into(),
            installed_at: 0,
            gloria_setup: "0.1.0".into(),
            dirs: InstallManifest::dirs_for(&files),
            files,
            created_install_dir: created,
            shortcuts: vec![],
            options: Options::default(),
            data_folders: vec![],
        }
    }

    #[test]
    fn removes_only_tracked_files_and_keeps_siblings() {
        let root = tmp("rm");
        let inst = root.join("GloriaDummy");
        fs::create_dir_all(inst.join("res")).unwrap();
        fs::write(inst.join("app.exe"), b"a").unwrap();
        fs::write(inst.join("uninstall.exe"), b"u").unwrap();
        fs::write(inst.join("res").join("x.dat"), b"x").unwrap();
        // Not ours:
        fs::write(inst.join("notes.txt"), b"mine").unwrap();
        fs::write(inst.join("res").join("user.cfg"), b"mine").unwrap();
        fs::write(root.join("sibling.txt"), b"sibling").unwrap();
        let m = manifest(
            vec![entry("app.exe", b"a"), entry("uninstall.exe", b"u"), entry("res\\x.dat", b"x")],
            true,
        );
        m.save(&inst).unwrap();
        assert_eq!(InstallManifest::load(&inst).unwrap(), m);

        let plan = plan_removal(&inst, &m);
        let dry = execute_removal(&plan, true);
        assert!(inst.join("app.exe").exists(), "dry run must not delete");
        assert!(!dry.done.is_empty());

        let rep = execute_removal(&plan, false);
        assert!(rep.skipped.is_empty(), "{:?}", rep.skipped);
        assert!(!inst.join("app.exe").exists());
        assert!(!inst.join("uninstall.exe").exists());
        assert!(!inst.join(INSTALL_MANIFEST_NAME).exists());
        assert!(inst.join("notes.txt").exists());
        assert!(inst.join("res").join("user.cfg").exists());
        assert!(root.join("sibling.txt").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn removes_folders_when_empty() {
        let root = tmp("rm2");
        let inst = root.join("App");
        fs::create_dir_all(inst.join("a").join("b")).unwrap();
        fs::write(inst.join("a").join("b").join("f"), b"f").unwrap();
        let m = manifest(vec![entry("a\\b\\f", b"f")], true);
        m.save(&inst).unwrap();
        let rep = execute_removal(&plan_removal(&inst, &m), false);
        assert!(rep.skipped.is_empty());
        assert!(!inst.exists());
        assert!(root.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tampered_manifest_cannot_escape() {
        let root = tmp("rm3");
        let inst = root.join("App");
        fs::create_dir_all(&inst).unwrap();
        fs::write(root.join("victim.txt"), b"v").unwrap();
        let mut m = manifest(vec![], false);
        m.files.push(entry("..\\victim.txt", b"v"));
        m.files.push(FileEntry { path: root.join("victim.txt").to_string_lossy().into(), size: 1, sha256: String::new() });
        m.dirs.push("..".into());
        execute_removal(&plan_removal(&inst, &m), false);
        assert!(root.join("victim.txt").exists());
        assert!(inst.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn purge_does_not_follow_junctions() {
        let root = tmp("purge");
        let data = root.join("data");
        let outside = root.join("outside");
        fs::create_dir_all(data.join("sub")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(data.join("sub").join("settings.json"), b"{}").unwrap();
        fs::write(outside.join("keep.txt"), b"keep").unwrap();
        // A junction inside the data folder pointing outside it (mklink /J needs no admin rights).
        let st = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(data.join("link"))
            .arg(&outside)
            .output()
            .unwrap();
        assert!(st.status.success(), "mklink failed: {st:?}");
        assert_eq!(tree_size(&data), 2);
        let mut seen = 0;
        remove_tree_no_follow(&data, true, &mut |_| seen += 1).unwrap();
        assert!(data.exists() && seen >= 3);
        remove_tree_no_follow(&data, false, &mut |_| {}).unwrap();
        assert!(!data.exists());
        assert!(outside.join("keep.txt").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn dirs_for_lists_parents_first() {
        let d = InstallManifest::dirs_for(&[entry("a\\b\\c.txt", b""), entry("a\\d.txt", b""), entry("e.txt", b"")]);
        assert_eq!(d, vec!["a".to_string(), "a\\b".to_string()]);
    }

    #[test]
    fn options_from_older_manifests_keep_the_start_menu_shortcut() {
        let o: Options = serde_json::from_str(r#"{"desktop_shortcut":true}"#).unwrap();
        assert_eq!(o, Options { start_menu_shortcut: true, desktop_shortcut: true });
        let o: Options = serde_json::from_str(r#"{"start_menu_shortcut":false,"desktop_shortcut":false}"#).unwrap();
        assert!(!o.start_menu_shortcut);
        assert!(Options::default().start_menu_shortcut);
    }
}
