//! View transforms: from scene-referred to display-referred.

use crate::{
    color::{REC2020, SRGB, srgb_encode},
    image::{ColorSpace, Image, Srgb8Image},
};
use anyhow::ensure;
use rayon::prelude::*;

/// The scene-linear value of middle gray, an 18% reflectance surface. Tone curves keep it
/// where it is, so a well-exposed image keeps its overall brightness.
pub const MIDDLE_GRAY: f32 = 0.18;

/// A tone curve, from scene-linear light (unbounded) to display-linear light (0 to 1).
///
/// It's an S-shaped curve when plotted on a log scale: black stays black, middle gray stays
/// middle gray, and a chosen white point becomes display white. Shadows and highlights are
/// compressed smoothly on the way there, and anything brighter than the white point clips.
///
/// The curve is a log-logistic function, `m / (1 + (k / x)^contrast)`, as used by darktable's
/// sigmoid module.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneCurve {
    contrast: f32,
    white: f32,
    /// The curve's asymptote, a little above 1, so that it reaches 1 at the white point.
    m: f32,
    /// The scene value at which the curve is halfway to `m`.
    k: f32,
}

impl ToneCurve {
    /// Creates a tone curve.
    ///
    /// - `contrast` is how steep the curve is. Higher values darken shadows and brighten
    ///   highlights more.
    /// - `white` is how many stops above middle gray becomes display white.
    ///
    /// Returns `None` unless both are positive and finite, and `contrast × white` is more than
    /// `log2(1 / MIDDLE_GRAY)` (about 2.47). Below that, a curve this shallow can't climb from
    /// middle gray to white in so few stops. Also returns `None` for curves too extreme to
    /// compute, such as a contrast near zero with a white point hundreds of stops up.
    #[must_use]
    pub fn new(contrast: f32, white: f32) -> Option<Self> {
        let (n, g) = (f64::from(contrast), f64::from(MIDDLE_GRAY));
        if !(n.is_finite() && n > 0.0 && white.is_finite() && white > 0.0) {
            return None;
        }
        // Solving m / (1 + (k / x)^n) = x at middle gray g, and = 1 at the white point gives
        //
        //   m = 1 + (1 - g) t / (g - t)
        //   k = g ((1 - g) / (g - t))^(1/n)
        //
        // where t = 2^(-n × white). Written in terms of t, which shrinks toward 0 rather than
        // overflowing, it works for any white point.
        let t = (-n * f64::from(white)).exp2();
        let denominator = g - t;
        if denominator <= 0.0 {
            return None;
        }
        let m = (1.0 + (1.0 - g) * t / denominator) as f32;
        let k = (g * ((1.0 - g) / denominator).powf(1.0 / n)) as f32;
        if !(m.is_finite() && k.is_finite() && k > 0.0) {
            return None;
        }
        Some(Self {
            contrast,
            white,
            m,
            k,
        })
    }

    /// Maps a scene-linear value to display-linear. Zero, negative and NaN values become 0.
    /// Values brighter than the white point come out above 1, for the caller to clip.
    #[must_use]
    pub fn apply(&self, x: f32) -> f32 {
        if x.is_nan() || x <= 0.0 {
            return 0.0;
        }
        // Written this way, huge inputs approach `m` rather than overflowing to inf / inf.
        self.m / (1.0 + (self.k / x).powf(self.contrast))
    }
}

/// Turns a scene-referred image into a display-referred one by applying `curve` to each channel.
/// The color space stays the same.
///
/// Applying the curve to each channel separately compresses bright, saturated colors toward
/// white, the way film and camera JPEGs do. How much depends on the color space's primaries, so
/// the result looks right only in the working space it was tuned for, Rec.2020.
#[must_use]
pub fn view_transform(image: &Image, curve: &ToneCurve) -> Image {
    let pixels = image
        .pixels
        .par_iter()
        .map(|pixel| pixel.map(|x| curve.apply(x)))
        .collect();
    Image { pixels, ..*image }
}

/// Encodes a display-referred Rec.2020 image as 8-bit sRGB. Colors outside sRGB's gamut, and
/// values outside the display's range, are clipped.
pub fn encode_srgb8(image: &Image) -> anyhow::Result<Srgb8Image> {
    ensure!(
        image.color_space == ColorSpace::Rec2020,
        "Expected a Rec.2020 image, not {:?}",
        image.color_space,
    );
    let rec2020_to_srgb = SRGB
        .rgb_to_xyz()
        .inverse()
        .expect("sRGB primaries are linearly independent")
        .mul(&REC2020.rgb_to_xyz());
    let pixels = image
        .pixels
        .par_iter()
        .map(|&pixel| rec2020_to_srgb.apply(pixel).map(encode_8_bit))
        .collect();
    Ok(Srgb8Image {
        width: image.width,
        height: image.height,
        pixels,
    })
}

/// Clips a display-linear value to [0, 1], then encodes it as an 8-bit sRGB value.
fn encode_8_bit(linear: f32) -> u8 {
    (srgb_encode(linear.clamp(0.0, 1.0)) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use hegel::{TestCase, generators as gs};

    #[hegel::composite]
    fn gen_curve(tc: &TestCase) -> ToneCurve {
        let contrast = tc.draw(gs::floats::<f32>().min_value(0.5).max_value(4.0));
        let min_white = 2.5 / contrast;
        let white = tc.draw(gs::floats::<f32>().min_value(min_white).max_value(12.0));
        tc.assume(ToneCurve::new(contrast, white).is_some());
        ToneCurve::new(contrast, white).unwrap()
    }

    /// Any scene value a pixel might hold, from far below black to far above white.
    #[hegel::composite]
    fn gen_scene_value(tc: &TestCase) -> f32 {
        tc.draw(gs::floats::<f32>().min_value(-1e6).max_value(1e30))
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-4 * a.abs().max(b.abs()).max(1.0)
    }

    #[hegel::test(test_cases = 10_000)]
    fn the_curve_keeps_black_and_middle_gray(tc: TestCase) {
        let curve = tc.draw(gen_curve());
        assert_eq!(curve.apply(0.0), 0.0);
        let gray = curve.apply(MIDDLE_GRAY);
        assert!(close(gray, MIDDLE_GRAY), "{gray}");
    }

    #[hegel::test(test_cases = 10_000)]
    fn the_curve_turns_the_white_point_white(tc: TestCase) {
        let curve = tc.draw(gen_curve());
        let white = curve.apply(MIDDLE_GRAY * curve.white.exp2());
        assert!(close(white, 1.0), "{white}");
    }

    #[hegel::test(test_cases = 10_000)]
    fn the_curve_never_gets_darker_as_light_increases(tc: TestCase) {
        let curve = tc.draw(gen_curve());
        let a = tc.draw(gen_scene_value());
        let b = tc.draw(gen_scene_value());
        let (low, high) = if a <= b { (a, b) } else { (b, a) };
        assert!(
            curve.apply(low) <= curve.apply(high),
            "f({low}) > f({high})"
        );
    }

    #[hegel::test(test_cases = 10_000)]
    fn the_curve_is_bounded_for_any_input(tc: TestCase) {
        let curve = tc.draw(gen_curve());
        let x = tc.draw(gs::floats::<f32>());
        let y = curve.apply(x);
        // Including NaN and infinities, which a broken upstream node could produce.
        assert!((0.0..=curve.m).contains(&y), "f({x}) = {y}");
    }

    #[hegel::test(test_cases = 10_000)]
    fn new_rejects_curves_that_cant_reach_white(tc: TestCase) {
        let contrast = tc.draw(gs::floats::<f32>());
        let white = tc.draw(gs::floats::<f32>());
        // Every sensible curve is accepted.
        let sensible = (0.1..=1e3).contains(&contrast)
            && (0.0..=64.0).contains(&white)
            && contrast * white > (1.0 / MIDDLE_GRAY).log2() + 1e-3;
        if sensible {
            assert!(
                ToneCurve::new(contrast, white).is_some(),
                "{contrast}, {white}"
            );
        }
        if let Some(curve) = ToneCurve::new(contrast, white) {
            assert!(curve.m.is_finite() && curve.m >= 1.0, "{curve:?}");
            assert!(curve.k.is_finite() && curve.k > 0.0, "{curve:?}");
        }
    }

    #[hegel::test(test_cases = 8_000)]
    fn neutral_colors_stay_neutral(tc: TestCase) {
        let curve = tc.draw(gen_curve());
        let value = tc.draw(gen_scene_value());
        let image = Image::rec2020(1, 1, vec![[value; 3]]);
        let display = view_transform(&image, &curve);
        let [r, g, b] = encode_srgb8(&display).unwrap().pixels[0];
        // Rec.2020 and sRGB share a white point, so neutrals only differ by rounding.
        assert!(
            r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1,
            "{value} → {r}, {g}, {b}"
        );
        assert_eq!(g, encode_8_bit(curve.apply(value)), "{value}");
    }
}
