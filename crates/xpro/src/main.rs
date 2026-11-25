mod app;

use anyhow::{Context as _, bail};
use clap::Parser as _;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use xpro_core::{edit::Edit, node::default_graph, path_template::escape_path_template};

#[derive(clap::Parser)]
#[command(args_conflicts_with_subcommands = true)]
struct Args {
    /// Raw file to open
    #[arg(value_name = "RAW_FILE")]
    path: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Render image files with their edits, writing the files their output nodes describe
    ///
    /// Each image file is rendered with its edit file if it has one, and otherwise with the
    /// default graph. Relative output paths are next to the image file. Existing files are
    /// never overwritten.
    Render {
        /// Image files to render
        #[arg(value_name = "IMAGE_FILE", required = true)]
        paths: Vec<PathBuf>,

        /// Render with this graph file or edit file instead
        #[arg(short, long, value_name = "FILE")]
        graph: Option<PathBuf>,

        /// Where the graph's only output node writes, relative to the current directory.
        /// `{image}` stands for the image file's name without its extension
        #[arg(short, long, value_name = "PATH")]
        output: Option<String>,
    },
    /// Start an edit file for an image file, to edit by hand
    NewEdit {
        /// Image file to edit
        #[arg(value_name = "IMAGE_FILE")]
        path: PathBuf,

        /// Start from this graph file or edit file, instead of the default graph
        #[arg(short, long, value_name = "FILE")]
        graph: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt::init();

    match args.command {
        Some(Command::Render {
            paths,
            graph,
            output,
        }) => render(&paths, graph.as_deref(), output),
        Some(Command::NewEdit { path, graph }) => new_edit(&path, graph.as_deref()),
        None => app::run(args.path),
    }
}

fn render(
    image_paths: &[PathBuf],
    graph_path: Option<&Path>,
    output: Option<String>,
) -> anyhow::Result<()> {
    let graph = graph_path.map(Edit::read_graph_or_edit).transpose()?;
    // Relative to the current directory, not to each image file's.
    let output = match output {
        Some(output) if Path::new(&output).is_relative() => {
            let dir = std::env::current_dir()?;
            let dir = dir.to_str().context("The current directory must be valid UTF-8")?;
            Some(format!("{}/{output}", escape_path_template(dir)))
        }
        output => output,
    };
    let mut failures = 0;
    for image_path in image_paths {
        let start = Instant::now();
        let result = (|| {
            let mut edit = match &graph {
                Some(graph) => graph.clone(),
                None => Edit::for_image(image_path)?,
            };
            if let Some(output) = &output {
                edit.set_output_path(output.clone())?;
            }
            edit.render(image_path)
        })();
        match result {
            Ok(written) => {
                for path in written {
                    tracing::info!(elapsed = ?start.elapsed(), "Wrote {}", path.display());
                }
            }
            Err(error) => {
                failures += 1;
                eprintln!("Failed to render {}: {error:#}", image_path.display());
            }
        }
    }
    if failures > 0 {
        bail!("Failed to render {failures} of {} image files", image_paths.len());
    }
    Ok(())
}

fn new_edit(image_path: &Path, graph_path: Option<&Path>) -> anyhow::Result<()> {
    let edit = match graph_path {
        Some(graph_path) => Edit::read_graph_or_edit(graph_path)?,
        None => Edit::new(default_graph()),
    };
    let edit_path = Edit::path_for(image_path);
    edit.save_new(&edit_path)?;
    println!("{}", edit_path.display());
    Ok(())
}
