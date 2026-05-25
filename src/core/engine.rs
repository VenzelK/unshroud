use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::time::interval;

use crate::core::state::{CoreState, SharedState};
use crate::core::triggers::{Trigger, TriggerEngine, TriggerAction};
use crate::storage::bundle::BundleBuilder;
use crate::plugins::protocol::hash_metric_id;

pub struct EngineConfig {
    pub poll_interval_ms: u64,
    pub buffer_capacity: usize,
    pub event_capacity: usize,
    pub output_dir: PathBuf,
    pub triggers: Vec<Trigger>,
    pub socket_path: String,
}

pub struct Engine {
    state: SharedState,
    triggers: TriggerEngine,
    bundle: BundleBuilder,
    poll_interval: Duration,
}

impl Engine {
    pub fn new(cfg: EngineConfig) -> Self {
        let state = Arc::new(std::sync::Mutex::new(
            CoreState::new(cfg.buffer_capacity, cfg.event_capacity)
        ));
        Self {
            state: state.clone(),
            triggers: TriggerEngine::new(cfg.triggers),
            bundle: BundleBuilder::new(&cfg.output_dir),
            poll_interval: Duration::from_millis(cfg.poll_interval_ms),
        }
    }

    pub fn for_test(state: SharedState, triggers: Vec<Trigger>, output_dir: &PathBuf) -> Self {
        Self {
            state,
            triggers: TriggerEngine::new(triggers),
            bundle: BundleBuilder::new(output_dir),
            poll_interval: Duration::from_millis(100),
        }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut ticker = interval(self.poll_interval);
        let state_clone = self.state.clone();

        let listener_task = tokio::spawn(async move {
            crate::plugins::start_listener("/tmp/unshroud.sock", state_clone).await
        });

        let collector_state = self.state.clone();
        let collector_task = tokio::spawn(async move {
            crate::plugins::run_cpu_collector(collector_state, 1000).await;
        });

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    self.process_cycle().await;
                }
                _ = signal::ctrl_c() => {
                    eprintln!("[engine] shutdown signal received");
                    break;
                }
            }
        }

        listener_task.abort();
        collector_task.abort();
        Ok(())
    }

    async fn process_cycle(&mut self) {
        let metrics = {
            let mut guard = self.state.lock().unwrap();
            guard.metrics.drain()
        };

        for point in &metrics {
            if let Some(value) = point.as_float() {
                if let Some(action) = self.triggers.check(point.metric_id, value) {
                    self.handle_trigger(action, &metrics).await;
                }
            }
        }
    }

    async fn handle_trigger(&self, _action: TriggerAction, metrics: &[crate::core::buffer::MetricPoint]) {
        let events = {
            let mut guard = self.state.lock().unwrap();
            guard.events.drain()
        };

        if let Err(e) = self.bundle.dump(metrics, &events.iter().map(|s| s.as_str()).collect::<Vec<_>>()) {
            eprintln!("[engine] bundle error: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::buffer::MetricPoint;
    use crate::core::triggers::Operator;
    use std::fs;

    fn test_output_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "unshroud_engine_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn test_engine_constructor() {
        let cfg = EngineConfig {
            poll_interval_ms: 100,
            buffer_capacity: 64,
            event_capacity: 16,
            output_dir: test_output_dir(),
            triggers: vec![],
            socket_path: "/tmp/test.sock".to_string(),
        };
        let engine = Engine::new(cfg);
        assert_eq!(engine.poll_interval, Duration::from_millis(100));
    }

    #[tokio::test]
    async fn test_process_cycle_no_triggers() {
        let state: SharedState = Arc::new(std::sync::Mutex::new(
            CoreState::new(64, 16)
        ));
        let output_dir = test_output_dir();
        fs::create_dir_all(&output_dir).unwrap();

        let mut engine = Engine::for_test(state.clone(), vec![], &output_dir);

        {
            let mut guard = state.lock().unwrap();
            guard.metrics.push(MetricPoint::new_float(0, hash_metric_id("test"), 0.5));
        }

        engine.process_cycle().await;

        let guard = state.lock().unwrap();
        assert_eq!(guard.metrics.len(), 0);
        fs::remove_dir_all(&output_dir).unwrap();
    }

    #[tokio::test]
    async fn test_process_cycle_with_trigger() {
        let state: SharedState = Arc::new(std::sync::Mutex::new(
            CoreState::new(64, 16)
        ));
        let output_dir = test_output_dir();
        fs::create_dir_all(&output_dir).unwrap();

        let trigger = Trigger {
            metric_id: hash_metric_id("cpu"),
            operator: Operator::Gt,
            threshold: 0.8,
            cooldown: Duration::from_millis(0),
        };

        let mut engine = Engine::for_test(state.clone(), vec![trigger], &output_dir);

        {
            let mut guard = state.lock().unwrap();
            guard.metrics.push(MetricPoint::new_float(0, hash_metric_id("cpu"), 0.95));
            guard.events.push("high load detected");
        }

        engine.process_cycle().await;

        let bundles: Vec<_> = fs::read_dir(&output_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |ext| ext == "zst"))
            .collect();

        assert_eq!(bundles.len(), 1);
        fs::remove_dir_all(&output_dir).unwrap();
    }

    #[tokio::test]
    async fn test_handle_trigger_bundle_creation() {
        let state: SharedState = Arc::new(std::sync::Mutex::new(
            CoreState::new(64, 16)
        ));
        let output_dir = test_output_dir();
        fs::create_dir_all(&output_dir).unwrap();

        let engine = Engine::for_test(state.clone(), vec![], &output_dir);

        let metrics = vec![MetricPoint::new_float(0, 123, 42.0)];
        let action = TriggerAction {
            metric_id: 123,
            value: 42.0,
        };

        engine.handle_trigger(action, &metrics).await;

        let bundles: Vec<_> = fs::read_dir(&output_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |ext| ext == "zst"))
            .collect();

        assert_eq!(bundles.len(), 1);
        fs::remove_dir_all(&output_dir).unwrap();
    }

    #[tokio::test]
    async fn test_trigger_cooldown_integration() {
        let state: SharedState = Arc::new(std::sync::Mutex::new(
            CoreState::new(64, 16)
        ));
        let output_dir = test_output_dir();
        fs::create_dir_all(&output_dir).unwrap();

        let trigger = Trigger {
            metric_id: hash_metric_id("mem"),
            operator: Operator::Lt,
            threshold: 0.2,
            cooldown: Duration::from_millis(50),
        };

        let mut engine = Engine::for_test(state.clone(), vec![trigger], &output_dir);

        {
            let mut guard = state.lock().unwrap();
            guard.metrics.push(MetricPoint::new_float(0, hash_metric_id("mem"), 0.1));
        }

        engine.process_cycle().await;
        engine.process_cycle().await;

        let bundles: Vec<_> = fs::read_dir(&output_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |ext| ext == "zst"))
            .collect();

        assert_eq!(bundles.len(), 1);
        fs::remove_dir_all(&output_dir).unwrap();
    }
}