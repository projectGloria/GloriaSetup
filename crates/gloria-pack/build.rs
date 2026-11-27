//! gloria-pack.exe itself is GloriaSetup's own tool, so it carries the GloriaSetup icon and version info.

use gloria_core::res;

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ico_path = root.join("assets/brand/GloriaSetup.ico");
    println!("cargo:rerun-if-changed={}", ico_path.display());
    println!("cargo:rerun-if-changed=build.rs");
    let images = res::parse_ico(&std::fs::read(&ico_path).expect("GloriaSetup.ico")).expect("valid ico");
    let ver = env!("CARGO_PKG_VERSION");
    let v = gloria_core::version::Version::parse(ver).unwrap().file_version();
    let strings = [
        ("CompanyName", "Project Gloria".to_string()),
        ("FileDescription", "GloriaSetup packer".to_string()),
        ("FileVersion", ver.to_string()),
        ("OriginalFilename", "gloria-pack.exe".to_string()),
        ("ProductName", "GloriaSetup".to_string()),
        ("ProductVersion", ver.to_string()),
    ];
    let version = res::version_info(&res::VersionInfo { file_version: v, product_version: v, strings: &strings });
    let group = res::group_icon(&images, 1);
    let mut entries: Vec<(u16, u16, u16, &[u8])> = Vec::new();
    for (i, im) in images.iter().enumerate() {
        entries.push((res::RT_ICON, 1 + i as u16, res::LANG_EN_US, &im.data));
    }
    entries.push((res::RT_GROUP_ICON, 1, res::LANG_EN_US, &group));
    entries.push((res::RT_VERSION, 1, res::LANG_EN_US, &version));
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("gloria-pack.res");
    std::fs::write(&out, res::res_file(&entries)).unwrap();
    println!("cargo:rustc-link-arg-bins={}", out.display());
}
