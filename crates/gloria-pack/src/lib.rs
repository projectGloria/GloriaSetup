//! gloria-pack: builds `<Name>_<version>_x64-setup.exe` from the stub, an app folder and `gloria.toml`.
//!
//! Order matters: patch resources (icon, version info) → append the payload → sign (optional, `--sign-pfx`).
//! The stub's GloriaSetup icon is purged from every copy that carries an app, so neither the setup nor the
//! `uninstall.exe` inside it ever shows it, unless `--allow-default-icon` makes an explicit test build.

pub mod icon;
pub mod resources;
pub mod sign;

use std::path::{Path, PathBuf};

use gloria_core::archive::{self, FileEntry};
use gloria_core::manifest::{AppManifest, RuntimeManifest, INSTALL_MANIFEST_NAME, PUBLISHER, UNINSTALLER_NAME, WEBVIEW2_DIR, WEBVIEW2_SETUP_NAME, WORK_DIR};
use gloria_core::payload::{self, FLAG_TEST_BUILD, FLAG_UNINSTALLER};
use gloria_core::res::{self, IconImage, RT_VERSION};

pub struct Options {
    pub stub: PathBuf,
    pub app_dir: PathBuf,
    pub manifest: PathBuf,
    /// Overrides `version` from gloria.toml (CI passes the tag).
    pub version: Option<String>,
    /// Overrides `icon` from gloria.toml. Relative to the current directory.
    pub icon: Option<PathBuf>,
    /// With no icon set, keep the GloriaSetup icon and mark the build TEST BUILD.
    pub allow_default_icon: bool,
    pub out_dir: PathBuf,
    /// zstd level (default 19).
    pub level: i32,
    /// The WebView2 Evergreen bootstrapper to bundle (apps with `needs_webview2`). Run only if the runtime is missing.
    pub webview2_bootstrapper: Option<PathBuf>,
    /// Authenticode-sign the uninstaller and the setup, each as its last change.
    pub sign: Option<sign::Sign>,
}

#[derive(Debug)]
pub struct Packed {
    pub setup: PathBuf,
    pub manifest: RuntimeManifest,
    pub files: Vec<FileEntry>,
    pub uninstaller_len: u64,
}

pub fn setup_file_name(app: &AppManifest) -> String {
    let name = gloria_core::paths::sanitize_file_name(&app.name).replace(' ', "");
    format!("{name}_{}_x64-setup.exe", app.version)
}

pub fn load_manifest(path: &Path, version: Option<&str>) -> Result<AppManifest, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut m: AppManifest = toml_parse(&text)?;
    if let Some(v) = version {
        m.version = v.strip_prefix('v').unwrap_or(v).to_string();
    }
    m.validate()?;
    Ok(m)
}

/// Parses without validating, so a `--version` override can replace the version before validation.
fn toml_parse(text: &str) -> Result<AppManifest, String> {
    toml::from_str(text).map_err(|e| format!("gloria.toml: {e}"))
}

fn version_info(app: &AppManifest, description: String, original: &str) -> Vec<u8> {
    let v = app.parsed_version().file_version();
    let strings = [
        ("CompanyName", PUBLISHER.to_string()),
        ("FileDescription", description),
        ("FileVersion", app.version.clone()),
        ("InternalName", original.trim_end_matches(".exe").to_string()),
        ("OriginalFilename", original.to_string()),
        ("ProductName", app.name.clone()),
        ("ProductVersion", app.version.clone()),
        ("Comments", format!("Built with GloriaSetup {}", gloria_core::GLORIA_SETUP_VERSION)),
    ];
    res::version_info(&res::VersionInfo { file_version: v, product_version: v, strings: &strings })
}

/// Copies the stub to `dest`, patches icon + version info, and returns the patched bytes.
fn patched_stub(stub: &[u8], dest: &Path, icon: Option<&[IconImage]>, vi: Vec<u8>) -> Result<Vec<u8>, String> {
    let info = payload::pe_info(stub).map_err(|e| format!("stub: {e}"))?;
    if info.security.is_some() {
        return Err("the stub is signed; pack an unsigned stub and sign the finished setup".into());
    }
    if info.overlay_start < stub.len() as u64 {
        return Err("the stub already carries data after its last section (is it already packed?)".into());
    }
    std::fs::write(dest, stub).map_err(|e| format!("{}: {e}", dest.display()))?;
    let r = resources::patch(dest, &resources::Patch { icon, version_info: vi });
    let bytes = r.and_then(|_| std::fs::read(dest)).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(dest);
    bytes
}

pub fn pack(o: &Options) -> Result<Packed, String> {
    let app = load_manifest(&o.manifest, o.version.as_deref())?;
    let manifest_dir = o.manifest.parent().unwrap_or(Path::new("."));

    // App folder checks.
    if !o.app_dir.is_dir() {
        return Err(format!("app folder {} does not exist", o.app_dir.display()));
    }
    for reserved in [UNINSTALLER_NAME, INSTALL_MANIFEST_NAME, WORK_DIR, WEBVIEW2_DIR] {
        if o.app_dir.join(reserved).exists() {
            return Err(format!(
                "the app folder contains {reserved:?}, which GloriaSetup writes itself; remove it from {}",
                o.app_dir.display()
            ));
        }
    }
    if !o.app_dir.join(app.main_exe_rel()).is_file() {
        return Err(format!("main_exe {:?} is not in {}", app.main_exe, o.app_dir.display()));
    }

    // Icon.
    let icon_path = match (&o.icon, &app.icon) {
        (Some(p), _) => Some(p.clone()),
        (None, Some(rel)) => Some(manifest_dir.join(rel)),
        (None, None) => None,
    };
    let icons = match icon_path {
        Some(p) => Some(icon::load(&p)?),
        None if o.allow_default_icon => None,
        None => {
            return Err("no icon: set `icon` in gloria.toml or pass --icon (or --allow-default-icon for a test build)".into())
        }
    };
    let test_build = icons.is_none();
    let test_flag = if test_build { FLAG_TEST_BUILD } else { 0 };

    let stub = std::fs::read(&o.stub).map_err(|e| format!("stub {}: {e}", o.stub.display()))?;
    std::fs::create_dir_all(&o.out_dir).map_err(|e| format!("{}: {e}", o.out_dir.display()))?;
    let tmp = o.out_dir.join(format!(".gloria-pack-{}.tmp.exe", std::process::id()));

    // Read the app files.
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (rel, path) in archive::list_dir(&o.app_dir).map_err(|e| format!("{}: {e}", o.app_dir.display()))? {
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        files.push((rel, data));
    }

    // The WebView2 bootstrapper rides in the archive under a reserved folder the engine never installs.
    match (&o.webview2_bootstrapper, app.needs_webview2) {
        (Some(p), true) => {
            let data = std::fs::read(p).map_err(|e| format!("webview2 bootstrapper {}: {e}", p.display()))?;
            if !data.starts_with(b"MZ") {
                return Err(format!("webview2 bootstrapper {} is not an exe", p.display()));
            }
            files.push((format!("{WEBVIEW2_DIR}\\{WEBVIEW2_SETUP_NAME}"), data));
        }
        (Some(_), false) => return Err("--webview2-bootstrapper needs `needs_webview2 = true` in gloria.toml".into()),
        (None, _) => {}
    }

    let mut runtime = RuntimeManifest {
        app: app.clone(),
        test_build,
        packer: gloria_core::GLORIA_SETUP_VERSION.to_string(),
        unpacked_size: 0,
        file_count: 0,
    };

    // 1. uninstall.exe: patched stub + manifest, no archive.
    let mut uninstaller = patched_stub(
        &stub,
        &tmp,
        icons.as_deref(),
        version_info(&app, format!("{} Uninstaller", app.name), UNINSTALLER_NAME),
    )?;
    payload::append(&mut uninstaller, &runtime.to_json(), &[], FLAG_UNINSTALLER | test_flag).map_err(|e| e.to_string())?;
    if let Some(s) = &o.sign {
        // Signed before it goes into the archive, so the installed uninstaller carries the signature too.
        std::fs::write(&tmp, &uninstaller).map_err(|e| format!("{}: {e}", tmp.display()))?;
        let r = s.sign(&tmp, &format!("{} Uninstaller", app.name)).and_then(|_| std::fs::read(&tmp).map_err(|e| e.to_string()));
        let _ = std::fs::remove_file(&tmp);
        uninstaller = r?;
        check_payload(&uninstaller, "uninstall.exe")?;
    }
    let uninstaller_len = uninstaller.len() as u64;
    files.push((UNINSTALLER_NAME.to_string(), uninstaller));

    // 2. Archive.
    let (archive_bytes, entries) = archive::pack(&files, o.level).map_err(|e| e.to_string())?;
    runtime.unpacked_size = entries.iter().map(|e| e.size).sum();
    runtime.file_count = entries.len() as u64;

    // 3. Setup.
    let file_name = setup_file_name(&app);
    let mut setup = patched_stub(&stub, &tmp, icons.as_deref(), version_info(&app, format!("{} Setup", app.name), &file_name))?;
    payload::append(&mut setup, &runtime.to_json(), &archive_bytes, test_flag).map_err(|e| e.to_string())?;
    let out = o.out_dir.join(&file_name);
    std::fs::write(&out, &setup).map_err(|e| format!("{}: {e}", out.display()))?;
    if let Some(s) = &o.sign {
        let signed = s.sign(&out, &format!("{} Setup", app.name)).and_then(|_| std::fs::read(&out).map_err(|e| e.to_string()));
        if let Err(e) = signed.and_then(|b| check_payload(&b, &file_name)) {
            let _ = std::fs::remove_file(&out);
            return Err(e);
        }
    }

    Ok(Packed { setup: out, manifest: runtime, files: entries, uninstaller_len })
}

/// After signing: the file must be signed and its payload must still be found and match its hash.
fn check_payload(bytes: &[u8], what: &str) -> Result<(), String> {
    let info = payload::pe_info(bytes).map_err(|e| format!("{what}: {e}"))?;
    if info.security.is_none() {
        return Err(format!("{what}: signtool reported success but the file has no signature"));
    }
    let mut c = std::io::Cursor::new(bytes);
    let p = payload::read(&mut c).map_err(|e| format!("{what} after signing: {e}"))?.ok_or_else(|| format!("{what}: payload lost by signing"))?;
    payload::verify(&mut c, &p).map_err(|e| format!("{what} after signing: {e}"))
}

/// What a built exe carries, for tests and `gloria-pack --inspect`.
#[derive(Debug)]
pub struct Inspect {
    pub icons: Vec<IconImage>,
    pub icon_group_count: usize,
    pub icon_count: usize,
    pub version: Option<([u16; 4], Vec<(String, String)>)>,
    pub payload: Option<payload::Payload>,
}

impl Inspect {
    pub fn string(&self, key: &str) -> Option<&str> {
        self.version.as_ref()?.1.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

pub fn inspect(path: &Path) -> Result<Inspect, String> {
    let all = resources::read_resources(path, &[res::RT_GROUP_ICON, res::RT_ICON, RT_VERSION]).map_err(|e| e.to_string())?;
    let group = resources::icon_group(path).map_err(|e| e.to_string())?;
    let icons = group
        .into_iter()
        .map(|(_, data)| {
            let (w, h) = icon_dims(&data);
            IconImage { width: w, height: h, bit_count: 32, data }
        })
        .collect();
    let version = all.iter().find(|e| e.0 == RT_VERSION).and_then(|e| res::parse_version_info(&e.3));
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let payload = payload::read(&mut f).map_err(|e| e.to_string())?;
    Ok(Inspect {
        icons,
        icon_group_count: all.iter().filter(|e| e.0 == res::RT_GROUP_ICON).count(),
        icon_count: all.iter().filter(|e| e.0 == res::RT_ICON).count(),
        version,
        payload,
    })
}

fn icon_dims(d: &[u8]) -> (u32, u32) {
    if d.starts_with(b"\x89PNG") && d.len() >= 24 {
        let w = u32::from_be_bytes(d[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(d[20..24].try_into().unwrap());
        (w, h)
    } else if d.len() >= 12 {
        let w = i32::from_le_bytes(d[4..8].try_into().unwrap()).unsigned_abs();
        let h = i32::from_le_bytes(d[8..12].try_into().unwrap()).unsigned_abs() / 2;
        (w, h)
    } else {
        (0, 0)
    }
}
