//! Pure builders for Win32 resources: icon groups from .ico files, VS_VERSIONINFO, and the `.res` container the
//! MSVC linker accepts directly. Shared by the stub's build script (bare stub resources) and gloria-pack.

use std::io;

pub const RT_ICON: u16 = 3;
pub const RT_GROUP_ICON: u16 = 14;
pub const RT_VERSION: u16 = 16;
pub const RT_MANIFEST: u16 = 24;
pub const LANG_EN_US: u16 = 0x0409;
pub const LANG_NEUTRAL: u16 = 0;

/// One image of an icon, exactly as stored in an RT_ICON resource (a DIB with AND mask, or a PNG).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconImage {
    pub width: u32,
    pub height: u32,
    pub bit_count: u16,
    pub data: Vec<u8>,
}

impl IconImage {
    pub fn is_png(&self) -> bool {
        self.data.starts_with(b"\x89PNG")
    }
}

/// Parses an .ico file into its images. Width/height 0 in the directory means 256.
pub fn parse_ico(b: &[u8]) -> io::Result<Vec<IconImage>> {
    let err = |m: &str| io::Error::new(io::ErrorKind::InvalidData, format!("icon: {m}"));
    if b.len() < 6 || b[0..4] != [0, 0, 1, 0] {
        return Err(err("not an .ico file"));
    }
    let n = u16::from_le_bytes([b[4], b[5]]) as usize;
    let mut out = Vec::new();
    for i in 0..n {
        let e = b.get(6 + i * 16..6 + i * 16 + 16).ok_or_else(|| err("truncated directory"))?;
        let w = if e[0] == 0 { 256 } else { e[0] as u32 };
        let h = if e[1] == 0 { 256 } else { e[1] as u32 };
        let bpp = u16::from_le_bytes([e[6], e[7]]);
        let len = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
        let off = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
        let data = b.get(off..off + len).ok_or_else(|| err("image out of bounds"))?.to_vec();
        // Prefer the real header values (the directory is often wrong for PNG entries).
        let (w, h, bpp) = if data.starts_with(b"\x89PNG") && data.len() >= 24 {
            let pw = u32::from_be_bytes(data[16..20].try_into().unwrap());
            let ph = u32::from_be_bytes(data[20..24].try_into().unwrap());
            (pw, ph, 32)
        } else if data.len() >= 16 {
            // BITMAPINFOHEADER: the height covers the XOR and AND masks, so it is doubled.
            let bw = i32::from_le_bytes(data[4..8].try_into().unwrap()).unsigned_abs();
            let bh = i32::from_le_bytes(data[8..12].try_into().unwrap()).unsigned_abs() / 2;
            let bc = u16::from_le_bytes(data[14..16].try_into().unwrap());
            (bw.max(1), bh.max(1), if bc == 0 { bpp } else { bc })
        } else {
            (w, h, bpp)
        };
        out.push(IconImage { width: w, height: h, bit_count: bpp, data });
    }
    if out.is_empty() {
        return Err(err("no images"));
    }
    Ok(out)
}

/// Builds an .ico file from images (used by tests and by gloria-pack to write a normalized copy).
pub fn build_ico(images: &[IconImage]) -> Vec<u8> {
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut off = 6 + images.len() * 16;
    for im in images {
        out.push(if im.width >= 256 { 0 } else { im.width as u8 });
        out.push(if im.height >= 256 { 0 } else { im.height as u8 });
        out.extend_from_slice(&[0, 0, 1, 0]);
        out.extend_from_slice(&im.bit_count.to_le_bytes());
        out.extend_from_slice(&(im.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(off as u32).to_le_bytes());
        off += im.data.len();
    }
    for im in images {
        out.extend_from_slice(&im.data);
    }
    out
}

/// GRPICONDIR for images that will be stored as RT_ICON ids `first_id..`.
pub fn group_icon(images: &[IconImage], first_id: u16) -> Vec<u8> {
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (i, im) in images.iter().enumerate() {
        out.push(if im.width >= 256 { 0 } else { im.width as u8 });
        out.push(if im.height >= 256 { 0 } else { im.height as u8 });
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&im.bit_count.to_le_bytes());
        out.extend_from_slice(&(im.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(first_id + i as u16).to_le_bytes());
    }
    out
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(std::iter::once(0)).flat_map(|c| c.to_le_bytes()).collect()
}

fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

/// One VS_VERSIONINFO-style node. `value_len` is in WCHARs for text nodes and bytes for binary ones.
fn node(key: &str, text: bool, value: &[u8], value_len: u16, children: &[Vec<u8>]) -> Vec<u8> {
    let mut v = vec![0u8; 6];
    v.extend_from_slice(&utf16z(key));
    pad4(&mut v);
    v.extend_from_slice(value);
    for c in children {
        pad4(&mut v);
        v.extend_from_slice(c);
    }
    let len = v.len() as u16;
    v[0..2].copy_from_slice(&len.to_le_bytes());
    v[2..4].copy_from_slice(&value_len.to_le_bytes());
    v[4..6].copy_from_slice(&(text as u16).to_le_bytes());
    v
}

pub struct VersionInfo<'a> {
    pub file_version: [u16; 4],
    pub product_version: [u16; 4],
    /// (key, value) pairs for the 040904B0 string table, e.g. ("ProductName", "OmniConvert").
    pub strings: &'a [(&'a str, String)],
}

/// Builds an RT_VERSION resource (US English, Unicode code page).
pub fn version_info(vi: &VersionInfo) -> Vec<u8> {
    let ms = |v: [u16; 4]| ((v[0] as u32) << 16) | v[1] as u32;
    let ls = |v: [u16; 4]| ((v[2] as u32) << 16) | v[3] as u32;
    let mut fixed = Vec::new();
    for x in [
        0xFEEF04BDu32,
        0x0001_0000,
        ms(vi.file_version),
        ls(vi.file_version),
        ms(vi.product_version),
        ls(vi.product_version),
        0x3F,
        0,
        0x0004_0004, // VOS_NT_WINDOWS32
        1,           // VFT_APP
        0,
        0,
        0,
    ] {
        fixed.extend_from_slice(&x.to_le_bytes());
    }
    let strings: Vec<Vec<u8>> = vi
        .strings
        .iter()
        .map(|(k, v)| {
            let val = utf16z(v);
            node(k, true, &val, (val.len() / 2) as u16, &[])
        })
        .collect();
    let table = node("040904B0", true, &[], 0, &strings);
    let sfi = node("StringFileInfo", true, &[], 0, &[table]);
    let mut tr = Vec::new();
    tr.extend_from_slice(&0x0409u16.to_le_bytes());
    tr.extend_from_slice(&0x04B0u16.to_le_bytes());
    let var = node("Translation", false, &tr, 4, &[]);
    let vfi = node("VarFileInfo", true, &[], 0, &[var]);
    node("VS_VERSION_INFO", false, &fixed, 52, &[sfi, vfi])
}

/// Reads the string table back (tests and `gloria-pack --inspect`). Returns (key, value) pairs and the fixed
/// file version.
pub fn parse_version_info(b: &[u8]) -> Option<([u16; 4], Vec<(String, String)>)> {
    fn rd16(b: &[u8], o: usize) -> Option<u16> {
        Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
    }
    fn key_end(b: &[u8], start: usize) -> Option<(String, usize)> {
        let mut o = start;
        let mut units = Vec::new();
        loop {
            let c = rd16(b, o)?;
            o += 2;
            if c == 0 {
                break;
            }
            units.push(c);
        }
        Some((String::from_utf16_lossy(&units), (o + 3) & !3))
    }
    fn walk(b: &[u8], start: usize, out: &mut Vec<(String, String)>, fixed: &mut [u16; 4]) -> Option<usize> {
        let len = rd16(b, start)? as usize;
        let vlen = rd16(b, start + 2)? as usize;
        let text = rd16(b, start + 4)? == 1;
        let (key, mut o) = key_end(b, start + 6)?;
        let end = start + len;
        if key == "VS_VERSION_INFO" && vlen == 52 {
            let ms = u32::from_le_bytes(b.get(o + 8..o + 12)?.try_into().ok()?);
            let ls = u32::from_le_bytes(b.get(o + 12..o + 16)?.try_into().ok()?);
            *fixed = [(ms >> 16) as u16, ms as u16, (ls >> 16) as u16, ls as u16];
            o += 52;
        } else if text && vlen > 0 {
            let units: Vec<u16> = (0..vlen).filter_map(|i| rd16(b, o + i * 2)).take_while(|&c| c != 0).collect();
            out.push((key, String::from_utf16_lossy(&units)));
            o += vlen * 2;
        } else {
            o += vlen;
        }
        o = (o + 3) & !3;
        while o + 6 <= end {
            let n = walk(b, o, out, fixed)?;
            o = (n + 3) & !3;
        }
        Some(end)
    }
    let mut out = Vec::new();
    let mut fixed = [0u16; 4];
    walk(b, 0, &mut out, &mut fixed)?;
    Some((fixed, out))
}

/// A `.res` file (the binary output of rc.exe) with numeric types and names. link.exe accepts it as an input.
pub fn res_file(entries: &[(u16, u16, u16, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let hdr = |out: &mut Vec<u8>, size: u32, ty: u16, name: u16, lang: u16, flags: u16| {
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&32u32.to_le_bytes());
        out.extend_from_slice(&0xFFFFu16.to_le_bytes());
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&0xFFFFu16.to_le_bytes());
        out.extend_from_slice(&name.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&lang.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    };
    hdr(&mut out, 0, 0, 0, 0, 0);
    for (ty, name, lang, data) in entries {
        hdr(&mut out, data.len() as u32, *ty, *name, *lang, 0x1030);
        out.extend_from_slice(data);
        pad4(&mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_info_round_trip() {
        let strings = [
            ("CompanyName", "projectGloria".to_string()),
            ("FileDescription", "Gloria Dummy Setup".to_string()),
            ("ProductName", "Gloria Dummy".to_string()),
            ("ProductVersion", "0.2.0".to_string()),
        ];
        let b = version_info(&VersionInfo { file_version: [0, 2, 0, 0], product_version: [0, 2, 0, 0], strings: &strings });
        assert_eq!(u16::from_le_bytes([b[0], b[1]]) as usize, b.len());
        let (fixed, got) = parse_version_info(&b).unwrap();
        assert_eq!(fixed, [0, 2, 0, 0]);
        for (k, v) in &strings {
            assert!(got.iter().any(|(gk, gv)| gk == k && gv == v), "{k} missing in {got:?}");
        }
    }

    #[test]
    fn brand_icon_parses() {
        let b = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/brand/GloriaSetup.ico")).unwrap();
        let ims = parse_ico(&b).unwrap();
        assert!(ims.len() >= 6);
        assert!(ims.iter().any(|i| i.width == 256));
        assert_eq!(parse_ico(&build_ico(&ims)).unwrap(), ims);
        let g = group_icon(&ims, 1);
        assert_eq!(g.len(), 6 + 14 * ims.len());
    }
}
