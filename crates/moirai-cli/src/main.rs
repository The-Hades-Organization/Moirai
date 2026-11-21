use clap::Parser;

/// Moirai: estimativa e uso por janela dos agentes do Hades.
#[derive(Parser)]
#[command(name = "moirai", version, about)]
struct Cli {}

fn main() {
    Cli::parse();
}
