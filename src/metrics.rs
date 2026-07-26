#[allow(unused)]
use metrics::{counter, gauge, histogram};

// ============================================================================
// CONSTANT NAMES
// ============================================================================

// Ingestion
pub const MESSAGES_PROCESSED: &str = "unshroud_messages_processed_total";
pub const PARSE_ERRORS: &str = "unshroud_parse_errors_total";
pub const SOCKET_CONNECTIONS_ACTIVE: &str = "unshroud_socket_connections_active";

// Triggers
pub const TRIGGERS_FIRED: &str = "unshroud_triggers_fired_total";
pub const TRIGGER_EVAL_DURATION_MS: &str = "unshroud_trigger_evaluation_duration_ms";
pub const LUA_COMPILE_DURATION_MS: &str = "unshroud_lua_compile_duration_ms";
pub const LUA_COMPILE_TOTAL_DURATION_MS: &str = "unshroud_lua_compile_total_duration_ms";

// Engine
pub const ENGINE_CYCLE_DURATION_MS: &str = "unshroud_engine_cycle_duration_ms";
pub const ENGINE_STARTS: &str = "unshroud_engine_starts_total";
pub const ENGINE_SHUTDOWNS: &str = "unshroud_engine_shutdowns_total";

// Storage
pub const BUNDLES_WRITTEN: &str = "unshroud_bundles_written_total";
pub const BUNDLE_DUMP_DURATION_MS: &str = "unshroud_bundle_dump_duration_ms";
pub const BUNDLE_RAW_SIZE_BYTES: &str = "unshroud_bundle_raw_size_bytes";
pub const BUNDLE_COMPRESSED_SIZE_BYTES: &str = "unshroud_bundle_compressed_size_bytes";

// Runtime / Process
pub const PROCESS_RESIDENT_MEMORY: &str = "process_resident_memory_bytes";
pub const PROCESS_CPU_SECONDS: &str = "process_cpu_seconds_total";
pub const BUILD_INFO: &str = "unshroud_build_info";

// ============================================================================
// INGESTION METRICS
// ============================================================================
pub mod ingestion {
    use super::*;

    /// Инкремент счётчика обработанных сообщений.
    /// 
    /// # Пример
    /// ```ignore
    /// crate::metrics::ingestion::message_processed("metric");
    /// ```
    #[inline]
    pub fn message_processed(msg_type: &'static str) {
        counter!(super::MESSAGES_PROCESSED, "type" => msg_type).increment(1);
    }

    #[inline]
    pub fn parse_error(error_type: &'static str) {
        counter!(super::PARSE_ERRORS, "type" => error_type).increment(1);
    }

    #[inline]
    pub fn set_active_connections(count: u64) {
        gauge!(super::SOCKET_CONNECTIONS_ACTIVE).set(count as f64);
    }

}

// ============================================================================
// TRIGGER METRICS
// ============================================================================
pub mod triggers {
    use super::*;

    #[inline]
    pub fn trigger_fired(metric_name: String) {
        counter!(super::TRIGGERS_FIRED, "metric_name" => metric_name).increment(1);
    }

    #[inline]
    pub fn record_eval_duration(ms: f64) {
        histogram!(super::TRIGGER_EVAL_DURATION_MS).record(ms);
    }

    #[inline]
    pub fn record_lua_compile_duration(script_name: String, ms: f64) {
        histogram!(super::LUA_COMPILE_DURATION_MS, "script" => script_name).record(ms);
    }

    #[inline]
    pub fn record_lua_compile_total_duration(ms: f64) {
        histogram!(super::LUA_COMPILE_TOTAL_DURATION_MS).record(ms);
    }

}

// ============================================================================
// ENGINE METRICS
// ============================================================================
pub mod engine {
    use super::*;

    #[inline]
    pub fn record_cycle_duration(ms: f64) {
        histogram!(super::ENGINE_CYCLE_DURATION_MS).record(ms);
    }

    #[inline]
    pub fn engine_started() {
        counter!(super::ENGINE_STARTS).increment(1);
    }

    #[inline]
    pub fn engine_shutdown() {
        counter!(super::ENGINE_SHUTDOWNS).increment(1);
    }

}

// ============================================================================
// STORAGE METRICS
// ============================================================================
pub mod storage {
    use super::*;

    #[inline]
    pub fn bundle_written() {
        counter!(super::BUNDLES_WRITTEN).increment(1);
    }

    #[inline]
    pub fn record_dump_duration(ms: f64) {
        histogram!(super::BUNDLE_DUMP_DURATION_MS).record(ms);
    }

    #[inline]
    pub fn record_raw_bytes(bytes: u64) {
        histogram!(super::BUNDLE_RAW_SIZE_BYTES).record(bytes as f64);
    }

    #[inline]
    pub fn record_compressed_bytes(bytes: f64) {
        histogram!(super::BUNDLE_COMPRESSED_SIZE_BYTES).record(bytes);
    }

}

// ============================================================================
// RUNTIME METRICS
// ============================================================================
pub mod runtime {
    use super::*;

    #[inline]
    pub fn set_resident_memory(bytes: u64) {
        gauge!(super::PROCESS_RESIDENT_MEMORY).set(bytes as f64);
    }

    #[inline]
    pub fn set_cpu_seconds(seconds: f64) {
        gauge!(super::PROCESS_CPU_SECONDS).set(seconds);
    }

    #[inline]
    pub fn set_build_info(version: &'static str) {
        gauge!(super::BUILD_INFO, "version" => version).set(1.0);
    }

}

// ============================================================================
// UTILS
// ============================================================================

pub fn init() {
    println!("metrics module init")
}

#[inline]
pub fn duration_to_ms(dur: std::time::Duration) -> f64 {
    dur.as_secs_f64() * 1000.0
}