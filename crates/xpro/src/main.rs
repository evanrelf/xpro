mod app;

use clap::Parser as _;
use anyhow::Context as _;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use xpro_core::{
    eval::{Input, Inputs, escape_path_template, evaluate_actions, execute},
    node::{Node, default_graph},
};

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
    /// Develop a raw file into a PNG file
    Render {
        /// Raw file to develop
        #[arg(value_name = "RAW_FILE")]
        path: PathBuf,

        /// PNG file to write (must not already exist)
        #[arg(short, long, value_name = "PNG_FILE")]
        output: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt::init();

    match args.command {
        Some(Command::Render { path, output }) => render(&path, &output),
        None => app::run(args.path),
    }
}

fn render(raw_path: &Path, png_path: &Path) -> anyhow::Result<()> {
    let mut graph = default_graph();
    let png_path = png_path
        .to_str()
        .context("The PNG file's path must be valid UTF-8")?;
    for (_, node) in graph.nodes_mut() {
        if let Node::WritePng(output) = &mut node.node {
            output.path = escape_path_template(png_path);
        }
    }
    let inputs = Inputs::new().bind("image", Input::Path(raw_path.to_owned()));

    let start = Instant::now();
    let actions = evaluate_actions(&graph, &inputs)?;
    tracing::info!(elapsed = ?start.elapsed(), "Evaluated");

    let start = Instant::now();
    execute(&actions, Path::new("."))?;
    tracing::info!(elapsed = ?start.elapsed(), "Wrote {png_path}");

    Ok(())
}
