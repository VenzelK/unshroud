use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use crate::core::buffer::MetricPoint;

const _: () = assert!(std::mem::size_of::<MetricPoint>() == 24);

pub struct BundleBuilder {
    output_dir: PathBuf,
}

impl BundleBuilder {
    pub fn new(output_dir: &Path) -> Self {
        Self { output_dir: output_dir.to_path_buf() }
    }

    pub fn dump(&self, metrics: &[MetricPoint], events: &[&str]) -> std::io::Result<PathBuf> {
        let ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let temp_path = self.output_dir.join(format!(".bundle.{}.tmp", ts));
        let final_path = self.output_dir.join(format!("bundle.{}.zst", ts));

        let file = File::create(&temp_path)?;
        let mut enc = zstd::Encoder::new(BufWriter::new(file), 3)?;

        enc.write_all(&(metrics.len() as u32).to_ne_bytes())?;
        for m in metrics {
            let ptr = m as *const MetricPoint as *const u8;
            let bytes = unsafe { std::slice::from_raw_parts(ptr, 24) };
            enc.write_all(bytes)?;
        }

        enc.write_all(&(events.len() as u32).to_ne_bytes())?;
        for e in events {
            let b = e.as_bytes();
            enc.write_all(&(b.len() as u32).to_ne_bytes())?;
            enc.write_all(b)?;
        }

        metrics::counter!("unshroud_bundles_written_total").increment(1);
        
        enc.finish()?.flush()?;
        fs::rename(&temp_path, &final_path)?;
        Ok(final_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::buffer::RingBuffer;

    #[test]
    fn test_dump_creates_file() {
        let dir = std::env::temp_dir().join("unshroud_test_bundle");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let builder = BundleBuilder::new(&dir);
        let path = builder.dump(&[], &[]).unwrap();

        assert!(path.exists());
        assert!(path.to_string_lossy().ends_with(".zst"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_dump_integrity() {
        let dir = std::env::temp_dir().join("unshroud_test_bundle_int");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut rb = RingBuffer::new(16);
        rb.push(MetricPoint::new_float(0, 123, 42.5));
        let metrics = rb.drain();
        let events = vec!["log1", "log2"];

        let builder = BundleBuilder::new(&dir);
        let path = builder.dump(&metrics, &events).unwrap();

        let mut file = File::open(&path).unwrap();
        let mut compressed = Vec::new();
        file.read_to_end(&mut compressed).unwrap();

        let decoded = zstd::decode_all(compressed.as_slice()).unwrap();
        let mut cursor = std::io::Cursor::new(decoded);

        let mut buf = [0u8; 4];
        cursor.read_exact(&mut buf).unwrap();
        let m_count = u32::from_ne_bytes(buf) as usize;
        assert_eq!(m_count, 1);

        let mut pt_buf = [0u8; 24];
        cursor.read_exact(&mut pt_buf).unwrap();
        let pt = unsafe { &*(pt_buf.as_ptr() as *const MetricPoint) };
        assert_eq!(pt.metric_id, 123);
        assert_eq!(pt.as_float().unwrap(), 42.5);

        let mut e_buf = [0u8; 4];
        cursor.read_exact(&mut e_buf).unwrap();
        let e_count = u32::from_ne_bytes(e_buf) as usize;
        assert_eq!(e_count, 2);

        for expected in &events {
            let mut len_buf = [0u8; 4];
            cursor.read_exact(&mut len_buf).unwrap();
            let len = u32::from_ne_bytes(len_buf) as usize;
            let mut str_buf = vec![0u8; len];
            cursor.read_exact(&mut str_buf).unwrap();
            assert_eq!(String::from_utf8(str_buf).unwrap(), *expected);
        }

        fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn test_zstd_compression_ratio() {
        use std::time::{SystemTime, UNIX_EPOCH};
        
        let dir = std::env::temp_dir().join(format!(
            "unshroud_test_bundle_{}_{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut metrics = Vec::with_capacity(1000);
        for i in 0..1000 {
            metrics.push(MetricPoint::new_float(i as u32, 100, 0.5));
        }
        let events = vec!["event_repeated_pattern"; 1000];

        let metric_data_size = metrics.len() * std::mem::size_of::<MetricPoint>();
        let event_data_size = events.iter().map(|e| 4 + e.len()).sum::<usize>();
        let raw_payload_size = 4 + metric_data_size + 4 + event_data_size;

        let builder = BundleBuilder::new(&dir);
        let path = builder.dump(&metrics, &events).unwrap();

        assert!(path.exists(), "Bundle file should exist after dump");
        let compressed_size = fs::metadata(&path).unwrap().len() as usize;

        assert!(compressed_size < raw_payload_size);
        assert!(compressed_size < raw_payload_size / 2);

        let compressed = fs::read(&path).expect("Failed to read bundle file");
        let decoded = zstd::decode_all(compressed.as_slice()).unwrap();
        assert_eq!(decoded.len(), raw_payload_size);

        fs::remove_dir_all(&dir).unwrap();
    }
}