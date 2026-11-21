//! In-memory images.

/// A linear, scene-referred RGB image with `f32` channels. Which RGB space the channels are in
/// depends on where the image came from.
#[derive(Clone, Debug)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    /// Row-major pixels.
    pub pixels: Vec<[f32; 3]>,
}

/// A display-referred image, encoded as 8-bit sRGB.
#[derive(Clone, Debug)]
pub struct DisplayImage {
    pub width: usize,
    pub height: usize,
    /// Row-major pixels.
    pub pixels: Vec<[u8; 3]>,
}
