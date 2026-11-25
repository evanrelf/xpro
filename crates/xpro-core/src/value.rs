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
