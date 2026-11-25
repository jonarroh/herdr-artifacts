//! Writes `examples/sample.png`, the image used by the demo script.
//!
//! Run with: `cargo run --example gen_sample`

use image::{Rgb, RgbImage};

fn main() {
    let (w, h) = (320u32, 200u32);
    let img = RgbImage::from_fn(w, h, |x, y| {
        // Colour gradient with a bright disc and a darker frame, so both
        // real-image protocols and the half-block fallback are easy to judge.
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        let (dx, dy) = (x as f32 - w as f32 / 2.0, y as f32 - h as f32 / 2.0);
        if x < 6 || y < 6 || x >= w - 6 || y >= h - 6 {
            Rgb([30, 30, 46])
        } else if (dx * dx + dy * dy).sqrt() < 60.0 {
            Rgb([250, 220, 90])
        } else {
            Rgb([(40.0 + 200.0 * fx) as u8, (60.0 + 120.0 * fy) as u8, (220.0 - 160.0 * fx) as u8])
        }
    });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join("sample.png");
    img.save(&path).expect("write sample.png");
    println!("wrote {}", path.display());
}
