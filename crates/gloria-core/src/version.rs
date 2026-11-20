//! SemVer 2.0 precedence with two extensions: a leading `v` is ignored, and the core may have 1 to 4 numeric parts
//! (missing parts are 0). Build metadata is ignored. See docs/design.md §10.

use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone)]
pub struct Version {
    pub core: [u64; 4],
    /// Number of core parts actually written (1..=4), used only for display.
    pub parts: usize,
    pub pre: Vec<PreId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreId {
    Num(u64),
    Alpha(String),
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let s = s.split_once('+').map(|(a, _)| a).unwrap_or(s);
        let (core_s, pre_s) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let mut core = [0u64; 4];
        let mut parts = 0;
        for (i, p) in core_s.split('.').enumerate() {
            if i >= 4 || p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            core[i] = p.parse().ok()?;
            parts = i + 1;
        }
        let mut pre = Vec::new();
        if let Some(p) = pre_s {
            for id in p.split('.') {
                if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                    return None;
                }
                if id.bytes().all(|b| b.is_ascii_digit()) {
                    pre.push(PreId::Num(id.parse().ok()?));
                } else {
                    pre.push(PreId::Alpha(id.to_string()));
                }
            }
        }
        Some(Version { core, parts, pre })
    }

    /// The four numeric parts for `VS_FIXEDFILEINFO` (each clamped to u16).
    pub fn file_version(&self) -> [u16; 4] {
        self.core.map(|p| p.min(u16::MAX as u64) as u16)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.parts.max(3);
        let core: Vec<String> = self.core[..n].iter().map(|p| p.to_string()).collect();
        write!(f, "{}", core.join("."))?;
        if !self.pre.is_empty() {
            let pre: Vec<String> = self
                .pre
                .iter()
                .map(|p| match p {
                    PreId::Num(n) => n.to_string(),
                    PreId::Alpha(a) => a.clone(),
                })
                .collect();
            write!(f, "-{}", pre.join("."))?;
        }
        Ok(())
    }
}

impl Ord for PreId {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (PreId::Num(a), PreId::Num(b)) => a.cmp(b),
            (PreId::Num(_), PreId::Alpha(_)) => Ordering::Less,
            (PreId::Alpha(_), PreId::Num(_)) => Ordering::Greater,
            (PreId::Alpha(a), PreId::Alpha(b)) => a.cmp(b),
        }
    }
}
impl PartialOrd for PreId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Equality is precedence equality: `1.2` == `1.2.0`.
impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Version {}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self.pre.cmp(&other.pre),
        })
    }
}
impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// How the packed version relates to what is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// Installed is older (or unparseable): offer an update.
    Update,
    Same,
    /// Installed is newer: downgrade warning.
    Downgrade,
}

/// Compares the installed version string against the version being installed.
/// An unparseable installed version counts as older, so the user is never shown a false downgrade warning.
pub fn relation(installed: &str, packed: &Version) -> Relation {
    match Version::parse(installed) {
        None => Relation::Update,
        Some(v) => match v.cmp(packed) {
            Ordering::Less => Relation::Update,
            Ordering::Equal => Relation::Same,
            Ordering::Greater => Relation::Downgrade,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| panic!("parse {s}"))
    }

    #[test]
    fn numeric_parts_compare_as_integers() {
        assert!(v("0.10.0") > v("0.2.0"));
        assert!(v("1.0.10") > v("1.0.9"));
    }

    #[test]
    fn equal_with_missing_parts_and_prefix() {
        assert_eq!(v("1.2"), v("1.2.0"));
        assert_eq!(v("1.2").cmp(&v("v1.2.0.0")), Ordering::Equal);
        assert_eq!(v("1.2.0+build.5").cmp(&v("1.2.0")), Ordering::Equal);
    }

    #[test]
    fn prerelease_ordering() {
        let order = [
            "1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11",
            "1.0.0-rc.1", "1.0.0",
        ];
        for w in order.windows(2) {
            assert!(v(w[0]) < v(w[1]), "{} < {}", w[0], w[1]);
        }
    }

    #[test]
    fn relations() {
        let packed = v("0.2.0");
        assert_eq!(relation("0.1.0", &packed), Relation::Update);
        assert_eq!(relation("0.2.0", &packed), Relation::Same);
        assert_eq!(relation("v0.2", &packed), Relation::Same);
        assert_eq!(relation("0.3.0", &packed), Relation::Downgrade);
        assert_eq!(relation("garbage", &packed), Relation::Update);
        assert_eq!(relation("", &packed), Relation::Update);
    }

    #[test]
    fn rejects_bad_input() {
        for s in ["", "1..2", "1.2.3.4.5", "a.b", "1.2-", "1.2-be ta", "-1"] {
            assert!(Version::parse(s).is_none(), "{s:?} should not parse");
        }
    }

    #[test]
    fn display_and_file_version() {
        assert_eq!(v("v1.2").to_string(), "1.2.0");
        assert_eq!(v("1.2.3-rc.1+x").to_string(), "1.2.3-rc.1");
        assert_eq!(v("1.2.3.4").file_version(), [1, 2, 3, 4]);
        assert_eq!(v("70000.1").file_version(), [65535, 1, 0, 0]);
    }
}
