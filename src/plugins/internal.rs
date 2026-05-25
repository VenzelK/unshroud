use crate::core::state::SharedState;
use crate::plugins::protocol::{MetricPayload, PluginMessage};
use tokio::time::{interval, Duration};
use std::fs;
use std::sync::Arc;
use std::sync::Mutex;
use crate::core::state::CoreState;

#[derive(Clone, Copy, Default)]
struct CpuTicks {
    user: u64, nice: u64, system: u64, idle: u64,
    iowait: u64, irq: u64, softirq: u64, steal: u64,
}

pub async fn run_cpu_collector(state: SharedState, interval_ms: u64) {
    let mut prev: Option<CpuTicks> = None;
    let mut tick = interval(Duration::from_millis(interval_ms));

    loop {
        tick.tick().await;
        if let Ok(content) = fs::read_to_string("/proc/stat") {
            if let Some(line) = content.lines().find(|l| l.starts_with("cpu ")) {
                let curr = parse_ticks(line);
                if let Some(p) = prev {
                    let usage = calculate_usage(&p, &curr);
                    route_cpu_metric(&state, usage);
                }
                prev = Some(curr);
            }
        }
    }
}

fn parse_ticks(line: &str) -> CpuTicks {
    let mut parts = line.split_whitespace();
    parts.next();
    let mut next = || parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    CpuTicks {
        user: next(), nice: next(), system: next(), idle: next(),
        iowait: next(), irq: next(), softirq: next(), steal: next(),
    }
}

fn calculate_usage(prev: &CpuTicks, curr: &CpuTicks) -> f64 {
    let total = (curr.user - prev.user) + (curr.nice - prev.nice) + (curr.system - prev.system)
              + (curr.idle - prev.idle) + (curr.iowait - prev.iowait) + (curr.irq - prev.irq)
              + (curr.softirq - prev.softirq) + (curr.steal - prev.steal);
    let idle = (curr.idle - prev.idle) + (curr.iowait - prev.iowait);
    if total == 0 { 0.0 } else { 1.0 - (idle as f64 / total as f64) }
}

fn route_cpu_metric(state: &SharedState, usage: f64) {
    if let Ok(mut guard) = state.lock() {
        PluginMessage::Metric(MetricPayload {
            id: "internal.cpu.usage".to_string(),
            value: Some(usage),
        }).route(&mut guard);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::buffer::MetricType;
    use crate::plugins::protocol::hash_metric_id;

    #[test]
    fn test_parse_ticks() {
        let line = "cpu  100 20 30 500 10 5 2 1 0 0";
        let ticks = parse_ticks(line);
        assert_eq!(ticks.user, 100);
        assert_eq!(ticks.nice, 20);
        assert_eq!(ticks.system, 30);
        assert_eq!(ticks.idle, 500);
        assert_eq!(ticks.iowait, 10);
        assert_eq!(ticks.irq, 5);
        assert_eq!(ticks.softirq, 2);
        assert_eq!(ticks.steal, 1);
    }

    #[test]
    fn test_parse_ticks_missing_values() {
        let line = "cpu  100 20";
        let ticks = parse_ticks(line);
        assert_eq!(ticks.user, 100);
        assert_eq!(ticks.idle, 0);
    }

    #[test]
    fn test_calculate_usage() {
        let prev = CpuTicks { user: 100, nice: 0, system: 50, idle: 850, iowait: 0, irq: 0, softirq: 0, steal: 0 };
        let curr = CpuTicks { user: 120, nice: 0, system: 60, idle: 920, iowait: 0, irq: 0, softirq: 0, steal: 0 };
        let usage = calculate_usage(&prev, &curr);
        assert!((usage - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn test_calculate_usage_zero_delta() {
        let ticks = CpuTicks::default();
        let usage = calculate_usage(&ticks, &ticks);
        assert_eq!(usage, 0.0);
    }

    #[test]
    fn test_route_via_plugin_api() {
        let state: SharedState = Arc::new(Mutex::new(CoreState::new(64, 16)));
        route_cpu_metric(&state, 0.75);
        let mut guard = state.lock().unwrap();
        assert_eq!(guard.metrics.len(), 1);
        let points = guard.metrics.drain();
        assert_eq!(points[0].metric_id, hash_metric_id("internal.cpu.usage"));
        assert_eq!(points[0].metric_type, MetricType::Float as u8);
        let val = points[0].as_float().unwrap();
        assert!((val - 0.75).abs() < f64::EPSILON);
    }
}