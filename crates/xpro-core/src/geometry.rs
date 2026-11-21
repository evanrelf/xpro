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

/// Halves an image's width and height (rounding up) by averaging each 2×2 block of pixels.
///
/// The values are linear light, so averaging them mixes light the way a sensor with larger
/// photosites would. Blocks along an odd right or bottom edge average the pixels they have.
#[must_use]
pub fn halve(image: &Image) -> Image {
    let (w, h) = (image.width, image.height);
    let (width, height) = (w.div_ceil(2), h.div_ceil(2));
    let mut pixels = vec![[0.0; 3]; width * height];
    pixels
        .par_chunks_mut(width.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, pixel) in row.iter_mut().enumerate() {
                // Summing in f64 keeps very large values from overflowing.
                let mut sum = [0.0_f64; 3];
                let mut count = 0.0;
                for sy in 2 * y..(2 * y + 2).min(h) {
                    for sx in 2 * x..(2 * x + 2).min(w) {
                        let source = image.pixels[sy * w + sx];
                        sum = std::array::from_fn(|c| sum[c] + f64::from(source[c]));
                        count += 1.0;
                    }
                }
                *pixel = sum.map(|s| (s / count) as f32);
            }
        });
    Image {
        width,
        height,
        pixels,
    }
}

/// Halves an image as many times as needed to fit within the given size, or returns it unchanged
/// if it already fits. Never shrinks an image below 1×1 pixel.
#[must_use]
pub fn shrink_to_fit(mut image: Image, max_width: usize, max_height: usize) -> Image {
    while (image.width > max_width || image.height > max_height)
        && (image.width > 1 || image.height > 1)
    {
        image = halve(&image);
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    use hegel::{TestCase, generators as gs};

    #[hegel::composite]
    fn gen_image(tc: &TestCase, max_side: usize) -> Image {
        let width = tc.draw(gs::integers::<usize>().min_value(1).max_value(max_side));
        let height = tc.draw(gs::integers::<usize>().min_value(1).max_value(max_side));
        // Scene-referred values have no upper limit, and can be negative when out of gamut.
        let values = gs::floats::<f32>().allow_nan(false).allow_infinity(false);
        let channels = width * height * 3;
        let flat = tc.draw(gs::vecs(values).min_size(channels).max_size(channels));
        let pixels = flat.as_chunks::<3>().0.to_vec();
        Image {
            width,
            height,
            pixels,
        }
    }

    #[hegel::test]
    fn halve_rounds_dimensions_up(tc: TestCase) {
        let image = tc.draw(gen_image(9));
        let halved = halve(&image);
        assert_eq!(halved.width, image.width.div_ceil(2));
        assert_eq!(halved.height, image.height.div_ceil(2));
        assert_eq!(halved.pixels.len(), halved.width * halved.height);
    }

    #[hegel::test]
    fn halve_stays_within_each_blocks_range(tc: TestCase) {
        let image = tc.draw(gen_image(9));
        let halved = halve(&image);
        for y in 0..halved.height {
            for x in 0..halved.width {
                for c in 0..3 {
                    let mut block = Vec::new();
                    for sy in 2 * y..(2 * y + 2).min(image.height) {
                        for sx in 2 * x..(2 * x + 2).min(image.width) {
                            block.push(image.pixels[sy * image.width + sx][c]);
                        }
                    }
                    let min = block.iter().copied().fold(f32::INFINITY, f32::min);
                    let max = block.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    let value = halved.pixels[y * halved.width + x][c];
                    assert!(
                        min <= value && value <= max,
                        "{value} is outside {block:?}",
                    );
                }
            }
        }
    }

    #[hegel::test]
    fn halve_preserves_total_light(tc: TestCase) {
        // Each output pixel stands for the input pixels it averaged, so weighting outputs by how
        // many inputs they cover recovers the input's total.
        let image = tc.draw(gen_image(9));
        let halved = halve(&image);
        for c in 0..3 {
            let input: f64 = image.pixels.iter().map(|p| f64::from(p[c])).sum();
            let output: f64 = (0..halved.height)
                .flat_map(|y| (0..halved.width).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let covered_x = (image.width - 2 * x).min(2);
                    let covered_y = (image.height - 2 * y).min(2);
                    let weight = (covered_x * covered_y) as f64;
                    weight * f64::from(halved.pixels[y * halved.width + x][c])
                })
                .sum();
            let magnitude: f64 = image.pixels.iter().map(|p| f64::from(p[c].abs())).sum();
            assert!(
                (input - output).abs() <= 1e-5 * magnitude.max(1.0),
                "{input} != {output}",
            );
        }
    }

    #[hegel::test]
    fn halve_keeps_a_flat_image_flat(tc: TestCase) {
        let image = tc.draw(gen_image(9));
        let flat = Image {
            pixels: vec![image.pixels[0]; image.pixels.len()],
            ..image
        };
        let halved = halve(&flat);
        for pixel in &halved.pixels {
            assert_eq!(*pixel, flat.pixels[0]);
        }
    }

    #[hegel::test]
    fn halve_commutes_with_transposing(tc: TestCase) {
        // Halving treats rows and columns alike, including odd edges.
        let image = tc.draw(gen_image(9));
        let a = halve(&orient(&image, Orientation::Transpose));
        let b = orient(&halve(&image), Orientation::Transpose);
        assert_eq!((a.width, a.height), (b.width, b.height));
        // Summing in a different order can round differently. The rounding error is relative to
        // the values summed, which can be much bigger than their mean when they cancel out.
        let largest = image
            .pixels
            .iter()
            .flatten()
            .fold(0.0_f32, |max, x| max.max(x.abs()));
        let tolerance = 1e-6 * largest;
        for (pa, pb) in a.pixels.iter().zip(&b.pixels) {
            for c in 0..3 {
                assert!((pa[c] - pb[c]).abs() <= tolerance, "{pa:?} != {pb:?}");
            }
        }
    }

    #[hegel::test]
    fn shrink_to_fit_halves_as_little_as_possible(tc: TestCase) {
        let image = tc.draw(gen_image(40));
        let max_width = tc.draw(gs::integers::<usize>().min_value(1).max_value(50));
        let max_height = tc.draw(gs::integers::<usize>().min_value(1).max_value(50));
        let shrunk = shrink_to_fit(image.clone(), max_width, max_height);

        assert!(shrunk.width <= max_width && shrunk.height <= max_height);
        // It's exactly some number of halvings of the input, and one fewer wouldn't fit.
        let mut expected = image;
        let mut previous = None;
        while (expected.width, expected.height) != (shrunk.width, shrunk.height) {
            assert!(expected.width > 1 || expected.height > 1, "not a halving");
            previous = Some((expected.width, expected.height));
            expected = halve(&expected);
        }
        assert_eq!(expected.pixels, shrunk.pixels);
        if let Some((width, height)) = previous {
            assert!(width > max_width || height > max_height);
        }
    }

    #[hegel::test]
    fn shrink_to_fit_stops_at_one_pixel(tc: TestCase) {
        let image = tc.draw(gen_image(40));
        let max_width = tc.draw(gs::integers::<usize>().max_value(50));
        let max_height = tc.draw(gs::integers::<usize>().max_value(50));
        let shrunk = shrink_to_fit(image, max_width, max_height);
        assert!(shrunk.width >= 1 && shrunk.height >= 1);
        assert!(
            (shrunk.width <= max_width && shrunk.height <= max_height)
                || (shrunk.width, shrunk.height) == (1, 1)
        );
    }

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
