use image::{Rgba, RgbaImage};

/// 26.3's `dark_cutout` path for foliage: dark transparent texels, average
/// nonempty colors in linear light, then scale alpha to retain cutout coverage.
pub fn dark_cutout(mut base: RgbaImage, levels: usize) -> Vec<RgbaImage> {
    // TextureUtil.fillEmptyAreasWithDarkColor scans X before Y and keeps the
    // first minimum. Image::pixels scans Y before X, which changes the edge
    // color when two opaque foliage texels have equal brightness.
    let mut darkest = Rgba([0, 0, 0, 255]);
    let mut min_brightness = u16::MAX;
    for x in 0..base.width() {
        for y in 0..base.height() {
            let pixel = *base.get_pixel(x, y);
            if pixel[3] != 0 {
                let brightness = pixel[0] as u16 + pixel[1] as u16 + pixel[2] as u16;
                if brightness < min_brightness {
                    min_brightness = brightness;
                    darkest = pixel;
                }
            }
        }
    }
    let dark = Rgba([
        (darkest[0] as u16 * 3 / 4) as u8,
        (darkest[1] as u16 * 3 / 4) as u8,
        (darkest[2] as u16 * 3 / 4) as u8,
        0,
    ]);
    for pixel in base.pixels_mut() {
        if pixel[3] == 0 {
            *pixel = dark;
        }
    }
    let desired = coverage(&base, 0.5, 1.0);
    let mut result = vec![base];
    for _ in 1..levels {
        let previous = result.last().unwrap();
        let (width, height) = (previous.width() / 2, previous.height() / 2);
        let mut next = RgbaImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let pixels = [
                    previous.get_pixel(x * 2, y * 2),
                    previous.get_pixel(x * 2 + 1, y * 2),
                    previous.get_pixel(x * 2, y * 2 + 1),
                    previous.get_pixel(x * 2 + 1, y * 2 + 1),
                ];
                let mut channels = [0u8; 4];
                for channel in 0..4 {
                    let linear = pixels
                        .iter()
                        .filter(|p| p[3] != 0)
                        .map(|p| srgb_to_linear(p[channel]))
                        .sum::<f32>()
                        / 4.0;
                    channels[channel] = linear_to_srgb(linear);
                }
                *next.get_pixel_mut(x, y) = Rgba(channels);
            }
        }
        scale_alpha_to_coverage(&mut next, desired);
        result.push(next);
    }
    result
}

fn srgb_to_linear(value: u8) -> f32 {
    let x = value as f32 / 255.0;
    if x >= 0.04045 {
        ((x + 0.055) / 1.055).powf(2.4)
    } else {
        x / 12.92
    }
}
fn linear_to_srgb(value: f32) -> u8 {
    let x = if value >= 0.0031308 {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    } else {
        12.92 * value
    };
    (x * 255.0).clamp(0.0, 255.0) as u8
}
fn coverage(image: &RgbaImage, cutoff: f32, scale: f32) -> f32 {
    if image.width() < 2 || image.height() < 2 {
        return 0.0;
    }
    let mut covered = 0.0;
    for y in 0..image.height() - 1 {
        for x in 0..image.width() - 1 {
            let alpha = [
                image.get_pixel(x, y)[3],
                image.get_pixel(x + 1, y)[3],
                image.get_pixel(x, y + 1)[3],
                image.get_pixel(x + 1, y + 1)[3],
            ]
            .map(|a| (a as f32 / 255.0 * scale).clamp(0.0, 1.0));
            for sy in 0..4 {
                for sx in 0..4 {
                    let fx = (sx as f32 + 0.5) / 4.0;
                    let fy = (sy as f32 + 0.5) / 4.0;
                    let a = alpha[0] * (1.0 - fx) * (1.0 - fy)
                        + alpha[1] * fx * (1.0 - fy)
                        + alpha[2] * (1.0 - fx) * fy
                        + alpha[3] * fx * fy;
                    if a > cutoff {
                        covered += 1.0 / 16.0;
                    }
                }
            }
        }
    }
    covered / ((image.width() - 1) * (image.height() - 1)) as f32
}
fn scale_alpha_to_coverage(image: &mut RgbaImage, desired: f32) {
    let (mut low, mut high) = (0.0, 4.0);
    let (mut scale, mut best_scale, mut best_error) = (1.0, 1.0, f32::MAX);
    for _ in 0..5 {
        let current = coverage(image, 0.5, scale);
        let error = (current - desired).abs();
        if error < best_error {
            best_error = error;
            best_scale = scale;
        }
        if current < desired {
            low = scale;
        } else if current > desired {
            high = scale;
        } else {
            break;
        }
        scale = (low + high) * 0.5;
    }
    for pixel in image.pixels_mut() {
        pixel[3] = ((pixel[3] as f32 / 255.0 * best_scale + 0.025).clamp(0.0, 1.0) * 255.0) as u8;
    }
}
