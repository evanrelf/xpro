//! Raw development: turning sensor data into a scene-referred image.

use crate::{
    color::{Mat3, REC2020},
    image::{ColorSpace, Image},
    raw::Mosaic,
};
use anyhow::{Context as _, bail};
use rayon::prelude::*;

/// Brightens an image by `stops` (or darkens it, if negative). Each stop doubles the light.
#[must_use]
pub fn exposure(image: &Image, stops: f32) -> Image {
    let gain = stops.exp2();
    let pixels = image
        .pixels
        .par_iter()
        .map(|pixel| pixel.map(|x| x * gain))
        .collect();
    Image { pixels, ..*image }
}

/// Multiplies each photosite by its channel's multiplier, then clips to 1.0.
///
/// Clipping keeps blown highlights neutral. Without it, a channel with a multiplier above 1.0
/// would push past the clipped channels and tint the highlights (usually magenta). The cost is
/// discarding highlight detail that only some channels captured.
#[must_use]
pub fn white_balance(mosaic: &Mosaic, multipliers: [f32; 3]) -> Mosaic {
    let mut values = mosaic.values.clone();
    values
        .par_chunks_mut(mosaic.width)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let multiplier = multipliers[mosaic.channel_at(x, y) as usize];
                *value = (*value * multiplier).min(1.0);
            }
        });
    Mosaic { values, ..*mosaic }
}

/// Bilinear demosaicing: each missing channel is the average of the nearest photosites that
/// measured it. The result is in the camera's own RGB.
#[must_use]
pub fn demosaic(mosaic: &Mosaic) -> Image {
    let Mosaic {
        width,
        height,
        ref values,
        ..
    } = *mosaic;
    let mut pixels = vec![[0.0; 3]; width * height];
    pixels
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, pixel) in row.iter_mut().enumerate() {
                // In a Bayer pattern, averaging each channel over the 3×3 neighborhood is
                // exactly bilinear interpolation.
                let mut sums = [0.0; 3];
                let mut counts = [0.0_f32; 3];
                for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                    for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                        let channel = mosaic.channel_at(nx, ny) as usize;
                        sums[channel] += values[ny * width + nx];
                        counts[channel] += 1.0;
                    }
                }
                *pixel = std::array::from_fn(|c| sums[c] / counts[c].max(1.0));
                pixel[mosaic.channel_at(x, y) as usize] = values[y * width + x];
            }
        });
    Image {
        width,
        height,
        pixels,
        color_space: ColorSpace::Camera {
            xyz_to_camera: mosaic.xyz_to_camera,
        },
        orientation: mosaic.orientation,
    }
}

/// Develops a half-size image straight from a mosaic, much faster than demosaicing: each 2×2
/// block of photosites, one red, two green and one blue, becomes one pixel. The size is rounded
/// up, so it matches [`halve`](crate::geometry::halve) of a full-size image. Along an odd right
/// or bottom edge, the last block overlaps the one before it. The result is in the camera's own
/// RGB.
#[must_use]
pub fn demosaic_half(mosaic: &Mosaic) -> Image {
    let Mosaic {
        width: w,
        height: h,
        ref values,
        ..
    } = *mosaic;
    let (width, height) = (w.div_ceil(2), h.div_ceil(2));
    let mut pixels = vec![[0.0; 3]; width * height];
    pixels
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            // A mosaic is at least 2×2, so every block has all three channels.
            let top = (2 * y).min(h - 2);
            for (x, pixel) in row.iter_mut().enumerate() {
                let left = (2 * x).min(w - 2);
                let mut sums = [0.0; 3];
                let mut counts = [0.0_f32; 3];
                for sy in top..top + 2 {
                    for sx in left..left + 2 {
                        let channel = mosaic.channel_at(sx, sy) as usize;
                        sums[channel] += values[sy * w + sx];
                        counts[channel] += 1.0;
                    }
                }
                *pixel = std::array::from_fn(|c| sums[c] / counts[c]);
            }
        });
    Image {
        width,
        height,
        pixels,
        color_space: ColorSpace::Camera {
            xyz_to_camera: mosaic.xyz_to_camera,
        },
        orientation: mosaic.orientation,
    }
}

/// Converts white-balanced camera RGB to linear Rec.2020.
pub fn camera_to_rec2020(image: &Image) -> anyhow::Result<Image> {
    let ColorSpace::Camera { xyz_to_camera } = image.color_space else {
        bail!(
            "Expected an image in a camera's RGB, not {:?}",
            image.color_space
        );
    };
    let rec2020_to_camera = xyz_to_camera.mul(&REC2020.rgb_to_xyz());
    // After white balancing, a neutral surface is (1, 1, 1) in camera RGB. Scale each row so
    // that Rec.2020 white maps to exactly that, so neutrals stay neutral.
    let normalized = Mat3(rec2020_to_camera.0.map(|row| {
        let sum: f32 = row.iter().sum();
        row.map(|x| x / sum)
    }));
    let camera_to_rec2020 = normalized
        .inverse()
        .context("Camera color matrix is singular")?;
    let pixels = image
        .pixels
        .par_iter()
        .map(|&pixel| camera_to_rec2020.apply(pixel))
        .collect();
    Ok(Image {
        pixels,
        color_space: ColorSpace::Rec2020,
        ..*image
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hegel::{TestCase, generators as gs};

    #[hegel::composite]
    fn gen_pixel(tc: &TestCase) -> Image {
        let value = || gs::floats::<f32>().min_value(-1e3).max_value(1e3);
        Image::rec2020(
            1,
            1,
            vec![[tc.draw(value()), tc.draw(value()), tc.draw(value())]],
        )
    }

    #[hegel::composite]
    fn gen_stops(tc: &TestCase) -> f32 {
        tc.draw(gs::floats::<f32>().min_value(-10.0).max_value(10.0))
    }

    fn close(a: f32, b: f32) -> bool {
        // Subnormal values, below `MIN_POSITIVE`, have too little precision for a relative
        // comparison.
        (a - b).abs() <= 1e-5 * a.abs().max(b.abs()) + f32::MIN_POSITIVE
    }

    #[hegel::test(test_cases = 10_000)]
    fn zero_stops_changes_nothing(tc: TestCase) {
        let image = tc.draw(gen_pixel());
        assert_eq!(exposure(&image, 0.0).pixels, image.pixels);
    }

    #[hegel::test(test_cases = 10_000)]
    fn each_stop_doubles_the_light(tc: TestCase) {
        let image = tc.draw(gen_pixel());
        let stops = tc.draw(gen_stops());
        let once = exposure(&image, stops).pixels[0];
        let twice = exposure(&image, stops + 1.0).pixels[0];
        for c in 0..3 {
            assert!(close(twice[c], 2.0 * once[c]), "{once:?} → {twice:?}");
        }
    }

    #[hegel::test(test_cases = 10_000)]
    fn exposures_add_up(tc: TestCase) {
        let image = tc.draw(gen_pixel());
        let (a, b) = (tc.draw(gen_stops()), tc.draw(gen_stops()));
        let separately = exposure(&exposure(&image, a), b).pixels[0];
        let together = exposure(&image, a + b).pixels[0];
        for c in 0..3 {
            assert!(
                close(separately[c], together[c]),
                "{separately:?} vs {together:?}"
            );
        }
    }

    /// A mosaic of a scene whose color is constant within each 2×2 block, so each block's
    /// color is known exactly.
    #[hegel::composite]
    fn gen_blocky_mosaic(tc: &TestCase) -> (Mosaic, Vec<[f32; 3]>) {
        let base = tc.draw(crate::testing::gen_mosaic());
        // Even sizes, so the blocks don't overlap.
        let (width, height) = (base.width / 2 * 2, base.height / 2 * 2);
        let value = || gs::floats::<f32>().min_value(0.0).max_value(1.0);
        let count = (width / 2) * (height / 2);
        let blocks: Vec<[f32; 3]> = (0..count)
            .map(|_| [tc.draw(value()), tc.draw(value()), tc.draw(value())])
            .collect();
        let mut values = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let block = blocks[(y / 2) * (width / 2) + x / 2];
                values.push(block[base.cfa[y % 2][x % 2] as usize]);
            }
        }
        let mosaic = Mosaic {
            width,
            height,
            values,
            ..base
        };
        (mosaic, blocks)
    }

    #[hegel::test(test_cases = 4_000)]
    fn half_size_demosaicing_recovers_each_blocks_color(tc: TestCase) {
        let (mosaic, blocks) = tc.draw(gen_blocky_mosaic());
        let image = demosaic_half(&mosaic);
        assert_eq!(
            (image.width, image.height),
            (mosaic.width / 2, mosaic.height / 2)
        );
        for (pixel, block) in image.pixels.iter().zip(&blocks) {
            for c in 0..3 {
                // Two greens are averaged, which can round.
                assert!(
                    (pixel[c] - block[c]).abs() <= 1e-6,
                    "{pixel:?} != {block:?}"
                );
            }
        }
    }

    #[hegel::test(test_cases = 4_000)]
    fn half_size_demosaicing_rounds_the_size_up(tc: TestCase) {
        let mosaic = tc.draw(crate::testing::gen_mosaic());
        let image = demosaic_half(&mosaic);
        assert_eq!(
            (image.width, image.height),
            (mosaic.width.div_ceil(2), mosaic.height.div_ceil(2))
        );
        assert_eq!(image.pixels.len(), image.width * image.height);
    }
}
