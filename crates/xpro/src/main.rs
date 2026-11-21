mod app;

use clap::Parser as _;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(clap::Parser)]
#[command(args_conflicts_with_subcommands = true)]
struct Args {
    /// Raw photo to open
    photo: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Develop a raw photo into an image file
    Render {
        /// Raw photo to develop
        input: PathBuf,

        /// PNG file to write (must not already exist)
        #[arg(short, long)]
        output: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt::init();

    match args.command {
        Some(Command::Render { input, output }) => render(&input, &output),
        None => app::run(args.photo),
    }
}

fn render(input: &Path, output: &Path) -> anyhow::Result<()> {
    let start = Instant::now();
    let raw = xpro_core::raw::decode(input)?;
    tracing::info!(elapsed = ?start.elapsed(), "Decoded");

    let start = Instant::now();
    let image = xpro_core::develop::develop(&raw)?;
    tracing::info!(elapsed = ?start.elapsed(), "Developed");

    let start = Instant::now();
    let display = xpro_core::view::tone_map_to_srgb(&image, &xpro_core::view::ToneCurve::standard());
    tracing::info!(elapsed = ?start.elapsed(), "Applied view transform");

    let start = Instant::now();
    xpro_core::export::write_png(&display, output)?;
    tracing::info!(elapsed = ?start.elapsed(), "Wrote {}", output.display());

    Ok(())
}
