//! Gives dummy.exe icon A, like a real app exe carries its own icon. The Start Menu entry and Settings → Apps show
//! the installed exe's icon, while the setup and uninstall.exe get whatever icon `gloria-pack --icon` was given.

use gloria_core::res;

fn main() {
    let ico_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/dummy-a.ico");
    println!("cargo:rerun-if-changed={}", ico_path.display());
    println!("cargo:rerun-if-changed=build.rs");
    let images = res::parse_ico(&std::fs::read(&ico_path).unwrap()).expect("valid dummy-a.ico");
    let group = res::group_icon(&images, 1);
    let mut entries: Vec<(u16, u16, u16, &[u8])> = Vec::new();
    for (i, im) in images.iter().enumerate() {
        entries.push((res::RT_ICON, 1 + i as u16, res::LANG_EN_US, &im.data));
    }
    entries.push((res::RT_GROUP_ICON, 1, res::LANG_EN_US, &group));
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("dummy.res");
    std::fs::write(&out, res::res_file(&entries)).unwrap();
    println!("cargo:rustc-link-arg-bins={}", out.display());
}
