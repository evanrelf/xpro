use clap::Parser as _;

#[derive(clap::Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    Greet,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt::init();

    match &args.command {
        Command::Greet => tracing::info!("Hello, world!"),
    }

    Ok(())
}
