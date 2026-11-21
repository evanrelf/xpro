//! View transforms: from scene-referred to display-referred.

use crate::{
    color::{REC2020, SRGB, srgb_encode},
    image::{DisplayImage, Image},
};
use rayon::prelude::*;

/// The simplest possible view transform: converts linear Rec.2020 to sRGB and clips anything
/// outside the display's range. There's no highlight roll-off yet, so bright areas clip
/// abruptly.
#[must_use]
pub fn clip_to_srgb(image: &Image) -> DisplayImage {
    let rec2020_to_srgb = SRGB
        .rgb_to_xyz()
        .inverse()
        .expect("sRGB primaries are linearly independent")
        .mul(&REC2020.rgb_to_xyz());
    let pixels = image
        .pixels
        .par_iter()
        .map(|&pixel| {
            rec2020_to_srgb.apply(pixel).map(|linear| {
                let encoded = srgb_encode(linear.clamp(0.0, 1.0));
                (encoded * 255.0).round() as u8
            })
        })
        .collect();
    DisplayImage {
        width: image.width,
        height: image.height,
        pixels,
    }
}
