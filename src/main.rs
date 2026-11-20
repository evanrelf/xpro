use clap::Parser as _;

#[derive(clap::Parser)]
struct Args {}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt::init();

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
