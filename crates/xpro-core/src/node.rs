//! Node types: each a struct of typed parameters, with a function that evaluates it.
//!
//! A node's inputs are fields too, each naming the node whose output it's wired to, or `None`
//! if it isn't wired yet.

use crate::{
    action::Action,
    develop, geometry,
    graph::NodeId,
    image::Image,
    raw::{self, Mosaic},
    value::ValueType,
    view::{self, ToneCurve},
};
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// A node's input port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputPort {
    pub name: &'static str,
    /// The type of value the port accepts.
    pub value_type: ValueType,
    /// The node whose output is wired to this port, if any.
    pub wire: Option<NodeId>,
}

/// What every node type knows about its ports.
pub trait Ports {
    /// The node's input ports, in a fixed order.
    fn inputs(&self) -> Vec<InputPort>;

    /// The wires to the node's input ports, in the same order as [`Ports::inputs`].
    fn wires_mut(&mut self) -> Vec<&mut Option<NodeId>>;

    /// The type of value the node's output provides.
    fn output_type(&self) -> ValueType;
}

/// Declares node types: the [`Node`] enum, and the basics of each node type.
macro_rules! node_types {
    ($($node_type:ident: $name:literal, $id_prefix:literal;)*) => {
        /// A node: one of the built-in node types.
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        pub enum Node {
            $($node_type($node_type),)*
        }

        impl Node {
            /// One node of each type, with default parameters and no wires.
            #[must_use]
            pub fn defaults() -> Vec<Self> {
                vec![$(Self::$node_type($node_type::default()),)*]
            }

            /// The node type's name, as people read it.
            #[must_use]
            pub fn type_name(&self) -> &'static str {
                match self {
                    $(Self::$node_type(_) => $name,)*
                }
            }

            /// The node type's name in snake case, which starts the IDs of nodes of this type.
            #[must_use]
            pub fn id_prefix(&self) -> &'static str {
                match self {
                    $(Self::$node_type(_) => $id_prefix,)*
                }
            }
        }

        impl Ports for Node {
            fn inputs(&self) -> Vec<InputPort> {
                match self {
                    $(Self::$node_type(node) => node.inputs(),)*
                }
            }

            fn wires_mut(&mut self) -> Vec<&mut Option<NodeId>> {
                match self {
                    $(Self::$node_type(node) => node.wires_mut(),)*
                }
            }

            fn output_type(&self) -> ValueType {
                match self {
                    $(Self::$node_type(node) => node.output_type(),)*
                }
            }
        }

        $(
            impl From<$node_type> for Node {
                fn from(node: $node_type) -> Self {
                    Self::$node_type(node)
                }
            }
        )*
    };
}

node_types! {
    ReadRaw: "Read Raw", "read_raw";
    WhiteBalance: "White Balance", "white_balance";
    Demosaic: "Demosaic", "demosaic";
    CameraToWorkingSpace: "Camera to Working Space", "camera_to_working_space";
    Orient: "Orient", "orient";
    Exposure: "Exposure", "exposure";
    ViewTransform: "View Transform", "view_transform";
    WritePng: "Write PNG", "write_png";
}

/// Implements [`Ports`] for a node type whose input ports are all fields of type
/// `Option<NodeId>`.
macro_rules! ports {
    ($node_type:ty, [$($input:ident: $value_type:ident),*] -> $output:ident) => {
        impl Ports for $node_type {
            fn inputs(&self) -> Vec<InputPort> {
                vec![$(InputPort {
                    name: stringify!($input),
                    value_type: ValueType::$value_type,
                    wire: self.$input.clone(),
                }),*]
            }

            fn wires_mut(&mut self) -> Vec<&mut Option<NodeId>> {
                vec![$(&mut self.$input),*]
            }

            fn output_type(&self) -> ValueType {
                ValueType::$output
            }
        }
    };
}

ports!(ReadRaw, [] -> Mosaic);
ports!(WhiteBalance, [mosaic: Mosaic] -> Mosaic);
ports!(Demosaic, [mosaic: Mosaic] -> SceneReferredImage);
ports!(CameraToWorkingSpace, [image: SceneReferredImage] -> SceneReferredImage);
ports!(Orient, [image: SceneReferredImage] -> SceneReferredImage);
ports!(Exposure, [image: SceneReferredImage] -> SceneReferredImage);
ports!(ViewTransform, [image: SceneReferredImage] -> DisplayReferredImage);
ports!(WritePng, [image: DisplayReferredImage] -> Action);

/// Reads a raw file, bound to one of the graph's inputs, into a mosaic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReadRaw {
    /// The name of the graph input to read.
    pub input: String,
}

impl Default for ReadRaw {
    fn default() -> Self {
        Self {
            input: "image".to_owned(),
        }
    }
}

impl ReadRaw {
    pub fn evaluate(&self, path: &Path) -> anyhow::Result<Mosaic> {
        raw::decode(path)
    }
}

/// Scales a mosaic's color channels so that neutral surfaces come out neutral.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WhiteBalance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mosaic: Option<NodeId>,
    pub multipliers: Multipliers,
}

/// Which white balance multipliers to use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Multipliers {
    /// The camera's choice, stored in the raw file.
    #[default]
    AsShot,
    /// Multipliers for red, green and blue.
    Custom([f32; 3]),
}

impl WhiteBalance {
    #[must_use]
    pub fn evaluate(&self, mosaic: &Mosaic) -> Mosaic {
        let multipliers = match self.multipliers {
            Multipliers::AsShot => mosaic.as_shot_white_balance,
            Multipliers::Custom(multipliers) => multipliers,
        };
        develop::white_balance(mosaic, multipliers)
    }
}

/// Fills in the colors each photosite didn't measure, turning a mosaic into a scene-referred
/// image in the camera's own RGB.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Demosaic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mosaic: Option<NodeId>,
}

impl Demosaic {
    #[must_use]
    pub fn evaluate(&self, mosaic: &Mosaic) -> Image {
        develop::demosaic(mosaic)
    }
}

/// Converts a scene-referred image from the camera's own RGB to the working space.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CameraToWorkingSpace {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<NodeId>,
}

impl CameraToWorkingSpace {
    pub fn evaluate(&self, image: &Image) -> anyhow::Result<Image> {
        develop::camera_to_rec2020(image)
    }
}

/// Rotates and flips an image so it displays upright, as the camera recorded.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Orient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<NodeId>,
}

impl Orient {
    #[must_use]
    pub fn evaluate(&self, image: &Image) -> Image {
        geometry::upright(image)
    }
}

/// Brightens or darkens a scene-referred image.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Exposure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<NodeId>,
    /// How much to brighten, in stops. Each stop doubles the light. Negative values darken.
    pub stops: f32,
}

impl Exposure {
    #[must_use]
    pub fn evaluate(&self, image: &Image) -> Image {
        develop::exposure(image, self.stops)
    }
}

/// Turns a scene-referred image into a display-referred one, rolling off the highlights.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ViewTransform {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<NodeId>,
    /// How steep the tone curve is. Higher values darken shadows and brighten highlights more.
    pub contrast: f32,
    /// How many stops above middle gray becomes display white.
    pub white: f32,
}

impl Default for ViewTransform {
    fn default() -> Self {
        Self {
            image: None,
            contrast: 1.5,
            white: 4.0,
        }
    }
}

impl ViewTransform {
    pub fn evaluate(&self, image: &Image) -> anyhow::Result<Image> {
        let curve = ToneCurve::new(self.contrast, self.white).with_context(|| {
            format!(
                "A contrast of {} can't reach white {} stops above middle gray",
                self.contrast, self.white,
            )
        })?;
        Ok(view::view_transform(image, &curve))
    }
}

/// An output node that writes a display-referred image to a PNG file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WritePng {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<NodeId>,
    pub path: PathBuf,
}

impl WritePng {
    #[must_use]
    pub fn evaluate(&self, image: Arc<Image>) -> Action {
        Action::WritePng {
            image,
            path: self.path.clone(),
        }
    }
}

/// How much the default graph's exposure brightens a developed image, in stops, so that a
/// subject the camera metered as correctly exposed lands near
/// [`MIDDLE_GRAY`](crate::view::MIDDLE_GRAY).
///
/// Cameras expose with headroom for highlights, so raw values for a correctly exposed subject
/// sit well below the sensor's white level. How much headroom varies by camera and settings.
/// This value matches the overall brightness of the camera's own JPEGs for a set of Nikon Z f
/// raw files, which were shot with Active D-Lighting (which underexposes to protect highlights).
/// Other cameras may want less.
///
/// The sensor's white level ends up 4 stops above middle gray, where the default
/// [`ViewTransform`] reaches white, so highlights roll off all the way to the sensor's limit and
/// no further.
pub const BASELINE_EXPOSURE: f32 = 1.5;

/// Develops a raw file into an upright, scene-referred Rec.2020 image, with its as-shot white
/// balance and [`BASELINE_EXPOSURE`].
pub fn develop(path: &Path) -> anyhow::Result<Image> {
    let mosaic = ReadRaw::default().evaluate(path)?;
    let mosaic = WhiteBalance::default().evaluate(&mosaic);
    let image = Demosaic::default().evaluate(&mosaic);
    let image = CameraToWorkingSpace::default().evaluate(&image)?;
    let image = Orient::default().evaluate(&image);
    let exposure = Exposure {
        stops: BASELINE_EXPOSURE,
        ..Exposure::default()
    };
    Ok(exposure.evaluate(&image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_view_transform_turns_the_sensors_white_level_white() {
        // A raw file's white level is 1.0 before the baseline exposure brightens it. If the
        // curve reached white sooner, it would clip highlights the sensor captured. If later,
        // the brightest highlights would look gray.
        let white_level = Image::rec2020(1, 1, vec![[BASELINE_EXPOSURE.exp2(); 3]]);
        let display = ViewTransform::default().evaluate(&white_level).unwrap();
        for c in display.pixels[0] {
            assert!((c - 1.0).abs() < 0.01, "{c}");
        }
    }
}
