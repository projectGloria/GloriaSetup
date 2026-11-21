//! gloria-pack CLI.

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
gloria-pack: build a GloriaSetup installer

USAGE:
  gloria-pack --app <folder> [--manifest <gloria.toml>] [--stub <gloria-stub.exe>]
              [--version <x.y.z>] [--icon <file.ico|file.png>] [--allow-default-icon]
              [--out <folder>] [--level <1-22>] [--webview2-bootstrapper <MicrosoftEdgeWebview2Setup.exe>]
              [--sign-pfx <cert.pfx> [--timestamp-url <url>] [--signtool <signtool.exe>]]
  gloria-pack --inspect <exe>

  --app        Folder with the app's files (main_exe must be inside).
  --manifest   gloria.toml (default: <app>/../gloria.toml, then ./gloria.toml).
  --stub       The GloriaSetup stub (default: gloria-stub.exe next to gloria-pack.exe).
  --version    Overrides `version` in gloria.toml (a leading v is dropped).
  --icon       Overrides `icon` in gloria.toml.
  --allow-default-icon
               With no icon, keep the GloriaSetup icon and mark the build TEST BUILD.
  --out        Output folder (default: dist).
  --webview2-bootstrapper
               Bundle Microsoft's WebView2 Evergreen bootstrapper (needs_webview2 apps). Setup runs it only
               when the runtime is missing; without it such a setup exits 5 on a PC with no WebView2.
  --sign-pfx   Authenticode-sign uninstall.exe and the setup with this PFX (password in GLORIA_SIGN_PASSWORD).
               Signing is the last step for each file; the signed setup is checked to still find its payload.
  --timestamp-url
               RFC 3161 timestamp server (e.g. http://timestamp.digicert.com). Recommended for releases.
  --signtool   signtool.exe to use (default: PATH, then the newest Windows SDK).
  --inspect    Print the icon sizes, version info and payload of a built exe.
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gloria-pack: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut app = None;
    let mut manifest = None;
    let mut stub = None;
    let mut version = None;
    let mut icon = None;
    let mut allow_default_icon = false;
    let mut out = PathBuf::from("dist");
    let mut level = 19;
    let mut inspect = None;
    let mut webview2: Option<PathBuf> = None;
    let mut sign_pfx: Option<PathBuf> = None;
    let mut timestamp_url = None;
    let mut signtool: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--app" => app = Some(PathBuf::from(val()?)),
            "--manifest" => manifest = Some(PathBuf::from(val()?)),
            "--stub" => stub = Some(PathBuf::from(val()?)),
            "--version" => version = Some(val()?),
            "--icon" => icon = Some(PathBuf::from(val()?)),
            "--allow-default-icon" => allow_default_icon = true,
            "--out" => out = PathBuf::from(val()?),
            "--level" => level = val()?.parse().map_err(|_| "--level must be a number".to_string())?,
            "--webview2-bootstrapper" => webview2 = Some(PathBuf::from(val()?)),
            "--sign-pfx" => sign_pfx = Some(PathBuf::from(val()?)),
            "--timestamp-url" => timestamp_url = Some(val()?),
            "--signtool" => signtool = Some(PathBuf::from(val()?)),
            "--inspect" => inspect = Some(PathBuf::from(val()?)),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            _ => return Err(format!("unknown argument {a:?}\n\n{USAGE}")),
        }
    }

    if let Some(p) = inspect {
        return print_inspect(&p);
    }

    let app = app.ok_or_else(|| format!("--app is required\n\n{USAGE}"))?;
    let manifest = match manifest {
        Some(m) => m,
        None => {
            let beside = app.parent().map(|p| p.join("gloria.toml"));
            match beside.filter(|p| p.is_file()) {
                Some(p) => p,
                None => PathBuf::from("gloria.toml"),
            }
        }
    };
    let stub = match stub {
        Some(s) => s,
        None => std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name("gloria-stub.exe"),
    };
    if sign_pfx.is_none() && (timestamp_url.is_some() || signtool.is_some()) {
        return Err("--timestamp-url and --signtool need --sign-pfx".into());
    }
    let sign = match sign_pfx {
        None => None,
        Some(pfx) => {
            if !pfx.is_file() {
                return Err(format!("--sign-pfx {}: no such file", pfx.display()));
            }
            let signtool = match signtool {
                Some(s) => s,
                None => gloria_pack::sign::find_signtool().ok_or("signtool.exe not found; install the Windows SDK or pass --signtool")?,
            };
            let password = std::env::var(gloria_pack::sign::PASSWORD_ENV).ok().filter(|p| !p.is_empty());
            Some(gloria_pack::sign::Sign { signtool, pfx, password, timestamp_url })
        }
    };
    let signed = sign.is_some();
    let packed = gloria_pack::pack(&gloria_pack::Options {
        stub,
        app_dir: app,
        manifest,
        version,
        icon,
        allow_default_icon,
        out_dir: out,
        level,
        webview2_bootstrapper: webview2,
        sign,
    })?;
    let size = std::fs::metadata(&packed.setup).map(|m| m.len()).unwrap_or(0);
    println!(
        "{} ({} files, {:.1} MB unpacked, setup {:.1} MB){}",
        packed.setup.display(),
        packed.manifest.file_count,
        packed.manifest.unpacked_size as f64 / 1e6,
        size as f64 / 1e6,
        if packed.manifest.test_build { " [TEST BUILD: GloriaSetup icon kept]" } else { "" }
    );
    println!("{}", if signed { "signed (uninstall.exe and setup)" } else { "not signed" });
    Ok(())
}

fn print_inspect(p: &std::path::Path) -> Result<(), String> {
    let i = gloria_pack::inspect(p)?;
    println!("{}", p.display());
    println!("  icon groups: {}, icon images: {}", i.icon_group_count, i.icon_count);
    let sizes: Vec<String> = i.icons.iter().map(|im| format!("{}x{}{}", im.width, im.height, if im.is_png() { " png" } else { "" })).collect();
    println!("  group 1: {}", sizes.join(", "));
    if let Some((v, strings)) = &i.version {
        println!("  file version: {}.{}.{}.{}", v[0], v[1], v[2], v[3]);
        for (k, v) in strings {
            println!("  {k}: {v}");
        }
    }
    match &i.payload {
        None => println!("  payload: none (bare stub)"),
        Some(pl) => {
            println!(
                "  payload: flags {:#x}{}{}, manifest {} bytes, archive {} bytes",
                pl.flags,
                if pl.is_uninstaller() { " uninstaller" } else { "" },
                if pl.is_test_build() { " test-build" } else { "" },
                pl.manifest_json.len(),
                pl.archive_len
            );
        }
    }
    Ok(())
}
