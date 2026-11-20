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
                "id = \"{}\"\nname = \"{}\"\nversion = \"0.0.0\"\nmain_exe = \"dummy.exe\"\ndata_folders = ['%APPDATA%\\{}']\n",
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
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = Command::new("reg").args(["delete", &self.key(), "/f"]).output();
        let _ = std::fs::remove_file(self.lnk());
        let _ = std::fs::remove_dir_all(&self.data);
        let _ = std::fs::remove_dir_all(&self.work);
    }
}

fn run(exe: &Path, args: &[&str]) -> i32 {
    // Built as one raw command line so `/D=` stays unquoted, like NSIS expects.
    use std::os::windows::process::CommandExt;
    Command::new(exe).raw_arg(args.join(" ")).status().unwrap().code().unwrap()
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

    // A file locked by a running copy of the app: silent mode closes it.
    // (Covered by the engine's close_apps; not exercised here to keep the test free of windows.)

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
