/*
* TODO: 
* TESTS!!!!
*/

use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio_stream::StreamExt;
use crate::core::state::CoreState;
use crate::plugins::protocol::PluginMessage;

pub async fn start_listener(
    socket_path: &str,
    state: Arc<std::sync::Mutex<CoreState>>,
) -> anyhow::Result<()> {
    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    eprintln!("[uds] listening on {}", socket_path);

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, state).await {
                eprintln!("[uds] client dropped: {}", e);
            }
        });
    }
}

async fn handle_client(
    stream: UnixStream,
    state: Arc<std::sync::Mutex<CoreState>>,
) -> anyhow::Result<()> {

    let reader = BufReader::new(stream);
    let mut lines = reader.lines();

    while let Some(line_result) = lines.next().await {
        let line = line_result?;
        
        if let Ok(msg) = serde_json::from_str::<PluginMessage>(&line) {
            route_immediately(msg, &state);
        }
    }
    Ok(())
}

#[inline]
fn route_immediately(msg: PluginMessage, state: &Arc<std::sync::Mutex<CoreState>>) {
    let base = state.lock().unwrap().base_time;
    let now = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() - base) as u32;

    match msg {
        PluginMessage::Metric(m) => {
            if let Some(val) = m.value {
                let id = hash_metric_id(&m.id);
                state.lock().unwrap().metrics.push_float(now, id, val);
            }
        }
        PluginMessage::Event(e) => {
            state.lock().unwrap().events.push(&format!("[{}] {}", e.level, e.message));
        }
        PluginMessage::Heartbeat(_) => {
            // TODO: update registry
        }
    }
}