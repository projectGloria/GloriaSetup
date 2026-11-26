//! What the UI shows and what its buttons do. The window and the screenshot code both drive an `App`; the engine
//! (real installs) and the preview simulator both report through `JobStatus`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gloria_core::manifest::RuntimeManifest;
use gloria_core::paths::{self, SystemDirs};

use crate::gfx::{Color, Image};
use crate::i18n::Strings;

pub fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

/// The folder rules for this machine. `%TEMP%` is often an 8.3 path (`C:\Users\RUNNER~1\...`), so every variable is
/// expanded to its long form first or a long-form install path inside it would not match.
pub fn system_dirs() -> SystemDirs {
    SystemDirs::from_env(|k| env(k).map(|v| long_path(&v)))
}

/// `GetLongPathNameW`, or the input unchanged when the path does not exist or the call fails.
pub fn long_path(p: &str) -> String {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetLongPathNameW;
    let h = HSTRING::from(p);
    let mut buf = vec![0u16; 1024];
    // SAFETY: `h` is NUL-terminated and `buf` is a writable slice; the call writes at most `buf.len()` units.
    let n = unsafe { GetLongPathNameW(&h, Some(&mut buf)) } as usize;
    if n == 0 || n >= buf.len() {
        return p.to_string();
    }
    String::from_utf16_lossy(&buf[..n])
}

// ───────────────────────────── app info ─────────────────────────────

pub struct AppInfo {
    pub m: RuntimeManifest,
    pub accent: Color,
    pub icon: Image,
    pub icon_gray: Image,
    /// Declared data folders with their size when known.
    pub data: Vec<(String, Option<u64>)>,
    /// Paths (relative to the install folder) the progress line cycles through in preview.
    pub files: Vec<String>,
}

impl AppInfo {
    pub fn new(m: RuntimeManifest, icon: Image, data: Vec<(String, Option<u64>)>, files: Vec<String>) -> AppInfo {
        let (r, g, b) = m.app.accent_rgb();
        AppInfo { accent: Color::rgb(r, g, b), icon_gray: icon.gray(), icon, data, files, m }
    }
    pub fn name(&self) -> &str {
        &self.m.app.name
    }
    pub fn version(&self) -> &str {
        &self.m.app.version
    }
    pub fn exe(&self) -> String {
        self.m.app.main_exe_rel().rsplit('\\').next().unwrap_or_default().to_string()
    }
    pub fn default_dir(&self) -> PathBuf {
        let lad = env("LOCALAPPDATA").unwrap_or_else(|| r"C:\Users\you\AppData\Local".into());
        paths::default_install_dir(Path::new(&lad), &self.m.app.name)
    }
    pub fn data_paths(&self) -> Vec<PathBuf> {
        self.m.app.data().iter().filter_map(|d| d.expand(env)).collect()
    }
    pub fn data_total(&self) -> u64 {
        self.data.iter().filter_map(|d| d.1).sum()
    }
}

// ───────────────────────────── pages ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Welcome,
    Progress,
    Done,
    Update,
    Reinstall,
    Downgrade,
    Migrate,
    Uninstall,
    UninstallProgress,
    UninstallDone,
    Error,
}

/// `--page` / screenshot names, in mockup order.
pub const PAGE_IDS: [&str; 12] = [
    "welcome",
    "progress",
    "done",
    "update",
    "update-running",
    "reinstall",
    "downgrade",
    "migrate",
    "uninstall",
    "uninstall-progress",
    "uninstall-done",
    "error",
];

impl Page {
    pub fn danger(self) -> bool {
        matches!(self, Page::Uninstall | Page::UninstallProgress | Page::Error)
    }
    pub fn uninstaller(self) -> bool {
        matches!(self, Page::Uninstall | Page::UninstallProgress | Page::UninstallDone)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modal {
    Ask,
    Waiting,
    Stuck,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Install,
    Update,
    Uninstall,
}

#[derive(Clone, Debug)]
pub struct Legacy {
    pub kind: String,
    pub scope: String,
    /// Where the old install lives (shown in the log; the engine works from the registry entry).
    #[allow(dead_code)]
    pub path: String,
    pub admin: bool,
    pub version: String,
}

#[derive(Clone, Debug)]
pub struct ErrorInfo {
    pub code: i32,
    /// i18n key of the reason, or a literal message.
    pub reason: String,
    pub log: String,
}

#[derive(Clone, Debug)]
pub struct Running {
    pub path: String,
    pub pid: u32,
}

/// Progress reported by the engine thread or the preview simulator.
#[derive(Clone, Debug, Default)]
pub struct JobStatus {
    pub pct: f32,
    pub file: String,
    pub result: Option<Result<(), ErrorInfo>>,
    /// Set by the window (Cancel / Close mid-job); the engine stops before it commits.
    pub cancel: bool,
}

pub type Shared = Arc<Mutex<JobStatus>>;

/// Everything the user can do in the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Start,
    Cancel,
    Choice(bool),
    /// "Add to Start menu" (done screen).
    StartMenu,
    /// "Add desktop shortcut" (done screen).
    Desktop,
    Purge,
    /// "Remove everything" on the uninstall page (turns `purge` on with it).
    Wipe,
    ConfirmDown,
    Modal(Option<Modal>),
    Force,
    Minimize,
    Close,
    LaunchApp,
    OpenLog,
    /// The projectGloria GitHub link in the footer.
    OpenGithub,
    Noop,
}

/// Where the footer link goes.
pub const GITHUB_URL: &str = "https://github.com/ProjectGloria";

/// What the window must do after an action.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    None,
    Exit(i32),
    Minimize,
    /// Apply the done screen's choices, launch the installed app, then exit 0.
    LaunchAndExit,
    /// Apply the done screen's choices (real mode), then exit 0.
    Finish,
    OpenLog,
    /// Open a web page in the default browser; the window stays.
    OpenUrl(&'static str),
    /// The engine should start this job (real mode).
    StartJob(Flow),
    /// Ask the running app to close (real mode).
    CloseApp,
    ForceClose,
}

pub struct FolderCheck {
    pub final_dir: PathBuf,
}

pub struct App {
    pub info: AppInfo,
    pub s: Strings,
    pub preview: bool,
    pub page: Page,
    pub page_t0: f64,
    pub folder: String,
    /// "Add to Start menu": on by default.
    pub start_menu: bool,
    /// "Add desktop shortcut": on by default.
    pub desktop: bool,
    pub purge: bool,
    pub wipe: bool,
    pub repair: bool,
    pub confirm_down: bool,
    pub modal: Option<Modal>,
    pub modal_t0: f64,
    pub flow: Flow,
    pub installed: Option<String>,
    pub legacy: Option<Legacy>,
    pub running: Option<Running>,
    pub error: Option<ErrorInfo>,
    pub job: Option<Shared>,
    pub job_t0: f64,
    pub status: JobStatus,
    /// The page the app opened on; preview "Cancel" returns there.
    pub home: Page,
    sys: SystemDirs,
    pub check: FolderCheck,
    /// GloriaSetup credit image for the done screens.
    pub credit: Option<Image>,
    /// projectGloria mark (GloriaRed) for the footer link.
    pub gloria: Option<Image>,
    /// "Close it for me": the closer thread writes Some(true) when the app is gone, Some(false) when it timed out.
    pub closing: Option<Arc<Mutex<Option<bool>>>>,
    /// An effect raised by `tick` (not by a click) for the window to run.
    pub pending: Option<Effect>,
}

impl App {
    pub fn new(info: AppInfo, s: Strings, preview: bool) -> App {
        let folder = info.default_dir().to_string_lossy().into_owned();
        let mut a = App {
            s,
            preview,
            page: Page::Welcome,
            page_t0: 0.0,
            folder,
            start_menu: true,
            desktop: true,
            purge: false,
            wipe: false,
            repair: true,
            confirm_down: false,
            modal: None,
            modal_t0: 0.0,
            flow: Flow::Install,
            installed: None,
            legacy: None,
            running: None,
            error: None,
            job: None,
            job_t0: 0.0,
            status: JobStatus::default(),
            home: Page::Welcome,
            sys: system_dirs(),
            check: FolderCheck { final_dir: PathBuf::new() },
            credit: None,
            gloria: None,
            closing: None,
            pending: None,
            info,
        };
        a.recheck();
        a
    }

    /// Fills the fake "existing install" data a preview needs and opens on `id` (a `PAGE_IDS` name).
    pub fn preview_page(&mut self, id: &str) {
        let v = self.info.m.app.parsed_version();
        let minor = v.core[1];
        let ver = |m: i64| {
            let mut c = v.core;
            c[1] = m.max(0) as u64;
            if m < 0 {
                c[0] = c[0].saturating_sub(1);
                c[1] = 9;
            }
            c[..v.parts.max(3)].iter().map(u64::to_string).collect::<Vec<_>>().join(".")
        };
        let name = self.info.name().to_string();
        let default = self.info.default_dir().to_string_lossy().into_owned();
        self.installed = Some(ver(minor as i64 - 1));
        self.legacy = Some(Legacy {
            kind: "NSIS".into(),
            scope: "mig.scopeAll".into(),
            path: format!(r"C:\Program Files\{name}"),
            admin: true,
            version: ver(minor as i64 - 1),
        });
        self.running = None;
        self.modal = None;
        self.purge = false;
        self.wipe = false;
        self.folder = default.clone();
        self.flow = if id.starts_with("update") { Flow::Update } else { Flow::Install };
        let page = match id {
            "progress" => Page::Progress,
            "done" => Page::Done,
            "update" => Page::Update,
            "update-running" | "update-waiting" | "update-stuck" => {
                self.running = Some(Running { path: format!(r"{default}\{}", self.info.exe()), pid: 14212 });
                self.modal = Some(match id {
                    "update-waiting" => Modal::Waiting,
                    "update-stuck" => Modal::Stuck,
                    _ => Modal::Ask,
                });
                Page::Update
            }
            "reinstall" => {
                self.installed = Some(self.info.version().into());
                Page::Reinstall
            }
            "downgrade" => {
                self.installed = Some(ver(minor as i64 + 1));
                Page::Downgrade
            }
            "migrate" => Page::Migrate,
            "uninstall" => {
                self.installed = Some(self.info.version().into());
                Page::Uninstall
            }
            // "Remove everything" ticked (it ticks the data checkbox with it).
            "uninstall-wipe" => {
                self.installed = Some(self.info.version().into());
                (self.purge, self.wipe) = (true, true);
                Page::Uninstall
            }
            "uninstall-progress" => Page::UninstallProgress,
            "uninstall-done" => Page::UninstallDone,
            "uninstall-done-wipe" => {
                (self.purge, self.wipe) = (true, true);
                Page::UninstallDone
            }
            "error" => {
                let ts = "20261120-2207";
                self.error = Some(ErrorInfo {
                    code: 3,
                    reason: "err.reasonV".into(),
                    log: format!(r"%TEMP%\GloriaSetup-{}-{ts}.log", name.replace(' ', "")),
                });
                Page::Error
            }
            _ => Page::Welcome,
        };
        if matches!(page, Page::Uninstall | Page::UninstallProgress | Page::UninstallDone) {
            self.flow = Flow::Uninstall;
        }
        self.page = page;
        self.home = page;
        self.recheck();
    }

    pub fn go(&mut self, page: Page, now: f64) {
        if page == Page::Welcome {
            self.flow = Flow::Install;
            self.modal = None;
            self.confirm_down = false;
        }
        self.page = page;
        self.page_t0 = now;
    }

    pub fn set_folder(&mut self, s: String) {
        self.folder = s;
        self.recheck();
    }

    /// Re-validates the folder field (cheap: no disk writes).
    pub fn recheck(&mut self) {
        self.check = check_folder(&self.folder, &self.info, &self.sys);
    }

    pub fn can_start(&self) -> bool {
        match self.page {
            Page::Downgrade => self.confirm_down,
            _ => true,
        }
    }

    pub fn act(&mut self, a: Act, now: f64) -> Effect {
        match a {
            Act::Start => {
                if !self.can_start() {
                    return Effect::None;
                }
                match self.page {
                    Page::Update if self.running.is_some() => {
                        self.modal = Some(Modal::Ask);
                        self.modal_t0 = now;
                    }
                    Page::Reinstall if !self.repair => {
                        self.flow = Flow::Uninstall;
                        self.go(Page::Uninstall, now);
                    }
                    Page::Uninstall => return self.start(Flow::Uninstall, now),
                    Page::Update => return self.start(Flow::Update, now),
                    Page::Error => return self.start(self.flow, now),
                    _ => return self.start(Flow::Install, now),
                }
            }
            Act::Cancel => {
                if self.preview {
                    let home = if matches!(self.home, Page::Progress | Page::UninstallProgress) { Page::Welcome } else { self.home };
                    self.job = None;
                    self.go(home, now);
                } else if let Some(job) = &self.job {
                    job.lock().unwrap().cancel = true;
                } else {
                    return Effect::Exit(gloria_core::exit::Exit::Error.code());
                }
            }
            Act::Choice(r) => self.repair = r,
            Act::StartMenu => self.start_menu = !self.start_menu,
            Act::Desktop => self.desktop = !self.desktop,
            Act::Purge => {
                self.purge = !self.purge;
                // Keeping the data means not removing everything.
                self.wipe &= self.purge;
            }
            Act::Wipe => {
                self.wipe = !self.wipe;
                self.purge |= self.wipe;
            }
            Act::ConfirmDown => self.confirm_down = !self.confirm_down,
            Act::Modal(m) => {
                self.modal = m;
                self.modal_t0 = now;
                if m == Some(Modal::Waiting) && !self.preview {
                    return Effect::CloseApp;
                }
            }
            Act::Force => {
                self.modal = None;
                if !self.preview {
                    return Effect::ForceClose;
                }
                return self.start(Flow::Update, now);
            }
            Act::Noop => {}
            Act::Minimize => return Effect::Minimize,
            Act::Close => {
                let code = match self.page {
                    Page::Done | Page::UninstallDone => 0,
                    Page::Error => self.error.as_ref().map(|e| e.code).unwrap_or(1),
                    _ => gloria_core::exit::Exit::Error.code(),
                };
                if let (Some(job), false) = (&self.job, self.preview) {
                    // Closing mid-install is a cancel; the engine rolls back and the error page says so.
                    job.lock().unwrap().cancel = true;
                    return Effect::None;
                }
                if self.page == Page::Done && !self.preview {
                    return Effect::Finish;
                }
                return Effect::Exit(code);
            }
            Act::LaunchApp => {
                if self.preview {
                    self.go(self.home, now);
                    return Effect::None;
                }
                return Effect::LaunchAndExit;
            }
            Act::OpenLog => return Effect::OpenLog,
            Act::OpenGithub => return Effect::OpenUrl(GITHUB_URL),
        }
        Effect::None
    }

    pub fn start(&mut self, flow: Flow, now: f64) -> Effect {
        self.flow = if flow == Flow::Install && self.page == Page::Update { Flow::Update } else { flow };
        self.modal = None;
        self.status = JobStatus::default();
        let shared: Shared = Arc::new(Mutex::new(JobStatus::default()));
        self.job = Some(shared);
        self.job_t0 = now;
        let page = if flow == Flow::Uninstall { Page::UninstallProgress } else { Page::Progress };
        self.go(page, now);
        if self.preview {
            Effect::None
        } else {
            Effect::StartJob(self.flow)
        }
    }

    /// Advances animations and reads job progress. Returns true while something moves (the window keeps its timer on).
    pub fn tick(&mut self, now: f64) -> bool {
        if self.preview {
            if self.modal == Some(Modal::Waiting) && now - self.modal_t0 > 2.2 {
                self.modal = Some(Modal::Stuck);
                self.modal_t0 = now;
            }
            if let Some(job) = &self.job {
                // ~3 s from 0 to 100, like the mockups.
                let pct = (((now - self.job_t0) / 3.0) as f32 * 100.0).min(100.0);
                let n = self.info.files.len().max(1);
                let f = &self.info.files[((pct / 100.0 * n as f32) as usize).min(n - 1)];
                let mut st = job.lock().unwrap();
                st.pct = pct;
                st.file = format!(r"{}\{}", self.check.final_dir.display(), f);
                if pct >= 100.0 && now - self.job_t0 > 3.45 {
                    st.result = Some(Ok(()));
                }
            }
        }
        if let Some(c) = self.closing.clone() {
            let done = *c.lock().unwrap();
            match done {
                Some(true) => {
                    self.closing = None;
                    self.running = None;
                    self.pending = Some(self.start(Flow::Update, now));
                }
                Some(false) => {
                    self.closing = None;
                    self.modal = Some(Modal::Stuck);
                    self.modal_t0 = now;
                }
                None => {}
            }
        }
        if let Some(job) = self.job.clone() {
            self.status = job.lock().unwrap().clone();
            if let Some(r) = self.status.result.clone() {
                self.job = None;
                match r {
                    Ok(()) => self.go(if self.flow == Flow::Uninstall { Page::UninstallDone } else { Page::Done }, now),
                    Err(e) => {
                        self.error = Some(e);
                        self.go(Page::Error, now);
                    }
                }
            }
        }
        let anim = now - self.page_t0 < 0.2 || now - self.modal_t0 < 0.2;
        anim || self.job.is_some()
            || matches!(self.page, Page::Progress | Page::UninstallProgress)
            || matches!(self.modal, Some(Modal::Waiting) | Some(Modal::Stuck))
            || self.closing.is_some()
    }

    /// Shows the progress page at `pct` without a job (screenshots).
    pub fn freeze_progress(&mut self, pct: f32) {
        let n = self.info.files.len().max(1);
        let f = &self.info.files[((pct / 100.0 * n as f32) as usize).min(n - 1)];
        self.status = JobStatus { pct, file: format!(r"{}\{}", self.check.final_dir.display(), f), result: None, cancel: false };
    }

    pub fn title(&self) -> String {
        self.s.t(if self.page.uninstaller() { "uninstaller" } else { "setup" })
    }
}

/// Resolves the folder field and applies the "append \<Name>" rule: a folder that exists, is not empty and is not
/// an install of this app gets the app name added.
pub fn check_folder(raw: &str, info: &AppInfo, sys: &SystemDirs) -> FolderCheck {
    let raw = raw.trim().trim_matches('"');
    let p = PathBuf::from(raw.trim_end_matches(['\\', '/']).to_string() + if raw.len() <= 3 { "\\" } else { "" });
    let root = match paths::check_install_dir(&p, sys, &info.data_paths()) {
        Ok(()) => false,
        // A shared root is only ever a parent: always append \<Name>.
        Err(paths::PathProblem::SharedRoot) => true,
        Err(_) => return FolderCheck { final_dir: p },
    };
    let ours = p.join(gloria_core::manifest::INSTALL_MANIFEST_NAME).exists();
    let non_empty = std::fs::read_dir(&p).map(|mut d| d.next().is_some()).unwrap_or(false);
    if root || (non_empty && !ours) {
        let fin = p.join(paths::sanitize_file_name(info.name()));
        if paths::check_install_dir(&fin, sys, &info.data_paths()).is_ok() {
            return FolderCheck { final_dir: fin };
        }
    }
    FolderCheck { final_dir: p }
}

