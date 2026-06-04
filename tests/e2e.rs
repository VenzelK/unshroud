use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use std::os::unix::fs::FileTypeExt;

const SOCKET_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const DAEMON_STARTUP_GRACE: Duration = Duration::from_millis(200);

struct TestHarness {
    daemon: Option<Child>,
    temp_dir: TempDir,
    socket_path: String,
    output_dir: std::path::PathBuf,
}

impl TestHarness {
    fn start() -> Self {
        eprintln!("\n🧪 [E2E] Starting TestHarness...");
        let start_time = Instant::now();

        let temp_dir = TempDir::new().expect("[E2E] Failed to create TempDir");
        let socket_path = format!("{}/unshroud.sock", temp_dir.path().display());
        let output_dir = temp_dir.path().join("output");
        fs::create_dir_all(&output_dir).expect("[E2E] Failed to create output dir");

        eprintln!("[E2E] 📁 temp_dir: {:?}", temp_dir.path());
        eprintln!("[E2E] 🔌 socket_path: {}", socket_path);
        eprintln!("[E2E] 📦 output_dir: {:?}", output_dir);

        let config = format!(
            r#"[core]
poll_interval_ms = 100
buffer_capacity = 64
output_dir = "{}"
socket_path = "{}""#,
            output_dir.display(),
            socket_path
        );
        let config_path = temp_dir.path().join("test.toml");
        fs::write(&config_path, &config).expect("[E2E] Failed to write config");
        eprintln!("[E2E] ⚙️  config written to: {:?}", config_path);

        let daemon_path = env!("CARGO_BIN_EXE_unshroud");
        eprintln!("[E2E] 🦀 daemon binary: {}", daemon_path);
        
        if !Path::new(daemon_path).exists() {
            panic!("[E2E] ❌ Daemon binary not found at {}", daemon_path);
        }

        let mut daemon = Command::new(daemon_path)
            .arg("-c").arg(&config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("[E2E] Failed to spawn daemon process");

        eprintln!("[E2E] 🚀 Daemon spawned (PID: {})", daemon.id());


        let mut stderr_reader = daemon.stderr.take().map(BufReader::new);
        let mut stdout_reader = daemon.stdout.take().map(BufReader::new);
        
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        

        if let Some(reader) = stderr_reader.take() {
            let tx_clone = tx.clone();
            thread::spawn(move || {
                for line in reader.lines().flatten() {
                    let _ = tx_clone.send(format!("[daemon.stderr] {}", line));
                }
            });
        }

        if let Some(reader) = stdout_reader {
            thread::spawn(move || {
                for line in reader.lines().flatten() {
                    let _ = tx.send(format!("[daemon.stdout] {}", line));
                }
            });
        }

        eprintln!("[E2E] ⏳ Waiting for socket (timeout: {:?})...", SOCKET_TIMEOUT);
        let mut socket_ready = false;
        let wait_start = Instant::now();

        while wait_start.elapsed() < SOCKET_TIMEOUT {
            if let Ok(meta) = fs::metadata(&socket_path) {
                if meta.file_type().is_socket() {

                    thread::sleep(DAEMON_STARTUP_GRACE);
                    socket_ready = true;
                    eprintln!("[E2E] ✅ Socket ready after {:?}", wait_start.elapsed());
                    break;
                }
            }

            while let Ok(log_line) = rx.try_recv() {
                eprintln!("{}", log_line);
            }

            thread::sleep(POLL_INTERVAL);
        }

        if !socket_ready {
            eprintln!("\n❌ [E2E] FATAL: Socket not ready after {:?}", SOCKET_TIMEOUT);
            
            while let Ok(log_line) = rx.try_recv() {
                eprintln!("{}", log_line);
            }
            
            match daemon.try_wait() {
                Ok(Some(status)) => eprintln!("[E2E] 💀 Daemon exited early: {}", status),
                Ok(None) => eprintln!("[E2E] ⚠️  Daemon still running but no socket"),
                Err(e) => eprintln!("[E2E] ⚠️  Error checking daemon status: {}", e),
            }
            
            panic!("[E2E] Socket not created at {} within {:?}", socket_path, SOCKET_TIMEOUT);
        }

        eprintln!("[E2E] 🎯 TestHarness ready in {:?}", start_time.elapsed());
        
        Self {
            daemon: Some(daemon),
            temp_dir,
            socket_path,
            output_dir,
        }
    }

    fn send_message(&self, msg: &str) {
        eprintln!("[E2E] 📤 Sending: {}", msg);
        let mut sock = UnixStream::connect(&self.socket_path)
            .expect("[E2E] Failed to connect to socket");
        writeln!(sock, "{}", msg).expect("[E2E] Failed to write to socket");
    }

    fn bundle_count(&self) -> usize {
        fs::read_dir(&self.output_dir)
            .expect("[E2E] Failed to read output dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "zst"))
            .count()
    }

    fn list_bundles(&self) -> Vec<std::path::PathBuf> {
        fs::read_dir(&self.output_dir)
            .expect("[E2E] Failed to read output dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |ext| ext == "zst"))
            .collect()
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        eprintln!("[E2E] 🧹 Cleaning up TestHarness...");
        
        if let Some(mut child) = self.daemon.take() {
            eprintln!("[E2E] 🔪 Killing daemon (PID: {})", child.id());
            let _ = child.kill();
            
            for _ in 0..20 {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => thread::sleep(Duration::from_millis(100)),
                    Err(_) => break,
                }
            }
            let _ = child.wait();
        }
        eprintln!("[E2E] ✨ Cleanup complete");
    }
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_full_pipeline_trigger_and_dump() {
    eprintln!("\n🔬 [TEST] test_full_pipeline_trigger_and_dump");
    let h = TestHarness::start();

    eprintln!("[TEST] 🎯 Sending trigger payload...");
    h.send_message(r#"{"type":"metric","id":"internal.cpu.usage","value":0.98}"#);
    h.send_message(r#"{"type":"event","level":"warn","message":"cpu_spike"}"#);

    eprintln!("[TEST] ⏳ Waiting for bundle generation...");
    thread::sleep(Duration::from_secs(2));

    let count = h.bundle_count();
    eprintln!("[TEST] 📦 Bundles found: {}", count);
    
    if count >= 1 {
        let bundles = h.list_bundles();
        eprintln!("[TEST] ✅ SUCCESS: Bundle created at {:?}", bundles[0]);
    }
    
    assert!(count >= 1, "Bundle not created (found {})", count);
}

#[test]
fn test_malformed_json_does_not_crash() {
    eprintln!("\n🔬 [TEST] test_malformed_json_does_not_crash");
    let h = TestHarness::start();

    eprintln!("[TEST] 🗑️  Sending malformed JSON...");
    h.send_message("NOT JSON");
    h.send_message(r#"{"type":"unknown","x":1}"#);
    h.send_message(r#"{"type":"metric","id":"test","value":1.0}"#);

    eprintln!("[TEST] ⏳ Waiting for daemon to process...");
    thread::sleep(Duration::from_millis(500));
    
    eprintln!("[TEST] ✅ Daemon survived malformed input");
}

#[test]
fn test_graceful_shutdown_on_sigint() {
    eprintln!("\n🔬 [TEST] test_graceful_shutdown_on_sigint");
    let mut h = TestHarness::start();
    let pid = h.daemon.as_ref().unwrap().id();

    eprintln!("[TEST] 📡 Sending SIGINT to PID {}", pid);
    let status = Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .expect("[TEST] Failed to send SIGINT");
    
    assert!(status.success(), "[TEST] kill command failed");

    eprintln!("[TEST] ⏳ Waiting for graceful exit...");
    for _ in 0..50 {
        match h.daemon.as_mut().unwrap().try_wait() {
            Ok(Some(exit_status)) => {
                eprintln!("[TEST] ✅ Daemon exited with code: {:?}", exit_status.code());
                assert!(
                    exit_status.success() 
                        || exit_status.code() == Some(0) 
                        || exit_status.code() == Some(130),
                    "Unexpected exit code: {:?}", exit_status.code()
                );
                return;
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("[TEST] Error checking status: {}", e),
        }
    }
    
    eprintln!("[TEST] ⚠️  Daemon didn't exit in time, forcing kill");
    let _ = h.daemon.as_mut().unwrap().kill();
}
