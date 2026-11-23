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
use gloria_core::journal::{self, Recovered};
use gloria_core::legacy::{self, Found};
use gloria_core::manifest::{AppManifest, RuntimeManifest, INSTALL_MANIFEST_NAME, WEBVIEW2_DIR, WEBVIEW2_SETUP_NAME, WORK_DIR};
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
        match journal::os_code(e) {
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
    // Ours: our install manifest, or the journal of an install of ours that was cut short.
    let ours = InstallManifest::load(raw).map(|m| m.app_id == app.id).unwrap_or(false) || journal::owner(raw).as_deref() == Some(app.id.as_str());
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
    pub start_menu: bool,
    pub desktop: bool,
    /// Remove user data (clean reinstall / downgrade, or `/PURGE`).
    pub purge: bool,
    pub dry_run: bool,
    /// Force-close the app if it does not close by itself (silent mode, or after the user chose Force close).
    pub force_close: bool,
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
    log!("install {} {} into {} (start menu: {}, desktop: {}, purge: {}, dry run: {})", app.id, app.version, dir.display(), req.start_menu, req.desktop, req.purge, req.dry_run);
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
        if req.start_menu {
            log!("[dry run] would create shortcut {:?}", start);
        }
        if req.desktop {
            log!("[dry run] would create shortcut {:?}", desk);
        }
        if req.purge {
            for d in data_folders(app, &[]) {
                log!("[dry run] would remove data folder {}", d.display());
            }
        }
        if app.needs_webview2 {
            let bundled = files.iter().any(|f| f.path.to_ascii_lowercase().starts_with(&format!("{WEBVIEW2_DIR}\\").to_ascii_lowercase()));
            log!("[dry run] would check WebView2 (present: {}, bootstrapper bundled: {bundled})", sys::webview2_present());
        }
        for (k, v) in registry::uninstall_values(app, dir, files.iter().map(|f| f.size).sum(), gloria_core::GLORIA_SETUP_VERSION) {
            log!("[dry run] would set HKCU\\{}\\{}\\{k} = {v:?}", registry::UNINSTALL_ROOT, app.uninstall_key());
        }
        for (key, values) in registry::context_menu_values(app, dir) {
            for (k, v) in values {
                log!("[dry run] would set HKCU\\{key}\\{k} = {v:?}");
            }
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

    // An earlier run that died mid-commit left a journal: put the previous version back first.
    recover(dir)?;
    let prev = InstallManifest::load(dir).ok().filter(|m| m.app_id == app.id);

    let created_dir = !dir.exists();
    fs::create_dir_all(dir).map_err(|e| Fail::new(Exit::InvalidArgs, format!("options.errWrite|{}: {e}", dir.display())))?;
    let work = dir.join(WORK_DIR);
    let staging = work.join(journal::STAGING);
    // Only for failures before the journal exists; after that, `journal::recover` undoes everything.
    let fail_clean = |f: Fail| -> Fail {
        let _ = install_manifest::remove_tree_no_follow(&work, false, &mut |_| {});
        if created_dir {
            let _ = fs::remove_dir(dir);
            remove_empty_suite_dir(dir);
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
    let mut entries: Vec<FileEntry> = staged.map_err(|e| fail_clean(Fail::io(&e, "extract")))?;
    // The WebView2 bootstrapper is staged like any file but never installed.
    let reserved = format!("{WEBVIEW2_DIR}\\").to_ascii_lowercase();
    entries.retain(|e| !e.path.to_ascii_lowercase().starts_with(&reserved));
    log!("staged {} files", entries.len());
    if !progress(70.0, "") {
        return Err(fail_clean(Fail::cancelled()));
    }

    // WebView2: before anything is committed, so a failure leaves the previous version as it was.
    if app.needs_webview2 {
        ensure_webview2(&staging.join(WEBVIEW2_DIR).join(WEBVIEW2_SETUP_NAME)).map_err(fail_clean)?;
    }

    // The new install manifest is staged too, so it is swapped in (and rolled back) with the files it lists.
    let new_dirs = InstallManifest::dirs_for(&entries);
    let prev_dirs: Vec<String> = prev.as_ref().map(|p| p.dirs.clone()).unwrap_or_default();
    // Directories: new ones we create, plus ones a previous install of ours created.
    let dirs: Vec<String> = new_dirs
        .iter()
        .filter(|d| !dir.join(d).exists() || prev_dirs.iter().any(|p| p.eq_ignore_ascii_case(d)))
        .cloned()
        .collect();
    let (start, desk) = shortcut_paths(app);
    let mut want = Vec::new();
    if req.start_menu {
        want.push(start);
    }
    if req.desktop {
        want.push(desk);
    }
    let want: Vec<PathBuf> = want.into_iter().flatten().collect();
    let obsolete: Vec<String> = prev
        .iter()
        .flat_map(|p| &p.files)
        .filter(|o| !entries.iter().any(|n| n.path.eq_ignore_ascii_case(&o.path)))
        .filter(|o| safe_rel_path(&o.path).is_some())
        .map(|o| o.path.clone())
        .collect();
    let mut m = InstallManifest {
        schema: install_manifest::SCHEMA,
        app_id: app.id.clone(),
        name: app.name.clone(),
        version: app.parsed_version().to_string(),
        installed_at: now_unix(),
        gloria_setup: gloria_core::GLORIA_SETUP_VERSION.into(),
        files: entries,
        dirs,
        created_install_dir: created_dir || prev.as_ref().is_some_and(|p| p.created_install_dir),
        shortcuts: want.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        options: Options { start_menu_shortcut: req.start_menu, desktop_shortcut: req.desktop },
        data_folders: app.data_folders.clone(),
    };
    m.save(&staging).map_err(|e| fail_clean(Fail::io(&e, INSTALL_MANIFEST_NAME)))?;
    let mut places: Vec<String> = m.files.iter().map(|e| e.path.clone()).collect();
    places.push(INSTALL_MANIFEST_NAME.into());

    // 2. Commit: the journal goes to disk, then every rename. Old files go to `.gloria\backup`.
    let commit = journal::Commit::begin(dir, &app.id, places, obsolete.clone(), created_dir).map_err(|e| fail_clean(Fail::io(&e, "journal")))?;
    let n = commit.len().max(1);
    let applied = commit.apply(|i| {
        crash_point(i);
        progress(70.0 + 8.0 * i as f32 / n as f32, "");
        true
    });
    let undo = |f: Fail| -> Fail {
        match journal::recover(dir) {
            Ok(_) => log!("rolled back; the previous version is unchanged"),
            Err(e) => log!("rollback incomplete ({e}); the next run of this setup finishes it"),
        }
        f
    };
    if let Err(e) = applied {
        return Err(undo(Fail::io(&e, "replace")));
    }
    crash_point(commit.len());
    for o in &obsolete {
        log!("removed obsolete {}", dir.join(o).display());
    }

    // 3. Register, then delete the journal: the commit point.
    progress(80.0, "");
    let values = registry::uninstall_values(app, dir, m.total_size(), gloria_core::GLORIA_SETUP_VERSION);
    if let Err(e) = sys::reg_write(&app.uninstall_key(), &values) {
        return Err(undo(Fail::new(Exit::Error, format!("registry: {e}"))));
    }
    log!("registered HKCU\\{}\\{}", registry::UNINSTALL_ROOT, app.uninstall_key());
    crash_point(commit.len() + 1);
    commit.finish().map_err(|e| undo(Fail::io(&e, "journal")))?;
    log!("committed");

    let mut gone: Vec<&String> = prev_dirs.iter().filter(|d| !new_dirs.iter().any(|n| n.eq_ignore_ascii_case(d))).collect();
    gone.sort_by_key(|d| std::cmp::Reverse(d.matches('\\').count()));
    for d in gone {
        if let Some(rel) = safe_rel_path(d) {
            let _ = fs::remove_dir(dir.join(rel));
        }
    }

    // 4. Shortcuts (after the commit: a failed shortcut is logged, never a reason to roll back).
    progress(85.0, "");
    let exe = dir.join(app.main_exe_rel());
    let mut shortcuts = Vec::new();
    for lnk in &want {
        match sys::create_shortcut(lnk, &exe, app.app_user_model_id.as_deref(), &app.name) {
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
    if shortcuts.len() != m.shortcuts.len() {
        m.shortcuts = shortcuts;
        if let Err(e) = m.save(dir) {
            log!("{INSTALL_MANIFEST_NAME}: {e}");
        }
    }

    // Explorer context menu (also after the commit: logged, never rolled back). An update that drops
    // `context_menu` removes the old entry.
    if app.context_menu.is_none() {
        for key in registry::context_menu_keys(app) {
            let _ = sys::reg_delete_hkcu(&key);
        }
    }
    for (key, values) in registry::context_menu_values(app, dir) {
        match sys::reg_write_hkcu(&key, &values) {
            Ok(()) => log!("context menu HKCU\\{key}"),
            Err(e) => log!("context menu HKCU\\{key} failed: {e}"),
        }
    }

    // 5. Finalize.
    if req.purge {
        progress(95.0, "");
        remove_data(app, &m.data_folders, dir, false);
    }
    sys::notify_shell();
    progress(100.0, "");
    log!("install done");
    Ok(())
}

/// Applies the done screen's "Add to Start menu" choice to a finished install: creates or removes the Start Menu
/// shortcut and records it in the install manifest. Logged, never fatal.
pub fn set_start_menu(app: &AppManifest, dir: &Path, on: bool, dry_run: bool) {
    let (Some(lnk), _) = shortcut_paths(app) else { return };
    let Ok(mut m) = InstallManifest::load(dir).map_err(|e| log!("start menu: {INSTALL_MANIFEST_NAME}: {e}")) else { return };
    if m.app_id != app.id {
        return;
    }
    let listed = m.shortcuts.iter().any(|s| norm(Path::new(s)) == norm(&lnk));
    if on == (listed && lnk.exists()) && m.options.start_menu_shortcut == on {
        return;
    }
    if dry_run {
        log!("[dry run] would {} shortcut {}", if on { "create" } else { "remove" }, lnk.display());
        return;
    }
    if on {
        match sys::create_shortcut(&lnk, &dir.join(app.main_exe_rel()), app.app_user_model_id.as_deref(), &app.name) {
            Ok(()) => {
                log!("shortcut {}", lnk.display());
                if !listed {
                    m.shortcuts.push(lnk.to_string_lossy().into_owned());
                }
            }
            Err(e) => {
                log!("shortcut {} failed: {e}", lnk.display());
                return;
            }
        }
    } else {
        // Only a shortcut this install made (a same-named one from elsewhere is not ours to delete).
        if listed && lnk.exists() {
            if let Err(e) = fs::remove_file(&lnk) {
                log!("removing shortcut {} failed: {e}", lnk.display());
                return;
            }
        }
        log!("removed shortcut {} (Add to Start menu unticked)", lnk.display());
        m.shortcuts.retain(|s| norm(Path::new(s)) != norm(&lnk));
    }
    m.options.start_menu_shortcut = on;
    if let Err(e) = m.save(dir) {
        log!("{INSTALL_MANIFEST_NAME}: {e}");
    }
    sys::notify_shell();
}

/// Undoes an install that died mid-commit (see `gloria_core::journal`). A failure here stops the run: the backups
/// stay where they are for the next attempt.
fn recover(dir: &Path) -> Result<(), Fail> {
    match journal::recover(dir) {
        Ok(Recovered::Nothing) => Ok(()),
        Ok(Recovered::Cleaned) => {
            log!("removed a leftover {}", dir.join(WORK_DIR).display());
            Ok(())
        }
        Ok(Recovered::RolledBack) => {
            log!("an earlier install into {} was interrupted; rolled it back", dir.display());
            Ok(())
        }
        Err(e) => Err(Fail::io(&e, "recover an interrupted install")),
    }
}

/// Debug builds only: `GLORIA_TEST_CRASH_AT=<n>` ends the process right before commit step `n`, to test recovery.
/// Steps `0..len` are the renames, `len` is the registry write, `len + 1` is deleting the journal.
fn crash_point(i: usize) {
    #[cfg(debug_assertions)]
    if std::env::var("GLORIA_TEST_CRASH_AT").ok().and_then(|v| v.parse::<usize>().ok()) == Some(i) {
        log!("GLORIA_TEST_CRASH_AT={i}: exiting without cleanup");
        std::process::exit(99);
    }
    let _ = i;
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

// ───────────────────────────── WebView2 ─────────────────────────────

/// Makes sure the WebView2 runtime is there: runs the bundled Evergreen bootstrapper if it isn't (it installs per
/// user without admin), then checks again. Exit 5 if it is still missing.
fn ensure_webview2(bootstrapper: &Path) -> Result<(), Fail> {
    if sys::webview2_present() {
        return Ok(());
    }
    if !bootstrapper.is_file() {
        return Err(Fail::new(Exit::WebView2Failed, "err.reasonWebView2|the runtime is missing and this setup does not bundle its installer"));
    }
    log!("installing WebView2 with {}", bootstrapper.display());
    match sys::run_wait(bootstrapper, "/silent /install", false, Duration::from_secs(600)) {
        Ok(code) => log!("WebView2 bootstrapper exit code {code:?}"),
        Err(sys::RunError::Cancelled) => log!("WebView2 bootstrapper cancelled"),
        Err(sys::RunError::Other(e)) => log!("WebView2 bootstrapper: {e}"),
    }
    if sys::webview2_present() {
        Ok(())
    } else {
        Err(Fail::new(Exit::WebView2Failed, "err.reasonWebView2|the WebView2 runtime is still missing after its installer ran"))
    }
}

// ───────────────────────────── legacy installs ─────────────────────────────

/// Does this exe carry the NSIS marker? (Tauri's NSIS uninstallers do.)
fn is_nsis(exe: &Path) -> bool {
    let Ok(f) = File::open(exe) else { return false };
    let mut buf = Vec::new();
    if f.take(16 << 20).read_to_end(&mut buf).is_err() {
        return false;
    }
    buf.windows(12).any(|w| w == b"NullsoftInst")
}

/// Installs of this app made by an older installer, per-user ones first.
pub fn detect_legacy(app: &AppManifest) -> Vec<Found> {
    let found = legacy::find(app, sys::uninstall_entries(), &is_nsis, &legacy::Guard::from_env(crate::app::env));
    for f in &found {
        log!(
            "legacy install: {:?}\\{} ({}, {}, version {:?}, folder {:?}, quiet: {:?})",
            f.entry.hive,
            f.entry.key,
            f.kind.label(),
            if f.needs_admin() { "all users" } else { "this user" },
            f.version,
            f.dir,
            f.quiet
        );
    }
    found
}

pub struct Migration<'a> {
    pub app: &'a AppManifest,
    pub found: &'a [Found],
    /// The user chose to remove their data; otherwise it is backed up first and anything missing afterwards is put
    /// back.
    pub purge: bool,
    pub dry_run: bool,
    /// No UI: an entry with no silent uninstall is refused instead of running its interactive uninstaller.
    pub silent: bool,
}

fn backup_dir(app: &AppManifest) -> PathBuf {
    std::env::temp_dir().join(format!("GloriaSetup-backup-{}", app.id))
}

/// Copies `src` into `dst`, never following links and never overwriting a file already in `dst`.
fn copy_missing(src: &Path, dst: &Path, n: &mut usize) -> io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    if md.file_type().is_symlink() || install_manifest::is_reparse_point(&md) {
        return Ok(());
    }
    if md.is_dir() {
        fs::create_dir_all(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_missing(&e.path(), &dst.join(e.file_name()), n)?;
        }
    } else if md.is_file() && fs::symlink_metadata(dst).is_err() {
        if let Some(p) = dst.parent() {
            fs::create_dir_all(p)?;
        }
        fs::copy(src, dst)?;
        *n += 1;
    }
    Ok(())
}

/// Removes every legacy install in `mg.found` by running its own uninstaller, keeping the user's data unless they
/// chose otherwise. Runs before the normal install, which then goes in fresh.
pub fn migrate(mg: &Migration, progress: Progress) -> Result<(), Fail> {
    let app = mg.app;
    let data = data_folders(app, &[]);
    let backup = backup_dir(app);
    log!("migrating {} legacy install(s) of {} (purge: {}, dry run: {})", mg.found.len(), app.id, mg.purge, mg.dry_run);
    progress(2.0, "");

    // 1. Back up the data folders. A failed backup stops here, before anything is removed.
    if !mg.purge {
        for (i, d) in data.iter().enumerate().filter(|(_, d)| d.exists()) {
            if mg.dry_run {
                log!("[dry run] would back up {} to {}", d.display(), backup.join(i.to_string()).display());
                continue;
            }
            let mut n = 0;
            copy_missing(d, &backup.join(i.to_string()), &mut n).map_err(|e| Fail::io(&e, &format!("back up {}", d.display())))?;
            log!("backed up {} ({n} files) to {}", d.display(), backup.join(i.to_string()).display());
        }
    }

    // 2. Run each old uninstaller and wait until its Uninstall entry is gone.
    let n = mg.found.len().max(1) as f32;
    let ran = (|| -> Result<(), Fail> {
        for (i, f) in mg.found.iter().enumerate() {
            let base = 5.0 + 85.0 * i as f32 / n;
            let (exe, args) = match (&f.quiet, mg.silent) {
                (Some(c), _) => (c.exe.clone(), c.args.clone()),
                (None, false) => match f.entry.get("UninstallString").and_then(legacy::split_command) {
                    Some((exe, args)) => (exe, args),
                    None => return Err(Fail::new(Exit::Error, format!("err.reasonLegacy|{} has no uninstall command", f.entry.key))),
                },
                (None, true) => return Err(Fail::new(Exit::Error, format!("err.reasonLegacy|{} has no silent uninstall", f.entry.key))),
            };
            let exe = if exe.is_absolute() { exe } else { sys::system_exe(&exe.to_string_lossy()) };
            if mg.dry_run {
                log!("[dry run] would run {}{} {args}", exe.display(), if f.needs_admin() { " (as admin)" } else { "" });
                continue;
            }
            if let Some(d) = &f.dir {
                if d.is_dir() && !sys::close_apps(d, None, Duration::from_secs(8), true).is_empty() {
                    return Err(Fail::new(Exit::FilesLocked, "err.reasonV"));
                }
            }
            progress(base + 2.0, &exe.to_string_lossy());
            log!("running {}{} {args}", exe.display(), if f.needs_admin() { " (as admin)" } else { "" });
            match sys::run_wait(&exe, &args, f.needs_admin(), Duration::from_secs(600)) {
                Ok(Some(code)) => log!("old uninstaller exit code {code}"),
                Ok(None) => return Err(Fail::new(Exit::Error, "err.reasonLegacy|the old uninstaller did not finish in 10 minutes")),
                Err(sys::RunError::Cancelled) => return Err(Fail::new(Exit::ElevationFailed, "err.reasonUac")),
                Err(sys::RunError::Other(e)) if f.needs_admin() => return Err(Fail::new(Exit::ElevationFailed, format!("err.reasonUac|{e}"))),
                Err(sys::RunError::Other(e)) => return Err(Fail::new(Exit::Error, format!("err.reasonLegacy|{}: {e}", exe.display()))),
            }
            // Inno and NSIS without `_?=` hand off to a %TEMP% copy; the entry going away is the real "done".
            let t = std::time::Instant::now();
            while sys::uninstall_entry_exists(&f.entry) {
                if t.elapsed() > Duration::from_secs(60) {
                    return Err(Fail::new(Exit::Error, format!("err.reasonLegacy|{} is still registered after its uninstaller ran", f.entry.key)));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            log!("legacy entry {} removed", f.entry.key);
            if let Some(x) = &f.leftover {
                // `_?=` leaves the NSIS uninstaller in place.
                match fs::remove_file(x) {
                    Ok(()) => log!("removed leftover {}", x.display()),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => log!("leftover {}: {e}", x.display()),
                }
            }
            if let Some(d) = &f.dir {
                if fs::remove_dir(d).is_ok() {
                    log!("removed empty folder {}", d.display());
                }
            }
            progress(base + 85.0 / n, "");
        }
        Ok(())
    })();

    // 3. Put back whatever an old uninstaller deleted, also when a later one failed or was refused, then drop the
    // backup. If restoring fails the backup stays, and the log says where.
    if !mg.purge && !mg.dry_run && backup.exists() {
        let mut restored = Ok(());
        for (i, d) in data.iter().enumerate() {
            let b = backup.join(i.to_string());
            if b.exists() {
                let mut n = 0;
                match copy_missing(&b, d, &mut n) {
                    Ok(()) => log!("restored {n} missing files into {}", d.display()),
                    Err(e) => {
                        log!("restore {} failed: {e}; backup kept in {}", d.display(), backup.display());
                        restored = Err(Fail::io(&e, &format!("restore {} (backup kept in {})", d.display(), backup.display())));
                    }
                }
            }
        }
        if restored.is_ok() {
            let r = install_manifest::remove_tree_no_follow(&backup, false, &mut |_| {});
            log!("removed backup {}: {r:?}", backup.display());
        }
        if ran.is_ok() {
            restored?;
        }
    }
    ran?;
    sys::notify_shell();
    progress(100.0, "");
    log!("migration done");
    Ok(())
}

// ───────────────────────────── uninstall ─────────────────────────────

pub struct Uninstall<'a> {
    pub app: &'a AppManifest,
    pub dir: PathBuf,
    pub purge: bool,
    /// "Remove everything": `purge`, plus the whole install folder (when setup created it, including files the app
    /// wrote there later) and everything GloriaSetup left in `%TEMP%` for this app.
    pub wipe: bool,
    pub dry_run: bool,
    /// The uninstaller that started this trampoline; waited for so its exe can be deleted.
    pub wait_pid: Option<u32>,
}

/// Removes exactly what the install manifest lists, the shortcuts and the registry key; user data only if `purge`;
/// with `wipe`, everything else this app and GloriaSetup left behind too.
pub fn uninstall(u: &Uninstall, progress: Progress) -> Result<(), Fail> {
    let app = u.app;
    let dir = &u.dir;
    log!("uninstall {} from {} (purge: {}, wipe: {}, dry run: {})", app.id, dir.display(), u.purge, u.wipe, u.dry_run);
    if let Some(pid) = u.wait_pid {
        if !sys::wait_pid(pid, Duration::from_secs(30)) {
            log!("uninstaller {pid} is still running");
        }
    }
    progress(2.0, "");
    if !paths::is_local_absolute(dir) {
        return Err(Fail::new(Exit::InvalidArgs, format!("not an absolute folder: {}", dir.display())));
    }
    let load = || -> Result<InstallManifest, Fail> {
        let m = InstallManifest::load(dir).map_err(|e| Fail::new(Exit::Error, format!("{}: {e}", dir.join(INSTALL_MANIFEST_NAME).display())))?;
        if m.app_id != app.id {
            return Err(Fail::new(Exit::Error, format!("{} belongs to {:?}, not {:?}", dir.display(), m.app_id, app.id)));
        }
        Ok(m)
    };
    let mut m = load()?;
    let key = app.uninstall_key();
    let reg_dir = sys::reg_read(&key, "InstallLocation");
    let ours = reg_dir.as_deref().map(|r| norm(Path::new(r)) == norm(dir)).unwrap_or(true);

    if !u.dry_run {
        let left = sys::close_apps(dir, u.wait_pid, Duration::from_secs(8), true);
        if !left.is_empty() {
            return Err(Fail::new(Exit::FilesLocked, "err.reasonV"));
        }
        // An interrupted update: restore the version the manifest on disk describes before removing it.
        recover(dir)?;
        m = load()?;
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

    if u.purge || u.wipe {
        progress(75.0, "");
        remove_data(app, &m.data_folders, dir, u.dry_run);
    }
    if u.wipe {
        wipe_install_dir(dir, &m, u.dry_run);
        remove_temp_leftovers(&app.id, u.dry_run);
    }
    if !u.dry_run {
        remove_empty_suite_dir(dir);
    }
    progress(90.0, "");
    if ours {
        if u.dry_run {
            log!("[dry run] would delete HKCU\\{}\\{key}", registry::UNINSTALL_ROOT);
        } else if let Err(e) = sys::reg_delete(&key) {
            log!("registry: {e}");
        }
        for k in registry::context_menu_keys(app) {
            if u.dry_run {
                log!("[dry run] would delete HKCU\\{k}");
            } else if let Err(e) = sys::reg_delete_hkcu(&k) {
                log!("context menu: {e}");
            }
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

/// "Remove everything": deletes what is still in the install folder after the manifest-driven removal (files the app
/// wrote there itself, logs, caches) and the folder. Only when setup created that folder, so a folder the user
/// installed into that already held their own files is never emptied; never follows links.
fn wipe_install_dir(dir: &Path, m: &InstallManifest, dry_run: bool) {
    if !m.created_install_dir {
        log!("{} existed before the install; leaving files GloriaSetup did not put there", dir.display());
        return;
    }
    let guard = legacy::Guard::from_env(crate::app::env);
    let sys_dirs = SystemDirs::from_env(crate::app::env);
    if !guard.is_app_folder(dir) || paths::check_install_dir(dir, &sys_dirs, &[]).is_err() {
        log!("{} is not a folder that is safe to empty; left alone", dir.display());
        return;
    }
    let mut n = 0usize;
    let r = install_manifest::remove_tree_no_follow(dir, dry_run, &mut |p| {
        n += 1;
        log!("{}{}", if dry_run { "[dry run] would remove " } else { "removed " }, p.display());
    });
    log!("emptied install folder {} ({n} leftover entries): {r:?}", dir.display());
}

/// "Remove everything": the logs, trampoline copies and data backup GloriaSetup put in `%TEMP%` for app `id`. The
/// running exe and the current log are deleted after exit by `sys::delete_self_later`.
fn remove_temp_leftovers(id: &str, dry_run: bool) {
    let tmp = std::env::temp_dir();
    let me = std::env::current_exe().ok().map(|p| norm(&p));
    let log_now = crate::log::path().map(|p| norm(&p));
    for e in fs::read_dir(&tmp).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let p = e.path();
        let n = Some(norm(&p));
        if !paths::is_temp_leftover(&name, id) || n == me || n == log_now {
            continue;
        }
        let r = if dry_run { Ok(()) } else { install_manifest::remove_tree_no_follow(&p, false, &mut |_| {}) };
        log!("{}removed {}: {r:?}", if dry_run { "[dry run] would have " } else { "" }, p.display());
    }
}

/// Removes the shared `Project Gloria Softwares` folder once the last app in it is gone (only if empty).
fn remove_empty_suite_dir(dir: &Path) {
    let Some(parent) = paths::suite_parent(dir) else { return };
    let plain_dir = fs::symlink_metadata(parent).is_ok_and(|md| md.is_dir() && !install_manifest::is_reparse_point(&md));
    if plain_dir && fs::remove_dir(parent).is_ok() {
        log!("removed the empty {}", parent.display());
    }
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
