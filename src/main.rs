mod cli;
mod config;
mod core;


use std::path::{Path, PathBuf};
use config::load_config;

fn main() {
    let args = cli::parse();
    let config_path = resolve_absolute(&args.config);

    match load_config(config_path.to_str().unwrap()) {
        Ok(cfg) => {
            if cfg!(debug_assertions) { eprintln!("[cli] parsed args:\n{:#?}", args)}
            // TODO: engine::run(cfg, args.dry_run);
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

fn resolve_absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        cwd.join(path)
    }
}