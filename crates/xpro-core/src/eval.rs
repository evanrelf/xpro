//! Evaluating graphs: computing nodes' values, with no side effects.

use crate::{
    action::Action,
    graph::{Graph, NodeId},
    node::Ports as _,
    raw::Mosaic,
    value::Value,
};
use anyhow::{Context as _, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

/// The name of the graph input that graphs read by default: the image file being edited.
pub const IMAGE_INPUT: &str = "image";

/// What a graph's inputs are bound to for one evaluation, by name.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    bindings: BTreeMap<String, Input>,
}

/// What a graph input is bound to.
#[derive(Clone, Debug)]
pub enum Input {
    /// An image file.
    Path(PathBuf),
    /// A mosaic that's already in memory.
    Mosaic(Arc<Mosaic>),
}

impl Inputs {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds the graph input `name`, replacing any earlier binding.
    #[must_use]
    pub fn bind(mut self, name: impl Into<String>, input: Input) -> Self {
        self.bindings.insert(name.into(), input);
        self
    }

    pub fn get(&self, name: &str) -> anyhow::Result<&Input> {
        self.bindings
            .get(name)
            .with_context(|| format!("The graph input \"{name}\" isn't bound to anything"))
    }
}

/// Computes a node's value, computing the values of the nodes it's wired to first.
pub fn evaluate(graph: &Graph, inputs: &Inputs, id: &NodeId) -> anyhow::Result<Value> {
    evaluate_with_path(graph, inputs, id, &mut Vec::new())
}

/// [`evaluate`], where `path` holds the nodes whose values are waiting on this one, to catch
/// cycles.
fn evaluate_with_path(
    graph: &Graph,
    inputs: &Inputs,
    id: &NodeId,
    path: &mut Vec<NodeId>,
) -> anyhow::Result<Value> {
    if path.contains(id) {
        bail!("{id} is wired in a cycle");
    }
    let node = &graph
        .get(id)
        .with_context(|| format!("There's no node {id}"))?
        .node;
    path.push(id.clone());
    let values = node
        .inputs()
        .into_iter()
        .map(|input| {
            let wire = input
                .wire
                .with_context(|| format!("{id}'s {} input isn't wired", input.name))?;
            evaluate_with_path(graph, inputs, &wire, path)
        })
        .collect::<anyhow::Result<Vec<Value>>>();
    path.pop();
    node.evaluate(&values?, inputs)
        .with_context(|| format!("Failed to evaluate {id}"))
}

/// Evaluates every output node, returning the actions they describe, in order of their IDs.
pub fn evaluate_actions(graph: &Graph, inputs: &Inputs) -> anyhow::Result<Vec<Arc<Action>>> {
    graph
        .nodes()
        .filter(|(_, node)| node.node.is_output())
        .map(|(id, _)| match evaluate(graph, inputs, id)? {
            Value::Action(action) => Ok(action),
            value => bail!(
                "{id} is an output node, but produced a {}",
                value.value_type()
            ),
        })
        .collect()
}

/// Executes actions, resolving relative paths against `dir`. Checks first that none would
/// overwrite an existing file, or a file another of the actions writes, so either all of them
/// run or none do (unless one fails partway).
pub fn execute(actions: &[Arc<Action>], dir: &Path) -> anyhow::Result<()> {
    let mut targets = BTreeSet::new();
    for action in actions {
        let target = dir.join(action.target());
        if target.exists() {
            bail!("{} already exists", target.display());
        }
        if !targets.insert(target.clone()) {
            bail!("More than one output writes {}", target.display());
        }
    }
    for action in actions {
        action.execute(dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        node::{Node, default_graph},
        path_template::escape_path_template,
        testing::{gen_any_graph, gen_mosaic, gen_valid_graph},
    };
    use hegel::{TestCase, generators as gs};

    #[hegel::test(test_cases = 1_000)]
    fn values_have_their_nodes_output_type(tc: TestCase) {
        let graph = tc.draw(gen_valid_graph());
        let mosaic = Arc::new(tc.draw(gen_mosaic()));
        let mut inputs = Inputs::new();
        for (_, node) in graph.nodes() {
            if let Node::ReadRaw(read) = &node.node {
                inputs = inputs.bind(read.input.clone(), Input::Mosaic(mosaic.clone()));
            }
        }
        for (id, node) in graph.nodes() {
            // Evaluating can fail, such as with an invalid parameter, but it never produces
            // the wrong type of value.
            if let Ok(value) = evaluate(&graph, &inputs, id) {
                assert_eq!(value.value_type(), node.node.output_type(), "{id}");
            }
        }
    }

    #[hegel::test(test_cases = 4_000)]
    fn any_graph_evaluates_or_fails_without_panicking(tc: TestCase) {
        // Including graphs with cycles, missing nodes and unwired inputs.
        let graph = tc.draw(gen_any_graph(true));
        let mosaic = Arc::new(tc.draw(gen_mosaic()));
        let inputs = Inputs::new().bind("", Input::Mosaic(mosaic));
        for (id, _) in graph.nodes() {
            let _ = evaluate(&graph, &inputs, id);
        }
    }

    #[test]
    fn invalid_graphs_fail_to_evaluate() {
        let mut graph = Graph::new();
        let a = NodeId::new("a");
        let b = NodeId::new("b");
        let exposure = |wire: &NodeId| crate::graph::GraphNode {
            label: None,
            node: crate::node::Exposure {
                image: Some(wire.clone()),
                stops: 1.0,
            }
            .into(),
        };
        graph.insert(a.clone(), exposure(&b));
        graph.insert(b.clone(), exposure(&a));
        assert!(evaluate(&graph, &Inputs::new(), &a).is_err(), "a cycle");
        graph.remove(&b);
        assert!(
            evaluate(&graph, &Inputs::new(), &a).is_err(),
            "a missing node"
        );
        assert!(
            evaluate(&graph, &Inputs::new(), &b).is_err(),
            "a node that isn't there"
        );
    }

    /// The default graph, bound to a mosaic and with its output writing `path`.
    fn default_graph_writing(path: &str) -> Graph {
        let mut graph = default_graph();
        for (_, node) in graph.nodes_mut() {
            if let Node::WritePng(output) = &mut node.node {
                output.path = escape_path_template(path);
            }
        }
        graph
    }

    #[hegel::test(test_cases = 1_000)]
    fn the_default_graph_writes_one_upright_png(tc: TestCase) {
        let mosaic = tc.draw(gen_mosaic());
        let (width, height) = (mosaic.width, mosaic.height);
        let swaps_axes = matches!(
            mosaic.orientation,
            crate::geometry::Orientation::Transpose
                | crate::geometry::Orientation::Rotate90
                | crate::geometry::Orientation::Transverse
                | crate::geometry::Orientation::Rotate270
        );
        let inputs = Inputs::new().bind("image", Input::Mosaic(Arc::new(mosaic)));
        let actions = evaluate_actions(&default_graph_writing("out.png"), &inputs).unwrap();
        let [action] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        let Action::WritePng { image, path } = &**action;
        assert_eq!(path, Path::new("out.png"));
        let expected = if swaps_axes {
            (height, width)
        } else {
            (width, height)
        };
        assert_eq!((image.width, image.height), expected);
    }

    #[hegel::test(test_cases = 40)]
    fn executing_writes_the_png(tc: TestCase) {
        let mosaic = Arc::new(tc.draw(gen_mosaic()));
        let inputs = Inputs::new().bind("image", Input::Mosaic(mosaic));
        let actions = evaluate_actions(&default_graph_writing("out.png"), &inputs).unwrap();
        let dir = tempfile::tempdir().unwrap();
        execute(&actions, dir.path()).unwrap();
        let Action::WritePng { image, .. } = &*actions[0];
        let written = ::image::open(dir.path().join("out.png")).unwrap().to_rgb8();
        let expected = crate::view::encode_srgb8(image).unwrap();
        assert_eq!(written.as_raw(), expected.pixels.as_flattened());
    }

    #[hegel::test(test_cases = 300)]
    fn executing_never_overwrites_and_writes_nothing_if_it_would(tc: TestCase) {
        let mosaic = Arc::new(tc.draw(gen_mosaic()));
        let inputs = Inputs::new().bind("image", Input::Mosaic(mosaic));
        let count = tc.draw(gs::integers::<usize>().min_value(1).max_value(4));
        let existing = tc.draw(gs::integers::<usize>().max_value(count - 1));
        let mut actions = Vec::new();
        for i in 0..count {
            let graph = default_graph_writing(&format!("{i}.png"));
            actions.extend(evaluate_actions(&graph, &inputs).unwrap());
        }
        let dir = tempfile::tempdir().unwrap();
        let existing_path = dir.path().join(format!("{existing}.png"));
        std::fs::write(&existing_path, "precious").unwrap();
        assert!(execute(&actions, dir.path()).is_err());
        assert_eq!(std::fs::read(&existing_path).unwrap(), b"precious");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn executing_refuses_two_actions_writing_one_file() {
        let image = Arc::new(crate::image::Image::rec2020(1, 1, vec![[0.5; 3]]));
        let action = Arc::new(Action::WritePng {
            image,
            path: PathBuf::from("same.png"),
        });
        let dir = tempfile::tempdir().unwrap();
        assert!(execute(&[action.clone(), action], dir.path()).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
