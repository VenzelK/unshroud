use clap::Parser;
use std::path::PathBuf;


/*
* TODO:
* RUN only cli (wothour cfg file)
* enable test run (check cfgs and plugins) 
*/
#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Args {
    #[arg(
        short = 'c',
        long,
        env = "UNSHROUD_CONFIG",
        default_value = "/etc/unshroud/unshroud.toml",
        value_name = "FILE"
    )]
    pub config: PathBuf,

    #[arg(long)]
    pub dry_run: bool,
}

pub fn parse() -> Args {
    let args = Args::parse();

    args
}