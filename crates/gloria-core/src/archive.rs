//! The app archive: a tar stream compressed with zstd. Entries are regular files with relative paths only.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative path with `\` separators, e.g. `resources\icon.png`.
    pub path: String,
    pub size: u64,
    /// Lowercase hex SHA-256.
    pub sha256: String,
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

/// Validates an archive path: relative, no `.`/`..`, no drive or root, no reserved characters.
pub fn safe_rel_path(p: &str) -> Option<PathBuf> {
    let p = p.replace('/', "\\");
    if p.is_empty() || p.starts_with('\\') || p.contains(':') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in p.split('\\') {
        if c.is_empty() || c == "." || c == ".." || c.contains(['*', '?', '<', '>', '|', '"']) || c.ends_with(['.', ' '])
        {
            return None;
        }
        out.push(c);
    }
    match Path::new(&out).components().next() {
        Some(Component::Normal(_)) => Some(out),
        _ => None,
    }
}

/// Collects every regular file under `dir` (sorted, relative, `\` separators). Symlinks and junctions are refused.
pub fn list_dir(dir: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    fn walk(base: &Path, cur: &Path, out: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
        let mut entries: Vec<_> = fs::read_dir(cur)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let ft = e.file_type()?;
            let p = e.path();
            if ft.is_symlink() {
                return Err(io::Error::other(format!("{} is a link; the app folder may only hold real files", p.display())));
            }
            if ft.is_dir() {
                walk(base, &p, out)?;
            } else if ft.is_file() {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('/', "\\");
                out.push((rel, p));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

/// Builds the compressed archive from (relative path, bytes) pairs. Returns the archive and its file list.
pub fn pack(files: &[(String, Vec<u8>)], level: i32) -> io::Result<(Vec<u8>, Vec<FileEntry>)> {
    let mut entries = Vec::new();
    let enc = zstd::stream::Encoder::new(Vec::new(), level)?;
    let mut tar = tar::Builder::new(enc);
    tar.mode(tar::HeaderMode::Deterministic);
    let mut sorted: Vec<&(String, Vec<u8>)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    for w in sorted.windows(2) {
        if w[0].0.eq_ignore_ascii_case(&w[1].0) {
            return Err(io::Error::other(format!("duplicate file {:?}", w[1].0)));
        }
    }
    for (rel, data) in sorted {
        if safe_rel_path(rel).is_none() {
            return Err(io::Error::other(format!("unsafe archive path {rel:?}")));
        }
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_entry_type(tar::EntryType::Regular);
        h.set_mtime(0);
        tar.append_data(&mut h, rel.replace('\\', "/"), data.as_slice())?;
        entries.push(FileEntry { path: rel.clone(), size: data.len() as u64, sha256: sha256_hex(data) });
    }
    let enc = tar.into_inner()?;
    Ok((enc.finish()?, entries))
}

/// Streams the archive into `dest`, calling `progress(bytes_written_so_far)` as it goes.
/// Every path is validated; anything that is not a regular file is skipped.
pub fn unpack<R: Read>(r: R, dest: &Path, mut progress: impl FnMut(&str, u64)) -> io::Result<Vec<FileEntry>> {
    let dec = zstd::stream::Decoder::new(r)?;
    let mut tar = tar::Archive::new(dec);
    let mut out = Vec::new();
    let mut total = 0u64;
    let mut buf = vec![0u8; 1 << 16];
    for e in tar.entries()? {
        let mut e = e?;
        if e.header().entry_type() != tar::EntryType::Regular {
            continue;
        }
        let rel = String::from_utf8_lossy(&e.path_bytes()).replace('/', "\\");
        let Some(relp) = safe_rel_path(&rel) else {
            return Err(io::Error::other(format!("unsafe archive path {rel:?}")));
        };
        let target = dest.join(&relp);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::File::create(&target)?;
        let mut h = Sha256::new();
        let mut size = 0u64;
        loop {
            let n = e.read(&mut buf)?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n])?;
            h.update(&buf[..n]);
            size += n as u64;
            total += n as u64;
            progress(&rel, total);
        }
        f.sync_all()?;
        out.push(FileEntry { path: rel, size, sha256: hex(&h.finalize()) });
    }
    Ok(out)
}

/// Reads the file list without writing anything (dry runs, tests).
pub fn list<R: Read>(r: R) -> io::Result<Vec<FileEntry>> {
    let dec = zstd::stream::Decoder::new(r)?;
    let mut tar = tar::Archive::new(dec);
    let mut out = Vec::new();
    for e in tar.entries()? {
        let mut e = e?;
        if e.header().entry_type() != tar::EntryType::Regular {
            continue;
        }
        let rel = String::from_utf8_lossy(&e.path_bytes()).replace('/', "\\");
        let mut data = Vec::new();
        e.read_to_end(&mut data)?;
        out.push(FileEntry { path: rel, size: data.len() as u64, sha256: sha256_hex(&data) });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_paths() {
        assert!(safe_rel_path("a.exe").is_some());
        assert!(safe_rel_path("res/icons/x.png").is_some());
        for p in ["", "..\\x", "a\\..\\..\\x", "\\abs", "C:\\x", "a\\\\b", "./a", "a\\.", "x*", "trail."] {
            assert!(safe_rel_path(p).is_none(), "{p:?}");
        }
    }

    #[test]
    fn pack_unpack_round_trip() {
        let files = vec![
            ("app.exe".to_string(), b"MZ fake".to_vec()),
            ("res\\data.bin".to_string(), vec![7u8; 100_000]),
        ];
        let (arc, entries) = pack(&files, 3).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(list(arc.as_slice()).unwrap(), entries);
        let dir = std::env::temp_dir().join(format!("gloria-core-arc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut last = 0;
        let got = unpack(arc.as_slice(), &dir, |_, n| last = n).unwrap();
        assert_eq!(got, entries);
        assert_eq!(last, 100_007);
        assert_eq!(fs::read(dir.join("res").join("data.bin")).unwrap().len(), 100_000);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_duplicates_and_bad_paths() {
        assert!(pack(&[("A.txt".into(), vec![]), ("a.txt".into(), vec![])], 1).is_err());
        assert!(pack(&[("../x".into(), vec![])], 1).is_err());
    }
}
