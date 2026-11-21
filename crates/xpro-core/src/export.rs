//! Writing images to files.

use crate::image::DisplayImage;
use ::image::{ImageEncoder as _, codecs::png::PngEncoder};
use anyhow::Context as _;
use std::{fs::File, io::BufWriter, path::Path};

/// Writes an image as a PNG. Fails rather than overwrite an existing file.
pub fn write_png(image: &DisplayImage, path: &Path) -> anyhow::Result<()> {
    let file =
        File::create_new(path).with_context(|| format!("Failed to create {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    PngEncoder::new(&mut writer)
        .write_image(
            image.pixels.as_flattened(),
            u32::try_from(image.width)?,
            u32::try_from(image.height)?,
            ::image::ExtendedColorType::Rgb8,
        )
        .with_context(|| format!("Failed to write {}", path.display()))?;
    writer.into_inner()?.sync_all()?;
    Ok(())
}
