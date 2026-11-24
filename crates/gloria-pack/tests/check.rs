//! `gloria-pack --check`: catches manifest mistakes before any build runs.

use std::path::PathBuf;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gloria-check-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const BASE: &str = "id = \"Demo\"\nname = \"Demo\"\nversion = \"1.0.0\"\nmain_exe = \"demo.exe\"\naccent = \"#22d3ee\"\n";

#[test]
fn bad_escape_in_data_folders_fails() {
    let d = dir("escape");
    let m = d.join("gloria.toml");
    std::fs::write(&m, format!("{BASE}{}\n", r#"data_folders = ["%APPDATA%\dev.demo"]"#)).unwrap();
    let e = gloria_pack::check(&m, None, None, true, None).unwrap_err();
    assert!(e.contains("invalid escape"), "{e}");
}

#[test]
fn literal_strings_and_version_override_pass() {
    let d = dir("ok");
    let m = d.join("gloria.toml");
    std::fs::write(&m, format!("{BASE}{}\n", r#"data_folders = ['%APPDATA%\dev.demo']"#)).unwrap();
    let app = gloria_pack::check(&m, Some("v2.0.0-alpha.1"), None, true, None).unwrap();
    assert_eq!(gloria_pack::setup_file_name(&app), "Demo_2.0.0-alpha.1_x64-setup.exe");
}

#[test]
fn missing_icon_and_missing_main_exe_fail() {
    let d = dir("icon");
    let m = d.join("gloria.toml");
    std::fs::write(&m, format!("{BASE}icon = \"nope.ico\"\n")).unwrap();
    assert!(gloria_pack::check(&m, None, None, false, None).is_err());
    std::fs::write(&m, BASE).unwrap();
    assert!(gloria_pack::check(&m, None, None, false, None).unwrap_err().contains("no icon"));
    std::fs::create_dir_all(d.join("app")).unwrap();
    let e = gloria_pack::check(&m, None, None, true, Some(&d.join("app"))).unwrap_err();
    assert!(e.contains("main_exe"), "{e}");
}
