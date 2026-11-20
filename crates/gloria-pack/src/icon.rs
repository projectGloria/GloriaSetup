//! Loads the app icon (`.ico` or `.png`) and fills in the standard sizes Windows asks for.

use std::path::Path;

use gloria_core::res::{self, IconImage};
use image::{imageops::FilterType, RgbaImage};

/// Sizes Explorer, the taskbar, Start and Apps & features ask for.
pub const STANDARD_SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];

pub fn load(path: &Path) -> Result<Vec<IconImage>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("icon {}: {e}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "ico" => from_ico(&bytes).map_err(|e| format!("icon {}: {e}", path.display())),
        "png" => from_png(&bytes).map_err(|e| format!("icon {}: {e}", path.display())),
        _ => Err(format!("icon {}: use a .ico or .png file", path.display())),
    }
}

/// Keeps every entry of the `.ico` as is and generates any missing standard size from the largest image.
pub fn from_ico(bytes: &[u8]) -> Result<Vec<IconImage>, String> {
    let mut images = res::parse_ico(bytes).map_err(|e| e.to_string())?;
    if images.is_empty() {
        return Err("the .ico has no images".into());
    }
    let largest = images.iter().max_by_key(|i| (i.width, i.bit_count)).unwrap().clone();
    let missing: Vec<u32> =
        STANDARD_SIZES.iter().copied().filter(|s| !images.iter().any(|i| i.width == *s && i.height == *s)).collect();
    if !missing.is_empty() {
        if let Some(src) = decode(&largest) {
            for s in missing.into_iter().filter(|s| *s <= src.width()) {
                images.push(encode(&resize(&src, s)));
            }
        }
    }
    images.sort_by_key(|i| (std::cmp::Reverse(i.width), std::cmp::Reverse(i.bit_count)));
    Ok(images)
}

/// A square PNG of at least 256x256 becomes all standard sizes.
pub fn from_png(bytes: &[u8]) -> Result<Vec<IconImage>, String> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).map_err(|e| e.to_string())?.to_rgba8();
    if img.width() != img.height() {
        return Err(format!("the PNG is {}x{}; it must be square", img.width(), img.height()));
    }
    if img.width() < 256 {
        return Err(format!("the PNG is {}x{}; it must be at least 256x256", img.width(), img.height()));
    }
    let mut out: Vec<IconImage> = STANDARD_SIZES.iter().rev().map(|&s| encode(&resize(&img, s))).collect();
    out.sort_by_key(|i| std::cmp::Reverse(i.width));
    Ok(out)
}

fn resize(src: &RgbaImage, size: u32) -> RgbaImage {
    if src.width() == size && src.height() == size {
        return src.clone();
    }
    image::imageops::resize(src, size, size, FilterType::Lanczos3)
}

/// 256 is stored as PNG, smaller sizes as 32-bit BMP DIBs (what older shell code paths read best).
pub fn encode(img: &RgbaImage) -> IconImage {
    let (w, h) = img.dimensions();
    if w >= 256 {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
            .expect("png encode");
        return IconImage { width: w, height: h, bit_count: 32, data: png };
    }
    IconImage { width: w, height: h, bit_count: 32, data: encode_dib(img) }
}

use image::ImageEncoder;

fn encode_dib(img: &RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mask_row = (w.div_ceil(32) * 4) as usize;
    let mut b = Vec::with_capacity(40 + (w * h * 4) as usize + mask_row * h as usize);
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&((h * 2) as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // planes
    b.extend_from_slice(&32u16.to_le_bytes()); // bit count
    b.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    b.extend_from_slice(&(w * h * 4).to_le_bytes());
    b.extend_from_slice(&[0u8; 16]); // ppm x/y, clr used/important
    for y in (0..h).rev() {
        for x in 0..w {
            let p = img.get_pixel(x, y).0;
            b.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    b.resize(b.len() + mask_row * h as usize, 0);
    b
}

/// Decodes a PNG entry or a 32-bit DIB entry; other DIB depths are not used as a resize source.
fn decode(im: &IconImage) -> Option<RgbaImage> {
    if im.is_png() {
        return image::load_from_memory_with_format(&im.data, image::ImageFormat::Png).ok().map(|i| i.to_rgba8());
    }
    let d = &im.data;
    if d.len() < 40 {
        return None;
    }
    let hdr = u32::from_le_bytes(d[0..4].try_into().ok()?) as usize;
    let w = i32::from_le_bytes(d[4..8].try_into().ok()?).unsigned_abs();
    let h = i32::from_le_bytes(d[8..12].try_into().ok()?).unsigned_abs() / 2;
    let bpp = u16::from_le_bytes(d[14..16].try_into().ok()?);
    if bpp != 32 || w == 0 || h == 0 {
        return None;
    }
    let px = d.get(hdr..hdr + (w * h * 4) as usize)?;
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = (((h - 1 - y) * w + x) * 4) as usize;
            out.put_pixel(x, y, image::Rgba([px[i + 2], px[i + 1], px[i], px[i + 3]]));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(size: u32) -> Vec<u8> {
        let img = RgbaImage::from_pixel(size, size, image::Rgba([200, 10, 10, 255]));
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(img.as_raw(), size, size, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    #[test]
    fn png_gives_all_standard_sizes() {
        let v = from_png(&png(512)).unwrap();
        let mut sizes: Vec<u32> = v.iter().map(|i| i.width).collect();
        sizes.sort();
        assert_eq!(sizes, STANDARD_SIZES.to_vec());
        assert!(v.iter().find(|i| i.width == 256).unwrap().is_png());
        assert!(!v.iter().find(|i| i.width == 16).unwrap().is_png());
        // The DIB survives the round trip through build_ico / parse_ico.
        let parsed = res::parse_ico(&res::build_ico(&v)).unwrap();
        assert_eq!(parsed.len(), 6);
        assert!(parsed.iter().any(|i| i.width == 16 && i.height == 16));
    }

    #[test]
    fn small_or_non_square_png_is_refused() {
        assert!(from_png(&png(128)).unwrap_err().contains("256"));
        let img = RgbaImage::new(300, 256);
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(img.as_raw(), 300, 256, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert!(from_png(&out).unwrap_err().contains("square"));
    }

    #[test]
    fn ico_keeps_entries_and_fills_gaps() {
        let only256 = res::build_ico(&[encode(&RgbaImage::from_pixel(256, 256, image::Rgba([1, 2, 3, 255])))]);
        let v = from_ico(&only256).unwrap();
        assert_eq!(v.len(), 6);
        let dib = v.iter().find(|i| i.width == 32).unwrap();
        let back = decode(dib).unwrap();
        assert_eq!(back.get_pixel(5, 5).0, [1, 2, 3, 255]);
    }
}
