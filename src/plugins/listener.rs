/*
* UDS
*/

use std::env;
use std::fs;
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixListener as StdUnixListener;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};
use anyhow::Result;
use crate::core::state::SharedState;

struct SocketPathGuard(String);
impl Drop for SocketPathGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn try_bind_listener(socket_path: &str) -> Result<(UnixListener, Option<SocketPathGuard>)> {

    if let (Ok(pid_str), Ok(fds_str)) = (env::var("LISTEN_PID"), env::var("LISTEN_FDS")) {
        if let (Ok(pid), Ok(fds)) = (pid_str.parse::<u32>(), fds_str.parse::<u32>()) {
            if pid == std::process::id() && fds >= 1 {

                let std_sock = unsafe { StdUnixListener::from_raw_fd(3) };
                std_sock.set_nonblocking(true)?;
                return Ok((UnixListener::from_std(std_sock)?, None));
            }
        }
    }

    eprintln!("[uds] 🔌 Socket bound: {}", socket_path);


    let _ = fs::remove_file(socket_path);
    let sock = UnixListener::bind(socket_path)?;
    Ok((sock, Some(SocketPathGuard(socket_path.to_string()))))
}

pub async fn start_listener(
    socket_path: &str,
    state: SharedState,
) -> Result<()> {
    let (listener, guard) = try_bind_listener(socket_path)?;
    if guard.is_none() {
        eprintln!("[uds] using socket activation");
    } else {
        eprintln!("[uds] listening on {}", socket_path);
    }
    let _guard = guard;

    loop {
        let (stream, addr) = listener.accept().await?;
        eprintln!("[uds] new connection from {:?}", addr);
        
        let state_clone = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, state_clone).await {
                eprintln!("[uds] client error: {}", e);
            }
        });
    }
}

async fn handle_client(
    stream: UnixStream,
    state: SharedState,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::with_capacity(256);

    loop {
        line.clear();
        match reader.read_line(&mut line).await? {
            0 => break,
            _ => {}
        }

        if let Ok(msg) = serde_json::from_str::<crate::plugins::protocol::PluginMessage>(&line) {
            let mut guard = state.lock().unwrap();
            msg.route(&mut guard);
        }
    }
    Ok(())
}

// ============================================================================
// TESTS
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;
    use crate::core::state::{CoreState, SharedState};

    fn tmp_path(suffix: &str) -> String {
        format!("/tmp/unshroud_test_{}_{}.sock", std::process::id(), suffix)
    }

    #[test]
    fn test_socket_path_guard_cleans_up() {
        let path = tmp_path("guard");
        fs::File::create(&path).unwrap().write_all(b"x").unwrap();
        assert!(fs::metadata(&path).is_ok());

        {
            let _guard = SocketPathGuard(path.clone());
            assert!(fs::metadata(&path).is_ok());
        }
        assert!(fs::metadata(&path).is_err(), "Guard failed to remove socket");
    }

    #[tokio::test]
    async fn test_standalone_bind_and_accept() {
        let path = tmp_path("standalone");
        let path_clone = path.clone();
        let sink: SharedState = Arc::new(std::sync::Mutex::new(
            CoreState::new(1024, 256)
        ));

        let task = tokio::spawn(async move {
            let _ = start_listener(&path_clone, sink).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(fs::metadata(&path).is_ok(), "Socket should exist");

        let stream = UnixStream::connect(&path).await.unwrap();
        drop(stream);
        
        tokio::time::sleep(Duration::from_millis(50)).await;
        task.abort();
    }

    #[tokio::test]
    async fn test_socket_rps() {
        let path = tmp_path("rps");
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();
        let path_clone = path.clone();

        let listener_task = tokio::spawn(async move {
            let (listener, guard) = try_bind_listener(&path_clone).unwrap();
            let _guard = guard;

            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::with_capacity(128);
                let c = count_clone.clone();
                
                tokio::spawn(async move {
                    loop {
                        line.clear();
                        match reader.read_line(&mut line).await {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {
                                c.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                });
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut client = UnixStream::connect(&path).await.unwrap();
        const N: usize = 50_000;
        let msg = b"{\"type\":\"metric\",\"id\":\"test\",\"value\":1.0}\n";

        let start = Instant::now();
        for _ in 0..N {
            client.write_all(msg).await.unwrap();
        }
        client.shutdown().await.unwrap();

        let _ = tokio::time::timeout(Duration::from_secs(3), async {
            while count.load(Ordering::Relaxed) < N {
                tokio::task::yield_now().await;
            }
        }).await;

        let elapsed = start.elapsed();
        let rps = N as f64 / elapsed.as_secs_f64();
        eprintln!("\n🚀 Throughput: {:.0} msg/sec", rps);
        assert!(rps > 100_000.0, "RPS too low");

        listener_task.abort();
    }

    #[tokio::test]
    async fn test_connection_drop_err_branch() {
        let path = tmp_path("drop");
        let path_clone = path.clone();

        let task = tokio::spawn(async move {
            let (listener, guard) = try_bind_listener(&path_clone).unwrap();
            let _guard = guard;
            
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {},
                    Err(_) => break,
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        
        let stream = UnixStream::connect(&path).await.unwrap();
        drop(stream);

        let result = task.await;
        assert!(result.is_ok(), "Listener panicked or hung on connection drop");
    }
}