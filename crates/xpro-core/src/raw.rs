//! Decoding camera raw files.

use crate::{color::Mat3, geometry::Orientation};
use anyhow::{Context as _, bail, ensure};
use rawler::{
    cfa::CFAColor, decoders::RawDecodeParams, imgop::xyz::Illuminant,
    rawimage::RawPhotometricInterpretation, rawsource::RawSource,
};
use std::path::Path;

/// A color channel measured by a photosite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Red = 0,
    Green = 1,
    Blue = 2,
}

/// The single-channel data from a Bayer sensor, before demosaicing, with the metadata needed to
/// develop it.
#[derive(Clone, Debug)]
pub struct Mosaic {
    pub width: usize,
    pub height: usize,
    /// The color filter array: which channel each photosite measures, repeating every 2×2
    /// photosites. Indexed as `cfa[y % 2][x % 2]`.
    pub cfa: [[Channel; 2]; 2],
    /// Row-major photosite values. 0.0 is black and 1.0 is the sensor's white level.
    pub values: Vec<f32>,
    /// White balance multipliers for red, green and blue as chosen by the camera, with green at
    /// 1.0.
    pub as_shot_white_balance: [f32; 3],
    /// Converts CIE XYZ (D65) to the camera's native RGB.
    pub xyz_to_camera: Mat3,
    /// How the sensor data must be transformed to display upright.
    pub orientation: Orientation,
}

impl Mosaic {
    #[must_use]
    pub fn channel_at(&self, x: usize, y: usize) -> Channel {
        self.cfa[y % 2][x % 2]
    }
}

/// Decodes a raw file from a camera with a Bayer sensor. The mosaic is cropped to the camera's
/// recommended image area.
pub fn decode(path: &Path) -> anyhow::Result<Mosaic> {
    let context = || format!("Failed to decode raw file {}", path.display());
    let source = RawSource::new(path).with_context(context)?;
    let params = RawDecodeParams::default();
    let raw = rawler::decode(&source, &params).with_context(context)?;
    // Not every decoder reports orientation in the raw image (NEF doesn't), so prefer EXIF.
    let orientation = rawler::get_decoder(&source)
        .and_then(|decoder| decoder.raw_metadata(&source, &params))
        .ok()
        .and_then(|metadata| metadata.exif.orientation)
        .unwrap_or_else(|| raw.orientation.to_u16());

    let RawPhotometricInterpretation::Cfa(config) = &raw.photometric else {
        bail!("Only Bayer sensors are supported");
    };
    let cfa = &config.cfa;
    ensure!(
        raw.cpp == 1 && cfa.width == 2 && cfa.height == 2 && cfa.is_rgb(),
        "Only Bayer sensors are supported, not {}",
        cfa.name,
    );
    let channel = |x: usize, y: usize| match cfa.cfa_color_at(y, x) {
        CFAColor::RED => Ok(Channel::Red),
        CFAColor::GREEN => Ok(Channel::Green),
        CFAColor::BLUE => Ok(Channel::Blue),
        color => bail!("Unsupported color filter {color:?}"),
    };

    let crop = raw.crop_area.or(raw.active_area);
    let (left, top, width, height) = match crop {
        Some(rect) => (rect.p.x, rect.p.y, rect.d.w, rect.d.h),
        None => (0, 0, raw.width, raw.height),
    };
    ensure!(
        left + width <= raw.width && top + height <= raw.height && width >= 2 && height >= 2,
        "Invalid crop area {crop:?}",
    );

    let cfa = [
        [channel(left, top)?, channel(left + 1, top)?],
        [channel(left, top + 1)?, channel(left + 1, top + 1)?],
    ];

    let black = raw.blacklevel.as_vec();
    let black_at = |x: usize, y: usize| {
        let (w, h) = (raw.blacklevel.width.max(1), raw.blacklevel.height.max(1));
        black.get((y % h) * w + x % w).copied().unwrap_or(black[0])
    };
    // White levels are either one for all channels, or one per channel.
    let white = raw.whitelevel.as_vec();
    let white_for = |channel: Channel| white.get(channel as usize).copied().unwrap_or(white[0]);

    let data = raw.data.as_f32();
    let mut values = Vec::with_capacity(width * height);
    for y in top..top + height {
        for x in left..left + width {
            let black = black_at(x, y);
            let white = white_for(cfa[(y - top) % 2][(x - left) % 2]);
            values.push((data[y * raw.width + x] - black) / (white - black));
        }
    }

    let [r, g, b, _] = raw.wb_coeffs;
    let as_shot_white_balance = if [r, g, b].iter().all(|c| c.is_finite() && *c > 0.0) {
        [r / g, 1.0, b / g]
    } else {
        let [r, g, b, _] = raw.neutralwb();
        [r / g, 1.0, b / g]
    };

    let (_, matrix) = raw
        .color_matrix_find_first([Illuminant::D65])
        .or_else(|| raw.color_matrix.iter().next().map(|(i, m)| (*i, m.clone())))
        .context("Camera has no color matrix")?;
    ensure!(matrix.len() >= 9, "Color matrix is not 3×3");
    let xyz_to_camera = Mat3(std::array::from_fn(|i| {
        std::array::from_fn(|j| matrix[i * 3 + j])
    }));

    Ok(Mosaic {
        width,
        height,
        cfa,
        values,
        as_shot_white_balance,
        xyz_to_camera,
        orientation: Orientation::from_exif(orientation),
    })
}
