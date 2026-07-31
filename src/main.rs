#[allow(unused)]

mod cli;
mod config;
mod core;
mod plugins;
mod storage;
mod logger;
mod metrics;

use std::path::PathBuf;
use std::process::ExitCode;
#[cfg(debug_assertions)]
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;

use crate::cli::Args;
use crate::config::load_config;
use crate::core::engine::{Engine, EngineConfig};
use crate::core::state::CoreState;
use crate::core::triggers::{Operator, Trigger};
use crate::plugins::protocol::hash_metric_id;

use metrics_exporter_prometheus::PrometheusBuilder;
use metrics_process::Collector;

#[tokio::main]
async fn main() -> ExitCode {

    logger::init();

    PrometheusBuilder::new()
    .install()
    .expect("Prometheus recorder failed");

    crate::metrics::init();

    info_log!("unshroud daemon starting (version {})", env!("CARGO_PKG_VERSION"));

    metrics::runtime::set_build_info(env!("CARGO_PKG_VERSION"));

    let collector = Collector::default();
    collector.describe();
    tokio::spawn(async move {
        loop { collector.collect(); tokio::time::sleep(std::time::Duration::from_secs(1)).await; }
    });


    if let Err(e) = run().await {
        error_log!("startup FAILED: {}", e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn run() -> anyhow::Result<()> {
    debug_log!("[run] entering run()");

    let args = Args::parse();
    let config_path = resolve_absolute(&args.config);

    let cfg = load_config(config_path.to_str().context("invalid config path")?)
        .context("failed to load config")?;

    let triggers = build_triggers_from_config(&cfg);



    let start_time = std::time::Instant::now();
    let dump_dir = cfg.core.output_dir.clone();

    let state = Arc::new(std::sync::Mutex::new(
            CoreState::new(cfg.core.buffer_capacity, 256)
        ));

    let engine_cfg = EngineConfig {
        poll_interval_ms: cfg.core.poll_interval_ms,
        output_dir: cfg.core.output_dir,
        triggers,
        socket_path: cfg.core.socket_path,
        lua_triggers_dir: cfg.core.lua_triggers_dir,
    };

    #[cfg(debug_assertions)]
    spawn_debug_dumper(state.clone(), dump_dir, start_time);

    
    let mut engine = Engine::new(engine_cfg, state)
        .context("Failed to initialize engine")?;

    let _ = engine.run().await;

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
                lua_script: None,
                cooldown: Duration::from_secs(30),
            });
        }
    }

    // NOT IMPLEMENT YET
    triggers.push(Trigger {
        metric_id: hash_metric_id("internal.cpu.usage"),
        operator: Operator::Gt,
        threshold: 0.95,
        lua_script: None,
        cooldown: Duration::from_secs(60),
    });
    triggers
}

#[cfg(debug_assertions)]
fn spawn_debug_dumper(
    state: Arc<std::sync::Mutex<crate::core::state::CoreState>>,
    dump_dir: std::path::PathBuf,
    start_time: std::time::Instant,
) {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sig = signal(SignalKind::user_defined1()).expect("Failed to bind SIGUSR1");

    tokio::spawn(async move {
        while sig.recv().await.is_some() {
            let guard = state.lock().unwrap();
            
            let snapshot = serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "uptime_sec": start_time.elapsed().as_secs(),
                "dump_timestamp": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                "core_state": {
                    "registered_metrics": guard.metric_names.len(),
                    "metric_registry": guard.metric_names.clone(),
                    "metrics_in_buffer": guard.metrics.len(),
                    "buffer_capacity": guard.metrics.capacity(),
                    "base_unix_time": guard.base_time,
                    "buffer_occupancy_pct": if guard.metrics.capacity() > 0 {
                        (guard.metrics.len() as f64 / guard.metrics.capacity() as f64 * 100.0).round()
                    } else { 0.0 },
                }
            });
            drop(guard);

            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let path = dump_dir.join(format!("debug_state_{}.json", ts));
            
            match serde_json::to_string_pretty(&snapshot) {
                Ok(json) => {
                    if std::fs::write(&path, json).is_ok() {
                        crate::info_log!("🐛 [DEBUG] Full state dumped to {}", path.display());
                    } else {
                        crate::debug_log!("🐛 [DEBUG] Failed to write state dump");
                    }
                }
                Err(e) => error_log!("[DEBUG] JSON serialization error: {}", e),
            }
        }
    });
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
            lua_triggers_dir: PathBuf::new(),
            socket_path: "/tmp/test.sock".to_string(),
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
                lua_triggers_dir: PathBuf::new(),
                socket_path: "/tmp/test.sock".to_string(),
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
                lua_triggers_dir: PathBuf::new(),
                socket_path: "/tmp/test.sock".to_string(),
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