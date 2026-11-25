//! Edits: graphs with an input bound to an image file, stored in edit files next to them.

use crate::{
    eval::{IMAGE_INPUT, Input, Inputs, render},
    graph::{FORMAT_VERSION, Format, Graph},
    node::{Node, default_graph},
};
use anyhow::{Context as _, bail};
use std::{
    ffi::OsString,
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

/// The extension of graph files, as in `portra-look.xpro`.
pub const GRAPH_EXTENSION: &str = "xpro";

/// The extension of edit files, added to the full filename of the image file they edit, as in
/// `DSC_4362.NEF.xpro-edit`.
pub const EDIT_EXTENSION: &str = "xpro-edit";

/// A graph with one of its inputs bound to an image file: the one its edit file is next to.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    /// The name of the graph input bound to the image file.
    pub input: String,
    pub graph: Graph,
}

impl Edit {
    /// An edit whose graph's [`IMAGE_INPUT`] is bound to the image file.
    #[must_use]
    pub fn new(graph: Graph) -> Self {
        Self {
            input: IMAGE_INPUT.to_owned(),
            graph,
        }
    }

    /// The edit for an image file: the one in its edit file if it has one, otherwise a new edit
    /// of the [default graph](default_graph).
    pub fn for_image(image_path: &Path) -> anyhow::Result<Self> {
        Ok(Self::read_for(image_path)?.unwrap_or_else(|| Self::new(default_graph())))
    }

    /// Reads a graph to apply to an image file: either an edit file, whose image file's input is
    /// bound to the new image file instead, or a graph file, whose [`IMAGE_INPUT`] is.
    pub fn read_graph_or_edit(path: &Path) -> anyhow::Result<Self> {
        if path
            .extension()
            .is_some_and(|extension| extension == EDIT_EXTENSION)
        {
            Self::read(path)
        } else {
            Graph::read(path).map(Self::new)
        }
    }

    /// Makes the graph's only output node write to `path`, a path template like
    /// `{image}-small.png`. Fails if the graph has more than one output node, or none.
    pub fn set_output_path(&mut self, path: String) -> anyhow::Result<()> {
        let mut outputs = self
            .graph
            .nodes_mut()
            .filter(|(_, node)| node.node.is_output());
        let (Some((_, output)), None) = (outputs.next(), outputs.next()) else {
            bail!("Can only choose the output path of a graph with exactly one output node");
        };
        match &mut output.node {
            Node::WritePng(output) => output.path = path,
            node => bail!(
                "Can't choose the output path of a {} node",
                node.type_name()
            ),
        }
        Ok(())
    }

    /// Renders an image file with this edit: evaluates every output node, and executes the
    /// actions they describe, resolving relative paths against the image file's directory.
    /// Writes nothing if the graph has errors, or if any file it writes already exists. Returns
    /// the paths written.
    pub fn render(&self, image_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
        let dir = image_path.parent().unwrap_or(Path::new(""));
        render(&self.graph, &self.inputs(image_path), dir)
    }

    /// The path of the edit file for an image file: its full filename plus `.xpro-edit`, so a
    /// raw file and a JPEG with the same name get separate edit files.
    #[must_use]
    pub fn path_for(image_path: &Path) -> PathBuf {
        let mut path = OsString::from(image_path);
        path.push(".");
        path.push(EDIT_EXTENSION);
        PathBuf::from(path)
    }

    /// The graph's inputs, with this edit's input bound to `image_path`.
    #[must_use]
    pub fn inputs(&self, image_path: &Path) -> Inputs {
        Inputs::new().bind(self.input.clone(), Input::Path(image_path.to_owned()))
    }

    /// Writes the edit in the edit file format: like a graph file, plus the name of the input
    /// bound to the image file.
    pub fn to_ron(&self) -> anyhow::Result<String> {
        Format {
            version: FORMAT_VERSION,
            input: Some(self.input.clone()),
            nodes: self.graph.clone().into_nodes(),
        }
        .to_ron()
    }

    /// Reads an edit written in the edit file format.
    pub fn from_ron(ron: &str) -> anyhow::Result<Self> {
        let format = Format::from_ron(ron).context("Failed to read edit")?;
        let Some(input) = format.input else {
            bail!("Expected an edit, but found a graph with no input bound to an image file");
        };
        Ok(Self {
            input,
            graph: Graph::from_nodes(format.nodes),
        })
    }

    /// Reads an edit file.
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let ron = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        Self::from_ron(&ron).with_context(|| format!("Failed to read {}", path.display()))
    }

    /// Reads the edit file for an image file, if it has one.
    pub fn read_for(image_path: &Path) -> anyhow::Result<Option<Self>> {
        let path = Self::path_for(image_path);
        match fs::read_to_string(&path) {
            Ok(ron) => Self::from_ron(&ron)
                .map(Some)
                .with_context(|| format!("Failed to read {}", path.display())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("Failed to read {}", path.display())),
        }
    }

    /// Writes an edit file, replacing any that's already there. The file is replaced all at
    /// once, so a crash midway leaves the old edit file intact.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let ron = self.to_ron()?;
        let dir = path.parent().filter(|dir| !dir.as_os_str().is_empty());
        let dir = dir.unwrap_or(Path::new("."));
        let context = || format!("Failed to write {}", path.display());
        let mut temporary = tempfile::NamedTempFile::new_in(dir).with_context(context)?;
        temporary.write_all(ron.as_bytes()).with_context(context)?;
        temporary.as_file().sync_all().with_context(context)?;
        temporary.persist(path).with_context(context)?;
        Ok(())
    }

    /// Writes a new edit file, failing if there's already one at `path`.
    pub fn save_new(&self, path: &Path) -> anyhow::Result<()> {
        let ron = self.to_ron()?;
        let context = || format!("Failed to create {}", path.display());
        let mut file = fs::File::create_new(path).with_context(context)?;
        file.write_all(ron.as_bytes()).with_context(context)?;
        file.sync_all().with_context(context)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        graph::NodeId,
        node::{Exposure, Ports as _, WritePng},
        testing::gen_any_graph,
    };
    use hegel::{TestCase, generators as gs};

    #[hegel::composite]
    fn gen_edit(tc: &TestCase) -> Edit {
        Edit {
            input: tc.draw(gs::text()),
            graph: tc.draw(gen_any_graph(false)),
        }
    }

    /// A file name, with or without an extension.
    fn gen_file_name() -> impl gs::PrintableGenerator<String> {
        gs::text().alphabet("abcXYZ_-. 0123").min_size(1)
    }

    #[hegel::test(test_cases = 1_200)]
    fn edits_round_trip_through_the_file_format(tc: TestCase) {
        let edit = tc.draw(gen_edit());
        let ron = edit.to_ron().unwrap();
        tc.note(&ron);
        assert_eq!(Edit::from_ron(&ron).unwrap(), edit);
    }

    #[hegel::test(test_cases = 600)]
    fn edits_and_graphs_arent_mistaken_for_each_other(tc: TestCase) {
        let edit = tc.draw(gen_edit());
        assert!(Graph::from_ron(&edit.to_ron().unwrap()).is_err());
        assert!(Edit::from_ron(&edit.graph.to_ron().unwrap()).is_err());
    }

    #[hegel::test(test_cases = 10_000)]
    fn an_edit_file_is_named_after_the_image_files_full_name(tc: TestCase) {
        let dir = tc.draw(gs::sampled_from(vec!["", "photos", "/a/b"]));
        let name = tc.draw(gen_file_name());
        tc.assume(!matches!(name.as_str(), "." | ".."));
        let image_path = Path::new(dir).join(&name);
        let edit_path = Edit::path_for(&image_path);
        assert_eq!(edit_path.parent(), image_path.parent());
        let expected = format!("{name}.{EDIT_EXTENSION}");
        assert_eq!(
            edit_path.file_name().unwrap().to_str(),
            Some(expected.as_str())
        );
    }

    #[hegel::test(test_cases = 3_000)]
    fn image_files_with_different_names_get_different_edit_files(tc: TestCase) {
        // Such as a raw file and a JPEG with the same name.
        let a = tc.draw(gen_file_name());
        let b = tc.draw(gen_file_name());
        tc.assume(a != b);
        assert_ne!(Edit::path_for(Path::new(&a)), Edit::path_for(Path::new(&b)));
    }

    #[hegel::test(test_cases = 15)]
    fn saving_replaces_the_edit_file(tc: TestCase) {
        let first = tc.draw(gen_edit());
        let second = tc.draw(gen_edit());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("DSC_4362.NEF.xpro-edit");
        first.save(&path).unwrap();
        assert_eq!(Edit::read(&path).unwrap(), first);
        second.save(&path).unwrap();
        assert_eq!(Edit::read(&path).unwrap(), second);
        // No temporary files are left behind.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn starting_an_edit_file_never_overwrites_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("DSC_4362.NEF.xpro-edit");
        fs::write(&path, "precious").unwrap();
        assert!(Edit::new(default_graph()).save_new(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "precious");
    }

    #[test]
    fn an_image_file_without_an_edit_file_gets_the_default_graph() {
        let dir = tempfile::tempdir().unwrap();
        let image_path = dir.path().join("DSC_4362.NEF");
        let edit = Edit::for_image(&image_path).unwrap();
        assert_eq!(edit.input, IMAGE_INPUT);
        // Node IDs are random, so compare the nodes in order of their types.
        let types = |graph: &Graph| {
            let mut types: Vec<_> = graph.nodes().map(|(_, n)| n.node.type_name()).collect();
            types.sort_unstable();
            types
        };
        assert_eq!(types(&edit.graph), types(&default_graph()));

        let mut graph = Graph::new();
        graph.add(Exposure::default());
        Edit::new(graph.clone())
            .save_new(&Edit::path_for(&image_path))
            .unwrap();
        assert_eq!(Edit::for_image(&image_path).unwrap().graph, graph);
    }

    #[test]
    fn a_graph_file_or_an_edit_file_can_be_applied() {
        let dir = tempfile::tempdir().unwrap();
        let mut graph = Graph::new();
        graph.add(Exposure::default());

        let graph_path = dir.path().join(format!("look.{GRAPH_EXTENSION}"));
        fs::write(&graph_path, graph.to_ron().unwrap()).unwrap();
        assert_eq!(
            Edit::read_graph_or_edit(&graph_path).unwrap(),
            Edit::new(graph.clone())
        );

        let edit = Edit {
            input: "photo".to_owned(),
            graph,
        };
        let edit_path = Edit::path_for(&dir.path().join("DSC_4362.NEF"));
        edit.save_new(&edit_path).unwrap();
        assert_eq!(Edit::read_graph_or_edit(&edit_path).unwrap(), edit);
    }

    #[hegel::test(test_cases = 3_000)]
    fn the_output_path_can_be_set_only_with_one_output_node(tc: TestCase) {
        let outputs = tc.draw(gs::integers::<usize>().max_value(3));
        let others = tc.draw(gs::integers::<usize>().max_value(3));
        let path = tc.draw(gs::text());
        let mut graph = Graph::new();
        for _ in 0..outputs {
            graph.add(WritePng::default());
        }
        for _ in 0..others {
            graph.add(Exposure::default());
        }
        let mut edit = Edit::new(graph);
        let result = edit.set_output_path(path.clone());
        if outputs == 1 {
            result.unwrap();
            let paths: Vec<&str> = edit
                .graph
                .nodes()
                .filter_map(|(_, node)| match &node.node {
                    Node::WritePng(output) => Some(output.path.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(paths, [path.as_str()]);
        } else {
            assert!(result.is_err());
        }
    }

    #[test]
    fn rendering_a_graph_with_errors_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut graph = default_graph();
        let id = graph
            .nodes()
            .find(|(_, node)| matches!(node.node, Node::Exposure(_)))
            .map(|(id, _)| id.clone())
            .unwrap();
        *graph.get_mut(&id).unwrap().node.wires_mut()[0] = Some(NodeId::new("missing"));
        let image_path = dir.path().join("DSC_4362.NEF");
        assert!(Edit::new(graph).render(&image_path).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
