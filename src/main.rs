#[allow(unused)]

mod cli;
mod config;
mod core;
mod plugins;
mod storage;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use tokio::signal;

use crate::cli::Args;
use crate::config::load_config;
use crate::core::engine::{Engine, EngineConfig};
use crate::core::triggers::{Operator, Trigger};
use crate::plugins::protocol::hash_metric_id;


use metrics_exporter_prometheus::PrometheusBuilder;

use metrics::{counter, gauge};
use metrics_process::Collector;

#[tokio::main]
async fn main() -> ExitCode {

    PrometheusBuilder::new()
        .install()
        .expect("failed to install Prometheus recorder");
    counter!("unshroud_startup_total").increment(1);
    gauge!("unshroud_build_info", "version" => "0.1.0").set(1.0);

    let collector = Collector::default();
    collector.describe();
    tokio::spawn(async move {
        loop { collector.collect(); tokio::time::sleep(std::time::Duration::from_secs(1)).await; }
    });


    if let Err(e) = run().await {
        eprintln!("error: {}", e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn run() -> anyhow::Result<()> {
    let args = Args::parse();
    let config_path = resolve_absolute(&args.config);

    let cfg = load_config(config_path.to_str().context("invalid config path")?)
        .context("failed to load config")?;

    let triggers = build_triggers_from_config(&cfg);

    let engine_cfg = EngineConfig {
        poll_interval_ms: cfg.core.poll_interval_ms,
        buffer_capacity: cfg.core.buffer_capacity,
        event_capacity: 256,
        output_dir: cfg.core.output_dir,
        triggers,
        socket_path: "/run/unshroud/unshroud.sock".to_string(),
    };

    let mut engine = Engine::new(engine_cfg);

    tokio::select! {
        result = engine.run() => {
            result.map_err(|e| anyhow::anyhow!("engine runtime error: {}", e))?;
        }
        _ = signal::ctrl_c() => {
            eprintln!("[main] received SIGINT, shutting down");
        }
    }

    Ok(())
}

fn resolve_absolute(path: &PathBuf) -> PathBuf {
    if path.is_absolute() {
        path.clone()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    }
}

fn build_triggers_from_config(cfg: &crate::config::types::Config) -> Vec<Trigger> {
    let mut triggers = Vec::new();

    for (name, module_cfg) in &cfg.modules {
        if module_cfg.lifecycle == crate::config::types::Lifecycle::Persistent {
            triggers.push(Trigger {
                metric_id: hash_metric_id(&format!("plugin.{}.alive", name)),
                operator: Operator::Eq,
                threshold: 1.0,
                cooldown: Duration::from_secs(30),
            });
        }
    }

    triggers.push(Trigger {
        metric_id: hash_metric_id("internal.cpu.usage"),
        operator: Operator::Gt,
        threshold: 0.95,
        cooldown: Duration::from_secs(60),
    });

    triggers
}

#[cfg(test)]
    mod tests {
    use super::*;
    use crate::config::types::{Config, CoreConfig, ModuleConfig, Lifecycle};
    use std::collections::HashMap;

    #[test]
    fn test_resolve_absolute_with_relative_path() {
        let rel = PathBuf::from("config.toml");
        let abs = resolve_absolute(&rel);
        assert!(abs.is_absolute());
        assert!(abs.ends_with("config.toml"));
    }

    #[test]
    fn test_resolve_absolute_with_absolute_path() {
        let abs = PathBuf::from("/etc/unshroud/unshroud.toml");
        let result = resolve_absolute(&abs);
        assert_eq!(result, abs);
    }

    #[test]
    fn test_build_triggers_from_empty_config() {
        let cfg = Config {
            core: CoreConfig {
                poll_interval_ms: 1000,
                buffer_capacity: 1024,
                output_dir: PathBuf::from("/tmp"),
            },
            modules: HashMap::new(),
        };
        let triggers = build_triggers_from_config(&cfg);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].metric_id, hash_metric_id("internal.cpu.usage"));
    }

    #[test]
    fn test_build_triggers_with_persistent_modules() {
        let mut modules = HashMap::new();
        modules.insert(
            "netmon".to_string(),
            ModuleConfig {
                binary: PathBuf::from("/usr/bin/netmon"),
                memory_limit_mb: 64,
                lifecycle: Lifecycle::Persistent,
            },
        );
        modules.insert(
            "diskmon".to_string(),
            ModuleConfig {
                binary: PathBuf::from("/usr/bin/diskmon"),
                memory_limit_mb: 32,
                lifecycle: Lifecycle::Ephemeral,
            },
        );

        let cfg = Config {
            core: CoreConfig {
                poll_interval_ms: 1000,
                buffer_capacity: 1024,
                output_dir: PathBuf::from("/tmp"),
            },
            modules,
        };

        let triggers = build_triggers_from_config(&cfg);
        assert_eq!(triggers.len(), 2);

        let plugin_trigger = triggers.iter().find(|t| t.metric_id == hash_metric_id("plugin.netmon.alive"));
        assert!(plugin_trigger.is_some());
        assert_eq!(plugin_trigger.unwrap().cooldown, Duration::from_secs(30));
    }

    #[test]
    fn test_build_triggers_cpu_threshold() {
        let cfg = Config {
            core: CoreConfig {
                poll_interval_ms: 1000,
                buffer_capacity: 1024,
                output_dir: PathBuf::from("/tmp"),
            },
            modules: HashMap::new(),
        };
        let triggers = build_triggers_from_config(&cfg);
        let cpu_trigger = triggers.iter().find(|t| t.metric_id == hash_metric_id("internal.cpu.usage"));
        assert!(cpu_trigger.is_some());
        assert_eq!(cpu_trigger.unwrap().threshold, 0.95);
        assert_eq!(cpu_trigger.unwrap().cooldown, Duration::from_secs(60));
    }
    }