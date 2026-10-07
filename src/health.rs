use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PROBE_IO_TIMEOUT: Duration = Duration::from_secs(2);

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
        let _ = stream.set_read_timeout(Some(PROBE_IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(PROBE_IO_TIMEOUT));
        let mut buf = [0u8; 256];
        let _ = stream.read(&mut buf);
        let request = String::from_utf8_lossy(&buf);
        let request_line = request.lines().next().unwrap_or_default();
        let (status, reason, body) = route(request_line, &state);

        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nX-Last-Success-Epoch: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
            state.last_success_epoch()
        );
        let _ = stream.write_all(response.as_bytes());
    }
}

fn route(request_line: &str, state: &HealthState) -> (u16, &'static str, &'static str) {
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/");
    match path {
        "/healthz" | "/livez" => (200, "OK", "ok"),
        "/readyz" => {
            if state.is_ready() {
                (200, "OK", "ready")
            } else {
                (503, "Service Unavailable", "not ready")
            }
        }
        _ => (404, "Not Found", "not found"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    #[test]
    fn ready_after_success() {
        let state = HealthState::new();
        assert!(!state.is_ready());
        state.mark_success();
        assert!(state.is_ready());
        assert!(state.last_success_epoch() > 0);
    }

    #[test]
    fn http_probes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let state = HealthState::new();
        let serving = state.clone();
        thread::spawn(move || serve(listener, serving));

        let (status, body, epoch) = http_get(addr, "/healthz");
        assert_eq!(status, 200);
        assert_eq!(body, "ok");
        assert_eq!(epoch, 0);

        let (status, body, epoch) = http_get(addr, "/readyz");
        assert_eq!(status, 503);
        assert_eq!(body, "not ready");
        assert_eq!(epoch, 0);

        state.mark_success();
        let (status, body, epoch) = http_get(addr, "/readyz");
        assert_eq!(status, 200);
        assert_eq!(body, "ready");
        assert!(epoch > 0);

        let (status, _, _) = http_get(addr, "/livez");
        assert_eq!(status, 200);

        let (status, _, _) = http_get(addr, "/nope");
        assert_eq!(status, 404);
    }

    fn http_get(addr: SocketAddr, path: &str) -> (u16, String, u64) {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .unwrap();
        let mut buf = String::new();
        stream.read_to_string(&mut buf).unwrap();
        let status = buf
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let epoch = buf
            .lines()
            .find_map(|line| line.strip_prefix("X-Last-Success-Epoch: "))
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0);
        let body = buf
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or_default()
            .trim()
            .to_string();
        (status, body, epoch)
    }
}
