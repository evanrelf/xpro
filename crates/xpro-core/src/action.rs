//! Actions, which describe side effects, and executing them.

use crate::{
    image::{Image, Srgb8Image},
    view::encode_srgb8,
};
use ::image::{ImageEncoder as _, codecs::png::PngEncoder};
use anyhow::Context as _;
use std::{
    io::{BufWriter, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
};

/// A description of a side effect, produced by evaluating an output node. Nothing happens until
/// it's executed.
#[derive(Clone, Debug)]
pub enum Action {
    /// Write a display-referred image to a PNG file. The path may be relative.
    WritePng { image: Arc<Image>, path: PathBuf },
}

impl Action {
    /// The path of the file the action writes, which may be relative.
    #[must_use]
    pub fn target(&self) -> &Path {
        match self {
            Self::WritePng { path, .. } => path,
        }
    }

    /// Carries out the action, resolving a relative path against `dir`. Never overwrites an
    /// existing file.
    pub fn execute(&self, dir: &Path) -> anyhow::Result<()> {
        match self {
            Self::WritePng { image, path } => write_png(&encode_srgb8(image)?, &dir.join(path)),
        }
    }
}

/// Writes an image as a PNG. Fails rather than overwrite an existing file. The PNG is written
/// to a temporary file first and moved into place when it's complete, so an interrupted write
/// never leaves part of one behind.
pub fn write_png(image: &Srgb8Image, path: &Path) -> anyhow::Result<()> {
    let context = || format!("Failed to write {}", path.display());
    let dir = path.parent().filter(|dir| !dir.as_os_str().is_empty());
    let mut temporary =
        tempfile::NamedTempFile::new_in(dir.unwrap_or(Path::new("."))).with_context(context)?;
    let mut writer = BufWriter::new(temporary.as_file_mut());
    PngEncoder::new(&mut writer)
        .write_image(
            image.pixels.as_flattened(),
            u32::try_from(image.width)?,
            u32::try_from(image.height)?,
            ::image::ExtendedColorType::Rgb8,
        )
        .with_context(context)?;
    writer.flush().with_context(context)?;
    drop(writer);
    temporary.as_file().sync_all().with_context(context)?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)
        .with_context(|| format!("Failed to create {}", path.display()))?;
    Ok(())
}
