#[allow(unused)]
use std::mem;
use std::time::{SystemTime, UNIX_EPOCH};


#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricType {
    Float = 0,
    Int   = 1,
    Bool  = 2,
    Bytes = 3,
}

/*
* TODO:
* Optimize buffer without alings
*
*/  
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MetricPoint {
    pub timestamp: u32,
    pub metric_id: u32,
    pub metric_type: u8,
    pub _align: [u8; 3],
    pub raw_value: [u8; 8],
    pub _pad: [u8; 4],
}

/*
* TODO: 
* TESTS!!!
*/
impl MetricPoint {

    #[inline]
    pub fn new_float(offset_sec: u32, id: u32, val: f64) -> Self {
        Self {
            timestamp: offset_sec,
            metric_id: id,
            metric_type: MetricType::Float as u8,
            _align: [0; 3],
            raw_value: val.to_ne_bytes(),
            _pad: [0; 4],
        }
    }

    #[inline]
    pub fn new_int(offset_sec: u32, id: u32, val: i64) -> Self {
        Self {
            timestamp: offset_sec,
            metric_id: id,
            metric_type: MetricType::Int as u8,
            _align: [0; 3],
            raw_value: val.to_ne_bytes(),
            _pad: [0; 4],
        }
    }

    #[inline]
    pub fn new_bool(offset_sec: u32, id: u32, val: bool) -> Self {
        Self {
            timestamp: offset_sec,
            metric_id: id,
            metric_type: MetricType::Bool as u8,
            _align: [0; 3],
            raw_value: (val as i64).to_ne_bytes(),
            _pad: [0; 4],
        }
    }

    #[inline]
    pub fn as_float(&self) -> Option<f64> {
        if self.metric_type == MetricType::Float as u8 {
            Some(f64::from_ne_bytes(self.raw_value))
        } else {
            None
        }
    }

    #[inline]
    pub fn as_int(&self) -> Option<i64> {
        if self.metric_type == MetricType::Int as u8 {
            Some(i64::from_ne_bytes(self.raw_value))
        } else {
            None
        }
    }

    #[inline]
    pub fn as_bool(&self) -> Option<bool> {
        if self.metric_type == MetricType::Bool as u8 {
            Some(i64::from_ne_bytes(self.raw_value) != 0)
        } else {
            None
        }
    }

    #[inline]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        if self.metric_type == MetricType::Bytes as u8 {
            Some(&self.raw_value)
        } else {
            None
        }
    }
}


pub struct RingBuffer {
    storage: Vec<MetricPoint>,
    capacity: usize,
    head: usize,
    count: usize,
    base_time: u64,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        let storage = vec![MetricPoint {
            timestamp: 0, metric_id: 0, metric_type: 0,
            _align: [0; 3], raw_value: [0; 8], _pad: [0; 4],
        }; capacity];

        Self {
            storage,
            capacity,
            head: 0,
            count: 0,
            base_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }

    #[inline]
    pub fn push(&mut self, point: MetricPoint) {

        self.storage[self.head] = point;

        self.head += 1;
        if self.head >= self.capacity {
            self.head = 0;
        }

        if self.count < self.capacity {
            self.count += 1;
        }
    }

    #[inline]
    pub fn len(&self) -> usize { self.count }
    #[inline]
    pub fn is_empty(&self) -> bool { self.count == 0 }
    #[inline]
    pub fn capacity(&self) -> usize { self.capacity }
    #[inline]
    pub fn base_time(&self) -> u64 { self.base_time }

    pub fn drain(&mut self) -> Vec<MetricPoint> {
        let mut result = Vec::with_capacity(self.count);
        if self.count == 0 { return result; }
        
        let mut idx = (self.head + self.capacity - self.count) % self.capacity;
        for _ in 0..self.count {
            result.push(self.storage[idx]);
            idx = (idx + 1) % self.capacity;
        }
        
        // 🔧 FIX: Actually clear the buffer state
        self.count = 0;
        self.head = 0;
        
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;}
