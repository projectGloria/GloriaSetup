//! Win32 resource reading and patching (`UpdateResource` needs a Windows host).

use std::path::Path;

use gloria_core::res::{self, IconImage, RT_GROUP_ICON, RT_ICON, RT_VERSION};
use windows::core::{BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::*;

/// A resource name: numeric id or string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResName {
    Id(u16),
    Str(String),
}

impl ResName {
    unsafe fn from_pcwstr(p: PCWSTR) -> ResName {
        let v = p.0 as usize;
        if v >> 16 == 0 {
            ResName::Id(v as u16)
        } else {
            ResName::Str(p.to_string().unwrap_or_default())
        }
    }
}

/// Keeps the wide string alive while the PCWSTR is in use.
enum NameBuf {
    Id(u16),
    Str(Vec<u16>),
}
impl NameBuf {
    fn new(n: &ResName) -> NameBuf {
        match n {
            ResName::Id(i) => NameBuf::Id(*i),
            ResName::Str(s) => NameBuf::Str(s.encode_utf16().chain(Some(0)).collect()),
        }
    }
    fn pcwstr(&self) -> PCWSTR {
        match self {
            NameBuf::Id(i) => PCWSTR(*i as usize as *const u16),
            NameBuf::Str(v) => PCWSTR(v.as_ptr()),
        }
    }
}

fn int_res(id: u16) -> PCWSTR {
    PCWSTR(id as usize as *const u16)
}

/// (type, name, language, data) for every resource of the given types.
pub type ResEntry = (u16, ResName, u16, Vec<u8>);

struct Module(HMODULE);
impl Drop for Module {
    fn drop(&mut self) {
        unsafe {
            let _ = FreeLibrary(self.0);
        }
    }
}

fn load_datafile(path: &Path) -> std::io::Result<Module> {
    let h = unsafe {
        LoadLibraryExW(
            &HSTRING::from(path.as_os_str()),
            None,
            LOAD_LIBRARY_AS_DATAFILE_EXCLUSIVE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
        )
    }
    .map_err(|e| std::io::Error::other(format!("cannot open {} for resources: {e}", path.display())))?;
    Ok(Module(h))
}

unsafe extern "system" fn enum_names(_: HMODULE, _: PCWSTR, name: PCWSTR, lp: isize) -> BOOL {
    let v = &mut *(lp as *mut Vec<ResName>);
    v.push(ResName::from_pcwstr(name));
    BOOL(1)
}

unsafe extern "system" fn enum_langs(_: HMODULE, _: PCWSTR, _: PCWSTR, lang: u16, lp: isize) -> BOOL {
    let v = &mut *(lp as *mut Vec<u16>);
    v.push(lang);
    BOOL(1)
}

/// Lists every resource of the given types (all names, all languages) with its data.
pub fn read_resources(path: &Path, types: &[u16]) -> std::io::Result<Vec<ResEntry>> {
    let m = load_datafile(path)?;
    let mut out = Vec::new();
    for &ty in types {
        let mut names: Vec<ResName> = Vec::new();
        unsafe {
            let _ = EnumResourceNamesW(Some(m.0), int_res(ty), Some(enum_names), &mut names as *mut _ as isize);
        }
        for n in names {
            let nb = NameBuf::new(&n);
            let mut langs: Vec<u16> = Vec::new();
            unsafe {
                let _ = EnumResourceLanguagesW(
                    Some(m.0),
                    int_res(ty),
                    nb.pcwstr(),
                    Some(enum_langs),
                    &mut langs as *mut _ as isize,
                );
            }
            for lang in langs {
                let data = unsafe {
                    let r = FindResourceExW(Some(m.0), int_res(ty), nb.pcwstr(), lang);
                    if r.is_invalid() {
                        continue;
                    }
                    let size = SizeofResource(Some(m.0), r) as usize;
                    let g = LoadResource(Some(m.0), r).map_err(std::io::Error::other)?;
                    let p = LockResource(g) as *const u8;
                    if p.is_null() {
                        continue;
                    }
                    std::slice::from_raw_parts(p, size).to_vec()
                };
                out.push((ty, n.clone(), lang, data));
            }
        }
    }
    Ok(out)
}

/// What to write into a stub copy.
pub struct Patch<'a> {
    /// `None` keeps the existing icon resources untouched (test builds with the default icon).
    pub icon: Option<&'a [IconImage]>,
    pub version_info: Vec<u8>,
}

/// Purges every RT_GROUP_ICON and RT_ICON (all names, all languages), writes the new icon group 1 with RT_ICON
/// ids 1..n, and replaces every RT_VERSION with the new one, all in one BeginUpdateResource session.
pub fn patch(path: &Path, p: &Patch) -> std::io::Result<()> {
    let mut purge_types = vec![RT_VERSION];
    if p.icon.is_some() {
        purge_types.extend([RT_GROUP_ICON, RT_ICON]);
    }
    // Enumerate before opening the update handle (the datafile mapping must be closed by then).
    let existing: Vec<(u16, ResName, u16)> =
        read_resources(path, &purge_types)?.into_iter().map(|(t, n, l, _)| (t, n, l)).collect();
    unsafe {
        let h = BeginUpdateResourceW(&HSTRING::from(path.as_os_str()), false)
            .map_err(|e| std::io::Error::other(format!("BeginUpdateResource {}: {e}", path.display())))?;
        let run = || -> windows::core::Result<()> {
            for (ty, name, lang) in &existing {
                let nb = NameBuf::new(name);
                UpdateResourceW(h, int_res(*ty), nb.pcwstr(), *lang, None, 0)?;
            }
            if let Some(icons) = p.icon {
                for (i, im) in icons.iter().enumerate() {
                    UpdateResourceW(
                        h,
                        int_res(RT_ICON),
                        int_res(1 + i as u16),
                        res::LANG_EN_US,
                        Some(im.data.as_ptr().cast()),
                        im.data.len() as u32,
                    )?;
                }
                let group = res::group_icon(icons, 1);
                UpdateResourceW(
                    h,
                    int_res(RT_GROUP_ICON),
                    int_res(1),
                    res::LANG_EN_US,
                    Some(group.as_ptr().cast()),
                    group.len() as u32,
                )?;
            }
            UpdateResourceW(
                h,
                int_res(RT_VERSION),
                int_res(1),
                res::LANG_EN_US,
                Some(p.version_info.as_ptr().cast()),
                p.version_info.len() as u32,
            )?;
            Ok(())
        };
        match run() {
            Ok(()) => EndUpdateResourceW(h, false)
                .map_err(|e| std::io::Error::other(format!("EndUpdateResource {}: {e}", path.display()))),
            Err(e) => {
                let _ = EndUpdateResourceW(h, true);
                Err(std::io::Error::other(format!("UpdateResource {}: {e}", path.display())))
            }
        }
    }
}

/// The icon images in group 1 of a PE (in group order), for tests and `--inspect`.
pub fn icon_group(path: &Path) -> std::io::Result<Vec<(u16, Vec<u8>)>> {
    let all = read_resources(path, &[RT_GROUP_ICON, RT_ICON])?;
    let Some((_, _, _, g)) = all.iter().find(|(t, n, _, _)| *t == RT_GROUP_ICON && *n == ResName::Id(1)) else {
        return Ok(vec![]);
    };
    let count = u16::from_le_bytes([g[4], g[5]]) as usize;
    let mut out = Vec::new();
    for i in 0..count {
        let e = &g[6 + i * 14..6 + i * 14 + 14];
        let id = u16::from_le_bytes([e[12], e[13]]);
        if let Some((_, _, _, d)) = all.iter().find(|(t, n, _, _)| *t == RT_ICON && *n == ResName::Id(id)) {
            out.push((id, d.clone()));
        }
    }
    Ok(out)
}
