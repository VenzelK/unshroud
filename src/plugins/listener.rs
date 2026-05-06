use std::env;
use std::fs;
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixListener as StdUnixListener;
use std::sync::Arc;
use tokio::net::UnixListener;
use anyhow::Result;

/// RAII-гвард: удаляет файл сокета при выходе из области видимости.
struct SocketPathGuard(String);
impl Drop for SocketPathGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Пытается использовать systemd-socket, иначе падает на standalone-бинд.
fn try_bind_listener(socket_path: &str) -> Result<(UnixListener, Option<SocketPathGuard>)> {
    // Проверка socket activation (работает с systemd, launchd, openrc и др.)
    if let (Ok(pid_str), Ok(fds_str)) = (env::var("LISTEN_PID"), env::var("LISTEN_FDS")) {
        if let (Ok(pid), Ok(fds)) = (pid_str.parse::<u32>(), fds_str.parse::<u32>()) {
            if pid == std::process::id() && fds >= 1 {
                // FD 3 — первый переданный сокет
                let std_sock = unsafe { StdUnixListener::from_raw_fd(3) };
                std_sock.set_nonblocking(true)?; // Tokio требует nonblocking
                return Ok((UnixListener::from_std(std_sock)?, None));
            }
        }
    }

    // Standalone fallback
    let _ = fs::remove_file(socket_path);
    let sock = UnixListener::bind(socket_path)?;
    Ok((sock, Some(SocketPathGuard(socket_path.to_string()))))
}

pub async fn start_listener(
    socket_path: &str,
    _sink: Arc<tokio::sync::Mutex<()>>,
) -> Result<()> {
    let (listener, guard) = try_bind_listener(socket_path)?;
    if guard.is_none() {
        eprintln!("[uds] using socket activation");
    } else {
        eprintln!("[uds] listening on {}", socket_path);
    }
    let _guard = guard; // Дропается при выходе из функции

    loop {
        let (stream, addr) = listener.accept().await?;
        eprintln!("[uds] new connection from {:?}", addr);
        drop(stream);
    }
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
        let path_clone = path.clone(); // 🔧 FIX: clone before move
        let sink = Arc::new(tokio::sync::Mutex::new(()));
        
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
        let path_clone = path.clone(); // 🔧 FIX

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
                                c.fetch_add(1, Ordering::Relaxed); // 🔧 FIX: wrapped in block
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
            
            // 🔧 FIX: Явно обрабатываем Ok(0) (EOF) и Err
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break, // Клиент закрыл/уронил сокет → EOF
                    Ok(_) => {},    // Прочитали строку (если успела прийти)
                    Err(_) => break, // Ошибка сети/сокета
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Подключаемся и РЕЗКО рвём соединение
        let stream = UnixStream::connect(&path).await.unwrap();
        drop(stream);

        // Теперь таска завершится мгновенно (через Ok(0) => break)
        let result = task.await;
        assert!(result.is_ok(), "Listener panicked or hung on connection drop");
    }
}