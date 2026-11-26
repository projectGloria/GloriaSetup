//! UI strings. The four TOML files are compiled in; English fills any gap. `{var}` placeholders are filled from a
//! per-call list plus the app defaults (`name`, `version`, `exe`).

use std::collections::HashMap;

pub const LANGS: [&str; 4] = ["en", "fr", "es", "tr"];

const SOURCES: [(&str, &str); 4] = [
    ("en", include_str!("../i18n/en.toml")),
    ("fr", include_str!("../i18n/fr.toml")),
    ("es", include_str!("../i18n/es.toml")),
    ("tr", include_str!("../i18n/tr.toml")),
];

pub fn table(lang: &str) -> HashMap<String, String> {
    let src = SOURCES.iter().find(|(l, _)| *l == lang).map(|(_, s)| *s).unwrap_or(SOURCES[0].1);
    toml::from_str(src).expect("i18n toml is valid (checked by a test)")
}

pub struct Strings {
    pub lang: &'static str,
    own: HashMap<String, String>,
    en: HashMap<String, String>,
    defaults: Vec<(&'static str, String)>,
}

impl Strings {
    pub fn new(lang: &str, defaults: Vec<(&'static str, String)>) -> Strings {
        let lang = LANGS.iter().copied().find(|l| *l == lang).unwrap_or("en");
        Strings { lang, own: table(lang), en: table("en"), defaults }
    }

    /// Looks up `key` and fills placeholders.
    pub fn t(&self, key: &str) -> String {
        self.tv(key, &[])
    }

    pub fn tv(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let raw = self.own.get(key).or_else(|| self.en.get(key)).map(String::as_str).unwrap_or(key);
        fill(raw, |k| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
                .or_else(|| self.defaults.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone()))
        })
    }

    /// Uppercase that respects Turkish dotted/dotless i.
    pub fn upper(&self, s: &str) -> String {
        if self.lang == "tr" {
            s.chars()
                .map(|c| match c {
                    'i' => "İ".to_string(),
                    'ı' => "I".to_string(),
                    c => c.to_uppercase().collect(),
                })
                .collect()
        } else {
            s.to_uppercase()
        }
    }

    /// Decimal separator for sizes.
    pub fn decimal(&self) -> char {
        if self.lang == "en" {
            '.'
        } else {
            ','
        }
    }

    /// "48.2 MB" with binary units, like Explorer.
    pub fn size(&self, bytes: u64) -> String {
        let units = ["B", "KB", "MB", "GB", "TB"];
        let mut v = bytes as f64;
        let mut u = 0;
        while v >= 1024.0 && u < units.len() - 1 {
            v /= 1024.0;
            u += 1;
        }
        let s = if u == 0 || v >= 100.0 { format!("{v:.0}") } else { format!("{v:.1}") };
        let s = s.strip_suffix(".0").map(str::to_string).unwrap_or(s);
        format!("{} {}", s.replace('.', &self.decimal().to_string()), units[u])
    }
}

pub fn fill(raw: &str, mut var: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    let mut rest = raw;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('}') {
            Some(j) if after[..j].chars().all(|c| c.is_ascii_alphanumeric()) => {
                out.push_str(&var(&after[..j]).unwrap_or_default());
                rest = &after[j + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Picks the UI language: `--lang`, else the Windows display language, else English.
pub fn detect(arg: Option<&str>) -> &'static str {
    if let Some(l) = arg {
        if let Some(x) = LANGS.iter().find(|x| **x == l) {
            return x;
        }
    }
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    match id & 0x3ff {
        0x0c => "fr",
        0x0a => "es",
        0x1f => "tr",
        _ => "en",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_every_key() {
        let en = table("en");
        for l in LANGS {
            let t = table(l);
            let missing: Vec<_> = en.keys().filter(|k| !t.contains_key(*k)).collect();
            let extra: Vec<_> = t.keys().filter(|k| !en.contains_key(*k)).collect();
            assert!(missing.is_empty() && extra.is_empty(), "{l}: missing {missing:?}, extra {extra:?}");
            for (k, v) in &t {
                let vars = |s: &str| {
                    let mut v: Vec<String> = s.split('{').skip(1).filter_map(|x| x.split_once('}')).map(|x| x.0.into()).collect();
                    v.sort();
                    v
                };
                assert_eq!(vars(v), vars(&en[k]), "{l}.{k}: placeholders differ from English");
            }
        }
    }

    #[test]
    fn fills_placeholders() {
        let s = Strings::new("en", vec![("name", "Foo".into())]);
        assert_eq!(s.t("setup"), "Foo Setup");
        assert_eq!(s.tv("data.total", &[("size", "1 MB")]), "1 MB · deleted permanently");
        assert_eq!(fill("{a} {b", |_| Some("x".into())), "x {b");
        let tr = Strings::new("tr", vec![]);
        assert_eq!(tr.upper("kaldırma iptal"), "KALDIRMA İPTAL");
        assert_eq!(s.size(48_600_000), "46.3 MB");
        assert_eq!(Strings::new("fr", vec![]).size(1536), "1,5 KB");
        assert_eq!(s.size(0), "0 B");
    }
}
