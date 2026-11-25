//! Graphs of nodes, their file format, and validation.

use crate::{
    node::{Node, Ports as _},
    value::ValueType,
};
use anyhow::{Context as _, bail};
use ron::{extensions::Extensions, ser::PrettyConfig};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// The version of the graph file format this code reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// A node's unique name within its graph, such as `exposure_k3f9`: its node type in snake case,
/// then a short random suffix. Generated, never typed by hand.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(String);

impl NodeId {
    /// Wraps an existing ID, such as one read from a file.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A directed acyclic graph of nodes, keyed by their IDs.
///
/// A graph can hold mistakes, such as a wire to a node that doesn't exist, so that it can be
/// edited one step at a time. [`Graph::validate`] finds them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    nodes: BTreeMap<NodeId, GraphNode>,
}

/// A node in a graph, with its label.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphNode {
    /// An optional human-readable name, which doesn't have to be unique.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub node: Node,
}

/// How a graph is laid out in a graph file.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Format {
    version: u32,
    nodes: BTreeMap<NodeId, GraphNode>,
}

/// Just the version of a graph file, read first to decide how to read the rest.
#[derive(Deserialize)]
struct Version {
    version: u32,
}

fn ron_options() -> ron::Options {
    // So wires are written as `image: "exposure_k3f9"`, not `image: Some("exposure_k3f9")`,
    // and nodes as `Exposure(stops: 1.0)`, not `Exposure((stops: 1.0))`.
    ron::Options::default()
        .with_default_extension(Extensions::IMPLICIT_SOME)
        .with_default_extension(Extensions::UNWRAP_VARIANT_NEWTYPES)
}

impl Graph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a node, generating a new ID for it.
    pub fn add(&mut self, node: impl Into<Node>) -> NodeId {
        self.add_labeled(node, None)
    }

    /// Adds a node with a label, generating a new ID for it.
    pub fn add_labeled(&mut self, node: impl Into<Node>, label: Option<String>) -> NodeId {
        let node = node.into();
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let id = loop {
            let suffix: String = (0..4)
                .map(|_| char::from(ALPHABET[fastrand::usize(..ALPHABET.len())]))
                .collect();
            let id = NodeId(format!("{}_{suffix}", node.id_prefix()));
            if !self.nodes.contains_key(&id) {
                break id;
            }
        };
        self.nodes.insert(id.clone(), GraphNode { label, node });
        id
    }

    /// Adds a node with a given ID, replacing any node that already has it.
    pub fn insert(&mut self, id: NodeId, node: GraphNode) -> Option<GraphNode> {
        self.nodes.insert(id, node)
    }

    /// Removes a node. Wires to it are left dangling.
    pub fn remove(&mut self, id: &NodeId) -> Option<GraphNode> {
        self.nodes.remove(id)
    }

    #[must_use]
    pub fn get(&self, id: &NodeId) -> Option<&GraphNode> {
        self.nodes.get(id)
    }

    pub fn get_mut(&mut self, id: &NodeId) -> Option<&mut GraphNode> {
        self.nodes.get_mut(id)
    }

    /// The nodes, in order of their IDs.
    pub fn nodes_mut(&mut self) -> impl Iterator<Item = (&NodeId, &mut GraphNode)> {
        self.nodes.iter_mut()
    }

    /// The nodes, in order of their IDs.
    pub fn nodes(&self) -> impl Iterator<Item = (&NodeId, &GraphNode)> {
        self.nodes.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Writes the graph in the graph file format: human-readable RON.
    pub fn to_ron(&self) -> anyhow::Result<String> {
        let format = Format {
            version: FORMAT_VERSION,
            nodes: self.nodes.clone(),
        };
        let config = PrettyConfig::new().indentor("    ");
        Ok(ron_options().to_string_pretty(&format, config)? + "\n")
    }

    /// Reads a graph written in the graph file format.
    pub fn from_ron(ron: &str) -> anyhow::Result<Self> {
        let options = ron_options();
        let Version { version } = options
            .from_str(ron)
            .context("Failed to read the graph's format version")?;
        if version != FORMAT_VERSION {
            bail!(
                "Graph format version {version} isn't supported (expected version {FORMAT_VERSION})"
            );
        }
        let Format { version: _, nodes } = options.from_str(ron).context("Failed to read graph")?;
        Ok(Self { nodes })
    }

    /// Finds everything wrong with the graph: unwired inputs, wires to missing nodes, wires
    /// between ports of different types, and cycles. Returns nothing if the graph is valid.
    #[must_use]
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        for (id, GraphNode { node, .. }) in &self.nodes {
            for input in node.inputs() {
                let Some(wire) = input.wire else {
                    errors.push(ValidationError::Unwired {
                        node: id.clone(),
                        input: input.name,
                    });
                    continue;
                };
                let Some(upstream) = self.nodes.get(&wire) else {
                    errors.push(ValidationError::MissingNode {
                        node: id.clone(),
                        input: input.name,
                        missing: wire,
                    });
                    continue;
                };
                let found = upstream.node.output_type();
                if found != input.value_type {
                    errors.push(ValidationError::TypeMismatch {
                        node: id.clone(),
                        input: input.name,
                        expected: input.value_type,
                        found,
                    });
                }
            }
        }
        errors.extend(
            self.cycles()
                .into_iter()
                .map(|nodes| ValidationError::Cycle { nodes }),
        );
        errors
    }

    /// The groups of nodes that are wired in a cycle, each sorted. Uses Tarjan's algorithm for
    /// strongly connected components: a component with more than one node, or a node wired to
    /// itself, is a cycle.
    fn cycles(&self) -> Vec<BTreeSet<NodeId>> {
        struct State<'a> {
            graph: &'a Graph,
            next_index: usize,
            index: BTreeMap<&'a NodeId, usize>,
            low_link: BTreeMap<&'a NodeId, usize>,
            stack: Vec<&'a NodeId>,
            on_stack: BTreeSet<&'a NodeId>,
            cycles: Vec<BTreeSet<NodeId>>,
        }

        impl<'a> State<'a> {
            fn upstream(&self, id: &NodeId) -> Vec<&'a NodeId> {
                let node = &self.graph.nodes[id].node;
                node.inputs()
                    .into_iter()
                    .filter_map(|input| input.wire)
                    .filter_map(|wire| self.graph.nodes.get_key_value(&wire).map(|(k, _)| k))
                    .collect()
            }

            // Recursion depth is bounded by the length of the longest chain of nodes, which is
            // small for any graph a person builds.
            fn visit(&mut self, id: &'a NodeId) {
                self.index.insert(id, self.next_index);
                self.low_link.insert(id, self.next_index);
                self.next_index += 1;
                self.stack.push(id);
                self.on_stack.insert(id);

                let mut wired_to_itself = false;
                for upstream in self.upstream(id) {
                    if upstream == id {
                        wired_to_itself = true;
                    }
                    if !self.index.contains_key(upstream) {
                        self.visit(upstream);
                        let low = self.low_link[id].min(self.low_link[upstream]);
                        self.low_link.insert(id, low);
                    } else if self.on_stack.contains(upstream) {
                        let low = self.low_link[id].min(self.index[upstream]);
                        self.low_link.insert(id, low);
                    }
                }

                if self.low_link[id] == self.index[id] {
                    let mut component = BTreeSet::new();
                    loop {
                        let member = self.stack.pop().expect("the node itself is on the stack");
                        self.on_stack.remove(member);
                        component.insert(member.clone());
                        if member == id {
                            break;
                        }
                    }
                    if component.len() > 1 || wired_to_itself {
                        self.cycles.push(component);
                    }
                }
            }
        }

        let mut state = State {
            graph: self,
            next_index: 0,
            index: BTreeMap::new(),
            low_link: BTreeMap::new(),
            stack: Vec::new(),
            on_stack: BTreeSet::new(),
            cycles: Vec::new(),
        };
        for id in self.nodes.keys() {
            if !state.index.contains_key(id) {
                state.visit(id);
            }
        }
        state.cycles
    }
}

/// Something wrong with a graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// An input isn't wired to anything.
    Unwired { node: NodeId, input: &'static str },
    /// An input is wired to a node that isn't in the graph.
    MissingNode {
        node: NodeId,
        input: &'static str,
        missing: NodeId,
    },
    /// An input is wired to an output of a different type.
    TypeMismatch {
        node: NodeId,
        input: &'static str,
        expected: ValueType,
        found: ValueType,
    },
    /// These nodes are wired in a cycle, so each depends on its own output.
    Cycle { nodes: BTreeSet<NodeId> },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unwired { node, input } => write!(f, "{node}'s {input} input isn't wired"),
            Self::MissingNode {
                node,
                input,
                missing,
            } => write!(
                f,
                "{node}'s {input} input is wired to {missing}, which doesn't exist"
            ),
            Self::TypeMismatch {
                node,
                input,
                expected,
                found,
            } => write!(
                f,
                "{node}'s {input} input takes a {expected}, but is wired to a {found}"
            ),
            Self::Cycle { nodes } => {
                let nodes: Vec<&str> = nodes.iter().map(NodeId::as_str).collect();
                write!(f, "These nodes are wired in a cycle: {}", nodes.join(", "))
            }
        }
    }
}

impl std::error::Error for ValidationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        node::Node,
        testing::{gen_any_graph, gen_valid_graph},
    };
    use hegel::{TestCase, generators as gs};

    /// Every node whose value depends on `id`'s, including `id` itself.
    fn downstream(graph: &Graph, id: &NodeId) -> BTreeSet<NodeId> {
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
    fn upstream(graph: &Graph, id: &NodeId) -> BTreeSet<NodeId> {
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

    /// A node with at least one input, and the index of one of its inputs.
    fn draw_input(tc: &TestCase, graph: &Graph) -> (NodeId, usize) {
        let with_inputs: Vec<NodeId> = graph
            .nodes()
            .filter(|(_, n)| !n.node.inputs().is_empty())
            .map(|(id, _)| id.clone())
            .collect();
        tc.assume(!with_inputs.is_empty());
        let id = tc.draw(gs::sampled_from(with_inputs));
        let count = graph.get(&id).unwrap().node.inputs().len();
        let index = tc.draw(gs::integers::<usize>().max_value(count - 1));
        (id, index)
    }

    fn rewire(graph: &mut Graph, id: &NodeId, index: usize, wire: Option<NodeId>) {
        *graph
            .get_mut(id)
            .unwrap()
            .node
            .wires_mut()
            .swap_remove(index) = wire;
    }

    #[hegel::test(test_cases = 1_200)]
    fn graphs_round_trip_through_the_file_format(tc: TestCase) {
        let graph = tc.draw(gen_any_graph(false));
        let ron = graph.to_ron().unwrap();
        tc.note(&ron);
        assert_eq!(Graph::from_ron(&ron).unwrap(), graph);
    }

    #[hegel::test(test_cases = 1_000)]
    fn rewriting_a_graph_file_changes_nothing(tc: TestCase) {
        // Including NaN parameters, which aren't equal to themselves.
        let graph = tc.draw(gen_any_graph(true));
        let ron = graph.to_ron().unwrap();
        tc.note(&ron);
        assert_eq!(Graph::from_ron(&ron).unwrap().to_ron().unwrap(), ron);
    }

    #[hegel::test(test_cases = 10_000)]
    fn other_format_versions_are_rejected(tc: TestCase) {
        let version = tc.draw(gs::integers::<u32>());
        tc.assume(version != FORMAT_VERSION);
        let ron = format!("(version: {version}, nodes: {{}})");
        assert!(Graph::from_ron(&ron).is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        // A typo shouldn't be silently ignored, leaving a parameter at its default.
        let ron = r#"(version: 1, nodes: { "exposure_k3f9": (node: Exposure(stop: 1.0)) })"#;
        assert!(Graph::from_ron(ron).is_err());
        let ron = r#"(version: 1, nodes: { "exposure_k3f9": (node: Exposure(stops: 1.0)) })"#;
        assert!(Graph::from_ron(ron).is_ok());
    }

    #[test]
    fn generated_ids_name_the_node_type_and_are_unique() {
        let mut graph = Graph::new();
        for _ in 0..1_000 {
            for node in Node::defaults() {
                let prefix = node.id_prefix();
                let id = graph.add(node);
                let suffix = id.as_str().strip_prefix(prefix).unwrap().strip_prefix('_');
                let suffix = suffix.unwrap();
                assert_eq!(suffix.len(), 4, "{id}");
                assert!(
                    suffix
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                );
            }
        }
        assert_eq!(graph.len(), 1_000 * Node::defaults().len());
    }

    #[hegel::test(test_cases = 3_500)]
    fn valid_graphs_have_no_errors(tc: TestCase) {
        let graph = tc.draw(gen_valid_graph());
        assert_eq!(graph.validate(), []);
    }

    #[hegel::test(test_cases = 3_000)]
    fn unwiring_an_input_is_caught(tc: TestCase) {
        let mut graph = tc.draw(gen_valid_graph());
        let (id, index) = draw_input(&tc, &graph);
        let input = graph.get(&id).unwrap().node.inputs()[index].name;
        rewire(&mut graph, &id, index, None);
        assert_eq!(
            graph.validate(),
            [ValidationError::Unwired { node: id, input }]
        );
    }

    #[hegel::test(test_cases = 3_500)]
    fn removing_a_node_leaves_its_wires_dangling(tc: TestCase) {
        let mut graph = tc.draw(gen_valid_graph());
        let ids: Vec<NodeId> = graph.nodes().map(|(id, _)| id.clone()).collect();
        let removed = tc.draw(gs::sampled_from(ids));
        graph.remove(&removed);
        let mut expected = Vec::new();
        for (id, node) in graph.nodes() {
            for input in node.node.inputs() {
                if input.wire.as_ref() == Some(&removed) {
                    expected.push(ValidationError::MissingNode {
                        node: id.clone(),
                        input: input.name,
                        missing: removed.clone(),
                    });
                }
            }
        }
        assert_eq!(graph.validate(), expected);
    }

    #[hegel::test(test_cases = 2_000)]
    fn wiring_mismatched_types_is_caught(tc: TestCase) {
        let mut graph = tc.draw(gen_valid_graph());
        let (id, index) = draw_input(&tc, &graph);
        let expected = graph.get(&id).unwrap().node.inputs()[index].clone();
        // Upstream of the node is anything not downstream of it, so no cycle forms.
        let downstream = downstream(&graph, &id);
        let mismatched: Vec<NodeId> = graph
            .nodes()
            .filter(|(other, n)| {
                !downstream.contains(other) && n.node.output_type() != expected.value_type
            })
            .map(|(other, _)| other.clone())
            .collect();
        tc.assume(!mismatched.is_empty());
        let wire = tc.draw(gs::sampled_from(mismatched));
        let found = graph.get(&wire).unwrap().node.output_type();
        rewire(&mut graph, &id, index, Some(wire));
        assert_eq!(
            graph.validate(),
            [ValidationError::TypeMismatch {
                node: id,
                input: expected.name,
                expected: expected.value_type,
                found,
            }]
        );
    }

    #[hegel::test(test_cases = 3_000)]
    fn wiring_a_cycle_is_caught(tc: TestCase) {
        let mut graph = tc.draw(gen_valid_graph());
        let (id, index) = draw_input(&tc, &graph);
        // Wiring the node to anything downstream of it, or to itself, closes a cycle.
        let downstream = downstream(&graph, &id);
        let wire = tc.draw(gs::sampled_from(
            downstream.iter().cloned().collect::<Vec<_>>(),
        ));
        // The cycle is every node on a path from `id` to `wire`.
        let expected: BTreeSet<NodeId> = downstream
            .intersection(&upstream(&graph, &wire))
            .cloned()
            .collect();
        rewire(&mut graph, &id, index, Some(wire));
        let cycles: Vec<_> = graph
            .validate()
            .into_iter()
            .filter_map(|error| match error {
                ValidationError::Cycle { nodes } => Some(nodes),
                _ => None,
            })
            .collect();
        assert_eq!(cycles, [expected]);
    }
}
