use std::path::Path;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use mlua::{Lua, Value, Function, RegistryKey};

use crate::{debug_log, error_log, info_log}; 

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
    metric_names: HashMap<u32, String>,

}

impl TriggerEngine {
    pub fn new(triggers: Vec<Trigger>, lua_dir: &Path) -> Result<Self, mlua::Error> {
        
        let compile_start = Instant::now();

        let lua = Lua::new();
        let mut all_triggers = triggers;
        let mut lua_scripts_loaded = 0;

        if lua_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(lua_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("lua") {
                        let script_name = path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("unknown")
                            .to_string();
                        
                        let script_start = Instant::now();
                        if let Ok(script) = std::fs::read_to_string(&path) {
                            let _ = lua.load(&script).into_function()?;
                            let compile_ms = script_start.elapsed().as_secs_f64() * 1000.0;
                            
                            debug_log!("Lua script '{}' compiled in {:.2}ms", script_name, compile_ms);
                            info_log!("Loaded Lua trigger: {}", script_name);

                            crate::metrics::triggers::record_lua_compile_duration(script_name, compile_ms);
                            
                            all_triggers.push(Trigger {
                                metric_id: 0,
                                operator: Operator::Lua,
                                threshold: 0.0,
                                lua_script: Some(script),
                                cooldown: Duration::ZERO,
                            });
                            lua_scripts_loaded += 1;
                        }
                    }
                }
            }
        }

        let total_compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;
        crate::metrics::triggers::record_lua_compile_total_duration(total_compile_ms);                            
        info_log!("TriggerEngine initialized: {} scripts in {:.2}ms", lua_scripts_loaded, total_compile_ms);

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

        Ok(Self {
            rules,
            last_fired: HashMap::new(),
            lua,
            lua_cache,
            metric_names: HashMap::new(), // ← ДОБАВЬ ЭТО
        })
    }

    pub fn check(&mut self, metric_id: u32, metric_name: &str, value: f64, timestamp: u32) -> Option<TriggerAction> {
        if let Some(triggers) = self.rules.get(&metric_id).cloned() {
            if let Some(action) = self.evaluate_triggers(metric_id, metric_name, &triggers, value, timestamp) {
                return Some(action);
            }
        }
        if let Some(triggers) = self.rules.get(&0).cloned() {
            if let Some(action) = self.evaluate_triggers(0, metric_name, &triggers, value, timestamp) {
                return Some(action);
            }
        }
        None
    }

    fn evaluate_triggers(
        &mut self, 
        trigger_key: u32, 
        metric_name: &str,
        triggers: &[Trigger], 
        value: f64, 
        timestamp: u32
    ) -> Option<TriggerAction> {
        for t in triggers {
            
            let eval_start = Instant::now();

            let matches = if t.operator == Operator::Lua {
                if let Some(key) = self.lua_cache.get(&trigger_key) {
                    let globals = self.lua.globals();
                    let _ = globals.set("value", value);
                    let _ = globals.set("metric_id", trigger_key);
                    let _ = globals.set("timestamp", timestamp);
                    
                    let _ = globals.set("metric_name", metric_name);

                    match self.lua.registry_value::<Function>(key) {
                        Ok(func) => match func.call::<Value>(()) {
                            Ok(Value::Boolean(b)) => b,
                            Err(e) => {
                                error_log!("[lua] runtime error: {}", e);
                                false
                            }
                            _ => false,
                        },
                        Err(e) => {
                            error_log!("[lua] cache error: {}", e);
                            false
                        }
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
            
            let eval_ms = eval_start.elapsed().as_secs_f64() * 1000.0;
            crate::metrics::triggers::record_eval_duration(eval_ms);

            debug_log!(
                "Trigger check: metric_id={}, metric_name={}, value={}, matches={}, eval={:.3}ms",
                trigger_key, metric_name, value, matches, eval_ms
            );

            if matches {
                let now = Instant::now();
                if let Some(&last) = self.last_fired.get(&t.metric_id) {
                    if now.duration_since(last) < t.cooldown { continue; }
                }
                self.last_fired.insert(t.metric_id, now);
                
                debug_log!("Trigger fired: name={}, value={}", metric_name, value);


                crate::metrics::triggers::trigger_fired(metric_name.to_string());

                return Some(TriggerAction { metric_id: trigger_key, value });
            }
        }
        None
    }
    
    pub fn register_metric_name(&mut self, id: u32, name: String) {
        self.metric_names.insert(id, name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::path::Path;
    use crate::plugins::protocol;


    fn make_trigger(id: &str, op: Operator, thr: f64, cd_ms: u64) -> Trigger {
        Trigger {
            metric_id: protocol::hash_metric_id(id),
            operator: op,
            threshold: thr,
            lua_script: None,
            cooldown: Duration::from_millis(cd_ms),
        }
    }

    #[test]
    fn test_fires_on_gt() {
        let mut engine = TriggerEngine::new(vec![make_trigger("cpu", Operator::Gt, 0.8, 0)], Path::new("")).unwrap();
        assert!(engine.check(protocol::hash_metric_id("cpu"), "cpu", 0.85, 0).is_some());
    }
    #[test]
    fn test_does_not_fire_on_unmet() {
        let mut engine = TriggerEngine::new(vec![make_trigger("cpu", Operator::Gt, 0.8, 0)], Path::new("")).unwrap();
        assert!(engine.check(protocol::hash_metric_id("cpu"), "cpu", 0.7, 0).is_none());
    }
    
    #[test]
    fn test_cooldown_suppresses() {
        let mut engine = TriggerEngine::new(vec![make_trigger("mem", Operator::Lt, 0.2, 100)], Path::new("")).unwrap();
        assert!(engine.check(protocol::hash_metric_id("mem"), "mem", 0.1, 0).is_some());
        assert!(engine.check(protocol::hash_metric_id("mem"), "mem", 0.05, 0).is_none());
        sleep(Duration::from_millis(150));
        assert!(engine.check(protocol::hash_metric_id("mem"), "mem", 0.15, 0).is_some());
    }

    #[test]
    fn test_different_metrics_independent() {
        let mut engine = TriggerEngine::new(vec![
            make_trigger("a", Operator::Eq, 1.0, 100),
            make_trigger("b", Operator::Eq, 1.0, 100),
        ], Path::new("")).unwrap();
        assert!(engine.check(protocol::hash_metric_id("a"), "a", 1.0, 0).is_some());
        assert!(engine.check(protocol::hash_metric_id("b"), "b", 1.0, 0).is_some());
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
        assert!(engine.check(123, "test.metric", 0.8, 42).is_some());
        assert!(engine.check(123, "test.metric", 0.3, 42).is_none());
    }
}