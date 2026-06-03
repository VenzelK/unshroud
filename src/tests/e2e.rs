use std::process::{Command, Child};
use std::os::unix::net::UnixStream;
use std::io::Write;
use std::time::Duration;
use tempfile::TempDir;

struct TestHarness {
    daemon: Child,
    temp_dir: TempDir,
    socket_path: String,
}

impl TestHarness {
    fn start() -> Self {
        let temp_dir = TempDir::new().unwrap();
        let socket_path = format!("{}/unshroud.sock", temp_dir.path().display());
        let output_dir = format!("{}/output", temp_dir.path().display());
        std::fs::create_dir_all(&output_dir).unwrap();

        let config = format!(
            r#"[core]
poll_interval_ms = 100
buffer_capacity = 64
output_dir = "{}""#,
            output_dir
        );
        let config_path = temp_dir.path().join("test.toml");
        std::fs::write(&config_path, config).unwrap();

        let daemon = Command::new("./target/release/unshroud")
            .arg("-c").arg(config_path)
            .spawn()
            .expect("Failed to start daemon");

        std::thread::sleep(Duration::from_millis(500));
        Self { daemon, temp_dir, socket_path }
    }

    fn output_dir(&self) -> std::path::PathBuf {
        self.temp_dir.path().join("output")
    }

    fn send_message(&self, msg: &str) {
        let mut sock = UnixStream::connect(&self.socket_path).unwrap();
        writeln!(sock, "{}", msg).unwrap();
    }

    fn bundle_count(&self) -> usize {
        std::fs::read_dir(self.output_dir()).unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "zst"))
            .count()
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
    }
}

#[test]
fn test_full_pipeline_trigger_and_dump() {
    let h = TestHarness::start();

    h.send_message(r#"{"type":"metric","id":"internal.cpu.usage","value":0.98}"#);
    h.send_message(r#"{"type":"event","level":"warn","message":"cpu_spike"}"#);

    std::thread::sleep(Duration::from_secs(2));

    assert!(h.bundle_count() >= 1, "Bundle not created");
}

#[test]
fn test_malformed_json_does_not_crash() {
    let h = TestHarness::start();

    h.send_message("NOT JSON");
    h.send_message(r#"{"type":"unknown","x":1}"#);
    h.send_message(r#"{"type":"metric","id":"test","value":1.0}"#);

    std::thread::sleep(Duration::from_millis(500));
}

#[test]
fn test_graceful_shutdown_on_sigint() {
    let mut h = TestHarness::start();
    let pid = h.daemon.id();

    let status = Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .expect("Failed to send SIGINT");
    assert!(status.success(), "kill command failed");

    let exit_status = h.daemon.wait().unwrap();
    assert!(
        exit_status.success() 
            || exit_status.code() == Some(0) 
            || exit_status.code() == Some(130),
        "Daemon exited with unexpected code: {:?}", 
        exit_status.code()
    );
}