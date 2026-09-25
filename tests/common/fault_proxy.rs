#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! A loopback TCP proxy in front of a REAL `comemory serve`, for the exchange
//! suites (#255).
//!
//! It forwards every request's bytes to the engine and the engine's response
//! bytes back, unchanged, unless a case arms one fault. Faults only ever take
//! something away or refuse — drop a response, hold a request, flip one byte of
//! a response, refuse above a rate with `429`, answer `502` while the engine is
//! down. Every success body a client reads was written by the engine.
//!
//! One request per client connection: the proxy adds `Connection: close` both
//! ways, so a pooled connection is never reused across a fault.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// A fault the proxy applies to requests whose path contains `path`.
#[derive(Debug, Clone)]
pub enum Fault {
    /// Forward the request, then close the client socket instead of relaying
    /// the engine's response — a lost acknowledgement.
    DropResponse { path: String, times: usize },
    /// Keep the request until [`FaultProxy::release`] is called.
    HoldRequest { path: String },
    /// Forward, then wait `delay` before relaying the response.
    DelayResponse {
        path: String,
        delay: Duration,
        times: usize,
    },
    /// Replace the first occurrence of `needle` in the response body with
    /// `replacement` (same length), once.
    FlipResponse {
        path: String,
        needle: String,
        replacement: String,
    },
    /// Refuse with `429` and `Retry-After: <retry_after>` once more than `max`
    /// requests arrived since the fault was armed.
    RateLimit { max: usize, retry_after: u64 },
}

/// What the proxy did with one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Relayed the engine's answer.
    Forwarded(u16),
    /// Forwarded, then dropped the answer.
    Dropped,
    /// Refused with `429` without contacting the engine.
    RateLimited,
    /// The engine could not be reached; answered `502`.
    BadGateway,
    /// Relayed an answer with one byte flipped.
    Flipped,
}

/// One request the proxy saw.
#[derive(Debug, Clone)]
pub struct Logged {
    /// HTTP method.
    pub method: String,
    /// Request target (path and query).
    pub path: String,
    /// Request body, lossily decoded.
    pub body: String,
    /// When it arrived.
    pub at: Instant,
    /// What the proxy did.
    pub outcome: Outcome,
}

#[derive(Default)]
struct State {
    upstream: Option<SocketAddr>,
    faults: Vec<Fault>,
    since_armed: usize,
    log: Vec<Logged>,
}

#[derive(Default)]
struct Latch {
    held: Mutex<usize>,
    released: Mutex<bool>,
    wake: Condvar,
}

/// The running proxy. Dropping it stops accepting.
pub struct FaultProxy {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    latch: Arc<Latch>,
}

impl FaultProxy {
    /// Listen on an ephemeral loopback port in front of `upstream`.
    pub fn start(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind proxy");
        let addr = listener.local_addr().expect("proxy addr");
        let state = Arc::new(Mutex::new(State {
            upstream: Some(upstream),
            ..State::default()
        }));
        let latch = Arc::new(Latch::default());
        let (state_c, latch_c) = (Arc::clone(&state), Arc::clone(&latch));
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (s, l) = (Arc::clone(&state_c), Arc::clone(&latch_c));
                thread::spawn(move || serve(stream, &s, &l));
            }
        });
        Self { addr, state, latch }
    }

    /// `http://127.0.0.1:<port>` — the origin a client's `api_url` names.
    pub fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Point the proxy at a restarted engine.
    pub fn set_upstream(&self, upstream: Option<SocketAddr>) {
        self.state.lock().unwrap().upstream = upstream;
    }

    /// Arm `fault` (in addition to any already armed).
    pub fn arm(&self, fault: Fault) {
        if matches!(fault, Fault::HoldRequest { .. }) {
            *self.latch.held.lock().unwrap() = 0;
            *self.latch.released.lock().unwrap() = false;
        }
        let mut state = self.state.lock().unwrap();
        state.since_armed = 0;
        state.faults.push(fault);
    }

    /// Disarm every fault and release any held request.
    pub fn clear(&self) {
        self.state.lock().unwrap().faults.clear();
        self.release();
    }

    /// Let held requests through.
    pub fn release(&self) {
        *self.latch.released.lock().unwrap() = true;
        self.latch.wake.notify_all();
    }

    /// Block until a request is being held, or `timeout` passes.
    pub fn wait_held(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if *self.latch.held.lock().unwrap() > 0 {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Every request seen so far.
    pub fn log(&self) -> Vec<Logged> {
        self.state.lock().unwrap().log.clone()
    }

    /// Requests whose path contains `fragment`.
    pub fn requests_to(&self, fragment: &str) -> Vec<Logged> {
        self.log()
            .into_iter()
            .filter(|l| l.path.contains(fragment))
            .collect()
    }
}

/// Read one HTTP request (headers plus a `Content-Length` body).
fn read_request(stream: &mut TcpStream) -> Option<(String, String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 8192];
    let header_end = loop {
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i + 4;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let length = content_length(&head);
    while buf.len() < header_end + length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let first = head.lines().next().unwrap_or_default().to_string();
    let mut parts = first.split(' ');
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    Some((method, path, rewrite_connection(&buf, header_end)))
}

fn serve(mut client: TcpStream, state: &Arc<Mutex<State>>, latch: &Arc<Latch>) {
    let Some((method, path, request)) = read_request(&mut client) else {
        return;
    };
    let body_at = find(&request, b"\r\n\r\n").map_or(request.len(), |i| i + 4);
    let seen = Seen {
        method,
        path,
        body: String::from_utf8_lossy(&request[body_at..]).to_string(),
    };
    let (fault, upstream) = {
        let mut s = state.lock().unwrap();
        s.since_armed += 1;
        let since = s.since_armed;
        (pick_fault(&mut s.faults, &seen.path, since), s.upstream)
    };
    let outcome = answer(&mut client, fault, upstream, &request, latch);
    state.lock().unwrap().log.push(Logged {
        method: seen.method,
        path: seen.path,
        body: seen.body,
        at: Instant::now(),
        outcome,
    });
}

/// The parts of a request the log keeps.
struct Seen {
    method: String,
    path: String,
    body: String,
}

/// Apply `fault` (if any) to one request and write whatever reaches the client.
fn answer(
    client: &mut TcpStream,
    fault: Option<Fault>,
    upstream: Option<SocketAddr>,
    request: &[u8],
    latch: &Latch,
) -> Outcome {
    if let Some(Fault::RateLimit { retry_after, .. }) = &fault {
        let refusal = format!(
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: {retry_after}\r\n\
             Content-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _ = client.write_all(refusal.as_bytes());
        return Outcome::RateLimited;
    }
    if matches!(fault, Some(Fault::HoldRequest { .. })) {
        *latch.held.lock().unwrap() += 1;
        let mut released = latch.released.lock().unwrap();
        while !*released {
            released = latch.wake.wait(released).unwrap();
        }
    }
    let Some(response) = upstream.and_then(|addr| exchange(addr, request)) else {
        let _ = client.write_all(
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return Outcome::BadGateway;
    };
    match fault {
        Some(Fault::DropResponse { .. }) => {
            let _ = client.shutdown(Shutdown::Both);
            Outcome::Dropped
        }
        Some(Fault::FlipResponse {
            needle,
            replacement,
            ..
        }) => {
            let _ = client.write_all(&replace_once(&response, &needle, &replacement));
            Outcome::Flipped
        }
        other => {
            if let Some(Fault::DelayResponse { delay, .. }) = other {
                thread::sleep(delay);
            }
            let _ = client.write_all(&response);
            Outcome::Forwarded(status_of(&response))
        }
    }
}

/// The fault that applies to this request, consuming a one-shot one.
fn pick_fault(faults: &mut Vec<Fault>, path: &str, since_armed: usize) -> Option<Fault> {
    let index = faults.iter().position(|f| match f {
        Fault::DropResponse { path: p, .. }
        | Fault::HoldRequest { path: p }
        | Fault::DelayResponse { path: p, .. }
        | Fault::FlipResponse { path: p, .. } => path.contains(p.as_str()),
        Fault::RateLimit { max, .. } => since_armed > *max,
    })?;
    let fault = faults[index].clone();
    match &mut faults[index] {
        Fault::DropResponse { times, .. } | Fault::DelayResponse { times, .. } => {
            *times = times.saturating_sub(1);
            if *times == 0 {
                faults.remove(index);
            }
        }
        Fault::FlipResponse { .. } => {
            faults.remove(index);
        }
        Fault::HoldRequest { .. } | Fault::RateLimit { .. } => {}
    }
    Some(fault)
}

/// Send the request to the engine and read its whole response.
fn exchange(addr: SocketAddr, request: &[u8]) -> Option<Vec<u8>> {
    let mut upstream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
    upstream.write_all(request).ok()?;
    let mut response = Vec::new();
    upstream.read_to_end(&mut response).ok()?;
    (!response.is_empty()).then_some(response)
}

/// Force `Connection: close` in a request or response head.
fn rewrite_connection(message: &[u8], header_end: usize) -> Vec<u8> {
    let head = String::from_utf8_lossy(&message[..header_end]);
    let mut lines: Vec<&str> = head
        .split("\r\n")
        .filter(|l| !l.to_ascii_lowercase().starts_with("connection:"))
        .collect();
    while lines.last() == Some(&"") {
        lines.pop();
    }
    let mut out = lines.join("\r\n").into_bytes();
    out.extend_from_slice(b"\r\nConnection: close\r\n\r\n");
    out.extend_from_slice(&message[header_end..]);
    out
}

fn replace_once(response: &[u8], needle: &str, replacement: &str) -> Vec<u8> {
    assert_eq!(needle.len(), replacement.len(), "a flip keeps the length");
    let mut out = response.to_vec();
    if let Some(i) = find(response, needle.as_bytes()) {
        out[i..i + needle.len()].copy_from_slice(replacement.as_bytes());
    }
    out
}

fn status_of(response: &[u8]) -> u16 {
    String::from_utf8_lossy(&response[..response.len().min(16)])
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|l| {
            let lower = l.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse().unwrap_or(0))
        })
        .unwrap_or(0)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
