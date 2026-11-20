//! Authenticode-safe PE payload. See docs/design.md §2.
//!
//! ```text
//! PE image | header (64 B) | manifest.json | archive (tar+zstd) | zero pad to 8 | [WIN_CERTIFICATE]
//!          ^ overlay_start = end of the last section's raw data; the header is read forward from here.
//! ```

use std::io::{self, Read, Seek, SeekFrom};

use sha2::{Digest, Sha256};

pub const MAGIC: [u8; 8] = *b"GLSETUP\0";
pub const FORMAT: u32 = 1;
pub const HEADER_LEN: usize = 64;

pub const FLAG_UNINSTALLER: u32 = 1;
pub const FLAG_TEST_BUILD: u32 = 2;

const IMAGE_DIRECTORY_ENTRY_SECURITY: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeInfo {
    /// End of the last section's raw data.
    pub overlay_start: u64,
    /// File offset of the 8-byte security data-directory entry (so tests and pack can patch it).
    pub security_entry_offset: u64,
    /// (file offset, size) of the certificate table, if signed.
    pub security: Option<(u64, u64)>,
}

fn rd_u16(b: &[u8], o: usize) -> io::Result<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| bad("truncated PE header"))
}
fn rd_u32(b: &[u8], o: usize) -> io::Result<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| bad("truncated PE header"))
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// Parses the headers. `head` must hold at least the DOS header, NT headers and the section table
/// (the first 4 KiB is enough for any normal image; pass more if needed).
pub fn pe_info(head: &[u8]) -> io::Result<PeInfo> {
    if head.get(0..2) != Some(b"MZ") {
        return Err(bad("not a PE file (no MZ)"));
    }
    let nt = rd_u32(head, 0x3c)? as usize;
    if head.get(nt..nt + 4) != Some(b"PE\0\0") {
        return Err(bad("not a PE file (no PE signature)"));
    }
    let coff = nt + 4;
    let nsec = rd_u16(head, coff + 2)? as usize;
    let opt_size = rd_u16(head, coff + 16)? as usize;
    let opt = coff + 20;
    let magic = rd_u16(head, opt)?;
    let (num_rva_off, dd_off) = match magic {
        0x20b => (opt + 108, opt + 112),
        0x10b => (opt + 92, opt + 96),
        _ => return Err(bad("unknown optional header magic")),
    };
    let num_rva = rd_u32(head, num_rva_off)? as usize;
    if num_rva <= IMAGE_DIRECTORY_ENTRY_SECURITY {
        return Err(bad("PE has no security directory slot"));
    }
    let sec_entry = dd_off + IMAGE_DIRECTORY_ENTRY_SECURITY * 8;
    let sec_off = rd_u32(head, sec_entry)? as u64;
    let sec_len = rd_u32(head, sec_entry + 4)? as u64;
    let sections = opt + opt_size;
    let mut end = 0u64;
    for i in 0..nsec {
        let s = sections + i * 40;
        let raw_size = rd_u32(head, s + 16)? as u64;
        let raw_ptr = rd_u32(head, s + 20)? as u64;
        if raw_size > 0 {
            end = end.max(raw_ptr + raw_size);
        }
    }
    if end == 0 {
        return Err(bad("PE has no section data"));
    }
    Ok(PeInfo {
        overlay_start: end,
        security_entry_offset: sec_entry as u64,
        security: (sec_off != 0 && sec_len != 0).then_some((sec_off, sec_len)),
    })
}

/// Builds header + manifest + archive + padding.
pub fn build(manifest_json: &[u8], archive: &[u8], flags: u32) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(manifest_json);
    h.update(archive);
    let sha: [u8; 32] = h.finalize().into();
    let mut out = Vec::with_capacity(HEADER_LEN + manifest_json.len() + archive.len() + 8);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&(manifest_json.len() as u64).to_le_bytes());
    out.extend_from_slice(&(archive.len() as u64).to_le_bytes());
    out.extend_from_slice(&sha);
    debug_assert_eq!(out.len(), HEADER_LEN);
    out.extend_from_slice(manifest_json);
    out.extend_from_slice(archive);
    while out.len() % 8 != 0 {
        out.push(0);
    }
    out
}

/// Appends a payload to an unsigned PE without an existing overlay. Signing must come after this step.
pub fn append(pe: &mut Vec<u8>, manifest_json: &[u8], archive: &[u8], flags: u32) -> io::Result<()> {
    let info = pe_info(pe)?;
    if info.security.is_some() {
        return Err(bad("the stub is already signed; append the payload before signing"));
    }
    if (pe.len() as u64) != info.overlay_start {
        return Err(bad("the stub already has data after its last section"));
    }
    // Keep the payload 8-aligned relative to the file (sections are file-aligned, so this is normally a no-op).
    while pe.len() % 8 != 0 {
        pe.push(0);
    }
    pe.extend_from_slice(&build(manifest_json, archive, flags));
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    pub flags: u32,
    pub manifest_json: Vec<u8>,
    /// Absolute file offset and length of the archive.
    pub archive_offset: u64,
    pub archive_len: u64,
    pub sha256: [u8; 32],
}

impl Payload {
    pub fn is_uninstaller(&self) -> bool {
        self.flags & FLAG_UNINSTALLER != 0
    }
    pub fn is_test_build(&self) -> bool {
        self.flags & FLAG_TEST_BUILD != 0
    }
}

/// Reads the payload header and manifest from a PE file. `Ok(None)` means a bare stub (no payload).
/// Does not hash the archive; call [`verify`] for that.
pub fn read<R: Read + Seek>(r: &mut R) -> io::Result<Option<Payload>> {
    let file_len = r.seek(SeekFrom::End(0))?;
    r.seek(SeekFrom::Start(0))?;
    let mut head = vec![0u8; 8192.min(file_len as usize)];
    r.read_exact(&mut head)?;
    let info = pe_info(&head)?;
    let limit = match info.security {
        Some((off, len)) => {
            if off < info.overlay_start || off.checked_add(len).map_or(true, |e| e > file_len) {
                return Err(bad("security directory out of bounds"));
            }
            off
        }
        None => file_len,
    };
    let start = info.overlay_start.next_multiple_of(8);
    if start + HEADER_LEN as u64 > file_len {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(start))?;
    let mut h = [0u8; HEADER_LEN];
    r.read_exact(&mut h)?;
    if h[0..8] != MAGIC {
        return Ok(None);
    }
    let format = u32::from_le_bytes(h[8..12].try_into().unwrap());
    if format != FORMAT {
        return Err(bad("unsupported payload format"));
    }
    let flags = u32::from_le_bytes(h[12..16].try_into().unwrap());
    let mlen = u64::from_le_bytes(h[16..24].try_into().unwrap());
    let alen = u64::from_le_bytes(h[24..32].try_into().unwrap());
    let sha: [u8; 32] = h[32..64].try_into().unwrap();
    let body = start + HEADER_LEN as u64;
    if body > limit {
        return Err(bad("payload overlaps the signature"));
    }
    let end = body.checked_add(mlen).and_then(|x| x.checked_add(alen)).ok_or_else(|| bad("payload lengths overflow"))?;
    if end > limit || mlen > 1 << 20 {
        return Err(bad("payload is truncated or overlaps the signature"));
    }
    let mut manifest_json = vec![0u8; mlen as usize];
    r.read_exact(&mut manifest_json)?;
    Ok(Some(Payload { flags, manifest_json, archive_offset: body + mlen, archive_len: alen, sha256: sha }))
}

/// Hashes manifest + archive and compares with the header.
pub fn verify<R: Read + Seek>(r: &mut R, p: &Payload) -> io::Result<()> {
    let mut h = Sha256::new();
    h.update(&p.manifest_json);
    r.seek(SeekFrom::Start(p.archive_offset))?;
    let mut left = p.archive_len;
    let mut buf = vec![0u8; 1 << 16];
    while left > 0 {
        let n = (left as usize).min(buf.len());
        r.read_exact(&mut buf[..n])?;
        h.update(&buf[..n]);
        left -= n as u64;
    }
    let got: [u8; 32] = h.finalize().into();
    if got != p.sha256 {
        return Err(bad("payload checksum mismatch (the file is damaged)"));
    }
    Ok(())
}

/// Test helper: imitates signtool by appending a WIN_CERTIFICATE blob (8-aligned) and pointing the security
/// directory at it. The content is fake; only the layout matters.
pub fn append_fake_certificate(pe: &mut Vec<u8>, cert_body_len: usize) -> io::Result<()> {
    let info = pe_info(pe)?;
    while pe.len() % 8 != 0 {
        pe.push(0);
    }
    let off = pe.len() as u32;
    let total = (8 + cert_body_len).next_multiple_of(8) as u32;
    pe.extend_from_slice(&total.to_le_bytes()); // dwLength
    pe.extend_from_slice(&0x0200u16.to_le_bytes()); // WIN_CERT_REVISION_2_0
    pe.extend_from_slice(&0x0002u16.to_le_bytes()); // WIN_CERT_TYPE_PKCS_SIGNED_DATA
    pe.extend((0..cert_body_len).map(|i| (i * 31 + 7) as u8));
    while pe.len() % 8 != 0 {
        pe.push(0);
    }
    let e = info.security_entry_offset as usize;
    pe[e..e + 4].copy_from_slice(&off.to_le_bytes());
    pe[e + 4..e + 8].copy_from_slice(&total.to_le_bytes());
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;

    /// A minimal but valid-looking PE32+ image: DOS header, NT headers, one section of 0x200 bytes.
    pub fn tiny_pe() -> Vec<u8> {
        let mut b = vec![0u8; 0x400];
        b[0..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        b[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
        b[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        b[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
        let opt = coff + 20;
        b[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        b[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        let sec = opt + 240;
        b[sec..sec + 5].copy_from_slice(b".text");
        b[sec + 16..sec + 20].copy_from_slice(&0x200u32.to_le_bytes());
        b[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes());
        b
    }

    #[test]
    fn round_trip_unsigned() {
        let mut pe = tiny_pe();
        append(&mut pe, br#"{"a":1}"#, b"ARCHIVE-BYTES", FLAG_TEST_BUILD).unwrap();
        assert_eq!(pe.len() % 8, 0);
        let mut c = Cursor::new(&pe);
        let p = read(&mut c).unwrap().unwrap();
        assert_eq!(p.manifest_json, br#"{"a":1}"#);
        assert!(p.is_test_build() && !p.is_uninstaller());
        verify(&mut c, &p).unwrap();
        assert_eq!(&pe[p.archive_offset as usize..][..p.archive_len as usize], b"ARCHIVE-BYTES");
    }

    #[test]
    fn round_trip_with_fake_signature() {
        let mut pe = tiny_pe();
        append(&mut pe, b"{}", &vec![0xAB; 5000], 0).unwrap();
        append_fake_certificate(&mut pe, 1234).unwrap();
        let info = pe_info(&pe).unwrap();
        assert!(info.security.is_some());
        let mut c = Cursor::new(&pe);
        let p = read(&mut c).unwrap().unwrap();
        verify(&mut c, &p).unwrap();
        assert_eq!(p.archive_len, 5000);
        // Appending after signing is refused (the pack order is enforced in code).
        assert!(append(&mut pe.clone(), b"{}", b"", 0).is_err());
    }

    #[test]
    fn bare_stub_has_no_payload() {
        let pe = tiny_pe();
        assert_eq!(read(&mut Cursor::new(&pe)).unwrap(), None);
        // A signed bare stub also has none (the certificate must not be mistaken for a payload).
        let mut pe = tiny_pe();
        append_fake_certificate(&mut pe, 100).unwrap();
        assert_eq!(read(&mut Cursor::new(&pe)).unwrap(), None);
    }

    #[test]
    fn detects_corruption_and_overlap() {
        let mut pe = tiny_pe();
        append(&mut pe, b"{}", b"0123456789", 0).unwrap();
        let mut broken = pe.clone();
        let n = broken.len();
        broken[n - 9] ^= 0xff; // inside the archive
        let mut c = Cursor::new(&broken);
        let p = read(&mut c).unwrap().unwrap();
        assert!(verify(&mut c, &p).is_err());
        // Security directory pointing into the payload → rejected.
        let mut overlap = pe.clone();
        let info = pe_info(&overlap).unwrap();
        let e = info.security_entry_offset as usize;
        overlap[e..e + 4].copy_from_slice(&((info.overlay_start + 16) as u32).to_le_bytes());
        overlap[e + 4..e + 8].copy_from_slice(&8u32.to_le_bytes());
        assert!(read(&mut Cursor::new(&overlap)).is_err());
        // Truncated file → rejected.
        let short = &pe[..pe.len() - 16];
        assert!(read(&mut Cursor::new(short)).is_err());
    }

    #[test]
    fn refuses_existing_overlay() {
        let mut pe = tiny_pe();
        pe.extend_from_slice(b"junk");
        assert!(append(&mut pe, b"{}", b"", 0).is_err());
    }
}
