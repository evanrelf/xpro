mod app;

use clap::Parser as _;
use std::{
    path::{Path, PathBuf},
    time::Instant,
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
    let start = Instant::now();
    let raw = xpro_core::raw::decode(raw_path)?;
    tracing::info!(elapsed = ?start.elapsed(), "Decoded");

    let start = Instant::now();
    let image = xpro_core::develop::develop(&raw)?;
    tracing::info!(elapsed = ?start.elapsed(), "Developed");

    let start = Instant::now();
    let display = xpro_core::view::tone_map_to_srgb(&image, &xpro_core::view::ToneCurve::standard());
    tracing::info!(elapsed = ?start.elapsed(), "Applied view transform");

    let start = Instant::now();
    xpro_core::export::write_png(&display, png_path)?;
    tracing::info!(elapsed = ?start.elapsed(), "Wrote {}", png_path.display());

    Ok(())
}
