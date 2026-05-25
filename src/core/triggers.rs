use std::collections::HashMap;
use std::time::{Duration, Instant};
use crate::plugins::protocol::hash_metric_id;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Operator {
    Gt,
    Lt,
    Eq,
}

#[derive(Debug, Clone)]
pub struct Trigger {
    pub metric_id: u32,
    pub operator: Operator,
    pub threshold: f64,
    pub cooldown: Duration,
}

pub struct TriggerAction {
    pub metric_id: u32,
    pub value: f64,
}

pub struct TriggerEngine {
    rules: HashMap<u32, Vec<Trigger>>,
    last_fired: HashMap<u32, Instant>,
}

impl TriggerEngine {
    pub fn new(triggers: Vec<Trigger>) -> Self {
        let mut rules: HashMap<u32, Vec<Trigger>> = HashMap::new();
        for t in triggers {
            rules.entry(t.metric_id).or_default().push(t);
        }
        Self { rules, last_fired: HashMap::new() }
    }

    pub fn check(&mut self, metric_id: u32, value: f64) -> Option<TriggerAction> {
        if let Some(trigger_list) = self.rules.get(&metric_id) {
            for t in trigger_list {
                let matches = match t.operator {
                    Operator::Gt => value > t.threshold,
                    Operator::Lt => value < t.threshold,
                    Operator::Eq => (value - t.threshold).abs() < 1e-6,
                };

                if matches {
                    let now = Instant::now();
                    if let Some(&last) = self.last_fired.get(&metric_id) {
                        if now.duration_since(last) < t.cooldown {
                            continue;
                        }
                    }
                    self.last_fired.insert(metric_id, now);
                    return Some(TriggerAction { metric_id, value });
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    fn make_trigger(id: &str, op: Operator, thr: f64, cd_ms: u64) -> Trigger {
        Trigger {
            metric_id: hash_metric_id(id),
            operator: op,
            threshold: thr,
            cooldown: Duration::from_millis(cd_ms),
        }
    }

    #[test]
    fn test_fires_on_gt() {
        let mut engine = TriggerEngine::new(vec![make_trigger("cpu", Operator::Gt, 0.8, 0)]);
        assert!(engine.check(hash_metric_id("cpu"), 0.85).is_some());
    }

    #[test]
    fn test_does_not_fire_on_unmet() {
        let mut engine = TriggerEngine::new(vec![make_trigger("cpu", Operator::Gt, 0.8, 0)]);
        assert!(engine.check(hash_metric_id("cpu"), 0.7).is_none());
    }

    #[test]
    fn test_cooldown_suppresses() {
        let mut engine = TriggerEngine::new(vec![make_trigger("mem", Operator::Lt, 0.2, 100)]);
        assert!(engine.check(hash_metric_id("mem"), 0.1).is_some());
        assert!(engine.check(hash_metric_id("mem"), 0.05).is_none());
        sleep(Duration::from_millis(150));
        assert!(engine.check(hash_metric_id("mem"), 0.15).is_some());
    }

    #[test]
    fn test_different_metrics_independent() {
        let mut engine = TriggerEngine::new(vec![
            make_trigger("a", Operator::Eq, 1.0, 100),
            make_trigger("b", Operator::Eq, 1.0, 100),
        ]);
        assert!(engine.check(hash_metric_id("a"), 1.0).is_some());
        assert!(engine.check(hash_metric_id("b"), 1.0).is_some());
    }

    #[test]
    fn test_eq_operator_precision() {
        let mut engine = TriggerEngine::new(vec![make_trigger("temp", Operator::Eq, 36.6, 0)]);
        assert!(engine.check(hash_metric_id("temp"), 36.6).is_some());
        assert!(engine.check(hash_metric_id("temp"), 36.6000001).is_some());
        assert!(engine.check(hash_metric_id("temp"), 37.0).is_none());
    }
}