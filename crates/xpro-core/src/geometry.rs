//! Geometric transformations.

use crate::image::Image;
use rayon::prelude::*;

/// How a stored image must be transformed to display upright, as in the EXIF Orientation tag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    #[default]
    Normal,
    FlipHorizontal,
    Rotate180,
    FlipVertical,
    /// Flip across the top-left to bottom-right diagonal.
    Transpose,
    /// Rotate 90° clockwise.
    Rotate90,
    /// Flip across the top-right to bottom-left diagonal.
    Transverse,
    /// Rotate 90° counter-clockwise.
    Rotate270,
}

impl Orientation {
    /// Parses an EXIF Orientation tag value. Unknown values are treated as normal.
    #[must_use]
    pub fn from_exif(value: u16) -> Self {
        match value {
            2 => Self::FlipHorizontal,
            3 => Self::Rotate180,
            4 => Self::FlipVertical,
            5 => Self::Transpose,
            6 => Self::Rotate90,
            7 => Self::Transverse,
            8 => Self::Rotate270,
            _ => Self::Normal,
        }
    }

    fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Transpose | Self::Rotate90 | Self::Transverse | Self::Rotate270
        )
    }
}

/// Transforms an image so it displays upright.
#[must_use]
pub fn orient(image: &Image, orientation: Orientation) -> Image {
    let (w, h) = (image.width, image.height);
    let (width, height) = if orientation.swaps_axes() {
        (h, w)
    } else {
        (w, h)
    };
    // For each output pixel, the input pixel it comes from.
    let source = |x: usize, y: usize| match orientation {
        Orientation::Normal => (x, y),
        Orientation::FlipHorizontal => (w - 1 - x, y),
        Orientation::Rotate180 => (w - 1 - x, h - 1 - y),
        Orientation::FlipVertical => (x, h - 1 - y),
        Orientation::Transpose => (y, x),
        Orientation::Rotate90 => (y, h - 1 - x),
        Orientation::Transverse => (w - 1 - y, h - 1 - x),
        Orientation::Rotate270 => (w - 1 - y, x),
    };
    let mut pixels = vec![[0.0; 3]; width * height];
    pixels
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, pixel) in row.iter_mut().enumerate() {
                let (sx, sy) = source(x, y);
                *pixel = image.pixels[sy * w + sx];
            }
        });
    Image {
        width,
        height,
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3×2 image whose pixels are numbered in reading order:
    ///
    /// ```text
    /// 0 1 2
    /// 3 4 5
    /// ```
    fn numbered() -> Image {
        Image {
            width: 3,
            height: 2,
            pixels: (0..6_u8).map(|i| [f32::from(i); 3]).collect(),
        }
    }

    fn numbers(image: &Image) -> Vec<f32> {
        image.pixels.iter().map(|p| p[0]).collect()
    }

    #[test]
    fn rotate_90_is_clockwise() {
        let rotated = orient(&numbered(), Orientation::Rotate90);
        assert_eq!((rotated.width, rotated.height), (2, 3));
        assert_eq!(numbers(&rotated), [3.0, 0.0, 4.0, 1.0, 5.0, 2.0]);
    }

    #[test]
    fn rotate_270_is_counter_clockwise() {
        let rotated = orient(&numbered(), Orientation::Rotate270);
        assert_eq!((rotated.width, rotated.height), (2, 3));
        assert_eq!(numbers(&rotated), [2.0, 5.0, 1.0, 4.0, 0.0, 3.0]);
    }

    #[test]
    fn every_orientation_is_a_permutation() {
        for value in 1..=8 {
            let oriented = orient(&numbered(), Orientation::from_exif(value));
            let mut sorted = numbers(&oriented);
            sorted.sort_by(f32::total_cmp);
            assert_eq!(
                sorted,
                [0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
                "orientation {value}"
            );
        }
    }
}
