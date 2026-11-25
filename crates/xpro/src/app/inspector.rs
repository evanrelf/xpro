//! The node list and the inspector, which shows and changes the selected node's parameters.

use egui::{RichText, SliderClamping};
use std::collections::BTreeMap;
use xpro_core::{
    graph::{Graph, GraphNode, NodeId, ValidationError},
    node::{Multipliers, Node, Ports as _},
};

/// The name a node is shown by: its label if it has one, otherwise its node type.
pub fn display_name(node: &GraphNode) -> &str {
    node.label.as_deref().unwrap_or(node.node.type_name())
}

/// Lists the graph's nodes in the order values flow through them. Clicking one selects it.
pub fn node_list(ui: &mut egui::Ui, graph: &Graph, selected: &mut Option<NodeId>) {
    let mut problems: BTreeMap<NodeId, Vec<String>> = BTreeMap::new();
    for error in graph.validate() {
        let nodes = match &error {
            ValidationError::Unwired { node, .. }
            | ValidationError::MissingNode { node, .. }
            | ValidationError::TypeMismatch { node, .. } => vec![node.clone()],
            ValidationError::Cycle { nodes } => nodes.iter().cloned().collect(),
        };
        for node in nodes {
            problems.entry(node).or_default().push(error.to_string());
        }
    }

    for id in graph.topological_order() {
        let Some(node) = graph.get(&id) else {
            continue;
        };
        ui.horizontal(|ui| {
            let is_selected = selected.as_ref() == Some(&id);
            let response = ui
                .selectable_label(is_selected, display_name(node))
                .on_hover_text(id.as_str());
            if response.clicked() {
                *selected = Some(id.clone());
            }
            if let Some(problems) = problems.get(&id) {
                ui.label(RichText::new("⚠").color(ui.visuals().warn_fg_color))
                    .on_hover_text(problems.join("\n"));
            }
        });
    }
}

/// Shows a node's ID, label, inputs and parameters, with controls to change them.
pub fn inspector(ui: &mut egui::Ui, graph: &mut Graph, id: &NodeId) {
    let names: BTreeMap<NodeId, String> = graph
        .nodes()
        .map(|(id, node)| (id.clone(), display_name(node).to_owned()))
        .collect();
    let Some(node) = graph.get_mut(id) else {
        return;
    };
    ui.heading(node.node.type_name());
    ui.label(RichText::new(id.as_str()).monospace().weak());
    ui.add_space(8.0);

    egui::Grid::new("inspector")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            let label = ui.label("Label");
            let mut text = node.label.clone().unwrap_or_default();
            let response = ui
                .add(egui::TextEdit::singleline(&mut text).hint_text(node.node.type_name()))
                .labelled_by(label.id);
            if response.changed() {
                node.label = (!text.is_empty()).then_some(text);
            }
            ui.end_row();

            for input in node.node.inputs() {
                ui.label(capitalize(input.name));
                let wired = match &input.wire {
                    Some(wire) => names.get(wire).map_or(wire.as_str(), String::as_str),
                    None => "Not wired",
                };
                ui.label(RichText::new(wired).weak());
                ui.end_row();
            }

            parameters(ui, &mut node.node);
        });
}

/// A control for each of a node's parameters, one row each.
fn parameters(ui: &mut egui::Ui, node: &mut Node) {
    match node {
        Node::ReadRaw(node) => {
            text_row(ui, "Input", &mut node.input);
        }
        Node::WhiteBalance(node) => {
            let label = ui.label("Multipliers");
            ui.horizontal(|ui| {
                let as_shot = matches!(node.multipliers, Multipliers::AsShot);
                if ui.selectable_label(as_shot, "As shot").clicked() {
                    node.multipliers = Multipliers::AsShot;
                }
                if ui.selectable_label(!as_shot, "Custom").clicked() && as_shot {
                    node.multipliers = Multipliers::Custom {
                        red: 1.0,
                        green: 1.0,
                        blue: 1.0,
                    };
                }
            })
            .response
            .labelled_by(label.id);
            ui.end_row();
            if let Multipliers::Custom { red, green, blue } = &mut node.multipliers {
                for (name, value) in [("Red", red), ("Green", green), ("Blue", blue)] {
                    slider_row(ui, name, value, 0.25..=4.0, "");
                }
            }
        }
        Node::Exposure(node) => slider_row(ui, "Stops", &mut node.stops, -5.0..=5.0, ""),
        Node::ViewTransform(node) => {
            slider_row(ui, "Contrast", &mut node.contrast, 0.5..=4.0, "");
            slider_row(ui, "White", &mut node.white, 1.0..=10.0, " stops");
        }
        Node::WritePng(node) => text_row(ui, "Path", &mut node.path),
        Node::Demosaic(_) | Node::CameraToWorkingSpace(_) | Node::Orient(_) => {}
    }
}

fn slider_row(
    ui: &mut egui::Ui,
    name: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) {
    let label = ui.label(name);
    ui.add(
        egui::Slider::new(value, range)
            .clamping(SliderClamping::Never)
            .suffix(suffix),
    )
    .labelled_by(label.id);
    ui.end_row();
}

fn text_row(ui: &mut egui::Ui, name: &str, text: &mut String) {
    let label = ui.label(name);
    ui.text_edit_singleline(text).labelled_by(label.id);
    ui.end_row();
}

fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
