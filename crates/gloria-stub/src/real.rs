//! Real (non-preview) runs: the setup, `uninstall.exe`, and the `%TEMP%` cleanup trampoline, each silent or with UI.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gloria_core::cli::Args;
use gloria_core::exit::Exit;
use gloria_core::install_manifest::{tree_size, InstallManifest};
use gloria_core::manifest::AppManifest;
use gloria_core::paths::{self, norm};
use gloria_core::version::{self, Relation};

use crate::app::{App, AppInfo, ErrorInfo, Effect, Flow, Legacy, Page, Running, Shared};
use crate::engine::{self, Existing, Fail, Migration, Request, Source, Uninstall};
use gloria_core::legacy::Found;
use crate::gfx::Gfx;
use crate::window::{self, Handler};
use crate::{i18n, log, sys};

fn info_of(f: &Fail) -> ErrorInfo {
    let key = f.reason.split('|').next().unwrap_or_default().to_string();
    let reason = if f.is_cancel() { "err.reasonCancel".to_string() } else { key };
    ErrorInfo { code: f.exit.code(), reason, log: log::display_path() }
}

fn default_dir(app: &AppManifest) -> PathBuf {
    let lad = crate::app::env("LOCALAPPDATA").unwrap_or_else(|| r"C:\Users\you\AppData\Local".into());
    paths::default_install_dir(Path::new(&lad), &app.name)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    norm(a) == norm(b)
}

/// Progress reporter writing into the window's shared status; scaled into `lo..hi` percent.
fn reporter(job: &Shared, lo: f32, hi: f32) -> impl FnMut(f32, &str) -> bool + '_ {
    move |pct, file| {
        let mut st = job.lock().unwrap();
        st.pct = st.pct.max(lo + (hi - lo) * pct / 100.0);
        if !file.is_empty() {
            st.file = file.to_string();
        }
        !st.cancel
    }
}

/// What a setup does around `engine::install`: remove the installed copy first for a clean reinstall / downgrade, or
/// when the user picked a different folder; or remove an install made by an older installer (`migrate`).
struct Plan {
    remove_first: Option<(PathBuf, bool)>,
    migrate: Vec<Found>,
    silent: bool,
    req: Request,
}

fn run_plan(src: &Source, plan: &Plan, job: Option<&Shared>) -> Result<(), Fail> {
    let mut quiet = |_: f32, _: &str| true;
    if !plan.migrate.is_empty() {
        let mg = Migration { app: &src.m.app, found: &plan.migrate, purge: plan.req.purge, dry_run: plan.req.dry_run, silent: plan.silent };
        match job {
            Some(j) => engine::migrate(&mg, &mut reporter(j, 0.0, 20.0))?,
            None => engine::migrate(&mg, &mut quiet)?,
        }
    }
    if let Some((dir, purge)) = &plan.remove_first {
        let u = Uninstall { app: &src.m.app, dir: dir.clone(), purge: *purge, dry_run: plan.req.dry_run, wait_pid: None };
        match job {
            Some(j) => engine::uninstall(&u, &mut reporter(j, 0.0, 7.0))?,
            None => engine::uninstall(&u, &mut quiet)?,
        }
    }
    match job {
        Some(j) => engine::install(src, &plan.req, &mut reporter(j, 0.0, 100.0)),
        None => engine::install(src, &plan.req, &mut quiet),
    }
}

// ───────────────────────────── setup ─────────────────────────────

pub fn setup(args: &Args, src: Source) -> i32 {
    let ex = engine::detect(&src.m.app);
    // Our own key wins; only without it does an install made by an older installer count.
    let legacy = if ex.is_none() { engine::detect_legacy(&src.m.app) } else { Vec::new() };
    if args.silent {
        return silent_setup(args, &src, ex.as_ref(), legacy);
    }
    match setup_ui(args, src, ex, legacy) {
        Ok(c) => c,
        Err(e) => {
            log!("window: {e}");
            Exit::Error.code()
        }
    }
}

/// Is `d` the folder of one of the old installs? (It still holds the old files, so the "append \<Name>" rule for
/// non-empty folders must not apply: the migration empties it first.)
fn is_legacy_dir(legacy: &[Found], d: &Path) -> bool {
    legacy.iter().any(|f| f.dir.as_deref().is_some_and(|x| same_dir(x, d)))
}

fn silent_setup(args: &Args, src: &Source, ex: Option<&Existing>, legacy: Vec<Found>) -> i32 {
    let app = &src.m.app;
    let rel = ex.map(|e| version::relation(&e.version, &app.parsed_version()));
    if rel == Some(Relation::Downgrade) && !args.allow_downgrade {
        log!("refusing to downgrade {} -> {} without /ALLOWDOWNGRADE", ex.unwrap().version, app.version);
        return Exit::DowngradeRefused.code();
    }
    let dir = match (&args.install_dir, ex) {
        (Some(d), _) => match engine::resolve_dir(app, d) {
            Ok(_) if is_legacy_dir(&legacy, d) => d.clone(),
            Ok(d) => d,
            Err(p) => {
                log!("install folder {} refused: {p:?}", d.display());
                return Exit::InvalidArgs.code();
            }
        },
        (None, Some(e)) => e.dir.clone(),
        (None, None) => default_dir(app),
    };
    if !args.dry_run && !engine::write_probe(&dir) {
        log!("cannot write to {}", dir.display());
        return Exit::InvalidArgs.code();
    }
    let clean = matches!(rel, Some(Relation::Downgrade)) || (rel == Some(Relation::Same) && args.purge);
    let remove_first = ex.filter(|e| e.manifest.is_some() && (clean || !same_dir(&e.dir, &dir))).map(|e| (e.dir.clone(), clean && args.purge));
    let desktop = ex.and_then(|e| e.manifest.as_ref()).map(|m| m.options.desktop_shortcut).unwrap_or(false);
    // Migrating keeps the user's data unless /PURGE.
    let purge = !legacy.is_empty() && args.purge;
    let plan = Plan { remove_first, migrate: legacy, silent: true, req: Request { dir: dir.clone(), desktop, purge, dry_run: args.dry_run, force_close: true } };
    match run_plan(src, &plan, None) {
        Ok(()) => {
            if args.relaunch && !args.dry_run {
                sys::launch(&dir.join(app.main_exe_rel()));
            }
            Exit::Ok.code()
        }
        Err(f) => f.exit.code(),
    }
}

fn data_sizes(app: &AppManifest) -> Vec<(String, Option<u64>)> {
    engine::data_folders(app, &[])
        .into_iter()
        .map(|p| {
            let size = p.exists().then(|| tree_size(&p));
            (p.to_string_lossy().into_owned(), size)
        })
        .collect()
}

fn new_app(g: &Gfx, src_m: gloria_core::manifest::RuntimeManifest, lang: &str) -> windows::core::Result<App> {
    let icon = crate::load_icon(g, None)?;
    let data = data_sizes(&src_m.app);
    let info = AppInfo::new(src_m, icon, data, vec![]);
    let s = crate::strings(lang, &info);
    let mut a = App::new(info, s, false);
    a.credit = crate::credit(g);
    a.gloria = crate::gloria_red(g);
    Ok(a)
}

fn setup_ui(args: &Args, src: Source, ex: Option<Existing>, legacy: Vec<Found>) -> windows::core::Result<i32> {
    let lang = i18n::detect(args.lang.as_deref());
    let g = Gfx::new(lang)?;
    let mut a = new_app(&g, src.m.clone(), lang)?;
    let app_m = src.m.app.clone();
    let page = match &ex {
        Some(e) => {
            a.installed = Some(e.version.clone());
            a.set_folder(e.dir.to_string_lossy().into_owned());
            if let Some(m) = &e.manifest {
                a.desktop = m.options.desktop_shortcut;
            }
            if let Some((pid, path)) = sys::processes_in(&e.dir, None).into_iter().next() {
                a.running = Some(Running { path: path.to_string_lossy().into_owned(), pid });
            }
            match version::relation(&e.version, &app_m.parsed_version()) {
                Relation::Update => {
                    a.flow = Flow::Update;
                    Page::Update
                }
                Relation::Same => Page::Reinstall,
                Relation::Downgrade => Page::Downgrade,
            }
        }
        None => {
            if let Some(d) = &args.install_dir {
                a.set_folder(d.to_string_lossy().into_owned());
            }
            match legacy.first() {
                Some(f) => {
                    a.legacy = Some(Legacy {
                        kind: f.kind.label().into(),
                        scope: if f.needs_admin() { "mig.scopeAll" } else { "mig.scopeUser" }.into(),
                        path: f.dir.as_ref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
                        admin: legacy.iter().any(Found::needs_admin),
                        version: f.version.clone(),
                    });
                    Page::Migrate
                }
                None => Page::Welcome,
            }
        }
    };
    a.page = page;
    a.home = page;
    let src = Arc::new(src);
    let ex = Arc::new(ex);
    let legacy = Arc::new(legacy);
    let dry_run = args.dry_run;
    let handler: Handler = Box::new(move |app, eff, _hwnd| match eff {
        Effect::StartJob(flow) => {
            let Some(job) = app.job.clone() else { return None };
            let home = app.home;
            let dir = app.check.final_dir.clone();
            let existing = ex.as_ref().as_ref().filter(|e| e.manifest.is_some()).map(|e| e.dir.clone());
            // A clean reinstall (Reinstall → Uninstall) or a downgrade removes the installed copy first; the user-data
            // question decides `purge`. Shown as the "prepare" step of the install progress.
            let clean = flow == Flow::Uninstall || home == Page::Downgrade;
            if flow == Flow::Uninstall {
                app.flow = Flow::Install;
                app.go(Page::Progress, 0.0);
            }
            let remove_first = existing.filter(|d| clean || !same_dir(d, &dir)).map(|d| (d, clean && app.purge));
            let desktop = app.desktop;
            let launch = app.launch;
            // From the migrate page: remove the old installs first; the data checkbox there decides `purge`.
            let migrate = if home == Page::Migrate { legacy.as_ref().clone() } else { Vec::new() };
            let purge = !migrate.is_empty() && app.purge;
            let plan = Plan { remove_first, migrate, silent: false, req: Request { dir: dir.clone(), desktop, purge, dry_run, force_close: false } };
            let src = src.clone();
            std::thread::spawn(move || {
                let r = run_plan(&src, &plan, Some(&job));
                if r.is_ok() && launch && !dry_run {
                    sys::launch(&dir.join(src.m.app.main_exe_rel()));
                }
                let mut st = job.lock().unwrap();
                if r.is_ok() {
                    st.pct = 100.0;
                }
                st.result = Some(r.map_err(|f| info_of(&f)));
            });
            None
        }
        Effect::LaunchAndExit => {
            sys::launch(&app.check.final_dir.join(app_m.main_exe_rel()));
            Some(0)
        }
        Effect::OpenLog => {
            if let Some(p) = log::path() {
                sys::open_file(&p);
            }
            None
        }
        Effect::CloseApp | Effect::ForceClose => {
            let force = matches!(eff, Effect::ForceClose);
            let dir = app.check.final_dir.clone();
            let cell = Arc::new(Mutex::new(None));
            app.closing = Some(cell.clone());
            std::thread::spawn(move || {
                let grace = if force { Duration::from_millis(500) } else { Duration::from_secs(8) };
                let left = sys::close_apps(&dir, None, grace, force);
                *cell.lock().unwrap() = Some(left.is_empty());
            });
            None
        }
        _ => None,
    });
    window::run(g, a, args.software, handler)
}

// ───────────────────────────── uninstall.exe ─────────────────────────────

fn trampoline_flags(args: &Args, purge: bool, lang: Option<&str>) -> String {
    let mut f = Vec::new();
    if args.silent {
        f.push("/S".to_string());
    }
    if purge {
        f.push("/PURGE".into());
    }
    if args.dry_run {
        f.push("--dry-run".into());
    }
    if let Some(l) = lang {
        f.push(format!("--lang {l}"));
    }
    f.join(" ")
}

/// `uninstall.exe` itself: asks the user-data question (UI), then hands off to a `%TEMP%` copy so this exe and its
/// folder can be deleted.
pub fn uninstaller(args: &Args, src: Source) -> i32 {
    let app = src.m.app.clone();
    let Some(dir) = src.exe.parent().map(Path::to_path_buf) else { return Exit::Error.code() };
    match InstallManifest::load(&dir) {
        Ok(m) if m.app_id == app.id => {}
        other => {
            log!("{} has no install manifest for {}: {:?}", dir.display(), app.id, other.err());
            return Exit::Error.code();
        }
    }
    if args.silent {
        return match engine::start_trampoline(&app, &dir, &trampoline_flags(args, args.purge, None)) {
            Ok(_) => Exit::Ok.code(),
            Err(e) => {
                log!("trampoline: {e}");
                Exit::Error.code()
            }
        };
    }
    let r = (|| -> windows::core::Result<i32> {
        let lang = i18n::detect(args.lang.as_deref());
        let g = Gfx::new(lang)?;
        let mut a = new_app(&g, src.m.clone(), lang)?;
        a.installed = Some(app.version.clone());
        a.flow = Flow::Uninstall;
        a.page = Page::Uninstall;
        a.home = Page::Uninstall;
        a.set_folder(dir.to_string_lossy().into_owned());
        let software = args.software;
        let args = args.clone();
        let handler: Handler = Box::new(move |a, eff, _| match eff {
            Effect::StartJob(_) => match engine::start_trampoline(&app, &dir, &trampoline_flags(&args, a.purge, Some(lang))) {
                Ok(_) => Some(0),
                Err(e) => {
                    log!("trampoline: {e}");
                    a.job = None;
                    a.error = Some(ErrorInfo { code: 1, reason: e.to_string(), log: log::display_path() });
                    a.go(Page::Error, 0.0);
                    None
                }
            },
            Effect::OpenLog => {
                if let Some(p) = log::path() {
                    sys::open_file(&p);
                }
                None
            }
            _ => None,
        });
        window::run(g, a, software, handler)
    })();
    r.unwrap_or_else(|e| {
        log!("window: {e}");
        Exit::Error.code()
    })
}

// ───────────────────────────── the %TEMP% trampoline ─────────────────────────────

pub fn cleanup(args: &Args, src: Source, dir: &Path) -> i32 {
    let app = src.m.app.clone();
    let u = Uninstall { app: &app, dir: dir.to_path_buf(), purge: args.purge, dry_run: args.dry_run, wait_pid: args.wait_pid };
    let code = if args.silent {
        match engine::uninstall(&u, &mut |_, _| true) {
            Ok(()) => 0,
            Err(f) => f.exit.code(),
        }
    } else {
        let r = (|| -> windows::core::Result<i32> {
            let lang = i18n::detect(args.lang.as_deref());
            let g = Gfx::new(lang)?;
            let mut a = new_app(&g, src.m.clone(), lang)?;
            a.installed = Some(app.version.clone());
            a.purge = args.purge;
            a.home = Page::Uninstall;
            a.set_folder(dir.to_string_lossy().into_owned());
            a.start(Flow::Uninstall, 0.0);
            let job = a.job.clone().expect("job");
            let (dir, purge, dry_run, wait_pid, app2) = (dir.to_path_buf(), args.purge, args.dry_run, args.wait_pid, app.clone());
            std::thread::spawn(move || {
                let u = Uninstall { app: &app2, dir, purge, dry_run, wait_pid };
                let r = engine::uninstall(&u, &mut reporter(&job, 0.0, 100.0));
                let mut st = job.lock().unwrap();
                st.result = Some(r.map_err(|f| info_of(&f)));
            });
            let handler: Handler = Box::new(|_, eff, _| {
                if let Effect::OpenLog = eff {
                    if let Some(p) = log::path() {
                        sys::open_file(&p);
                    }
                }
                None
            });
            window::run(g, a, args.software, handler)
        })();
        r.unwrap_or_else(|e| {
            log!("window: {e}");
            Exit::Error.code()
        })
    };
    if !args.dry_run {
        sys::delete_self_later();
    }
    code
}
