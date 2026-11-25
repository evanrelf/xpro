//! Evaluating graphs: computing nodes' values, with no side effects.

use crate::{
    action::Action,
    graph::{Graph, NodeId},
    node::Ports as _,
    raw::Mosaic,
    request::{PreviewScale, Request},
    value::{Value, ValueType},
};
use anyhow::{Context as _, bail};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    hash::{DefaultHasher, Hash as _, Hasher as _},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// The name of the graph input that graphs read by default: the image file being edited.
pub const IMAGE_INPUT: &str = "image";

/// What a graph's inputs are bound to for one evaluation, by name.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    /// Each binding, and a number unique to it, which identifies a mosaic in memory.
    bindings: BTreeMap<String, (Input, u64)>,
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
        static NEXT_BINDING: AtomicU64 = AtomicU64::new(0);
        let binding = NEXT_BINDING.fetch_add(1, Ordering::Relaxed);
        self.bindings.insert(name.into(), (input, binding));
        self
    }

    /// A hash of what the inputs are bound to. Image files are identified by their path, size
    /// and modification time, so a changed file gets a new key.
    fn key(&self) -> Key {
        let mut hasher = DefaultHasher::new();
        for (name, (input, binding)) in &self.bindings {
            name.hash(&mut hasher);
            match input {
                Input::Path(path) => {
                    path.hash(&mut hasher);
                    let metadata = std::fs::metadata(path).ok();
                    metadata
                        .as_ref()
                        .map(std::fs::Metadata::len)
                        .hash(&mut hasher);
                    let modified = metadata.and_then(|metadata| metadata.modified().ok());
                    modified.hash(&mut hasher);
                }
                // A mosaic in memory can't change, so it's identified by its binding.
                Input::Mosaic(_) => binding.hash(&mut hasher),
            }
        }
        hasher.finish()
    }

    pub fn get(&self, name: &str) -> anyhow::Result<&Input> {
        self.bindings
            .get(name)
            .map(|(input, _)| input)
            .with_context(|| format!("The graph input \"{name}\" isn't bound to anything"))
    }
}

/// Computes a node's value, computing the values of the nodes it's wired to first. Shares no
/// work with other evaluations; use an [`Evaluator`] for that.
pub fn evaluate(graph: &Graph, inputs: &Inputs, id: &NodeId) -> anyhow::Result<Value> {
    Evaluator::new(0).evaluate(graph, inputs, id, &Request::default())
}

/// Evaluates every output node, returning the actions they describe, in order of their IDs.
pub fn evaluate_actions(graph: &Graph, inputs: &Inputs) -> anyhow::Result<Vec<Arc<Action>>> {
    Evaluator::new(0).evaluate_actions(graph, inputs)
}

/// Evaluates graphs, reusing values from earlier evaluations where nothing they depend on has
/// changed.
///
/// Each value is cached under a key that combines its node's parameters, the keys of the values
/// wired to its inputs, and what the graph's inputs are bound to. So changing a parameter
/// recomputes only the nodes downstream of it, and the node IDs don't matter: two nodes with the
/// same parameters and inputs share a value.
#[derive(Debug)]
pub struct Evaluator {
    cache: HashMap<Key, Entry>,
    /// How many bytes of values to keep between evaluations.
    budget: usize,
    /// Counts evaluations, to know which cached values were used least recently.
    clock: u64,
    /// How many times a node has been evaluated, rather than its value found in the cache.
    pub nodes_evaluated: usize,
}

/// A hash identifying a value: what it was computed from.
type Key = u64;

#[derive(Debug)]
struct Entry {
    value: Value,
    last_used: u64,
}

impl Evaluator {
    /// An evaluator that keeps up to `budget` bytes of values between evaluations, discarding
    /// the least recently used values first.
    #[must_use]
    pub fn new(budget: usize) -> Self {
        Self {
            cache: HashMap::new(),
            budget,
            clock: 0,
            nodes_evaluated: 0,
        }
    }

    /// How many bytes of values are cached.
    #[must_use]
    pub fn cached_size(&self) -> usize {
        self.cache.values().map(|entry| entry.value.size()).sum()
    }

    /// Computes a node's value as requested, reusing cached values where possible.
    pub fn evaluate(
        &mut self,
        graph: &Graph,
        inputs: &Inputs,
        id: &NodeId,
        request: &Request,
    ) -> anyhow::Result<Value> {
        self.clock += 1;
        let inputs_key = inputs.key();
        let result =
            self.evaluate_with_path(graph, inputs, inputs_key, request, id, &mut Vec::new());
        self.trim();
        result.map(|(value, _)| value)
    }

    /// Evaluates every output node at full size, returning the actions they describe, in order
    /// of their IDs.
    pub fn evaluate_actions(
        &mut self,
        graph: &Graph,
        inputs: &Inputs,
    ) -> anyhow::Result<Vec<Arc<Action>>> {
        self.clock += 1;
        let inputs_key = inputs.key();
        let request = Request::default();
        let result = graph
            .nodes()
            .filter(|(_, node)| node.node.is_output())
            .map(|(id, _)| {
                let path = &mut Vec::new();
                match self.evaluate_with_path(graph, inputs, inputs_key, &request, id, path)? {
                    (Value::Action(action), _) => Ok(action),
                    (value, _) => {
                        bail!(
                            "{id} is an output node, but produced a {}",
                            value.value_type()
                        )
                    }
                }
            })
            .collect();
        self.trim();
        result
    }

    /// Computes a node's value and its key. `path` holds the nodes whose values are waiting on
    /// this one, to catch cycles.
    fn evaluate_with_path(
        &mut self,
        graph: &Graph,
        inputs: &Inputs,
        inputs_key: Key,
        request: &Request,
        id: &NodeId,
        path: &mut Vec<NodeId>,
    ) -> anyhow::Result<(Value, Key)> {
        if path.contains(id) {
            bail!("{id} is wired in a cycle");
        }
        let node = &graph
            .get(id)
            .with_context(|| format!("There's no node {id}"))?
            .node;
        path.push(id.clone());
        let upstream = node
            .inputs()
            .into_iter()
            .map(|input| {
                let wire = input
                    .wire
                    .with_context(|| format!("{id}'s {} input isn't wired", input.name))?;
                self.evaluate_with_path(graph, inputs, inputs_key, request, &wire, path)
            })
            .collect::<anyhow::Result<Vec<(Value, Key)>>>();
        path.pop();
        let upstream = upstream?;

        let mut hasher = DefaultHasher::new();
        inputs_key.hash(&mut hasher);
        // The node's parameters, without the IDs of the nodes it's wired to.
        let mut parameters = node.clone();
        for wire in parameters.wires_mut() {
            *wire = None;
        }
        ron::to_string(&parameters)?.hash(&mut hasher);
        for (_, key) in &upstream {
            key.hash(&mut hasher);
        }
        // A mosaic is always full size, so it's shared by every scale.
        let scale = match node.output_type() {
            ValueType::Mosaic => PreviewScale::FULL,
            _ => request.scale,
        };
        scale.hash(&mut hasher);
        let key = hasher.finish();

        if let Some(entry) = self.cache.get_mut(&key) {
            entry.last_used = self.clock;
            return Ok((entry.value.clone(), key));
        }
        request.cancel.check()?;
        let values: Vec<Value> = upstream.into_iter().map(|(value, _)| value).collect();
        self.nodes_evaluated += 1;
        let value = node
            .evaluate(&values, inputs, scale)
            .with_context(|| format!("Failed to evaluate {id}"))?;
        self.cache.insert(
            key,
            Entry {
                value: value.clone(),
                last_used: self.clock,
            },
        );
        Ok((value, key))
    }

    /// Discards the least recently used values until the rest fit within the budget.
    fn trim(&mut self) {
        let mut total: usize = self.cache.values().map(|entry| entry.value.size()).sum();
        if total <= self.budget {
            return;
        }
        let mut entries: Vec<(Key, u64, usize)> = self
            .cache
            .iter()
            .map(|(key, entry)| (*key, entry.last_used, entry.value.size()))
            .collect();
        entries.sort_unstable_by_key(|&(_, last_used, _)| last_used);
        for (key, _, size) in entries {
            if total <= self.budget {
                break;
            }
            self.cache.remove(&key);
            total -= size;
        }
    }
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
    use crate::request::Cancelled;
    use crate::{
        node::{Node, default_graph},
        path_template::escape_path_template,
        testing::{
            downstream, gen_any_graph, gen_mosaic, gen_valid_graph, randomize_parameters, upstream,
        },
    };
    use hegel::{TestCase, generators as gs};
    use std::collections::BTreeSet;

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

    /// Whether two values are the same, down to the bits of each number.
    fn same(a: &Value, b: &Value) -> bool {
        let same_image = |a: &crate::image::Image, b: &crate::image::Image| {
            (a.width, a.height, a.color_space, a.orientation)
                == (b.width, b.height, b.color_space, b.orientation)
                && a.pixels.iter().flatten().map(|x| x.to_bits()).eq(b
                    .pixels
                    .iter()
                    .flatten()
                    .map(|x| x.to_bits()))
        };
        match (a, b) {
            (Value::Mosaic(a), Value::Mosaic(b)) => {
                a.values
                    .iter()
                    .map(|x| x.to_bits())
                    .eq(b.values.iter().map(|x| x.to_bits()))
                    && (a.width, a.height, a.cfa) == (b.width, b.height, b.cfa)
            }
            (Value::SceneReferredImage(a), Value::SceneReferredImage(b))
            | (Value::DisplayReferredImage(a), Value::DisplayReferredImage(b)) => same_image(a, b),
            (Value::Action(a), Value::Action(b)) => {
                let (
                    Action::WritePng { image: a, path: pa },
                    Action::WritePng { image: b, path: pb },
                ) = (&**a, &**b);
                pa == pb && same_image(a, b)
            }
            _ => false,
        }
    }

    /// Binds every graph input that a Read Raw node reads to `mosaic`.
    fn bind_all(graph: &Graph, mosaic: &Arc<Mosaic>) -> Inputs {
        let mut inputs = Inputs::new();
        for (_, node) in graph.nodes() {
            if let Node::ReadRaw(read) = &node.node {
                inputs = inputs.bind(read.input.clone(), Input::Mosaic(mosaic.clone()));
            }
        }
        inputs
    }

    #[hegel::test(test_cases = 250)]
    fn cached_values_match_freshly_computed_ones(tc: TestCase) {
        let mut graph = tc.draw(gen_valid_graph());
        let mosaic = Arc::new(tc.draw(gen_mosaic()));
        let inputs = bind_all(&graph, &mosaic);
        let budget = tc.draw(gs::sampled_from(vec![0, 1_000, usize::MAX]));
        let mut evaluator = Evaluator::new(budget);
        let ids: Vec<NodeId> = graph.nodes().map(|(id, _)| id.clone()).collect();
        let changes = tc.draw(gs::integers::<usize>().max_value(4));
        for step in 0..=changes {
            if step > 0 {
                let id = tc.draw(gs::sampled_from(ids.clone()));
                randomize_parameters(&tc, &mut graph.get_mut(&id).unwrap().node, true);
            }
            let request = Request::at_scale(PreviewScale(tc.draw(gs::integers().max_value(3))));
            for id in &ids {
                let cached = evaluator.evaluate(&graph, &inputs, id, &request);
                let fresh = Evaluator::new(0).evaluate(&graph, &inputs, id, &request);
                match (cached, fresh) {
                    (Ok(cached), Ok(fresh)) => assert!(same(&cached, &fresh), "{id}"),
                    (Err(_), Err(_)) => {}
                    (cached, fresh) => panic!("{id}: {cached:?} vs {fresh:?}"),
                }
            }
        }
    }

    #[hegel::test(test_cases = 800)]
    fn reevaluating_an_unchanged_graph_does_no_work(tc: TestCase) {
        let graph = tc.draw(gen_valid_graph());
        let mosaic = Arc::new(tc.draw(gen_mosaic()));
        let inputs = bind_all(&graph, &mosaic);
        let mut evaluator = Evaluator::new(usize::MAX);
        let succeeded: Vec<&NodeId> = graph
            .nodes()
            .map(|(id, _)| id)
            .filter(|id| {
                evaluator
                    .evaluate(&graph, &inputs, id, &Request::default())
                    .is_ok()
            })
            .collect();
        let before = evaluator.nodes_evaluated;
        for id in succeeded {
            evaluator
                .evaluate(&graph, &inputs, id, &Request::default())
                .unwrap();
        }
        assert_eq!(evaluator.nodes_evaluated, before);
    }

    #[hegel::test(test_cases = 1_500)]
    fn changing_a_parameter_only_recomputes_whats_downstream_of_it(tc: TestCase) {
        let mut graph = tc.draw(gen_valid_graph());
        let mosaic = Arc::new(tc.draw(gen_mosaic()));
        let inputs = bind_all(&graph, &mosaic);
        let ids: Vec<NodeId> = graph.nodes().map(|(id, _)| id.clone()).collect();
        let target = tc.draw(gs::sampled_from(ids.clone()));
        let mut evaluator = Evaluator::new(usize::MAX);
        tc.assume(
            evaluator
                .evaluate(&graph, &inputs, &target, &Request::default())
                .is_ok(),
        );

        let changed = tc.draw(gs::sampled_from(ids));
        let affected: BTreeSet<NodeId> = downstream(&graph, &changed)
            .intersection(&upstream(&graph, &target))
            .cloned()
            .collect();
        randomize_parameters(&tc, &mut graph.get_mut(&changed).unwrap().node, false);
        let before = evaluator.nodes_evaluated;
        let _ = evaluator.evaluate(&graph, &inputs, &target, &Request::default());
        let evaluated = evaluator.nodes_evaluated - before;
        assert!(evaluated <= affected.len(), "{evaluated} > {affected:?}");
    }

    #[hegel::test(test_cases = 800)]
    fn the_cache_stays_within_its_budget(tc: TestCase) {
        let graph = tc.draw(gen_valid_graph());
        let mosaic = Arc::new(tc.draw(gen_mosaic()));
        let inputs = bind_all(&graph, &mosaic);
        let budget = tc.draw(gs::integers::<usize>().max_value(10_000));
        let mut evaluator = Evaluator::new(budget);
        for (id, _) in graph.nodes() {
            let _ = evaluator.evaluate(&graph, &inputs, id, &Request::default());
            assert!(evaluator.cached_size() <= budget);
        }
    }

    #[test]
    fn changing_exposure_recomputes_only_what_follows_it() {
        let mut graph = default_graph_writing("out.png");
        let mosaic = Arc::new(Mosaic {
            width: 4,
            height: 4,
            cfa: [[crate::raw::Channel::Red, crate::raw::Channel::Green]; 2],
            values: vec![0.5; 16],
            as_shot_white_balance: [2.0, 1.0, 1.5],
            xyz_to_camera: crate::color::REC2020.rgb_to_xyz().inverse().unwrap(),
            orientation: crate::geometry::Orientation::Normal,
        });
        let inputs = Inputs::new().bind(IMAGE_INPUT, Input::Mosaic(mosaic));
        let mut evaluator = Evaluator::new(usize::MAX);
        evaluator.evaluate_actions(&graph, &inputs).unwrap();
        assert_eq!(evaluator.nodes_evaluated, graph.len());

        for (_, node) in graph.nodes_mut() {
            if let Node::Exposure(exposure) = &mut node.node {
                exposure.stops += 1.0;
            }
        }
        evaluator.evaluate_actions(&graph, &inputs).unwrap();
        // Exposure, View Transform and Write PNG.
        assert_eq!(evaluator.nodes_evaluated, graph.len() + 3);
    }

    #[hegel::test(test_cases = 650)]
    fn previews_are_the_size_of_halved_full_size_images(tc: TestCase) {
        let graph = tc.draw(gen_valid_graph());
        let mosaic = Arc::new(tc.draw(gen_mosaic()));
        let inputs = bind_all(&graph, &mosaic);
        let scale = PreviewScale(tc.draw(gs::integers::<u8>().max_value(4)));
        let mut evaluator = Evaluator::new(usize::MAX);
        let size = |value: &Value| match value {
            Value::Mosaic(mosaic) => (mosaic.width, mosaic.height),
            Value::SceneReferredImage(image) | Value::DisplayReferredImage(image) => {
                (image.width, image.height)
            }
            Value::Action(action) => {
                let Action::WritePng { image, .. } = &**action;
                (image.width, image.height)
            }
        };
        for (id, node) in graph.nodes() {
            let Ok(full) = evaluator.evaluate(&graph, &inputs, id, &Request::default()) else {
                continue;
            };
            let preview = evaluator.evaluate(&graph, &inputs, id, &Request::at_scale(scale));
            let (mut width, mut height) = size(&full);
            if node.node.output_type() != ValueType::Mosaic {
                for _ in 0..scale.halvings() {
                    (width, height) = (width.div_ceil(2), height.div_ceil(2));
                }
            }
            assert_eq!(size(&preview.unwrap()), (width, height), "{id}");
        }
    }

    #[test]
    fn previews_share_the_mosaic_with_full_size_images() {
        let graph = default_graph();
        let view_transform = graph
            .nodes()
            .find(|(_, node)| matches!(node.node, Node::ViewTransform(_)))
            .map(|(id, _)| id.clone())
            .unwrap();
        let mosaic = Arc::new(Mosaic {
            width: 4,
            height: 4,
            cfa: [[crate::raw::Channel::Red, crate::raw::Channel::Green]; 2],
            values: vec![0.5; 16],
            as_shot_white_balance: [2.0, 1.0, 1.5],
            xyz_to_camera: crate::color::REC2020.rgb_to_xyz().inverse().unwrap(),
            orientation: crate::geometry::Orientation::Normal,
        });
        let inputs = Inputs::new().bind(IMAGE_INPUT, Input::Mosaic(mosaic));
        let mut evaluator = Evaluator::new(usize::MAX);
        let preview = Request::at_scale(PreviewScale(1));
        evaluator
            .evaluate(&graph, &inputs, &view_transform, &preview)
            .unwrap();
        // Everything up to the view transform.
        assert_eq!(evaluator.nodes_evaluated, 7);
        evaluator
            .evaluate(&graph, &inputs, &view_transform, &Request::default())
            .unwrap();
        // Everything but Read Raw and White Balance, which make mosaics.
        assert_eq!(evaluator.nodes_evaluated, 7 + 5);
    }

    #[hegel::test(test_cases = 2_000)]
    fn a_cancelled_evaluation_computes_nothing(tc: TestCase) {
        let graph = tc.draw(gen_valid_graph());
        let mosaic = Arc::new(tc.draw(gen_mosaic()));
        let inputs = bind_all(&graph, &mosaic);
        let ids: Vec<NodeId> = graph.nodes().map(|(id, _)| id.clone()).collect();
        let id = tc.draw(gs::sampled_from(ids));
        let request = Request::default();
        request.cancel.cancel();
        let mut evaluator = Evaluator::new(usize::MAX);
        let error = evaluator
            .evaluate(&graph, &inputs, &id, &request)
            .unwrap_err();
        assert!(Cancelled::caused(&error), "{error:#}");
        assert_eq!(evaluator.nodes_evaluated, 0);
        assert_eq!(evaluator.cached_size(), 0);
    }

    #[hegel::test(test_cases = 1_500)]
    fn cancelling_doesnt_stop_reusing_cached_values(tc: TestCase) {
        let graph = tc.draw(gen_valid_graph());
        let mosaic = Arc::new(tc.draw(gen_mosaic()));
        let inputs = bind_all(&graph, &mosaic);
        let ids: Vec<NodeId> = graph.nodes().map(|(id, _)| id.clone()).collect();
        let id = tc.draw(gs::sampled_from(ids));
        let mut evaluator = Evaluator::new(usize::MAX);
        let first = evaluator.evaluate(&graph, &inputs, &id, &Request::default());
        tc.assume(first.is_ok());
        let request = Request::default();
        request.cancel.cancel();
        let again = evaluator.evaluate(&graph, &inputs, &id, &request).unwrap();
        assert!(same(&first.unwrap(), &again));
    }
}
