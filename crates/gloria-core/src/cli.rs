//! NSIS-style command line parsing over the raw `GetCommandLineW()` string. See docs/design.md §11.
//!
//! `/D=` must be the last argument and takes the rest of the line verbatim, so `/D=C:\My Apps\Foo` works without
//! quotes. Slash flags are case-insensitive. Backslashes are never escapes (they are path separators here).

use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    pub silent: bool,
    pub install_dir: Option<PathBuf>,
    pub purge: bool,
    pub allow_downgrade: bool,
    pub relaunch: bool,
    pub dry_run: bool,

    pub preview: bool,
    pub page: Option<String>,
    pub lang: Option<String>,
    pub manifest: Option<PathBuf>,
    pub icon: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
    /// Force Direct2D software rendering (to check the WARP-like path outside Sandbox).
    pub software: bool,

    /// Internal: uninstall trampoline.
    pub cleanup: Option<PathBuf>,
    pub wait_pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(pub String);

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Skips the program name (quoted or not) the way the CRT does.
fn skip_program(line: &str) -> &str {
    let s = line.trim_start();
    if let Some(rest) = s.strip_prefix('"') {
        match rest.find('"') {
            Some(i) => &rest[i + 1..],
            None => "",
        }
    } else {
        match s.find(char::is_whitespace) {
            Some(i) => &s[i..],
            None => "",
        }
    }
}

/// Splits on whitespace; double quotes group and are removed. Returns (token, byte offset where it started).
fn tokens(s: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut start = None;
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
            start.get_or_insert(i);
        } else if c.is_whitespace() && !quoted {
            if let Some(st) = start.take() {
                out.push((std::mem::take(&mut cur), st));
            }
        } else {
            start.get_or_insert(i);
            cur.push(c);
        }
    }
    if let Some(st) = start {
        out.push((cur, st));
    }
    out
}

/// Parses a full command line, including the program name.
pub fn parse_command_line(line: &str) -> Result<Args, CliError> {
    parse_args_str(skip_program(line))
}

/// Parses the argument part only (no program name).
pub fn parse_args_str(rest: &str) -> Result<Args, CliError> {
    // /D= takes everything after it, so cut it off first. NSIS requires it last; we find the first token that
    // starts with /D= (case-insensitive) at a token boundary.
    let mut head = rest;
    let mut install_dir = None;
    for (tok, off) in tokens(rest) {
        if tok.len() >= 3 && tok[..3].eq_ignore_ascii_case("/D=") {
            let raw = rest[off..].trim();
            let raw = &raw[raw.find('=').map(|i| i + 1).unwrap_or(raw.len())..];
            let raw = raw.trim().trim_matches('"').trim();
            if raw.is_empty() {
                return Err(CliError("/D= needs a folder".into()));
            }
            install_dir = Some(PathBuf::from(raw));
            head = &rest[..off];
            break;
        }
    }

    let mut a = Args { install_dir, ..Args::default() };
    let toks = tokens(head);
    let mut it = toks.into_iter().map(|(t, _)| t);
    while let Some(t) = it.next() {
        let lower = t.to_ascii_lowercase();
        let mut value = |name: &str| -> Result<String, CliError> {
            it.next().filter(|v| !v.is_empty()).ok_or_else(|| CliError(format!("{name} needs a value")))
        };
        match lower.as_str() {
            "/s" => a.silent = true,
            "/purge" => a.purge = true,
            "/allowdowngrade" => a.allow_downgrade = true,
            "/relaunch" => a.relaunch = true,
            "--dry-run" => a.dry_run = true,
            "--preview" => a.preview = true,
            "--software" => a.software = true,
            "--page" => a.page = Some(value("--page")?),
            "--lang" => a.lang = Some(value("--lang")?.to_ascii_lowercase()),
            "--manifest" => a.manifest = Some(value("--manifest")?.into()),
            "--icon" => a.icon = Some(value("--icon")?.into()),
            "--screenshot" => a.screenshot = Some(value("--screenshot")?.into()),
            "--cleanup" => a.cleanup = Some(value("--cleanup")?.into()),
            "--wait-pid" => {
                let v = value("--wait-pid")?;
                a.wait_pid = Some(v.parse().map_err(|_| CliError(format!("bad pid {v:?}")))?);
            }
            // Windows passes this when a user picks "Run as administrator"; harmless.
            "/ncrc" => {}
            _ => return Err(CliError(format!("unknown argument {t:?}"))),
        }
    }
    if a.screenshot.is_some() {
        a.preview = true;
    }
    if (a.page.is_some() || a.icon.is_some()) && !a.preview {
        return Err(CliError("--page and --icon only work with --preview".into()));
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Args {
        parse_command_line(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn nsis_flags_case_insensitive() {
        let a = p(r#""C:\Users\you\Downloads\Foo_0.2.0_x64-setup.exe" /S /purge /AllowDowngrade /RELAUNCH"#);
        assert!(a.silent && a.purge && a.allow_downgrade && a.relaunch);
        assert_eq!(a.install_dir, None);
    }

    #[test]
    fn unquoted_d_with_spaces_takes_rest_of_line() {
        let a = p(r"setup.exe /S /D=C:\Path With Spaces\My App");
        assert!(a.silent);
        assert_eq!(a.install_dir, Some(PathBuf::from(r"C:\Path With Spaces\My App")));
    }

    #[test]
    fn quoted_d_and_trailing_space() {
        let a = p(r#"setup.exe /D="C:\Apps\Foo"  "#);
        assert_eq!(a.install_dir, Some(PathBuf::from(r"C:\Apps\Foo")));
        let a = p(r"setup.exe /d=C:\x /S");
        // /D= is last by contract: everything after it belongs to the path.
        assert_eq!(a.install_dir, Some(PathBuf::from(r"C:\x /S")));
        assert!(!a.silent);
    }

    #[test]
    fn program_name_with_spaces() {
        let a = p(r#""C:\Program Files Test\setup.exe" /S"#);
        assert!(a.silent);
        let a = p("setup.exe");
        assert_eq!(a, Args::default());
    }

    #[test]
    fn preview_options() {
        let a = p(r#"stub.exe --preview --page done --lang TR --manifest "C:\x y\gloria.toml" --icon a.ico"#);
        assert!(a.preview);
        assert_eq!(a.page.as_deref(), Some("done"));
        assert_eq!(a.lang.as_deref(), Some("tr"));
        assert_eq!(a.manifest, Some(PathBuf::from(r"C:\x y\gloria.toml")));
        let a = p("stub.exe --screenshot out");
        assert!(a.preview);
    }

    #[test]
    fn trampoline_args() {
        let a = p(r#"t.exe --cleanup "C:\Users\you\AppData\Local\Programs\Foo" --wait-pid 1234 /S /PURGE"#);
        assert_eq!(a.cleanup, Some(PathBuf::from(r"C:\Users\you\AppData\Local\Programs\Foo")));
        assert_eq!(a.wait_pid, Some(1234));
        assert!(a.silent && a.purge);
    }

    #[test]
    fn errors() {
        assert!(parse_command_line("x.exe /bogus").is_err());
        assert!(parse_command_line("x.exe --page").is_err());
        assert!(parse_command_line("x.exe /D=").is_err());
        assert!(parse_command_line("x.exe --wait-pid abc").is_err());
        assert!(parse_command_line("x.exe --page done").is_err());
    }
}
