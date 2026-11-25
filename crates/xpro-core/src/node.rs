//! Node types: each a struct of typed parameters, with a function that evaluates it.

use crate::{
    action::Action,
    develop, geometry,
    image::Image,
    raw::{self, Mosaic},
    view::{self, ToneCurve},
};
use anyhow::Context as _;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Reads a raw file into a mosaic.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReadRaw {}

impl ReadRaw {
    pub fn evaluate(&self, path: &Path) -> anyhow::Result<Mosaic> {
        raw::decode(path)
    }
}

/// Scales a mosaic's color channels so that neutral surfaces come out neutral.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WhiteBalance {
    pub multipliers: Multipliers,
}

/// Which white balance multipliers to use.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Multipliers {
    /// The camera's choice, stored in the raw file.
    #[default]
    AsShot,
    /// Multipliers for red, green and blue.
    Custom([f32; 3]),
}

impl WhiteBalance {
    #[must_use]
    pub fn evaluate(&self, mosaic: &Mosaic) -> Mosaic {
        let multipliers = match self.multipliers {
            Multipliers::AsShot => mosaic.as_shot_white_balance,
            Multipliers::Custom(multipliers) => multipliers,
        };
        develop::white_balance(mosaic, multipliers)
    }
}

/// Fills in the colors each photosite didn't measure, turning a mosaic into a scene-referred
/// image in the camera's own RGB.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Demosaic {}

impl Demosaic {
    #[must_use]
    pub fn evaluate(&self, mosaic: &Mosaic) -> Image {
        develop::demosaic(mosaic)
    }
}

/// Converts a scene-referred image from the camera's own RGB to the working space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraToWorkingSpace {}

impl CameraToWorkingSpace {
    pub fn evaluate(&self, image: &Image) -> anyhow::Result<Image> {
        develop::camera_to_rec2020(image)
    }
}

/// Rotates and flips an image so it displays upright, as the camera recorded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Orient {}

impl Orient {
    #[must_use]
    pub fn evaluate(&self, image: &Image) -> Image {
        geometry::upright(image)
    }
}

/// Brightens or darkens a scene-referred image.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exposure {
    /// How much to brighten, in stops. Each stop doubles the light. Negative values darken.
    pub stops: f32,
}

impl Exposure {
    #[must_use]
    pub fn evaluate(&self, image: &Image) -> Image {
        develop::exposure(image, self.stops)
    }
}

/// Turns a scene-referred image into a display-referred one, rolling off the highlights.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewTransform {
    /// How steep the tone curve is. Higher values darken shadows and brighten highlights more.
    pub contrast: f32,
    /// How many stops above middle gray becomes display white.
    pub white: f32,
}

impl Default for ViewTransform {
    fn default() -> Self {
        Self {
            contrast: 1.5,
            white: 4.0,
        }
    }
}

impl ViewTransform {
    pub fn evaluate(&self, image: &Image) -> anyhow::Result<Image> {
        let curve = ToneCurve::new(self.contrast, self.white).with_context(|| {
            format!(
                "A contrast of {} can't reach white {} stops above middle gray",
                self.contrast, self.white,
            )
        })?;
        Ok(view::view_transform(image, &curve))
    }
}

/// An output node that writes a display-referred image to a PNG file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WritePng {
    pub path: PathBuf,
}

impl WritePng {
    #[must_use]
    pub fn evaluate(&self, image: Arc<Image>) -> Action {
        Action::WritePng {
            image,
            path: self.path.clone(),
        }
    }
}

/// How much the default graph's exposure brightens a developed image, in stops, so that a
/// subject the camera metered as correctly exposed lands near
/// [`MIDDLE_GRAY`](crate::view::MIDDLE_GRAY).
///
/// Cameras expose with headroom for highlights, so raw values for a correctly exposed subject
/// sit well below the sensor's white level. How much headroom varies by camera and settings.
/// This value matches the overall brightness of the camera's own JPEGs for a set of Nikon Z f
/// raw files, which were shot with Active D-Lighting (which underexposes to protect highlights).
/// Other cameras may want less.
///
/// The sensor's white level ends up 4 stops above middle gray, where the default
/// [`ViewTransform`] reaches white, so highlights roll off all the way to the sensor's limit and
/// no further.
pub const BASELINE_EXPOSURE: f32 = 1.5;

/// Develops a raw file into an upright, scene-referred Rec.2020 image, with its as-shot white
/// balance and [`BASELINE_EXPOSURE`].
pub fn develop(path: &Path) -> anyhow::Result<Image> {
    let mosaic = ReadRaw::default().evaluate(path)?;
    let mosaic = WhiteBalance::default().evaluate(&mosaic);
    let image = Demosaic::default().evaluate(&mosaic);
    let image = CameraToWorkingSpace::default().evaluate(&image)?;
    let image = Orient::default().evaluate(&image);
    let exposure = Exposure {
        stops: BASELINE_EXPOSURE,
    };
    Ok(exposure.evaluate(&image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_view_transform_turns_the_sensors_white_level_white() {
        // A raw file's white level is 1.0 before the baseline exposure brightens it. If the
        // curve reached white sooner, it would clip highlights the sensor captured. If later,
        // the brightest highlights would look gray.
        let white_level = Image::rec2020(1, 1, vec![[BASELINE_EXPOSURE.exp2(); 3]]);
        let display = ViewTransform::default().evaluate(&white_level).unwrap();
        for c in display.pixels[0] {
            assert!((c - 1.0).abs() < 0.01, "{c}");
        }
    }
}
