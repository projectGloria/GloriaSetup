//! End-to-end tests: build the stub and the dummy app, pack real setups, and run them silently against this user's
//! HKCU and a folder under `%LOCALAPPDATA%\GloriaSetupTests`. Each test uses its own app id, name and data folder, and
//! a guard removes whatever is left (registry key, Start Menu shortcut, folders) even when the test fails.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gloria_core::archive;
use gloria_core::install_manifest::InstallManifest;
use gloria_core::payload;
use gloria_core::res;
use gloria_pack::sign::{find_signtool, Sign};
use gloria_pack::{icon, inspect, pack, Options};

const UNINSTALL: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root().join("target"))
}

/// Builds `gloria-stub` and the dummy app once per test run (debug profile).
fn built() -> &'static (PathBuf, PathBuf) {
    static B: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    B.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let st = Command::new(cargo).current_dir(root()).args(["build", "-p", "gloria-stub", "-p", "gloria-dummy"]).status().unwrap();
        assert!(st.success(), "cargo build failed");
        let d = target_dir().join("debug");
        (d.join("gloria-stub.exe"), d.join("dummy.exe"))
    })
}

struct TestApp {
    id: String,
    name: String,
    work: PathBuf,
    data: PathBuf,
    install: PathBuf,
}

impl TestApp {
    fn new(tag: &str) -> TestApp {
        let n = std::process::id();
        let id = format!("GloriaE2E{tag}{n}");
        let lad = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap());
        let base = lad.join("GloriaSetupTests").join(&id);
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let appdata = PathBuf::from(std::env::var("APPDATA").unwrap());
        TestApp {
            name: format!("GloriaSetup E2E {tag} {n}"),
            data: appdata.join(format!("com.gloria.e2e.{}", id.to_lowercase())),
            install: base.join("install"),
            work: base,
            id,
        }
    }

    fn key(&self) -> String {
        format!(r"{UNINSTALL}\projectGloria.{}", self.id)
    }

    fn lnk(&self) -> PathBuf {
        PathBuf::from(std::env::var("APPDATA").unwrap()).join(r"Microsoft\Windows\Start Menu\Programs").join(format!("{}.lnk", self.name))
    }

    /// Packs version `v` (0.1.0 carries an extra file that later versions drop) with `ico`.
    fn pack(&self, v: &str, ico: &str) -> PathBuf {
        self.pack_with(v, ico, "", None)
    }

    /// `pack` with extra gloria.toml lines and an optional WebView2 bootstrapper to bundle.
    fn pack_with(&self, v: &str, ico: &str, extra_toml: &str, webview2: Option<PathBuf>) -> PathBuf {
        self.pack_signed(v, ico, extra_toml, webview2, None)
    }

    fn pack_signed(&self, v: &str, ico: &str, extra_toml: &str, webview2: Option<PathBuf>, sign: Option<Sign>) -> PathBuf {
        let (stub, dummy) = built();
        let app = self.work.join(format!("app-{v}"));
        let _ = std::fs::remove_dir_all(&app);
        std::fs::create_dir_all(app.join("docs")).unwrap();
        std::fs::copy(dummy, app.join("dummy.exe")).unwrap();
        std::fs::write(app.join("version.txt"), v).unwrap();
        std::fs::write(app.join(r"docs\readme.txt"), format!("readme {v}")).unwrap();
        if v == "0.1.0" {
            std::fs::write(app.join("old-notes.txt"), "only in 0.1.0").unwrap();
        }
        let manifest = self.work.join("gloria.toml");
        std::fs::write(
            &manifest,
            format!(
                "id = \"{}\"\nname = \"{}\"\nversion = \"0.0.0\"\nmain_exe = \"dummy.exe\"\ndata_folders = ['%APPDATA%\\{}']\n{extra_toml}",
                self.id,
                self.name,
                self.data.file_name().unwrap().to_string_lossy()
            ),
        )
        .unwrap();
        let out = self.work.join(format!("dist-{v}"));
        let p = pack(&Options {
            stub: stub.clone(),
            app_dir: app,
            manifest,
            version: Some(v.into()),
            icon: Some(root().join("examples/dummy/icons").join(ico)),
            allow_default_icon: false,
            out_dir: out,
            level: 3,
            webview2_bootstrapper: webview2,
            sign,
        })
        .unwrap();
        p.setup
    }

    fn reg(&self, value: &str) -> Option<String> {
        let o = Command::new("reg").args(["query", &self.key(), "/v", value]).output().unwrap();
        if !o.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&o.stdout).into_owned();
        let line = text.lines().find(|l| l.trim_start().starts_with(value))?;
        let mut parts = line.trim().splitn(3, "    ");
        let (_, ty, v) = (parts.next()?, parts.next()?, parts.next().unwrap_or("").trim().to_string());
        Some(if ty == "REG_DWORD" { u64::from_str_radix(v.trim_start_matches("0x"), 16).unwrap().to_string() } else { v })
    }

    fn has_key(&self) -> bool {
        Command::new("reg").args(["query", &self.key()]).output().unwrap().status.success()
    }

    fn make_data(&self) {
        std::fs::create_dir_all(self.data.join("sub")).unwrap();
        std::fs::write(self.data.join(r"sub\settings.json"), "{}").unwrap();
    }

    fn uninstall(&self, args: &[&str]) -> i32 {
        let code = run(&self.install.join("uninstall.exe"), args);
        // The uninstaller hands off to a %TEMP% copy and returns at once; wait for the copy to finish.
        let t = Instant::now();
        while self.install.join("install-manifest.json").exists() || self.has_key() {
            assert!(t.elapsed() < Duration::from_secs(60), "uninstall did not finish");
            std::thread::sleep(Duration::from_millis(200));
        }
        std::thread::sleep(Duration::from_millis(300));
        code
    }

    /// Everything this app's setups and uninstallers logged in this test run.
    fn logs(&self) -> String {
        let mut out = String::new();
        for e in std::fs::read_dir(std::env::temp_dir()).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with(&format!("GloriaSetup-{}-", self.id)) && n.ends_with(".log") {
                out += &std::fs::read_to_string(e.path()).unwrap_or_default();
            }
        }
        out
    }

    fn version(&self) -> String {
        std::fs::read_to_string(self.install.join("version.txt")).unwrap()
    }

    /// Starts the installed dummy in a test mode; the guard kills it if the test fails first.
    fn start(&self, mode: &str) -> Running {
        let c = Command::new(self.install.join("dummy.exe")).arg(mode).spawn().unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        Running(c)
    }
}

struct Running(std::process::Child);

impl Running {
    fn exited(&mut self) -> bool {
        self.0.try_wait().unwrap().is_some()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = Command::new("reg").args(["delete", &self.key(), "/f"]).output();
        let _ = std::fs::remove_file(self.lnk());
        let _ = std::fs::remove_dir_all(&self.data);
        let _ = std::fs::remove_dir_all(&self.work);
        for e in std::fs::read_dir(std::env::temp_dir()).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with(&format!("GloriaSetup-{}-", self.id)) && n.ends_with(".log") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn run(exe: &Path, args: &[&str]) -> i32 {
    run_env(exe, args, &[])
}

fn run_env(exe: &Path, args: &[&str], env: &[(&str, &str)]) -> i32 {
    // Built as one raw command line so `/D=` stays unquoted, like NSIS expects.
    use std::os::windows::process::CommandExt;
    let mut c = Command::new(exe);
    c.raw_arg(args.join(" "));
    for (k, v) in env {
        c.env(k, v);
    }
    c.status().unwrap().code().unwrap()
}

/// `files` without the `.gloria` work folder.
fn app_files(dir: &Path) -> Vec<String> {
    files(dir).into_iter().filter(|f| !f.starts_with(".gloria")).collect()
}

fn files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(base: &Path, d: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.push(p.strip_prefix(base).unwrap().to_string_lossy().replace('/', "\\"));
            }
        }
    }
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn gloria_icon_images() -> Vec<Vec<u8>> {
    let ico = std::fs::read(root().join("assets/brand/GloriaSetup.ico")).unwrap();
    res::parse_ico(&ico).unwrap().into_iter().map(|i| i.data).collect()
}

// ───────────────────────────── icons and version info ─────────────────────────────

#[test]
fn bare_stub_carries_the_gloriasetup_icon() {
    let (stub, _) = built();
    let i = inspect(stub).unwrap();
    let ours = gloria_icon_images();
    assert!(!i.icons.is_empty());
    assert!(i.icons.iter().all(|img| ours.contains(&img.data)), "stub icon is not GloriaSetup.ico");
    assert!(i.payload.is_none());
}

#[test]
fn each_setup_gets_its_own_icon_and_version_and_no_gloria_icons() {
    let t = TestApp::new("Icons");
    let ours = gloria_icon_images();
    for (v, ico) in [("0.1.0", "dummy-a.ico"), ("0.2.0", "dummy-b.ico")] {
        let setup = t.pack(v, ico);
        let want = icon::load(&root().join("examples/dummy/icons").join(ico)).unwrap();
        let i = inspect(&setup).unwrap();
        assert_eq!(i.icon_group_count, 1);
        assert_eq!(i.icon_count, want.len(), "{v}: RT_ICON count");
        for w in &want {
            assert!(i.icons.iter().any(|g| g.data == w.data), "{v}: missing {}px image", w.width);
        }
        assert!(i.icons.iter().all(|g| !ours.contains(&g.data)), "{v}: a GloriaSetup RT_ICON is left");
        assert_eq!(i.string("ProductVersion"), Some(v));
        assert_eq!(i.string("ProductName"), Some(t.name.as_str()));
        assert_eq!(i.string("CompanyName"), Some("projectGloria"));
        assert_eq!(&i.version.as_ref().unwrap().0[..3], &v.split('.').map(|x| x.parse().unwrap()).collect::<Vec<u16>>()[..]);
        // The uninstaller inside the archive is patched the same way and carries no archive of its own.
        let p = i.payload.unwrap();
        let mut f = std::fs::File::open(&setup).unwrap();
        use std::io::{Read, Seek, SeekFrom};
        f.seek(SeekFrom::Start(p.archive_offset)).unwrap();
        let dir = t.work.join(format!("unpacked-{v}"));
        archive::unpack(f.take(p.archive_len), &dir, |_, _| {}).unwrap();
        let un = dir.join("uninstall.exe");
        let ui = inspect(&un).unwrap();
        assert!(ui.icons.iter().all(|g| !ours.contains(&g.data)), "{v}: uninstall.exe has the GloriaSetup icon");
        assert!(ui.icons.iter().any(|g| want.iter().any(|w| w.data == g.data)));
        let up = payload::read(&mut std::fs::File::open(&un).unwrap()).unwrap().unwrap();
        assert!(up.is_uninstaller());
        assert_eq!(up.archive_len, 0, "{v}: uninstall.exe carries an archive");
        assert!(std::fs::metadata(&un).unwrap().len() < std::fs::metadata(&setup).unwrap().len());
    }
}

// ───────────────────────────── signing ─────────────────────────────

/// Runs only with `GLORIA_TEST_SIGN_PFX=<test .pfx>` (password in `GLORIA_SIGN_PASSWORD`, may be empty) and signtool
/// installed; CI makes a throwaway self-signed certificate for it. No timestamp server, so it works offline.
#[test]
fn signed_setup_and_uninstaller_keep_their_payload_and_install() {
    let Some(pfx) = std::env::var_os("GLORIA_TEST_SIGN_PFX").map(PathBuf::from) else {
        eprintln!("skipped: set GLORIA_TEST_SIGN_PFX to a test code-signing .pfx");
        return;
    };
    let signtool = find_signtool().expect("GLORIA_TEST_SIGN_PFX is set but signtool.exe was not found");
    let sign = Sign { signtool, pfx, password: std::env::var("GLORIA_SIGN_PASSWORD").ok().filter(|p| !p.is_empty()), timestamp_url: None };
    let t = TestApp::new("Signed");
    let setup = t.pack_signed("0.1.0", "dummy-a.ico", "", None, Some(sign.clone()));

    let signed = |p: &Path| payload::pe_info(&std::fs::read(p).unwrap()).unwrap().security.is_some();
    assert!(signed(&setup), "setup signed");
    let i = inspect(&setup).unwrap();
    assert!(i.payload.is_some(), "payload found after signing");
    assert_eq!(i.string("ProductVersion"), Some("0.1.0"), "resources untouched by signing");

    let d = t.install.to_string_lossy().into_owned();
    assert_eq!(run(&setup, &["/S", &format!("/D={d}")]), 0);
    let un = t.install.join("uninstall.exe");
    assert!(signed(&un), "the installed uninstaller is signed too");
    assert!(payload::read(&mut std::fs::File::open(&un).unwrap()).unwrap().unwrap().is_uninstaller());
    assert_eq!(t.uninstall(&["/S"]), 0);
    assert!(!t.has_key());

    // A signed stub is refused: resources can't be patched without breaking its signature.
    let (stub, _) = built();
    let signed_stub = t.work.join("signed-stub.exe");
    std::fs::copy(stub, &signed_stub).unwrap();
    sign.sign(&signed_stub, "stub").unwrap();
    let err = pack(&Options {
        stub: signed_stub,
        app_dir: t.work.join("app-0.1.0"),
        manifest: t.work.join("gloria.toml"),
        version: Some("0.1.0".into()),
        icon: Some(root().join("examples/dummy/icons/dummy-a.ico")),
        allow_default_icon: false,
        out_dir: t.work.join("dist-signed-stub"),
        level: 3,
        webview2_bootstrapper: None,
        sign: None,
    })
    .unwrap_err();
    assert!(err.contains("signed"), "{err}");
}

// ───────────────────────────── install, update, uninstall ─────────────────────────────

#[test]
fn install_update_reinstall_downgrade_uninstall() {
    let t = TestApp::new("Flow");
    let s1 = t.pack("0.1.0", "dummy-a.ico");
    let s2 = t.pack("0.2.0", "dummy-b.ico");
    let d = t.install.to_string_lossy().into_owned();

    // Fresh install.
    assert_eq!(run(&s1, &["/S", &format!("/D={d}")]), 0);
    assert_eq!(files(&t.install), ["docs\\readme.txt", "dummy.exe", "install-manifest.json", "old-notes.txt", "uninstall.exe", "version.txt"]);
    assert_eq!(t.reg("DisplayName").as_deref(), Some(t.name.as_str()));
    assert_eq!(t.reg("DisplayVersion").as_deref(), Some("0.1.0"));
    assert_eq!(t.reg("Publisher").as_deref(), Some("projectGloria"));
    assert_eq!(t.reg("InstallLocation").as_deref(), Some(d.as_str()));
    assert_eq!(t.reg("DisplayIcon"), Some(format!(r"{d}\dummy.exe")));
    assert_eq!(t.reg("UninstallString"), Some(format!(r#""{d}\uninstall.exe""#)));
    assert_eq!(t.reg("QuietUninstallString"), Some(format!(r#""{d}\uninstall.exe" /S"#)));
    assert_eq!(t.reg("NoModify").as_deref(), Some("1"));
    assert_eq!(t.reg("NoRepair").as_deref(), Some("1"));
    let m = InstallManifest::load(&t.install).unwrap();
    let kb = (m.total_size() + 1023) / 1024;
    assert_eq!(t.reg("EstimatedSize"), Some(kb.to_string()));
    assert!(t.lnk().exists(), "Start Menu shortcut");
    assert_eq!(m.app_id, t.id);
    assert_eq!(m.files.len(), 5);

    // Update: replaces files, drops the obsolete one, keeps unrelated files and user data.
    t.make_data();
    std::fs::write(t.install.join("user-notes.txt"), "not ours").unwrap();
    assert_eq!(run(&s2, &["/S"]), 0);
    assert_eq!(t.reg("DisplayVersion").as_deref(), Some("0.2.0"));
    assert!(!t.install.join("old-notes.txt").exists());
    assert!(t.install.join("user-notes.txt").exists());
    assert_eq!(std::fs::read_to_string(t.install.join("version.txt")).unwrap(), "0.2.0");
    assert!(!t.install.join(".gloria").exists());
    assert!(t.data.join(r"sub\settings.json").exists(), "an update must keep user data");

    // Same-version reinstall keeps data; with /PURGE it is a clean reinstall that removes it.
    assert_eq!(run(&s2, &["/S"]), 0);
    assert!(t.data.exists());
    assert_eq!(run(&s2, &["/S", "/PURGE"]), 0);
    assert!(!t.data.exists(), "/PURGE clean reinstall removes data");
    assert!(t.install.join("dummy.exe").exists());

    // Downgrade: refused (2) without /ALLOWDOWNGRADE, data kept when allowed without /PURGE.
    t.make_data();
    assert_eq!(run(&s1, &["/S"]), 2);
    assert_eq!(t.reg("DisplayVersion").as_deref(), Some("0.2.0"));
    assert_eq!(run(&s1, &["/S", "/ALLOWDOWNGRADE"]), 0);
    assert_eq!(t.reg("DisplayVersion").as_deref(), Some("0.1.0"));
    assert!(t.install.join("old-notes.txt").exists());
    assert!(t.data.exists(), "a downgrade without /PURGE keeps data");

    // Silent uninstall keeps data and unrelated files, removes exactly what it installed.
    assert_eq!(t.uninstall(&["/S"]), 0);
    assert_eq!(files(&t.install), ["user-notes.txt"]);
    assert!(!t.has_key());
    assert!(!t.lnk().exists());
    assert!(t.data.join(r"sub\settings.json").exists(), "uninstall without /PURGE keeps data");

    // Reinstall into that folder, which is now someone else's non-empty folder: setup appends \<Name>. Uninstalling
    // with /PURGE removes the data too and the folder setup created, never the sibling file.
    assert_eq!(run(&s2, &["/S", &format!("/D={d}")]), 0);
    let sub = t.install.join(&t.name);
    assert_eq!(t.reg("InstallLocation"), Some(sub.to_string_lossy().into_owned()));
    assert_eq!(run(&sub.join("uninstall.exe"), &["/S", "/PURGE"]), 0);
    let t0 = Instant::now();
    while sub.exists() {
        assert!(t0.elapsed() < Duration::from_secs(60), "uninstall did not finish");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(!t.has_key());
    assert!(!t.data.exists(), "/PURGE removes data");
    assert_eq!(files(&t.install), ["user-notes.txt"]);
}

#[test]
fn silent_exit_codes_and_dry_run() {
    let t = TestApp::new("Codes");
    let s = t.pack("0.2.0", "dummy-b.ico");
    let pf = std::env::var("ProgramFiles").unwrap();
    assert_eq!(run(&s, &["/S", &format!(r"/D={pf}\Nope")]), 6, "Program Files is refused");
    assert_eq!(run(&s, &["/S", "/D=relative\\path"]), 6);
    assert_eq!(run(&s, &["/S", "--bogus"]), 6);
    let d = t.install.to_string_lossy().into_owned();
    assert_eq!(run(&s, &["/S", "--dry-run", &format!("/D={d}")]), 0);
    assert!(!t.install.exists(), "--dry-run writes nothing");
    assert!(!t.has_key());
    assert!(!t.lnk().exists());
}

#[test]
fn spaces_in_unquoted_install_dir() {
    let t = TestApp::new("Spaces");
    let s = t.pack("0.1.0", "dummy-a.ico");
    let d = t.work.join("With Spaces Here");
    assert_eq!(run(&s, &["/S", &format!("/D={}", d.display())]), 0);
    assert!(d.join("dummy.exe").exists());
    assert_eq!(t.reg("InstallLocation"), Some(d.to_string_lossy().into_owned()));
    let un = d.join("uninstall.exe");
    assert_eq!(run(&un, &["/S"]), 0);
    let t0 = Instant::now();
    while d.exists() {
        assert!(t0.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(200));
    }
}

// ───────────────────────────── interrupted installs, running apps, locked files ─────────────────────────────

/// The update from 0.1.0 to 0.2.0 has 6 renames: 4 files and the install manifest placed (0..=4), 1 obsolete file
/// moved away (5). Then come the registry write (6) and deleting the journal (7). A debug stub exits with 99 at
/// `GLORIA_TEST_CRASH_AT`. Each crash must leave a journal that the next run rolls back exactly.
#[test]
fn update_killed_mid_commit_is_rolled_back_by_the_next_run() {
    let t = TestApp::new("Crash");
    let s1 = t.pack("0.1.0", "dummy-a.ico");
    let s2 = t.pack("0.2.0", "dummy-b.ico");
    let d = t.install.to_string_lossy().into_owned();
    assert_eq!(run(&s1, &["/S", &format!("/D={d}")]), 0);
    std::fs::write(t.install.join("user-notes.txt"), "not ours").unwrap();
    let before = app_files(&t.install);
    let manifest = std::fs::read(t.install.join("install-manifest.json")).unwrap();

    for at in ["0", "3", "5", "6", "7"] {
        assert_eq!(run_env(&s2, &["/S"], &[("GLORIA_TEST_CRASH_AT", at)]), 99, "crash at {at}");
        assert!(t.install.join(r".gloria\journal").exists(), "crash at {at} left no journal");
        // The next run recovers first. Stopping it before its own first rename shows the folder as recovery left it.
        assert_eq!(run_env(&s2, &["/S"], &[("GLORIA_TEST_CRASH_AT", "0")]), 99, "probe after {at}");
        assert_eq!(app_files(&t.install), before, "crash at {at}");
        assert_eq!(t.version(), "0.1.0", "crash at {at}");
        assert_eq!(std::fs::read(t.install.join("install-manifest.json")).unwrap(), manifest, "crash at {at}");
    }
    assert!(t.logs().contains("was interrupted; rolled it back"));

    // A normal run recovers and updates.
    assert_eq!(run(&s2, &["/S"]), 0);
    assert_eq!(app_files(&t.install), ["docs\\readme.txt", "dummy.exe", "install-manifest.json", "uninstall.exe", "user-notes.txt", "version.txt"]);
    assert!(!t.install.join(".gloria").exists());
    assert_eq!(t.version(), "0.2.0");
    assert_eq!(t.reg("DisplayVersion").as_deref(), Some("0.2.0"));

    // The uninstaller also recovers first: an interrupted reinstall, then uninstall, leaves only the user's file.
    assert_eq!(run_env(&s2, &["/S"], &[("GLORIA_TEST_CRASH_AT", "3")]), 99);
    assert_eq!(t.uninstall(&["/S"]), 0);
    assert_eq!(files(&t.install), ["user-notes.txt"]);
}

#[test]
fn fresh_install_killed_mid_commit_is_cleaned_up_by_the_next_run() {
    let t = TestApp::new("CrashFresh");
    let s1 = t.pack("0.1.0", "dummy-a.ico");
    let d = t.install.to_string_lossy().into_owned();
    assert_eq!(run_env(&s1, &["/S", &format!("/D={d}")], &[("GLORIA_TEST_CRASH_AT", "2")]), 99);
    assert!(t.install.join(r".gloria\journal").exists());
    assert!(!t.has_key());
    // Same folder again: the half-done install is ours, so setup must not append \<Name>.
    assert_eq!(run(&s1, &["/S", &format!("/D={d}")]), 0);
    assert_eq!(files(&t.install), ["docs\\readme.txt", "dummy.exe", "install-manifest.json", "old-notes.txt", "uninstall.exe", "version.txt"]);
    assert_eq!(t.reg("InstallLocation").as_deref(), Some(d.as_str()));
}

#[test]
fn silent_update_closes_the_running_app() {
    let t = TestApp::new("Close");
    let s1 = t.pack("0.1.0", "dummy-a.ico");
    let s2 = t.pack("0.2.0", "dummy-b.ico");
    let d = t.install.to_string_lossy().into_owned();
    assert_eq!(run(&s1, &["/S", &format!("/D={d}")]), 0);

    // An app that closes when asked: no force needed.
    let mut app = t.start("--hold");
    assert!(!app.exited());
    assert_eq!(run(&s2, &["/S"]), 0);
    assert!(app.exited(), "the app was not closed: {}", t.logs());
    assert_eq!(t.version(), "0.2.0");
    assert!(t.logs().contains("running: "));
    assert!(!t.logs().contains("force-closing"), "a polite app was force-closed");

    // An app that ignores the request: silent mode force-closes it after the grace period.
    let mut app = t.start("--stubborn");
    assert!(!app.exited());
    assert_eq!(run(&s2, &["/S"]), 0);
    assert!(app.exited(), "the stubborn app was not force-closed");
    assert!(t.logs().contains("force-closing"));
}

#[test]
fn locked_file_fails_with_3_and_keeps_the_old_version() {
    use std::os::windows::fs::OpenOptionsExt;
    let t = TestApp::new("Locked");
    let s1 = t.pack("0.1.0", "dummy-a.ico");
    let s2 = t.pack("0.2.0", "dummy-b.ico");
    let d = t.install.to_string_lossy().into_owned();
    assert_eq!(run(&s1, &["/S", &format!("/D={d}")]), 0);
    let before = files(&t.install);
    let manifest = std::fs::read(t.install.join("install-manifest.json")).unwrap();

    // Another program holds version.txt open with no sharing. It can't be moved, so the update must roll back.
    let lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(t.install.join("version.txt")).unwrap();
    assert_eq!(run(&s2, &["/S"]), 3);
    drop(lock);
    assert_eq!(files(&t.install), before, "no .gloria left, nothing half-replaced");
    assert_eq!(t.version(), "0.1.0");
    assert_eq!(std::fs::read(t.install.join("install-manifest.json")).unwrap(), manifest);
    assert_eq!(t.reg("DisplayVersion").as_deref(), Some("0.1.0"));
    assert!(t.logs().contains("rolled back; the previous version is unchanged"));

    assert_eq!(run(&s2, &["/S"]), 0);
    assert_eq!(t.version(), "0.2.0");
}

#[test]
fn relaunch_starts_the_app_only_when_asked() {
    let t = TestApp::new("Relaunch");
    let s1 = t.pack("0.1.0", "dummy-a.ico");
    let d = t.install.to_string_lossy().into_owned();
    let data = t.data.to_string_lossy().into_owned();
    // The dummy inherits these: it writes its settings into the test's data folder and shows no window.
    let env = [("GLORIA_DUMMY_DATA", data.as_str()), ("GLORIA_DUMMY_QUIET", "1")];
    let settings = t.data.join("settings.json");

    assert_eq!(run_env(&s1, &["/S", &format!("/D={d}")], &env), 0);
    std::thread::sleep(Duration::from_secs(2));
    assert!(!settings.exists(), "started without /RELAUNCH");

    assert_eq!(run_env(&s1, &["/S", "/RELAUNCH"], &env), 0);
    let t0 = Instant::now();
    while !settings.exists() {
        assert!(t0.elapsed() < Duration::from_secs(20), "/RELAUNCH did not start the app");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(std::fs::read_to_string(&settings).unwrap().contains("0.1.0"));
}

// ───────────────────────────── legacy migration ─────────────────────────────

/// A fake install by an older installer: a folder with the app and an uninstaller (the dummy in its fake-uninstaller
/// mode), registered under a non-projectGloria key in HKCU. The key is deleted on drop.
struct LegacyInstall {
    key: String,
    dir: PathBuf,
}

impl LegacyInstall {
    /// `nsis` marks the uninstaller with the NSIS signature, so the setup runs it as `/S _?=<dir>`; otherwise it is
    /// registered with a `QuietUninstallString`.
    fn new(t: &TestApp, nsis: bool) -> LegacyInstall {
        let (_, dummy) = built();
        let dir = t.work.join("Old Install");
        std::fs::create_dir_all(dir.join("resources")).unwrap();
        std::fs::copy(dummy, dir.join("dummy.exe")).unwrap();
        std::fs::write(dir.join(r"resources\old.dat"), "old").unwrap();
        let un = dir.join("uninstall.exe");
        let mut bytes = std::fs::read(dummy).unwrap();
        if nsis {
            bytes.extend_from_slice(b"\0NullsoftInst\0");
        }
        std::fs::write(&un, bytes).unwrap();
        let key = format!(r"{UNINSTALL}\{}", t.name);
        let add = |v: &str, d: &str| {
            let o = Command::new("reg").args(["add", &key, "/v", v, "/t", "REG_SZ", "/d", d, "/f"]).output().unwrap();
            assert!(o.status.success(), "reg add {v}");
        };
        add("DisplayName", &format!("{} 0.9.0", t.name));
        add("DisplayVersion", "0.9.0");
        add("Publisher", "Somebody Else");
        add("InstallLocation", &dir.to_string_lossy());
        add("UninstallString", &format!("\"{}\"", un.display()));
        if !nsis {
            add("QuietUninstallString", &format!("\"{}\" /S", un.display()));
        }
        LegacyInstall { key, dir }
    }

    fn registered(&self) -> bool {
        Command::new("reg").args(["query", &self.key]).output().unwrap().status.success()
    }
}

impl Drop for LegacyInstall {
    fn drop(&mut self) {
        let _ = Command::new("reg").args(["delete", &self.key, "/f"]).output();
    }
}

#[test]
fn silent_setup_removes_an_old_install_and_keeps_the_data() {
    let t = TestApp::new("Legacy");
    let old = LegacyInstall::new(&t, false);
    t.make_data();
    let setup = t.pack("0.1.0", "dummy-a.ico");
    let data = t.data.to_string_lossy().into_owned();
    // The fake uninstaller also wipes the data folder, like some do; the setup must put it back.
    let env = [("GLORIA_TEST_NO_HKLM", "1"), ("GLORIA_DUMMY_LEGACY_KEY", old.key.as_str()), ("GLORIA_DUMMY_LEGACY_WIPE", data.as_str())];

    assert_eq!(run_env(&setup, &["/S", "--dry-run"], &env), 0);
    assert!(old.registered() && old.dir.join("dummy.exe").exists(), "dry run changed nothing");

    assert_eq!(run_env(&setup, &["/S"], &env), 0);
    assert!(!old.registered(), "old entry removed");
    // The fake can't delete its own running exe (real ones do it from a %TEMP% copy); nothing else is left, and the
    // setup doesn't touch files it didn't put there.
    assert_eq!(app_files(&old.dir), ["uninstall.exe"], "old files removed");
    assert!(t.has_key());
    assert_eq!(t.reg("DisplayVersion").as_deref(), Some("0.1.0"));
    assert!(t.data.join(r"sub\settings.json").exists(), "data restored after the old uninstaller wiped it");
    let logs = t.logs();
    assert!(logs.contains("legacy install:") && logs.contains("backed up"), "{logs}");
    let tmp_backup = std::env::temp_dir().join(format!("GloriaSetup-backup-{}", t.id));
    assert!(!tmp_backup.exists(), "backup deleted");
}

#[test]
fn a_failed_migration_keeps_the_data_and_leaves_no_backup() {
    let t = TestApp::new("LegacyFail");
    let old = LegacyInstall::new(&t, false);
    // No QuietUninstallString and not NSIS: silent mode can't remove it and stops after the backup was made.
    let o = Command::new("reg").args(["delete", &old.key, "/v", "QuietUninstallString", "/f"]).output().unwrap();
    assert!(o.status.success());
    t.make_data();
    let setup = t.pack("0.1.0", "dummy-a.ico");
    let env = [("GLORIA_TEST_NO_HKLM", "1"), ("GLORIA_DUMMY_LEGACY_KEY", old.key.as_str())];

    assert_eq!(run_env(&setup, &["/S"], &env), 1);
    assert!(old.registered() && old.dir.join("dummy.exe").exists(), "old install untouched");
    assert!(!t.has_key(), "nothing installed");
    assert!(t.data.join(r"sub\settings.json").exists(), "data kept");
    assert!(t.logs().contains("backed up"), "the backup was made");
    let tmp_backup = std::env::temp_dir().join(format!("GloriaSetup-backup-{}", t.id));
    assert!(!tmp_backup.exists(), "backup deleted after the failure");
}

#[test]
fn silent_setup_replaces_an_nsis_install_in_place_and_purge_drops_the_data() {
    let t = TestApp::new("LegacyNsis");
    let old = LegacyInstall::new(&t, true);
    t.make_data();
    let setup = t.pack("0.1.0", "dummy-a.ico");
    let env = [("GLORIA_TEST_NO_HKLM", "1"), ("GLORIA_DUMMY_LEGACY_KEY", old.key.as_str())];
    let d = old.dir.to_string_lossy().into_owned();

    // /D at the old folder installs right there, not in "<old folder>\<Name>", even though it isn't empty yet.
    assert_eq!(run_env(&setup, &["/S", "/PURGE", &format!("/D={d}")], &env), 0);
    assert!(!old.registered());
    assert!(t.logs().contains("_?="), "NSIS uninstaller run in place");
    let f = app_files(&old.dir);
    assert!(f.contains(&"dummy.exe".to_string()) && f.contains(&"install-manifest.json".to_string()), "{f:?}");
    assert!(!f.iter().any(|x| x.starts_with("resources")), "old files gone: {f:?}");
    assert_eq!(t.reg("InstallLocation").as_deref(), Some(d.as_str()));
    assert!(!t.data.exists(), "/PURGE removes the data");

    // Our own key now wins: a second run is a plain reinstall with no migration.
    assert_eq!(run_env(&setup, &["/S"], &env), 0);
    assert_eq!(t.reg("InstallLocation").as_deref(), Some(d.as_str()));
}

// ───────────────────────────── WebView2 ─────────────────────────────

#[test]
fn missing_webview2_is_installed_first_and_a_failed_bootstrapper_exits_5() {
    let t = TestApp::new("WebView2");
    let (_, dummy) = built();
    let setup = t.pack_with("0.1.0", "dummy-a.ico", "needs_webview2 = true\n", Some(dummy.clone()));
    let marker = t.work.join("webview2-runtime");
    let m = marker.to_string_lossy().into_owned();
    let d = t.install.to_string_lossy().into_owned();

    // The bootstrapper fails: exit 5, nothing installed.
    let fail = [("GLORIA_TEST_WEBVIEW2", m.as_str()), ("GLORIA_DUMMY_WEBVIEW2_FAIL", "1")];
    assert_eq!(run_env(&setup, &["/S", &format!("/D={d}")], &fail), 5);
    assert!(!t.has_key());
    assert!(files(&t.install).is_empty(), "nothing left in the folder: {:?}", files(&t.install));

    // It works: the runtime appears, the install goes on, and the bootstrapper itself is never installed.
    let ok = [("GLORIA_TEST_WEBVIEW2", m.as_str())];
    assert_eq!(run_env(&setup, &["/S", &format!("/D={d}")], &ok), 0);
    assert!(marker.exists());
    assert!(t.has_key());
    assert!(!app_files(&t.install).iter().any(|f| f.to_lowercase().contains("webview2")), "{:?}", app_files(&t.install));

    // Present now: the bootstrapper isn't run again (it would fail).
    assert_eq!(run_env(&setup, &["/S"], &fail), 0);
}

#[test]
fn bootstrapper_needs_needs_webview2() {
    let t = TestApp::new("WebView2Pack");
    let (stub, dummy) = built();
    let app = t.work.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::copy(dummy, app.join("dummy.exe")).unwrap();
    let manifest = t.work.join("gloria.toml");
    let toml = |extra: &str| format!("id = \"{}\"\nname = \"{}\"\nversion = \"0.1.0\"\nmain_exe = \"dummy.exe\"\n{extra}", t.id, t.name);
    let o = |bootstrapper: PathBuf| Options {
        stub: stub.clone(),
        app_dir: app.clone(),
        manifest: manifest.clone(),
        version: None,
        icon: Some(root().join("examples/dummy/icons/dummy-a.ico")),
        allow_default_icon: false,
        out_dir: t.work.join("dist"),
        level: 3,
        webview2_bootstrapper: Some(bootstrapper),
        sign: None,
    };
    std::fs::write(&manifest, toml("")).unwrap();
    assert!(pack(&o(dummy.clone())).unwrap_err().contains("needs_webview2"));
    std::fs::write(&manifest, toml("needs_webview2 = true\n")).unwrap();
    assert!(pack(&o(manifest.clone())).unwrap_err().contains("not an exe"));
    let p = pack(&o(dummy.clone())).unwrap();
    assert!(p.files.iter().any(|f| f.path.ends_with("MicrosoftEdgeWebview2Setup.exe")), "bundled");
}
