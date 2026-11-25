//! In-memory images.

use crate::{color::Mat3, geometry::Orientation};

/// A linear RGB image with `f32` channels.
///
/// The same struct holds both scene-referred images, whose values are proportional to light in
/// the scene, and display-referred images, whose values are display-linear light from 0 (black)
/// to 1 (display white). Which one it is depends on the [`Value`](crate::value::Value) holding
/// it.
#[derive(Clone, Debug)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    /// Row-major pixels.
    pub pixels: Vec<[f32; 3]>,
    /// Which RGB space the pixels are in.
    pub color_space: ColorSpace,
    /// How the image must be transformed to display upright.
    pub orientation: Orientation,
}

impl Image {
    /// An upright Rec.2020 image.
    #[must_use]
    pub fn rec2020(width: usize, height: usize, pixels: Vec<[f32; 3]>) -> Self {
        Self {
            width,
            height,
            pixels,
            color_space: ColorSpace::Rec2020,
            orientation: Orientation::Normal,
        }
    }
}

/// A linear RGB color space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorSpace {
    /// A camera's own RGB, as its sensor measures it.
    Camera {
        /// Converts CIE XYZ (D65) to this camera's RGB.
        xyz_to_camera: Mat3,
    },
    /// Linear ITU-R BT.2020, the default working space.
    Rec2020,
}

/// An image encoded as 8-bit sRGB, ready for a screen or an image file.
#[derive(Clone, Debug)]
pub struct Srgb8Image {
    pub width: usize,
    pub height: usize,
    /// Row-major pixels.
    pub pixels: Vec<[u8; 3]>,
}
