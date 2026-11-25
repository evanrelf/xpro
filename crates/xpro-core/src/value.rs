//! Values, which flow from nodes' outputs to other nodes' inputs.

use crate::{action::Action, image::Image, raw::Mosaic};
use std::{fmt, sync::Arc};

/// A value produced by a node. Values are shared rather than copied, so passing one along is
/// cheap.
#[derive(Clone, Debug)]
pub enum Value {
    Mosaic(Arc<Mosaic>),
    /// An image whose values are proportional to light in the scene, with no upper limit.
    SceneReferredImage(Arc<Image>),
    /// An image whose values describe how things should look on a display, within its range.
    DisplayReferredImage(Arc<Image>),
    Action(Arc<Action>),
}

/// The type of a [`Value`], which says which ports it can flow between.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueType {
    Mosaic,
    SceneReferredImage,
    DisplayReferredImage,
    Action,
}

impl Value {
    #[must_use]
    pub fn value_type(&self) -> ValueType {
        match self {
            Self::Mosaic(_) => ValueType::Mosaic,
            Self::SceneReferredImage(_) => ValueType::SceneReferredImage,
            Self::DisplayReferredImage(_) => ValueType::DisplayReferredImage,
            Self::Action(_) => ValueType::Action,
        }
    }
}

impl Value {
    /// The mosaic, if this is one.
    pub fn as_mosaic(&self) -> anyhow::Result<&Arc<Mosaic>> {
        match self {
            Self::Mosaic(mosaic) => Ok(mosaic),
            value => Err(value.mismatch(ValueType::Mosaic)),
        }
    }

    /// The image, if this is a scene-referred image.
    pub fn as_scene_referred_image(&self) -> anyhow::Result<&Arc<Image>> {
        match self {
            Self::SceneReferredImage(image) => Ok(image),
            value => Err(value.mismatch(ValueType::SceneReferredImage)),
        }
    }

    /// The image, if this is a display-referred image.
    pub fn as_display_referred_image(&self) -> anyhow::Result<&Arc<Image>> {
        match self {
            Self::DisplayReferredImage(image) => Ok(image),
            value => Err(value.mismatch(ValueType::DisplayReferredImage)),
        }
    }

    fn mismatch(&self, expected: ValueType) -> anyhow::Error {
        anyhow::anyhow!("Expected a {expected}, not a {}", self.value_type())
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Mosaic => "mosaic",
            Self::SceneReferredImage => "scene-referred image",
            Self::DisplayReferredImage => "display-referred image",
            Self::Action => "action",
        })
    }
}
