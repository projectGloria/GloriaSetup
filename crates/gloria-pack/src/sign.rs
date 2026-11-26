//! Optional Authenticode signing with `signtool`. It is always the last change to a file: the uninstaller is signed
//! after its payload is appended and before it goes into the archive, the setup after its payload is appended. The
//! payload reader stops at the security directory, so a signed file still finds its payload; `pack` checks that.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The environment variable that holds the PFX password. It isn't a CLI argument so it stays out of shell history
/// and CI logs.
pub const PASSWORD_ENV: &str = "GLORIA_SIGN_PASSWORD";

#[derive(Debug, Clone)]
pub struct Sign {
    pub signtool: PathBuf,
    pub pfx: PathBuf,
    pub password: Option<String>,
    /// RFC 3161 timestamp server. Without one the signature stops being valid when the certificate expires.
    pub timestamp_url: Option<String>,
}

impl Sign {
    /// Signs `file` in place. `description` is what the UAC and SmartScreen prompts show as the program name.
    pub fn sign(&self, file: &Path, description: &str) -> Result<(), String> {
        let mut c = Command::new(&self.signtool);
        c.args(["sign", "/fd", "SHA256", "/d", description, "/f"]).arg(&self.pfx);
        if let Some(p) = &self.password {
            // signtool takes a PFX password only as `/p`. It is on signtool's command line (readable by this user's
            // own processes) for the seconds it runs, never in our arguments, logs or errors. Importing the key into
            // a certificate store instead would leave it on the machine, which is worse.
            c.args(["/p", p]);
        }
        if let Some(t) = &self.timestamp_url {
            c.args(["/tr", t, "/td", "SHA256"]);
        }
        c.arg(file);
        let o = c.output().map_err(|e| format!("signtool {}: {e}", self.signtool.display()))?;
        if !o.status.success() {
            // signtool prints the reason on stdout; the password is never part of its output.
            let out = String::from_utf8_lossy(&o.stdout);
            let err = String::from_utf8_lossy(&o.stderr);
            return Err(format!("signing {} failed ({}): {}{}", file.display(), o.status, out.trim(), err.trim()));
        }
        Ok(())
    }
}

/// `signtool.exe` from PATH, else the newest x64 one in the Windows 10/11 SDK.
pub fn find_signtool() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join("signtool.exe"))
        .find(|p| p.is_file())
    {
        return Some(p);
    }
    let kits = PathBuf::from(std::env::var_os("ProgramFiles(x86)")?).join(r"Windows Kits\10\bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&kits).ok()?.flatten().map(|e| e.path()).filter(|p| p.join(r"x64\signtool.exe").is_file()).collect();
    versions.sort_by_key(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.split('.').map(|x| x.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>())
            .unwrap_or_default()
    });
    versions.pop().map(|v| v.join(r"x64\signtool.exe"))
}
