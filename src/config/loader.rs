use std::fs;
use thiserror::Error;
use crate::config::types::Config;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found: {path}")]
    NotFound { path: String },
    
    #[error("read config error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("error when parsing TOML: {0}")]
    Parse(#[from] toml::de::Error),
}

pub fn load_config(path: &str) -> Result<Config, ConfigError> {
    let content = fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ConfigError::NotFound { path: path.to_string() },
        _ => ConfigError::Io(e),
    })?;

    let config: Config = toml::from_str(&content)?;
    Ok(config)
}