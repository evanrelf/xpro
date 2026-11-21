//! Raw development: turning sensor data into a scene-referred image.

use crate::{
    color::{Mat3, REC2020},
    geometry::orient,
    image::Image,
    raw::{Mosaic, RawPhoto},
};
use anyhow::Context as _;
use rayon::prelude::*;

/// Develops a raw photo with its as-shot white balance into upright, linear Rec.2020.
pub fn develop(raw: &RawPhoto) -> anyhow::Result<Image> {
    let mosaic = white_balance(&raw.mosaic, raw.as_shot_white_balance);
    let camera_rgb = demosaic(&mosaic);
    let rec2020 = camera_to_rec2020(&camera_rgb, &raw.xyz_to_camera)?;
    Ok(orient(&rec2020, raw.orientation))
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
    Mosaic {
        values,
        ..mosaic.clone()
    }
}

/// Bilinear demosaicing: each missing channel is the average of the nearest photosites that
/// measured it.
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
    }
}

/// Converts white-balanced camera RGB to linear Rec.2020.
pub fn camera_to_rec2020(image: &Image, xyz_to_camera: &Mat3) -> anyhow::Result<Image> {
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
    Ok(Image { pixels, ..*image })
}
