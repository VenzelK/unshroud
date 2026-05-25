use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH, Instant};
use crate::core::buffer::RingBuffer;

pub struct CoreState {
    pub metrics: RingBuffer,
    pub events: EventBuffer,
    pub base_time: u64,
    cached_offset: u32,
    last_tick: Instant,
}

impl CoreState {
    pub fn new(metric_cap: usize, event_cap: usize) -> Self {
        let base = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        Self {
            metrics: RingBuffer::new(metric_cap),
            events: EventBuffer::new(event_cap),
            base_time: base,
            cached_offset: 0,
            last_tick: Instant::now(),
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