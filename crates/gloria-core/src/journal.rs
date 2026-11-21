//! The commit journal: makes the file swap of an install undoable even if the process dies halfway (crash, power
//! loss, Task Manager).
//!
//! Before the first rename, `<install>\.gloria\journal` lists every file that will be placed and every obsolete file
//! that will be moved away, and is flushed to disk. Each step is a rename between three places, so the state of each
//! file can be read back from the disk alone:
//!
//! - `staging\<rel>` exists: the new file has not been placed yet.
//! - `backup\<rel>` exists: the old file was moved away and can be put back.
//!
//! Deleting the journal is the commit point. On the next run, [`recover`] finds a journal and undoes every step in
//! reverse; without one, a leftover `.gloria` folder holds nothing worth keeping and is removed.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::archive::safe_rel_path;
use crate::install_manifest::remove_tree_no_follow;
use crate::manifest::WORK_DIR;

const MAGIC: &str = "GLORIA-JOURNAL 1";
pub const JOURNAL: &str = "journal";
pub const STAGING: &str = "staging";
pub const BACKUP: &str = "backup";

/// A commit in progress. Create it with [`Commit::begin`] once staging is complete.
pub struct Commit {
    dir: PathBuf,
    work: PathBuf,
    places: Vec<String>,
    obsolete: Vec<String>,
}

/// What [`recover`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Recovered {
    /// No `.gloria` folder.
    Nothing,
    /// A `.gloria` folder without a journal (staging never committed, or cleanup after a commit was cut short).
    Cleaned,
    /// An unfinished commit was undone; the previous version is back.
    RolledBack,
}

fn work_dir(dir: &Path) -> PathBuf {
    dir.join(WORK_DIR)
}

fn rel(p: &str) -> io::Result<PathBuf> {
    safe_rel_path(p).ok_or_else(|| io::Error::other(format!("unsafe path in journal: {p:?}")))
}

fn exists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

/// An I/O error with the path it happened on. Kept as the inner error so [`os_code`] still finds the OS error code
/// (a sharing violation must still read as "file locked").
#[derive(Debug)]
pub struct PathError {
    pub path: PathBuf,
    pub err: io::Error,
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.err)
    }
}

impl std::error::Error for PathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.err)
    }
}

fn with_path(e: io::Error, p: &Path) -> io::Error {
    io::Error::new(e.kind(), PathError { path: p.to_path_buf(), err: e })
}

/// The OS error code of `e`, also when it was wrapped with its path.
pub fn os_code(e: &io::Error) -> Option<i32> {
    e.raw_os_error().or_else(|| e.get_ref()?.downcast_ref::<PathError>()?.err.raw_os_error())
}

/// The app id recorded in a pending journal in `dir`, if there is one.
pub fn owner(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(work_dir(dir).join(JOURNAL)).ok()?;
    let mut lines = text.lines();
    (lines.next()? == MAGIC).then_some(())?;
    lines.find_map(|l| l.strip_prefix("A\t")).map(str::to_string)
}

impl Commit {
    /// Writes and flushes the journal. `places` are files in `staging` (relative, `\`-separated) that will replace or
    /// join the install folder; `obsolete` are files of the previous install to move away. `created_dir` records that
    /// the install folder itself is new, so a rollback can remove it.
    pub fn begin(dir: &Path, app_id: &str, places: Vec<String>, obsolete: Vec<String>, created_dir: bool) -> io::Result<Commit> {
        let work = work_dir(dir);
        let mut text = format!("{MAGIC}\nA\t{}\n", app_id.replace(['\t', '\n', '\r'], " "));
        if created_dir {
            text.push_str("N\n");
        }
        // Folders that don't exist yet: a rollback removes them again (when empty).
        let mut new_dirs: Vec<String> = Vec::new();
        for p in &places {
            let parts: Vec<&str> = p.split('\\').collect();
            for i in 1..parts.len() {
                let d = parts[..i].join("\\");
                if !new_dirs.iter().any(|x| x.eq_ignore_ascii_case(&d)) && !dir.join(rel(&d)?).exists() {
                    new_dirs.push(d);
                }
            }
        }
        for d in &new_dirs {
            text.push_str(&format!("D\t{d}\n"));
        }
        for p in &places {
            rel(p)?;
            text.push_str(&format!("P\t{p}\n"));
        }
        for p in &obsolete {
            rel(p)?;
            text.push_str(&format!("O\t{p}\n"));
        }
        let mut f = File::create(work.join(JOURNAL))?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        Ok(Commit { dir: dir.to_path_buf(), work, places, obsolete })
    }

    /// Number of renames [`Commit::apply`] performs at most (for progress).
    pub fn len(&self) -> usize {
        self.places.len() + self.obsolete.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Does the renames: each placed file's old version goes to `backup` first, then obsolete files go to `backup`.
    /// `step(i)` runs before step `i`; returning `false` stops there with an `Interrupted` error (used by tests to
    /// simulate a crash). On any error the journal stays, and [`recover`] undoes what was done.
    pub fn apply(&self, mut step: impl FnMut(usize) -> bool) -> io::Result<()> {
        let staging = self.work.join(STAGING);
        let backup = self.work.join(BACKUP);
        let mut i = 0;
        for p in &self.places {
            if !step(i) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
            }
            i += 1;
            let r = rel(p)?;
            let dest = self.dir.join(&r);
            if exists(&dest) {
                let bak = backup.join(&r);
                fs::create_dir_all(bak.parent().unwrap())?;
                fs::rename(&dest, &bak).map_err(|e| with_path(e, &dest))?;
            }
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(staging.join(&r), &dest).map_err(|e| with_path(e, &dest))?;
        }
        for p in &self.obsolete {
            if !step(i) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
            }
            i += 1;
            let r = rel(p)?;
            let orig = self.dir.join(&r);
            if exists(&orig) {
                let bak = backup.join(&r);
                fs::create_dir_all(bak.parent().unwrap())?;
                fs::rename(&orig, &bak).map_err(|e| with_path(e, &orig))?;
            }
        }
        Ok(())
    }

    /// The commit point: removes the journal (so the new version stays), then the rest of `.gloria`.
    pub fn finish(self) -> io::Result<()> {
        fs::remove_file(self.work.join(JOURNAL))?;
        let _ = remove_tree_no_follow(&self.work, false, &mut |_| {});
        Ok(())
    }
}

/// Undoes an unfinished commit in `dir` (or clears a leftover `.gloria`). Safe to call on any folder, any number of
/// times: every step checks the disk first.
pub fn recover(dir: &Path) -> io::Result<Recovered> {
    let work = work_dir(dir);
    if !exists(&work) {
        return Ok(Recovered::Nothing);
    }
    let text = match fs::read_to_string(work.join(JOURNAL)) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            remove_tree_no_follow(&work, false, &mut |_| {})?;
            return Ok(Recovered::Cleaned);
        }
        Err(e) => return Err(e),
    };
    let mut lines = text.lines();
    if lines.next() != Some(MAGIC) {
        // A journal we can't read is also one that never got its first rename: `begin` flushes before returning.
        // Still, don't delete backups we don't understand.
        return Err(io::Error::other(format!("{} is not a GloriaSetup journal", work.join(JOURNAL).display())));
    }
    let staging = work.join(STAGING);
    let backup = work.join(BACKUP);
    let (mut created_dir, mut dirs, mut places, mut obsolete) = (false, Vec::new(), Vec::new(), Vec::new());
    for l in lines {
        match l.split_once('\t') {
            Some(("D", p)) => dirs.push(rel(p)?),
            Some(("P", p)) => places.push(rel(p)?),
            Some(("O", p)) => obsolete.push(rel(p)?),
            Some(("A", _)) => {}
            None if l == "N" => created_dir = true,
            _ if l.is_empty() => {}
            _ => return Err(io::Error::other(format!("bad journal line {l:?}"))),
        }
    }

    // Reverse order: obsolete files first, then placed files from last to first.
    for r in obsolete.iter().rev() {
        let (orig, bak) = (dir.join(r), backup.join(r));
        if exists(&bak) && !exists(&orig) {
            fs::rename(&bak, &orig).map_err(|e| with_path(e, &orig))?;
        }
    }
    for r in places.iter().rev() {
        let (dest, stg, bak) = (dir.join(r), staging.join(r), backup.join(r));
        if !exists(&stg) && exists(&dest) {
            // The new file was placed: take it out again (back to staging, which is deleted below).
            fs::create_dir_all(stg.parent().unwrap())?;
            fs::rename(&dest, &stg).map_err(|e| with_path(e, &dest))?;
        }
        if exists(&bak) && !exists(&dest) {
            fs::rename(&bak, &dest).map_err(|e| with_path(e, &dest))?;
        }
    }
    for d in dirs.iter().rev() {
        let _ = fs::remove_dir(dir.join(d));
    }
    // Everything is back; the journal goes last, so a second crash during recovery just recovers again.
    remove_tree_no_follow(&staging, false, &mut |_| {})?;
    fs::remove_file(work.join(JOURNAL))?;
    remove_tree_no_follow(&work, false, &mut |_| {})?;
    if created_dir {
        let _ = fs::remove_dir(dir);
    }
    Ok(Recovered::RolledBack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gloria-journal-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(base: &Path, rel: &str, data: &str) {
        let p = base.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    }

    /// Every file and folder under `dir` with its contents (folders as `<dir>`), `\`-separated.
    fn snapshot(dir: &Path) -> BTreeMap<String, String> {
        fn walk(base: &Path, p: &Path, out: &mut BTreeMap<String, String>) {
            for e in fs::read_dir(p).unwrap().flatten() {
                let path = e.path();
                let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('/', "\\");
                if e.file_type().unwrap().is_dir() {
                    out.insert(rel, "<dir>".into());
                    walk(base, &path, out);
                } else {
                    out.insert(rel, fs::read_to_string(&path).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        if dir.exists() {
            walk(dir, dir, &mut out);
        }
        out
    }

    /// Old install: app.exe, lib\a.dll, old-notes.txt, plus a user file. New: app.exe, lib\a.dll, lib\b.dll,
    /// res\new\x.dat. old-notes.txt is obsolete.
    fn old_install(dir: &Path) {
        put(dir, "app.exe", "old app");
        put(dir, "lib\\a.dll", "old a");
        put(dir, "old-notes.txt", "old notes");
        put(dir, "mine.txt", "user file");
    }

    fn stage(dir: &Path) -> Vec<String> {
        let staging = dir.join(WORK_DIR).join(STAGING);
        let files = [("app.exe", "new app"), ("lib\\a.dll", "new a"), ("lib\\b.dll", "new b"), ("res\\new\\x.dat", "new x")];
        for (p, d) in files {
            put(&staging, p, d);
        }
        files.iter().map(|(p, _)| p.to_string()).collect()
    }

    #[test]
    fn crash_at_every_step_rolls_back_exactly() {
        let total = 5; // 4 places + 1 obsolete
        for stop in 0..=total {
            let dir = tmp(&format!("step{stop}"));
            old_install(&dir);
            let before = snapshot(&dir);
            let places = stage(&dir);
            let c = Commit::begin(&dir, "App", places, vec!["old-notes.txt".into()], false).unwrap();
            assert_eq!(c.len(), total);
            assert_eq!(owner(&dir).as_deref(), Some("App"));
            let r = c.apply(|i| i < stop);
            assert_eq!(r.is_err(), stop < total, "stop {stop}");
            // `stop == total` finished every rename but never reached `finish`: still rolled back.
            assert_eq!(recover(&dir).unwrap(), Recovered::RolledBack, "stop {stop}");
            assert_eq!(snapshot(&dir), before, "stop {stop}");
            // A second recovery is a no-op.
            assert_eq!(recover(&dir).unwrap(), Recovered::Nothing);
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn crash_during_recovery_recovers_again() {
        let dir = tmp("double");
        old_install(&dir);
        let before = snapshot(&dir);
        let c = Commit::begin(&dir, "App", stage(&dir), vec!["old-notes.txt".into()], false).unwrap();
        c.apply(|_| true).unwrap();
        // Simulate a recovery cut short after its first step: put one obsolete file back by hand.
        let work = dir.join(WORK_DIR);
        fs::rename(work.join(BACKUP).join("old-notes.txt"), dir.join("old-notes.txt")).unwrap();
        assert_eq!(recover(&dir).unwrap(), Recovered::RolledBack);
        assert_eq!(snapshot(&dir), before);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finished_commit_keeps_the_new_version() {
        let dir = tmp("finish");
        old_install(&dir);
        let c = Commit::begin(&dir, "App", stage(&dir), vec!["old-notes.txt".into()], false).unwrap();
        c.apply(|_| true).unwrap();
        c.finish().unwrap();
        assert_eq!(owner(&dir), None);
        let s = snapshot(&dir);
        assert_eq!(s.get("app.exe").map(String::as_str), Some("new app"));
        assert_eq!(s.get("lib\\b.dll").map(String::as_str), Some("new b"));
        assert_eq!(s.get("mine.txt").map(String::as_str), Some("user file"));
        assert!(!s.contains_key("old-notes.txt"));
        assert!(!s.keys().any(|k| k.starts_with(WORK_DIR)));
        assert_eq!(recover(&dir).unwrap(), Recovered::Nothing);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn leftover_work_without_journal_is_cleared_and_never_restored() {
        let dir = tmp("nojournal");
        put(&dir, "app.exe", "new app");
        // A commit whose journal was deleted (committed) but whose cleanup was cut short.
        put(&dir.join(WORK_DIR).join(BACKUP), "app.exe", "old app");
        put(&dir.join(WORK_DIR).join(STAGING), "half.dll", "x");
        assert_eq!(recover(&dir).unwrap(), Recovered::Cleaned);
        let s = snapshot(&dir);
        assert_eq!(s.get("app.exe").map(String::as_str), Some("new app"));
        assert_eq!(s.len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fresh_install_rollback_removes_the_new_folder() {
        let parent = tmp("fresh");
        let dir = parent.join("App");
        fs::create_dir_all(dir.join(WORK_DIR)).unwrap();
        let c = Commit::begin(&dir, "App", stage(&dir), vec![], true).unwrap();
        let _ = c.apply(|i| i < 3);
        assert_eq!(recover(&dir).unwrap(), Recovered::RolledBack);
        assert!(!dir.exists(), "{:?}", snapshot(&dir));
        fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn unreadable_journal_keeps_backups() {
        let dir = tmp("garbage");
        put(&dir.join(WORK_DIR), JOURNAL, "not a journal");
        put(&dir.join(WORK_DIR).join(BACKUP), "app.exe", "old app");
        assert!(recover(&dir).is_err());
        assert!(dir.join(WORK_DIR).join(BACKUP).join("app.exe").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_unsafe_paths() {
        let dir = tmp("unsafe");
        fs::create_dir_all(dir.join(WORK_DIR)).unwrap();
        assert!(Commit::begin(&dir, "App", vec!["..\\evil.dll".into()], vec![], false).is_err());
        assert!(Commit::begin(&dir, "App", vec![], vec!["C:\\Windows\\x".into()], false).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn os_code_survives_the_path_wrapper() {
        let e = with_path(io::Error::from_raw_os_error(32), Path::new(r"C:\x\a.dll"));
        assert_eq!(os_code(&e), Some(32));
        assert!(e.to_string().contains("a.dll"), "{e}");
        assert_eq!(os_code(&io::Error::other("plain")), None);
    }
}
