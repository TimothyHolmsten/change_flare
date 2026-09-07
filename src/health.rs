use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::Error;

/// Shared process health for Kubernetes probes.
#[derive(Clone)]
pub struct HealthState {
    ready: Arc<AtomicBool>,
    last_success_epoch: Arc<AtomicU64>,
}

impl HealthState {
    pub fn new() -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(false)),
            last_success_epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn mark_success(&self) {
        self.ready.store(true, Ordering::SeqCst);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.last_success_epoch.store(now, Ordering::SeqCst);
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    pub fn last_success_epoch(&self) -> u64 {
        self.last_success_epoch.load(Ordering::SeqCst)
    }
}

impl Default for HealthState {
    fn default() -> Self {
        Self::new()
    }
}

/// Bind an HTTP listener for `/healthz` (liveness) and `/readyz` (readiness).
/// Returns `Ok(None)` when health is disabled.
pub fn spawn_listener(
    bind: Option<SocketAddr>,
    state: HealthState,
) -> Result<Option<thread::JoinHandle<()>>, Error> {
    let Some(addr) = bind else {
        return Ok(None);
    };
    let listener = TcpListener::bind(addr)?;
    listener.set_nonblocking(false)?;
    log::info!("health listening on {addr}");

    let handle = thread::Builder::new()
        .name("health".into())
        .spawn(move || serve(listener, state))?;
    Ok(Some(handle))
}

fn serve(listener: TcpListener, state: HealthState) {
    for incoming in listener.incoming() {
        let Ok(mut stream) = incoming else {
            continue;
        };
        let mut buf = [0u8; 256];
        let _ = stream.read(&mut buf);
        let request = String::from_utf8_lossy(&buf);
        let path = request.lines().next().unwrap_or_default();

        let (status, body) = if path.contains("/readyz") {
            if state.is_ready() {
                (200, "ready")
            } else {
                (503, "not ready")
            }
        } else {
            // liveness: process is up
            (200, "ok")
        };

        let response = format!(
            "HTTP/1.1 {status} {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            if status == 200 {
                "OK"
            } else {
                "Service Unavailable"
            },
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_after_success() {
        let state = HealthState::new();
        assert!(!state.is_ready());
        state.mark_success();
        assert!(state.is_ready());
        assert!(state.last_success_epoch() > 0);
    }
}
