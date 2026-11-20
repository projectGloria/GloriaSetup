//! The install and uninstall work (docs/design.md §5–§8). UI-free: the window and silent mode both call it from a
//! worker thread and get progress through a callback.
//!
//! Install stages the whole archive in `<install>\.gloria\staging`, then swaps files in with renames (old files go to
//! `.gloria\backup`), and undoes every rename if one fails, so a locked file leaves the old version working.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gloria_core::archive::{self, safe_rel_path, FileEntry};
use gloria_core::exit::Exit;
use gloria_core::install_manifest::{self, plan_removal, InstallManifest, Options, Removal};
use gloria_core::manifest::{AppManifest, RuntimeManifest, INSTALL_MANIFEST_NAME, WORK_DIR};
use gloria_core::paths::{self, is_within, norm, parse_data_folder, PathProblem, SystemDirs};
use gloria_core::payload::{self, Payload};
use gloria_core::registry;

use crate::{log, sys};

#[derive(Debug, Clone)]
pub struct Fail {
    pub exit: Exit,
    /// i18n key (`err.reason…`) or a literal message.
    pub reason: String,
}

impl Fail {
    fn new(exit: Exit, reason: impl Into<String>) -> Fail {
        let f = Fail { exit, reason: reason.into() };
        log!("FAIL ({}): {}", exit.code(), f.reason);
        f
    }
    fn io(e: &io::Error, what: &str) -> Fail {
        match e.raw_os_error() {
            // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION, ERROR_ACCESS_DENIED (a running exe can't be renamed over)
            Some(32) | Some(33) | Some(5) => Fail::new(Exit::FilesLocked, format!("err.reasonV|{what}: {e}")),
            // ERROR_DISK_FULL, ERROR_HANDLE_DISK_FULL
            Some(112) | Some(39) => Fail::new(Exit::Error, format!("err.reasonSpace|{what}: {e}")),
            _ => Fail::new(Exit::Error, format!("{what}: {e}")),
        }
    }
    pub const CANCELLED: &'static str = "cancelled";
    fn cancelled() -> Fail {
        Fail::new(Exit::Error, Fail::CANCELLED)
    }
    pub fn is_cancel(&self) -> bool {
        self.reason == Fail::CANCELLED
    }
}

/// Progress callback: (percent, current path). Returning false asks the engine to cancel (honoured before commit).
pub type Progress<'a> = &'a mut dyn FnMut(f32, &str) -> bool;

// ───────────────────────────── the running exe ─────────────────────────────

pub struct Source {
    pub exe: PathBuf,
    pub payload: Payload,
    pub m: RuntimeManifest,
}

/// Reads this exe's payload. `Ok(None)` for a bare stub.
pub fn open_self() -> io::Result<Option<Source>> {
    let exe = std::env::current_exe()?;
    let mut f = File::open(&exe)?;
    let Some(p) = payload::read(&mut f)? else { return Ok(None) };
    let m = RuntimeManifest::from_json(&p.manifest_json).map_err(io::Error::other)?;
    Ok(Some(Source { exe, payload: p, m }))
}

// ───────────────────────────── existing installs ─────────────────────────────

#[derive(Debug, Clone)]
pub struct Existing {
    pub dir: PathBuf,
    pub version: String,
    pub manifest: Option<InstallManifest>,
}

/// Our own `projectGloria.<id>` key, with its tracked manifest when the folder still has one for this app.
pub fn detect(app: &AppManifest) -> Option<Existing> {
    let key = app.uninstall_key();
    let dir = PathBuf::from(sys::reg_read(&key, "InstallLocation")?);
    let version = sys::reg_read(&key, "DisplayVersion").unwrap_or_default();
    let manifest = InstallManifest::load(&dir).ok().filter(|m| m.app_id == app.id);
    log!("existing install: {} {} (manifest: {})", dir.display(), version, manifest.is_some());
    Some(Existing { dir, version, manifest })
}

// ───────────────────────────── folder checks ─────────────────────────────

/// Expanded data folders of `app` plus any recorded in an install manifest, all re-validated.
pub fn data_folders(app: &AppManifest, extra: &[String]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for d in app.data_folders.iter().chain(extra) {
        if let Some(p) = parse_data_folder(d).ok().and_then(|df| df.expand(crate::app::env)) {
            if !out.iter().any(|x| norm(x) == norm(&p)) {
                out.push(p);
            }
        }
    }
    out
}

/// The silent-mode folder rule: same checks as the Options page, plus the "append \<Name>" rule and a write probe.
pub fn resolve_dir(app: &AppManifest, raw: &Path) -> Result<PathBuf, PathProblem> {
    let sys = SystemDirs::from_env(crate::app::env);
    let data = data_folders(app, &[]);
    paths::check_install_dir(raw, &sys, &data)?;
    let ours = InstallManifest::load(raw).map(|m| m.app_id == app.id).unwrap_or(false);
    let non_empty = fs::read_dir(raw).map(|mut d| d.next().is_some()).unwrap_or(false);
    let dir = if non_empty && !ours { raw.join(paths::sanitize_file_name(&app.name)) } else { raw.to_path_buf() };
    paths::check_install_dir(&dir, &sys, &data)?;
    Ok(dir)
}

/// Creates the folder chain up to `dir` if needed and writes/deletes a probe file. Folders it created are removed.
pub fn write_probe(dir: &Path) -> bool {
    let mut created = Vec::new();
    let mut p = Some(dir);
    while let Some(x) = p {
        if x.exists() {
            break;
        }
        created.push(x.to_path_buf());
        p = x.parent();
    }
    let ok = fs::create_dir_all(dir).is_ok() && {
        let probe = dir.join(format!(".gloria-probe-{}", sys::rand_suffix()));
        let w = fs::write(&probe, b"probe").is_ok();
        let _ = fs::remove_file(&probe);
        w
    };
    for d in created {
        let _ = fs::remove_dir(d);
    }
    ok
}

// ───────────────────────────── install ─────────────────────────────

pub struct Request {
    pub dir: PathBuf,
    pub desktop: bool,
    /// Remove user data (clean reinstall / downgrade, or `/PURGE`).
    pub purge: bool,
    pub dry_run: bool,
    /// Force-close the app if it does not close by itself (silent mode, or after the user chose Force close).
    pub force_close: bool,
}

enum Op {
    /// An existing file moved to the backup folder: (original, backup).
    Backup(PathBuf, PathBuf),
    /// A staged file moved into place: (final, staged).
    Place(PathBuf, PathBuf),
}

fn rollback(ops: &mut Vec<Op>) {
    while let Some(op) = ops.pop() {
        let (from, to) = match &op {
            Op::Place(dest, staged) => (dest, staged),
            Op::Backup(orig, bak) => (bak, orig),
        };
        if let Err(e) = fs::rename(from, to) {
            log!("rollback: {} -> {}: {e}", from.display(), to.display());
        }
    }
}

fn set_hidden(p: &Path) {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN};
    unsafe {
        let _ = SetFileAttributesW(&HSTRING::from(p.as_os_str()), FILE_ATTRIBUTE_HIDDEN);
    }
}

fn shortcut_paths(app: &AppManifest) -> (Option<PathBuf>, Option<PathBuf>) {
    let file = format!("{}.lnk", paths::sanitize_file_name(&app.name));
    (sys::start_menu_dir().map(|d| d.join(&file)), sys::desktop_dir().map(|d| d.join(&file)))
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn archive_reader(src: &Source) -> io::Result<impl Read> {
    let mut f = File::open(&src.exe)?;
    f.seek(SeekFrom::Start(src.payload.archive_offset))?;
    Ok(io::BufReader::with_capacity(1 << 16, f.take(src.payload.archive_len)))
}

/// Installs, updates or reinstalls into `req.dir`.
pub fn install(src: &Source, req: &Request, progress: Progress) -> Result<(), Fail> {
    let app = &src.m.app;
    let dir = &req.dir;
    log!("install {} {} into {} (desktop: {}, purge: {}, dry run: {})", app.id, app.version, dir.display(), req.desktop, req.purge, req.dry_run);
    let prev = InstallManifest::load(dir).ok().filter(|m| m.app_id == app.id);
    if let Some(p) = &prev {
        log!("replacing {} ({} files)", p.version, p.files.len());
    }
    progress(1.0, "");

    let mut f = File::open(&src.exe).map_err(|e| Fail::io(&e, "open setup"))?;
    payload::verify(&mut f, &src.payload).map_err(|e| Fail::new(Exit::Error, format!("err.reasonCorrupt|{e}")))?;
    drop(f);
    if !progress(4.0, "") {
        return Err(Fail::cancelled());
    }

    if req.dry_run {
        let files = archive::list(archive_reader(src).map_err(|e| Fail::io(&e, "archive"))?).map_err(|e| Fail::io(&e, "archive"))?;
        for e in &files {
            log!("[dry run] would install {} ({} bytes)", dir.join(&e.path).display(), e.size);
        }
        if let Some(p) = &prev {
            for old in p.files.iter().filter(|o| !files.iter().any(|n| n.path.eq_ignore_ascii_case(&o.path))) {
                log!("[dry run] would remove obsolete {}", dir.join(&old.path).display());
            }
        }
        let (start, desk) = shortcut_paths(app);
        log!("[dry run] would create shortcut {:?}", start);
        if req.desktop {
            log!("[dry run] would create shortcut {:?}", desk);
        }
        if req.purge {
            for d in data_folders(app, &[]) {
                log!("[dry run] would remove data folder {}", d.display());
            }
        }
        for (k, v) in registry::uninstall_values(app, dir, files.iter().map(|f| f.size).sum(), gloria_core::GLORIA_SETUP_VERSION) {
            log!("[dry run] would set HKCU\\{}\\{}\\{k} = {v:?}", registry::UNINSTALL_ROOT, app.uninstall_key());
        }
        progress(100.0, "");
        return Ok(());
    }

    // Close the app if it runs from this folder.
    if dir.exists() {
        let left = sys::close_apps(dir, None, Duration::from_secs(8), req.force_close);
        if !left.is_empty() {
            return Err(Fail::new(Exit::FilesLocked, "err.reasonV"));
        }
    }

    let created_dir = !dir.exists();
    fs::create_dir_all(dir).map_err(|e| Fail::new(Exit::InvalidArgs, format!("options.errWrite|{}: {e}", dir.display())))?;
    let work = dir.join(WORK_DIR);
    if work.exists() {
        log!("removing a stale {}", work.display());
        let _ = install_manifest::remove_tree_no_follow(&work, false, &mut |_| {});
    }
    let staging = work.join("staging");
    let backup = work.join("backup");
    let fail_clean = |f: Fail| -> Fail {
        let _ = install_manifest::remove_tree_no_follow(&work, false, &mut |_| {});
        if created_dir {
            let _ = fs::remove_dir(dir);
        }
        f
    };
    fs::create_dir_all(&staging).map_err(|e| fail_clean(Fail::new(Exit::InvalidArgs, format!("options.errWrite|{e}"))))?;
    set_hidden(&work);

    // 1. Stage.
    let total = src.m.unpacked_size.max(1) as f64;
    let mut cancelled = false;
    let staged = {
        let r = archive_reader(src).map_err(|e| fail_clean(Fail::io(&e, "archive")))?;
        let mut last = 0.0f32;
        archive::unpack(r, &staging, |rel, done| {
            let pct = 8.0 + 62.0 * (done as f64 / total).min(1.0) as f32;
            if pct - last >= 0.25 || last == 0.0 {
                last = pct;
                if !progress(pct, &dir.join(rel).to_string_lossy()) {
                    cancelled = true;
                }
            }
        })
    };
    if cancelled {
        return Err(fail_clean(Fail::cancelled()));
    }
    let entries: Vec<FileEntry> = staged.map_err(|e| fail_clean(Fail::io(&e, "extract")))?;
    log!("staged {} files", entries.len());
    if !progress(70.0, "") {
        return Err(fail_clean(Fail::cancelled()));
    }

    // 2. Commit: back up what is replaced or obsolete, move staged files in. Any failure undoes every rename.
    let new_dirs = InstallManifest::dirs_for(&entries);
    let existed: Vec<bool> = new_dirs.iter().map(|d| dir.join(d).exists()).collect();
    let mut ops: Vec<Op> = Vec::new();
    let commit = (|| -> io::Result<()> {
        let n = entries.len().max(1);
        for (i, e) in entries.iter().enumerate() {
            let rel = safe_rel_path(&e.path).ok_or_else(|| io::Error::other("unsafe path"))?;
            let dest = dir.join(&rel);
            if fs::symlink_metadata(&dest).is_ok() {
                let bak = backup.join(&rel);
                fs::create_dir_all(bak.parent().unwrap())?;
                fs::rename(&dest, &bak).map_err(|err| io::Error::new(err.kind(), format!("{}: {err}", dest.display())))?;
                ops.push(Op::Backup(dest.clone(), bak));
            }
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let from = staging.join(&rel);
            fs::rename(&from, &dest).map_err(|err| io::Error::new(err.kind(), format!("{}: {err}", dest.display())))?;
            ops.push(Op::Place(dest, from));
            progress(70.0 + 8.0 * (i + 1) as f32 / n as f32, "");
        }
        if let Some(p) = &prev {
            for old in p.files.iter().filter(|o| !entries.iter().any(|n| n.path.eq_ignore_ascii_case(&o.path))) {
                let Some(rel) = safe_rel_path(&old.path) else { continue };
                let orig = dir.join(&rel);
                if fs::symlink_metadata(&orig).is_ok() {
                    let bak = backup.join(&rel);
                    fs::create_dir_all(bak.parent().unwrap())?;
                    fs::rename(&orig, &bak)?;
                    log!("removed obsolete {}", orig.display());
                    ops.push(Op::Backup(orig, bak));
                }
            }
        }
        Ok(())
    })();
    if let Err(e) = commit {
        rollback(&mut ops);
        return Err(fail_clean(Fail::io(&e, "replace")));
    }

    // Directories: new ones we created, plus ones a previous install of ours created.
    let prev_dirs: Vec<String> = prev.as_ref().map(|p| p.dirs.clone()).unwrap_or_default();
    let dirs: Vec<String> = new_dirs
        .iter()
        .zip(&existed)
        .filter(|(d, ex)| !**ex || prev_dirs.iter().any(|p| p.eq_ignore_ascii_case(d)))
        .map(|(d, _)| d.clone())
        .collect();
    let mut gone: Vec<&String> = prev_dirs.iter().filter(|d| !new_dirs.iter().any(|n| n.eq_ignore_ascii_case(d))).collect();
    gone.sort_by_key(|d| std::cmp::Reverse(d.matches('\\').count()));
    for d in gone {
        if let Some(rel) = safe_rel_path(d) {
            let _ = fs::remove_dir(dir.join(rel));
        }
    }

    // 3. Shortcuts.
    progress(80.0, "");
    let exe = dir.join(app.main_exe_rel());
    let (start, desk) = shortcut_paths(app);
    let mut shortcuts = Vec::new();
    let mut want = vec![start];
    if req.desktop {
        want.push(desk);
    }
    for lnk in want.into_iter().flatten() {
        match sys::create_shortcut(&lnk, &exe, app.app_user_model_id.as_deref(), &app.name) {
            Ok(()) => {
                log!("shortcut {}", lnk.display());
                shortcuts.push(lnk.to_string_lossy().into_owned());
            }
            Err(e) => log!("shortcut {} failed: {e}", lnk.display()),
        }
    }
    if let Some(p) = &prev {
        for old in p.shortcuts.iter().filter(|s| !shortcuts.iter().any(|n| norm(Path::new(n)) == norm(Path::new(s)))) {
            let path = PathBuf::from(old);
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) && paths::is_local_absolute(&path) {
                let _ = fs::remove_file(&path);
                log!("removed old shortcut {}", path.display());
            }
        }
    }

    // 4. Finalize.
    progress(90.0, "");
    let m = InstallManifest {
        schema: install_manifest::SCHEMA,
        app_id: app.id.clone(),
        name: app.name.clone(),
        version: app.parsed_version().to_string(),
        installed_at: now_unix(),
        gloria_setup: gloria_core::GLORIA_SETUP_VERSION.into(),
        files: entries,
        dirs,
        created_install_dir: created_dir || prev.as_ref().is_some_and(|p| p.created_install_dir),
        shortcuts,
        options: Options { desktop_shortcut: req.desktop },
        data_folders: app.data_folders.clone(),
    };
    if let Err(e) = m.save(dir) {
        rollback(&mut ops);
        return Err(fail_clean(Fail::io(&e, "install-manifest.json")));
    }
    let values = registry::uninstall_values(app, dir, m.total_size(), gloria_core::GLORIA_SETUP_VERSION);
    if let Err(e) = sys::reg_write(&app.uninstall_key(), &values) {
        return Err(fail_clean(Fail::new(Exit::Error, format!("registry: {e}"))));
    }
    log!("registered HKCU\\{}\\{}", registry::UNINSTALL_ROOT, app.uninstall_key());
    if req.purge {
        progress(95.0, "");
        remove_data(app, &m.data_folders, dir, false);
    }
    let _ = install_manifest::remove_tree_no_follow(&work, false, &mut |_| {});
    sys::notify_shell();
    progress(100.0, "");
    log!("install done");
    Ok(())
}

/// Removes the declared data folders (never one that overlaps the install folder). Errors are logged, not fatal.
fn remove_data(app: &AppManifest, recorded: &[String], install_dir: &Path, dry_run: bool) {
    for d in data_folders(app, recorded) {
        if is_within(&d, install_dir) || is_within(install_dir, &d) {
            log!("skipping data folder {} (overlaps the install folder)", d.display());
            continue;
        }
        let mut n = 0usize;
        let r = install_manifest::remove_tree_no_follow(&d, dry_run, &mut |_| n += 1);
        log!("{}removed data folder {} ({n} entries): {r:?}", if dry_run { "[dry run] would have " } else { "" }, d.display());
    }
}

// ───────────────────────────── uninstall ─────────────────────────────

pub struct Uninstall<'a> {
    pub app: &'a AppManifest,
    pub dir: PathBuf,
    pub purge: bool,
    pub dry_run: bool,
    /// The uninstaller that started this trampoline; waited for so its exe can be deleted.
    pub wait_pid: Option<u32>,
}

/// Removes exactly what the install manifest lists, the shortcuts and the registry key; user data only if `purge`.
pub fn uninstall(u: &Uninstall, progress: Progress) -> Result<(), Fail> {
    let app = u.app;
    let dir = &u.dir;
    log!("uninstall {} from {} (purge: {}, dry run: {})", app.id, dir.display(), u.purge, u.dry_run);
    if let Some(pid) = u.wait_pid {
        if !sys::wait_pid(pid, Duration::from_secs(30)) {
            log!("uninstaller {pid} is still running");
        }
    }
    progress(2.0, "");
    if !paths::is_local_absolute(dir) {
        return Err(Fail::new(Exit::InvalidArgs, format!("not an absolute folder: {}", dir.display())));
    }
    let m = InstallManifest::load(dir).map_err(|e| Fail::new(Exit::Error, format!("{}: {e}", dir.join(INSTALL_MANIFEST_NAME).display())))?;
    if m.app_id != app.id {
        return Err(Fail::new(Exit::Error, format!("{} belongs to {:?}, not {:?}", dir.display(), m.app_id, app.id)));
    }
    let key = app.uninstall_key();
    let reg_dir = sys::reg_read(&key, "InstallLocation");
    let ours = reg_dir.as_deref().map(|r| norm(Path::new(r)) == norm(dir)).unwrap_or(true);

    if !u.dry_run {
        let left = sys::close_apps(dir, u.wait_pid, Duration::from_secs(8), true);
        if !left.is_empty() {
            return Err(Fail::new(Exit::FilesLocked, "err.reasonV"));
        }
    }
    progress(10.0, "");

    let plan = plan_removal(dir, &m);
    let n = plan.len().max(1);
    let mut skipped = 0;
    for (i, step) in plan.iter().enumerate() {
        let r = install_manifest::execute_removal(std::slice::from_ref(step), u.dry_run);
        for s in &r.done {
            log!("{}{s:?}", if u.dry_run { "[dry run] would remove " } else { "removed " });
        }
        for (p, why) in &r.skipped {
            skipped += 1;
            log!("could not remove {}: {why}", p.display());
        }
        let shown = match step {
            Removal::File(p) | Removal::Shortcut(p) | Removal::Dir(p) => p.to_string_lossy(),
        };
        progress(10.0 + 60.0 * (i + 1) as f32 / n as f32, &shown);
    }

    if u.purge {
        progress(75.0, "");
        remove_data(app, &m.data_folders, dir, u.dry_run);
    }
    progress(90.0, "");
    if ours {
        if u.dry_run {
            log!("[dry run] would delete HKCU\\{}\\{key}", registry::UNINSTALL_ROOT);
        } else if let Err(e) = sys::reg_delete(&key) {
            log!("registry: {e}");
        }
    } else {
        log!("registry key points at {:?}, left alone", reg_dir);
    }
    if !u.dry_run {
        sys::notify_shell();
    }
    progress(100.0, "");
    log!("uninstall done ({skipped} items skipped)");
    Ok(())
}

/// Copies `uninstall.exe` to `%TEMP%` and starts it as the cleanup trampoline. Returns its pid.
pub fn start_trampoline(app: &AppManifest, dir: &Path, flags: &str) -> io::Result<u32> {
    let me = std::env::current_exe()?;
    let tmp = std::env::temp_dir().join(format!("GloriaSetup-{}-{}.exe", app.id, sys::rand_suffix()));
    fs::copy(&me, &tmp)?;
    let dir_s = dir.to_string_lossy();
    let args = format!(r#"{flags} --cleanup "{}" --wait-pid {}"#, dir_s.trim_end_matches('\\'), sys::current_pid());
    log!("trampoline {} {args}", tmp.display());
    sys::spawn(&tmp, args.trim())
}
