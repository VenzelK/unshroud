use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH, Instant};
use crate::core::buffer::RingBuffer;

pub struct CoreState {
    pub metrics: RingBuffer,
    pub events: EventBuffer,
    pub base_time: u64,
    pub metric_names: HashMap<u32, String>,
    cached_offset: u32,
    last_tick: Instant,
}

impl CoreState {
    pub fn new(metrics_capacity: usize, events_capacity: usize) -> Self {
        Self {
            metrics: RingBuffer::new(metrics_capacity),
            events: EventBuffer::new(events_capacity),
            base_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            cached_offset: 0,
            last_tick: Instant::now(),
            metric_names: HashMap::new(),
        }
    }

    #[inline]
    pub fn current_offset(&mut self) -> u32 {
        if self.last_tick.elapsed().as_millis() >= 10 {
            self.cached_offset = (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() - self.base_time) as u32;
            self.last_tick = Instant::now();
        }
        self.cached_offset
    }
}

pub type SharedState = Arc<Mutex<CoreState>>;

pub struct EventBuffer {
    storage: VecDeque<Box<str>>,
    capacity: usize,
}

impl EventBuffer {
    pub fn new(capacity: usize) -> Self {
        Self { storage: VecDeque::with_capacity(capacity), capacity }
    }

    #[inline]
    pub fn push(&mut self, event: &str) {
        if self.storage.len() >= self.capacity {
            self.storage.pop_front();
        }
        self.storage.push_back(event.into());
    }

    pub fn drain(&self) -> Vec<String> {
        self.storage.iter().map(|s| s.to_string()).collect()
    }
}