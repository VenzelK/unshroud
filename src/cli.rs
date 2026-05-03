use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "ada", about = "Autonomous Diagnostic Agent", version)]
pub struct Args {
    #[arg(
        short = 'c',
        long,
        env = "ADA_CONFIG",
        default_value = "/etc/ada/ada.toml",
        value_name = "FILE"
    )]
    pub config: PathBuf,

    #[arg(long)]
    pub dry_run: bool,
}

pub fn parse() -> Args {
    let args = Args::parse();

    if cfg!(debug_assertions) {
        eprintln!("[cli] parsed args:\n{:#?}", args);
    }

    args
}