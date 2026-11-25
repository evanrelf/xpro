//! Generators for property tests.

use crate::{
    color::REC2020,
    geometry::Orientation,
    graph::{Graph, GraphNode, NodeId},
    node::{Multipliers, Node, Ports as _},
    raw::{Channel, Mosaic},
    value::ValueType,
};
use hegel::{TestCase, generators as gs};
use std::collections::BTreeSet;

pub fn gen_float(allow_nan: bool) -> impl hegel::generators::PrintableGenerator<f32> {
    gs::floats::<f32>().allow_nan(allow_nan)
}

/// Gives a node random parameters. Leaves its wires alone.
pub fn randomize_parameters(tc: &TestCase, node: &mut Node, allow_nan: bool) {
    match node {
        Node::ReadRaw(node) => node.input = tc.draw(gs::text()),
        Node::WhiteBalance(node) => {
            node.multipliers = if tc.draw(gs::booleans()) {
                Multipliers::AsShot
            } else {
                Multipliers::Custom {
                    red: tc.draw(gen_float(allow_nan)),
                    green: tc.draw(gen_float(allow_nan)),
                    blue: tc.draw(gen_float(allow_nan)),
                }
            };
        }
        Node::Exposure(node) => node.stops = tc.draw(gen_float(allow_nan)),
        Node::ViewTransform(node) => {
            node.contrast = tc.draw(gen_float(allow_nan));
            node.white = tc.draw(gen_float(allow_nan));
        }
        Node::WritePng(node) => node.path = tc.draw(gs::text()),
        Node::Demosaic(_) | Node::CameraToWorkingSpace(_) | Node::Orient(_) => {}
    }
}

/// Any graph, as a hand-edited file might hold: any IDs, labels and parameters, with wires
/// to anything, including nodes that don't exist.
#[hegel::composite]
pub fn gen_any_graph(tc: &TestCase, allow_nan: bool) -> Graph {
    let ids: Vec<NodeId> = tc
        .draw(gs::vecs(gs::text()).max_size(8))
        .into_iter()
        .map(NodeId::new)
        .collect();
    let extra_ids: Vec<NodeId> = tc
        .draw(gs::vecs(gs::text()).max_size(2))
        .into_iter()
        .map(NodeId::new)
        .collect();
    let targets: Vec<NodeId> = ids.iter().chain(&extra_ids).cloned().collect();
    let mut graph = Graph::new();
    for id in ids {
        let mut node = tc.draw(gs::sampled_from(Node::defaults()));
        randomize_parameters(tc, &mut node, allow_nan);
        for wire in node.wires_mut() {
            if !targets.is_empty() && tc.draw(gs::booleans()) {
                *wire = Some(tc.draw(gs::sampled_from(targets.clone())));
            }
        }
        let label = tc.draw(gs::optional(gs::text()));
        graph.insert(id, GraphNode { label, node });
    }
    graph
}

/// A valid graph, built by adding nodes one at a time, each wired only to nodes added
/// before it whose outputs have the right types.
#[hegel::composite]
pub fn gen_valid_graph(tc: &TestCase) -> Graph {
    let count = tc.draw(gs::integers::<usize>().min_value(1).max_value(12));
    let mut graph = Graph::new();
    for i in 0..count {
        // Only node types whose inputs can all be wired to an existing node.
        let candidates_for = |value_type: ValueType| -> Vec<NodeId> {
            graph
                .nodes()
                .filter(|(_, n)| n.node.output_type() == value_type)
                .map(|(id, _)| id.clone())
                .collect()
        };
        let types: Vec<Node> = Node::defaults()
            .into_iter()
            .filter(|node| {
                node.inputs()
                    .iter()
                    .all(|input| !candidates_for(input.value_type).is_empty())
            })
            .collect();
        let mut node = tc.draw(gs::sampled_from(types));
        randomize_parameters(tc, &mut node, false);
        let inputs = node.inputs();
        for (input, wire) in inputs.iter().zip(node.wires_mut()) {
            *wire = Some(tc.draw(gs::sampled_from(candidates_for(input.value_type))));
        }
        let id = NodeId::new(format!("{}_{i:04}", node.id_prefix()));
        graph.insert(id, GraphNode { label: None, node });
    }
    graph
}

/// A small mosaic from a Bayer sensor whose own RGB is Rec.2020, with any orientation.
#[hegel::composite]
pub fn gen_mosaic(tc: &TestCase) -> Mosaic {
    use Channel::{Blue as B, Green as G, Red as R};
    let width = tc.draw(gs::integers::<usize>().min_value(2).max_value(8));
    let height = tc.draw(gs::integers::<usize>().min_value(2).max_value(8));
    let cfa = tc.draw(gs::sampled_from(vec![
        [[R, G], [G, B]],
        [[B, G], [G, R]],
        [[G, R], [B, G]],
        [[G, B], [R, G]],
    ]));
    let value = gs::floats::<f32>().min_value(0.0).max_value(1.0);
    let count = width * height;
    let values = tc.draw(gs::vecs(value).min_size(count).max_size(count));
    let multiplier = || gs::floats::<f32>().min_value(0.5).max_value(4.0);
    let orientation = tc.draw(gs::integers::<u16>().min_value(1).max_value(8));
    Mosaic {
        width,
        height,
        cfa,
        values,
        as_shot_white_balance: [tc.draw(multiplier()), 1.0, tc.draw(multiplier())],
        xyz_to_camera: REC2020.rgb_to_xyz().inverse().unwrap(),
        orientation: Orientation::from_exif(orientation),
    }
}

/// Every node whose value depends on `id`'s, including `id` itself.
pub fn downstream(graph: &Graph, id: &NodeId) -> BTreeSet<NodeId> {
    let mut found = BTreeSet::from([id.clone()]);
    loop {
        let before = found.len();
        for (other, node) in graph.nodes() {
            if node
                .node
                .inputs()
                .iter()
                .any(|input| input.wire.as_ref().is_some_and(|wire| found.contains(wire)))
            {
                found.insert(other.clone());
            }
        }
        if found.len() == before {
            return found;
        }
    }
}

/// Every node whose value `id`'s depends on, including `id` itself.
pub fn upstream(graph: &Graph, id: &NodeId) -> BTreeSet<NodeId> {
    let mut found = BTreeSet::from([id.clone()]);
    let mut queue = vec![id.clone()];
    while let Some(next) = queue.pop() {
        let Some(node) = graph.get(&next) else {
            continue;
        };
        for wire in node
            .node
            .inputs()
            .into_iter()
            .filter_map(|input| input.wire)
        {
            if found.insert(wire.clone()) {
                queue.push(wire);
            }
        }
    }
    found
}
