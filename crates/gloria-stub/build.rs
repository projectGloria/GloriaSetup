//! Embeds the bare stub's resources: GloriaSetup icon (group 1, all sizes), the application manifest
//! (per-monitor DPI v2, asInvoker) and version info. Writes a `.res` file and hands it to link.exe, so no rc.exe or
//! Windows SDK tools are needed.

use gloria_core::res;

const APP_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="projectGloria.GloriaSetup" version="1.0.0.0" processorArchitecture="amd64"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application><supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/></application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
    </windowsSettings>
  </application>
  <dependency><dependentAssembly>
    <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
  </dependentAssembly></dependency>
</assembly>
"#;

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ico_path = root.join("assets/brand/GloriaSetup.ico");
    println!("cargo:rerun-if-changed={}", ico_path.display());
    println!("cargo:rerun-if-changed=build.rs");
    let ico = std::fs::read(&ico_path).expect("assets/brand/GloriaSetup.ico");
    let images = res::parse_ico(&ico).expect("valid GloriaSetup.ico");

    let ver = env!("CARGO_PKG_VERSION");
    let v = gloria_core::version::Version::parse(ver).unwrap().file_version();
    let strings = [
        ("CompanyName", "projectGloria".to_string()),
        ("FileDescription", "GloriaSetup".to_string()),
        ("FileVersion", ver.to_string()),
        ("InternalName", "gloria-stub".to_string()),
        ("OriginalFilename", "gloria-stub.exe".to_string()),
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
    entries.push((res::RT_MANIFEST, 1, res::LANG_EN_US, APP_MANIFEST.as_bytes()));

    // The small "Packaged with GloriaSetup" credit image on the done screens. It is an embedded file, not an RT_ICON,
    // so packed installers (whose icon resources are replaced) still carry it.
    let credit = images.iter().filter(|i| i.width >= 32).min_by_key(|i| i.width).expect("a 32 px or larger icon");
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::write(out_dir.join("credit.ico"), res::build_ico(std::slice::from_ref(credit))).unwrap();

    let out = out_dir.join("gloria-stub.res");
    std::fs::write(&out, res::res_file(&entries)).unwrap();
    println!("cargo:rustc-link-arg-bins={}", out.display());
}
