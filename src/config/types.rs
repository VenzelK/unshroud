use std::collections::HashMap;
use std::path::PathBuf;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub core: CoreConfig,
    #[serde(default)]
    pub modules: HashMap<String, ModuleConfig>,
}

#[derive(Debug, Deserialize)]
pub struct CoreConfig {
    #[serde(default = "defaults::poll_interval_ms")]
    pub poll_interval_ms: u64,

    #[serde(default = "defaults::buffer_capacity")]
    pub buffer_capacity: usize,

    #[serde(default = "defaults::output_dir")]
    pub output_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct ModuleConfig {
    pub binary: PathBuf,

    #[serde(default = "defaults::memory_limit_mb")]
    pub memory_limit_mb: u64,

    #[serde(default)]
    pub lifecycle: Lifecycle,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Ephemeral,
    Persistent,
}

mod defaults {
    use std::path::PathBuf;
    pub fn poll_interval_ms() -> u64 { 1000 }
    pub fn buffer_capacity() -> usize { 1024 }
    pub fn output_dir() -> PathBuf { PathBuf::from("/var/lib/ada") }
    pub fn memory_limit_mb() -> u64 { 64 }
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self::Ephemeral
    }
}