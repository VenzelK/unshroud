use serde::Deserialize;
use crate::core::state::CoreState;

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginMessage {
    Metric(MetricPayload),
    Event(EventPayload),
    Heartbeat(HeartbeatPayload),
}

impl PluginMessage {
    #[inline]
    pub fn route(self, state: &mut CoreState) {
        match self {
            Self::Metric(m)   => m.process(state),
            Self::Event(e)    => e.process(state),
            Self::Heartbeat(h) => h.process(state),
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct MetricPayload {
    pub id: String,
    pub value: Option<f64>,
}

impl MetricPayload {
    #[inline]
    pub fn process(self, state: &mut CoreState) {
        if let Some(val) = self.value {
            let now = state.current_offset();
            let point = crate::core::buffer::MetricPoint::new_float(
                now, 
                hash_metric_id(&self.id), 
                val
            );
            state.metrics.push(point);
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct EventPayload {
    pub level: String,
    pub message: String,
}

impl EventPayload {
    #[inline]
    pub fn process(self, state: &mut CoreState) {
        state.events.push(&format!("[{}] {}", self.level, self.message));
    }
}

#[derive(Deserialize, Debug)]
pub struct HeartbeatPayload {
    pub status: String,
    pub load: Option<String>,
}
impl HeartbeatPayload {
    #[inline]
    pub fn process(self, _state: &mut CoreState) {}
}

#[inline]
pub fn hash_metric_id(id: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in id.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::state::CoreState;
    use std::mem::{size_of, align_of};

    #[test]
    fn test_metric_point_layout() {
        assert_eq!(size_of::<crate::core::buffer::MetricPoint>(), 24, 
                   "MetricPoint should be 24 bytes");
        assert_eq!(align_of::<crate::core::buffer::MetricPoint>(), 4,
                   "MetricPoint should be 4-byte aligned (u32 fields, [u8;8] value)");

        use std::mem::offset_of;
        assert_eq!(offset_of!(crate::core::buffer::MetricPoint, timestamp), 0);
        assert_eq!(offset_of!(crate::core::buffer::MetricPoint, metric_id), 4);
        assert_eq!(offset_of!(crate::core::buffer::MetricPoint, metric_type), 8);
        assert_eq!(offset_of!(crate::core::buffer::MetricPoint, raw_value), 12);

        eprintln!("\n📦 MetricPoint layout:");
        eprintln!("  size: {} bytes", size_of::<crate::core::buffer::MetricPoint>());
        eprintln!("  align: {} bytes", align_of::<crate::core::buffer::MetricPoint>());
        eprintln!("  timestamp @ 0..4 (u32)");
        eprintln!("  metric_id @ 4..8 (u32)");
        eprintln!("  metric_type @ 8 (u8) + 3 padding");
        eprintln!("  raw_value @ 12..20 ([u8; 8], holds f64/i64 bytes)");
        eprintln!("  _pad @ 20..24");
    }

    #[test]
    fn test_hash_metric_id_deterministic() {

        let id = "cpu.load.avg1";
        let h1 = hash_metric_id(id);
        let h2 = hash_metric_id(id);
        assert_eq!(h1, h2, "Hash must be deterministic");

        let h3 = hash_metric_id("cpu.load.avg5");
        assert_ne!(h1, h3, "Different ids should produce different hashes");

        let h_empty = hash_metric_id("");
        assert_eq!(h_empty, 0x811c9dc5, "FNV-1a empty string hash");
    }

    #[test]
    fn test_metric_payload_routes_to_metrics_buffer() {
        let mut state = CoreState::new(16, 8);

        let payload = MetricPayload {
            id: "test.cpu".to_string(),
            value: Some(42.5),
        };

        payload.process(&mut state);

        assert_eq!(state.metrics.len(), 1, "Expected 1 metric after process()");
        
        let points = state.metrics.drain();
        let point = &points[0];
        
        assert_eq!(point.metric_id, hash_metric_id("test.cpu"));
        assert_eq!(point.metric_type, crate::core::buffer::MetricType::Float as u8);
        
        let val = point.as_float().unwrap();
        assert!((val - 42.5).abs() < f64::EPSILON, "Value mismatch: {}", val);
        
        assert!(point.timestamp < 10, "Timestamp offset too large: {}", point.timestamp);
    }

    #[test]
    fn test_event_payload_routes_to_events_buffer() {
        let mut state = CoreState::new(8, 16);
        
        let payload = EventPayload {
            level: "warn".to_string(),
            message: "Test alert".to_string(),
        };

        payload.process(&mut state);

        let events = state.events.drain();
        assert_eq!(events.len(), 1, "Expected 1 event after process()");
        assert!(events[0].contains("[warn]"), "Event should contain level");
        assert!(events[0].contains("Test alert"), "Event should contain message");
    }

    #[test]
    fn test_heartbeat_payload_is_noop() {
        let mut state = CoreState::new(8, 8);
        let initial_metrics = state.metrics.len();
        let initial_events = state.events.drain().len();

        let payload = HeartbeatPayload {
            status: "ok".to_string(),
            load: Some("low".to_string()),
        };

        payload.process(&mut state);

        assert_eq!(state.metrics.len(), initial_metrics, "Heartbeat should not push metrics");
        assert_eq!(state.events.drain().len(), initial_events, "Heartbeat should not push events");
    }

    #[test]
    fn test_plugin_message_enum_dispatch() {
        let mut state = CoreState::new(16, 16);
        
        let msg = PluginMessage::Metric(MetricPayload {
            id: "dispatch.test".to_string(),
            value: Some(1.0),
        });
        msg.route(&mut state);
        assert_eq!(state.metrics.len(), 1);

        let msg = PluginMessage::Event(EventPayload {
            level: "info".to_string(),
            message: "Dispatched".to_string(),
        });
        msg.route(&mut state);
        assert_eq!(state.events.drain().len(), 1);

        let msg = PluginMessage::Heartbeat(HeartbeatPayload {
            status: "ok".to_string(),
            load: None,
        });
        msg.route(&mut state);
    }

    #[test]
    fn test_json_deserialize_and_route_metric() {
        let mut state = CoreState::new(16, 8);
        let json = r#"{"type":"metric","id":"json.test","value":3.14}"#;
        
        if let Ok(msg) = serde_json::from_str::<PluginMessage>(json) {
            msg.route(&mut state);
        } else {
            panic!("Failed to deserialize JSON");
        }

        assert_eq!(state.metrics.len(), 1);
        let points = state.metrics.drain();
        let point = &points[0];
        
        let val = point.as_float().expect("Expected float metric");
        assert!((val - 3.14).abs() < 0.01, "Deserialized value mismatch: {}", val);
    }

    #[test]
    fn test_json_deserialize_and_route_event() {
        let mut state = CoreState::new(8, 16);
        let json = r#"{"type":"event","level":"error","message":"JSON test"}"#;
        
        if let Ok(msg) = serde_json::from_str::<PluginMessage>(json) {
            msg.route(&mut state);
        }

        let events = state.events.drain();
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("[error]"));
        assert!(events[0].contains("JSON test"));
    }

    #[test]
    fn test_json_deserialize_invalid_type_fails_gracefully() {
        let invalid = r#"{"type":"unknown","id":"test"}"#;
        let result = serde_json::from_str::<PluginMessage>(invalid);
        assert!(result.is_err(), "Unknown type should fail deserialization");
    }

    #[test]
    fn test_protocol_throughput_cpu_only() {
        use std::time::Instant;
        
        let mut state = CoreState::new(1024, 256);
        const N: usize = 100_000;
        
        let start = Instant::now();
        for i in 0..N {
            let msg = PluginMessage::Metric(MetricPayload {
                id: format!("bench.{}", i),
                value: Some(i as f64 * 0.1),
            });
            msg.route(&mut state);
        }
        let elapsed = start.elapsed();
        
        let rps = N as f64 / elapsed.as_secs_f64();
        eprintln!("\n⚡ Protocol CPU throughput: {:.0} msg/sec ({} msgs in {:?})", 
                  rps, N, elapsed);
        
        assert!(rps > 500_000.0, "Protocol dispatch too slow: {:.0}", rps);
        
        assert_eq!(state.metrics.len(), N.min(1024), "Buffer should contain min(N, capacity)");
    }
}