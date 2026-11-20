//! Regenerates the two placeholder icons in examples/dummy/icons (run: cargo run -p gloria-pack --example make-dummy-icons).
//! Shapes only, so they are clearly not the GloriaSetup icon: an orange disc and a teal diamond.

use image::{Rgba, RgbaImage};

fn shape(size: u32, bg: [u8; 3], inside: impl Fn(f32, f32) -> bool) -> RgbaImage {
    let ss = 4; // supersampling for smooth edges
    let mut img = RgbaImage::new(size, size);
    for y in 0..size {
        for x in 0..size {
            let mut tile = 0u32;
            let mut mark = 0u32;
            for sy in 0..ss {
                for sx in 0..ss {
                    let u = (x as f32 + (sx as f32 + 0.5) / ss as f32) / size as f32 * 2.0 - 1.0;
                    let v = (y as f32 + (sy as f32 + 0.5) / ss as f32) / size as f32 * 2.0 - 1.0;
                    // Rounded square tile.
                    let r = 0.28;
                    let (qx, qy) = ((u.abs() - (0.94 - r)).max(0.0), (v.abs() - (0.94 - r)).max(0.0));
                    if (qx * qx + qy * qy).sqrt() <= r {
                        tile += 1;
                        if inside(u, v) {
                            mark += 1;
                        }
                    }
                }
            }
            let n = (ss * ss) as f32;
            let a = tile as f32 / n;
            let m = if tile > 0 { mark as f32 / tile as f32 } else { 0.0 };
            let mix = |c: u8| (c as f32 * (1.0 - m) + 255.0 * m) as u8;
            img.put_pixel(x, y, Rgba([mix(bg[0]), mix(bg[1]), mix(bg[2]), (a * 255.0) as u8]));
        }
    }
    img
}

fn write(name: &str, img: RgbaImage) {
    let mut png = Vec::new();
    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8)
        .unwrap();
    let images = gloria_pack::icon::from_png(&png).unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/dummy/icons");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.ico")), gloria_core::res::build_ico(&images)).unwrap();
    std::fs::write(dir.join(format!("{name}.png")), png).unwrap();
    println!("wrote {}", dir.join(name).display());
}

fn main() {
    write("dummy-a", shape(256, [0xe0, 0x6a, 0x2b], |u, v| u * u + v * v <= 0.42 * 0.42));
    write("dummy-b", shape(256, [0x1f, 0x8a, 0x86], |u, v| u.abs() + v.abs() <= 0.52));
}
