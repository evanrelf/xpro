//! Color math: matrices, RGB color spaces, and transfer functions.

/// A 3×3 matrix, stored row-major.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3(pub [[f32; 3]; 3]);

impl Mat3 {
    /// Matrix product `self × rhs`. Applying the result is the same as applying `rhs`, then
    /// `self`.
    #[must_use]
    pub fn mul(&self, rhs: &Self) -> Self {
        let a = &self.0;
        let b = &rhs.0;
        Self(std::array::from_fn(|i| {
            std::array::from_fn(|j| a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j])
        }))
    }

    /// Multiplies a column vector by this matrix.
    #[must_use]
    pub fn apply(&self, v: [f32; 3]) -> [f32; 3] {
        let m = &self.0;
        std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
    }

    /// The inverse matrix, or `None` if this matrix is singular.
    #[must_use]
    pub fn inverse(&self) -> Option<Self> {
        let m = &self.0;
        // Cofactors, transposed (the adjugate).
        let adj = [
            [
                m[1][1] * m[2][2] - m[1][2] * m[2][1],
                m[0][2] * m[2][1] - m[0][1] * m[2][2],
                m[0][1] * m[1][2] - m[0][2] * m[1][1],
            ],
            [
                m[1][2] * m[2][0] - m[1][0] * m[2][2],
                m[0][0] * m[2][2] - m[0][2] * m[2][0],
                m[0][2] * m[1][0] - m[0][0] * m[1][2],
            ],
            [
                m[1][0] * m[2][1] - m[1][1] * m[2][0],
                m[0][1] * m[2][0] - m[0][0] * m[2][1],
                m[0][0] * m[1][1] - m[0][1] * m[1][0],
            ],
        ];
        let det = m[0][0] * adj[0][0] + m[0][1] * adj[1][0] + m[0][2] * adj[2][0];
        if det.abs() < f32::EPSILON {
            return None;
        }
        Some(Self(adj.map(|row| row.map(|x| x / det))))
    }
}

/// An RGB color space's primaries and white point, as CIE xy chromaticities.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primaries {
    pub red: [f32; 2],
    pub green: [f32; 2],
    pub blue: [f32; 2],
    pub white: [f32; 2],
}

/// CIE standard illuminant D65.
const D65: [f32; 2] = [0.3127, 0.3290];

/// ITU-R BT.2020.
pub const REC2020: Primaries = Primaries {
    red: [0.708, 0.292],
    green: [0.170, 0.797],
    blue: [0.131, 0.046],
    white: D65,
};

/// sRGB (ITU-R BT.709).
pub const SRGB: Primaries = Primaries {
    red: [0.64, 0.33],
    green: [0.30, 0.60],
    blue: [0.15, 0.06],
    white: D65,
};

impl Primaries {
    /// The matrix converting linear RGB in this space to CIE XYZ, where white has Y = 1.
    #[must_use]
    pub fn rgb_to_xyz(&self) -> Mat3 {
        let xyz = |[x, y]: [f32; 2]| [x / y, 1.0, (1.0 - x - y) / y];
        let [r, g, b] = [xyz(self.red), xyz(self.green), xyz(self.blue)];
        let primaries = Mat3([[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]]);
        // Scale each primary so that RGB (1, 1, 1) is the white point.
        let scale = primaries
            .inverse()
            .expect("primaries are linearly independent")
            .apply(xyz(self.white));
        Mat3(
            primaries
                .0
                .map(|row| std::array::from_fn(|j| row[j] * scale[j])),
        )
    }
}

/// The sRGB transfer function, from linear light to encoded values. Both are in [0, 1].
#[must_use]
pub fn srgb_encode(linear: f32) -> f32 {
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(a: &Mat3, b: &Mat3) {
        for (row_a, row_b) in a.0.iter().zip(&b.0) {
            for (x, y) in row_a.iter().zip(row_b) {
                assert!((x - y).abs() < 1e-4, "{a:?} != {b:?}");
            }
        }
    }

    #[test]
    fn srgb_to_xyz_matches_the_standard() {
        let expected = Mat3([
            [0.4124, 0.3576, 0.1805],
            [0.2126, 0.7152, 0.0722],
            [0.0193, 0.1192, 0.9505],
        ]);
        assert_close(&SRGB.rgb_to_xyz(), &expected);
    }

    #[test]
    fn rec2020_to_xyz_matches_the_standard() {
        let expected = Mat3([
            [0.6370, 0.1446, 0.1689],
            [0.2627, 0.6780, 0.0593],
            [0.0000, 0.0281, 1.0610],
        ]);
        assert_close(&REC2020.rgb_to_xyz(), &expected);
    }

    #[test]
    fn inverse_round_trips() {
        let m = REC2020.rgb_to_xyz();
        let identity = Mat3([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_close(&m.mul(&m.inverse().unwrap()), &identity);
    }
}
