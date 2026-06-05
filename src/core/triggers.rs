use std::path::Path;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use mlua::{Lua, Value, Function, RegistryKey};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Operator {
    Gt,
    Lt,
    Eq,
    Lua,
}

#[derive(Debug, Clone)]
pub struct Trigger {
    pub metric_id: u32,
    pub operator: Operator,
    pub threshold: f64,
    pub lua_script: Option<String>,
    pub cooldown: Duration,
}

pub struct TriggerAction {
    pub metric_id: u32,
    pub value: f64,
}

pub struct TriggerEngine {
    rules: HashMap<u32, Vec<Trigger>>,
    last_fired: HashMap<u32, Instant>,
    lua: Lua,
    lua_cache: HashMap<u32, RegistryKey>,
}

impl TriggerEngine {
    pub fn new(triggers: Vec<Trigger>, lua_dir: &Path) -> Result<Self, mlua::Error> {
        let lua = Lua::new();
        let mut all_triggers = triggers;

        if lua_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(lua_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("lua") {
                        if let Ok(script) = std::fs::read_to_string(&path) {
                            let _ = lua.load(&script).into_function()?;
                            all_triggers.push(Trigger {
                                metric_id: 0,
                                operator: Operator::Lua,
                                threshold: 0.0,
                                lua_script: Some(script),
                                cooldown: Duration::from_secs(60),
                            });
                        }
                    }
                }
            }
        }

        let mut rules: HashMap<u32, Vec<Trigger>> = HashMap::new();
        let mut lua_cache = HashMap::new();

        for t in &all_triggers {
            if t.operator == Operator::Lua {
                if let Some(script) = &t.lua_script {
                    let func = lua.load(script).into_function()?;
                    let key = lua.create_registry_value(func)?;
                    lua_cache.insert(t.metric_id, key);
                }
            }
            rules.entry(t.metric_id).or_default().push(t.clone());
        }

        Ok(Self { rules, last_fired: HashMap::new(), lua, lua_cache })
    }

    pub fn check(&mut self, metric_id: u32, value: f64, timestamp: u32) -> Option<TriggerAction> {
        if let Some(triggers) = self.rules.get(&metric_id).cloned() {
            if let Some(action) = self.evaluate_triggers(metric_id, &triggers, value, timestamp) {
                return Some(action);
            }
        }
        if let Some(triggers) = self.rules.get(&0).cloned() {
            if let Some(action) = self.evaluate_triggers(0, &triggers, value, timestamp) {
                return Some(action);
            }
        }
        None
    }

    fn evaluate_triggers(
        &mut self, 
        trigger_key: u32, 
        triggers: &[Trigger], 
        value: f64, 
        timestamp: u32
    ) -> Option<TriggerAction> {
        for t in triggers {
            let matches = if t.operator == Operator::Lua {
                if let Some(key) = self.lua_cache.get(&trigger_key) {
                    let _ = self.lua.globals().set("value", value);
                    let _ = self.lua.globals().set("metric_id", trigger_key);
                    let _ = self.lua.globals().set("timestamp", timestamp);

                    match self.lua.registry_value::<Function>(key) {
                        Ok(func) => matches!(func.call::<Value>(()), Ok(Value::Boolean(true))),
                        _ => false,
                    }
                } else { false }
            } else {
                match t.operator {
                    Operator::Gt => value > t.threshold,
                    Operator::Lt => value < t.threshold,
                    Operator::Eq => (value - t.threshold).abs() < 1e-6,
                    Operator::Lua => unreachable!(),
                }
            };

            if matches {
                let now = Instant::now();
                if let Some(&last) = self.last_fired.get(&t.metric_id) {
                    if now.duration_since(last) < t.cooldown { continue; }
                }
                self.last_fired.insert(t.metric_id, now);
                metrics::counter!("unshroud_triggers_fired_total", "metric" => t.metric_id.to_string()).increment(1);
                return Some(TriggerAction { metric_id: trigger_key, value });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::path::Path;


    fn make_trigger(id: &str, op: Operator, thr: f64, cd_ms: u64) -> Trigger {
        Trigger {
            metric_id: crate::plugins::protocol::hash_metric_id(id),
            operator: op,
            threshold: thr,
            lua_script: None,
            cooldown: Duration::from_millis(cd_ms),
        }
    }

    #[test]
    fn test_fires_on_gt() {
        let mut engine = TriggerEngine::new(vec![make_trigger("cpu", Operator::Gt, 0.8, 0)], Path::new("")).unwrap();
        assert!(engine.check(crate::plugins::protocol::hash_metric_id("cpu"), 0.85, 0).is_some());
    }

    #[test]
    fn test_does_not_fire_on_unmet() {
        let mut engine = TriggerEngine::new(vec![make_trigger("cpu", Operator::Gt, 0.8, 0)], Path::new("")).unwrap();
        assert!(engine.check(crate::plugins::protocol::hash_metric_id("cpu"), 0.7, 0).is_none());
    }

    #[test]
    fn test_cooldown_suppresses() {
        let mut engine = TriggerEngine::new(vec![make_trigger("mem", Operator::Lt, 0.2, 100)], Path::new("")).unwrap();
        assert!(engine.check(crate::plugins::protocol::hash_metric_id("mem"), 0.1, 0).is_some());
        assert!(engine.check(crate::plugins::protocol::hash_metric_id("mem"), 0.05, 0).is_none());
        sleep(Duration::from_millis(150));
        assert!(engine.check(crate::plugins::protocol::hash_metric_id("mem"), 0.15, 0).is_some());
    }

    #[test]
    fn test_different_metrics_independent() {
        let mut engine = TriggerEngine::new(vec![
            make_trigger("a", Operator::Eq, 1.0, 100),
            make_trigger("b", Operator::Eq, 1.0, 100),
        ], Path::new("")).unwrap();
        assert!(engine.check(crate::plugins::protocol::hash_metric_id("a"), 1.0, 0).is_some());
        assert!(engine.check(crate::plugins::protocol::hash_metric_id("b"), 1.0, 0).is_some());
    }

    #[test]
    fn test_lua_trigger_simple() {
        let script = r#"return value > 0.5 and metric_id > 100"#.to_string();
        let trigger = Trigger {
            metric_id: 123,
            operator: Operator::Lua,
            threshold: 0.0,
            lua_script: Some(script),
            cooldown: Duration::from_secs(0),
        };
        let mut engine = TriggerEngine::new(vec![trigger], Path::new("")).unwrap();
        assert!(engine.check(123, 0.8, 42).is_some());
        assert!(engine.check(123, 0.3, 42).is_none());
    }
}