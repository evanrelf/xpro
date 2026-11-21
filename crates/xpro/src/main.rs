use clap::Parser as _;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(clap::Parser)]
struct Args {
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

    match &args.command {
        Some(Command::Render { input, output }) => render(input, output),
        None => run_gui(args),
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
    let display = xpro_core::view::clip_to_srgb(&image);
    tracing::info!(elapsed = ?start.elapsed(), "Applied view transform");

    let start = Instant::now();
    xpro_core::export::write_png(&display, output)?;
    tracing::info!(elapsed = ?start.elapsed(), "Wrote {}", output.display());

    Ok(())
}

fn run_gui(args: Args) -> anyhow::Result<()> {
    let mut state = State { counter: 0 };

    eframe::run_ui_native(
        "Xpro",
        eframe::NativeOptions::default(),
        move |ui, frame| run_ui(&args, &mut state, ui, frame).unwrap(),
    )?;

    Ok(())
}

struct State {
    counter: i32,
}

fn run_ui(
    _args: &Args,
    state: &mut State,
    ui: &mut egui::Ui,
    _frame: &mut eframe::Frame,
) -> anyhow::Result<()> {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("Hello, world!");

        ui.horizontal(|ui| {
            if ui.button("−").clicked() {
                state.counter -= 1;
            }
            ui.label(state.counter.to_string());
            if ui.button("+").clicked() {
                state.counter += 1;
            }
        });
    });

    Ok(())
}
