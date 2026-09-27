//! Blocking client for the coordinator's control socket: find the socket,
//! check who owns it, prove identities both ways, then send one op (or hold
//! a subscription). Every read and write runs under a deadline, so a hung or
//! stopped coordinator costs the caller the bound, never more.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::Paths;
use crate::domains::sync::daemon::control::{
    Event, Hello, MAX_FRAME, Op, PROTOCOL, Request, Response, Wake,
};
use crate::domains::sync::daemon::readiness::{PassSummary, Readiness};
use crate::domains::sync::daemon::{handshake, identity, runtime_record, socket_path};
use crate::prelude::*;

/// How long a probe waits for a verified answer.
pub const PROBE_BOUND: Duration = Duration::from_secs(2);

/// What a probe found.
#[derive(Debug)]
pub enum Probe {
    /// A verified coordinator for this data directory.
    Healthy(Box<Readiness>),
    /// Nothing is listening (no token, no socket, or a dead leftover file).
    NotRunning(String),
    /// Something is there but is not a verified coordinator for this
    /// directory: a foreign or symlinked socket, a failed proof, another
    /// protocol, another directory, or no answer within the bound.
    Stale(String),
}

/// Why a connection was not established.
enum Refusal {
    Absent(String),
    Foreign(String),
}

/// An authenticated connection whose next frame is the client's request.
struct Link {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    token: String,
    server_nonce: String,
}

/// Probe the coordinator for `paths` within `bound`.
#[must_use]
pub fn probe(paths: &Paths, bound: Duration) -> Probe {
    let deadline = Instant::now() + bound;
    let link = match Link::open(paths, deadline) {
        Ok(link) => link,
        Err(Refusal::Absent(why)) => return Probe::NotRunning(why),
        Err(Refusal::Foreign(why)) => return Probe::Stale(why),
    };
    let readiness: Readiness = match link.call(&Op::Status {}, deadline) {
        Ok(r) => r,
        Err(e) => return Probe::Stale(e.to_string()),
    };
    match identity::canonical_data_dir(paths) {
        Ok(dir) if dir == readiness.data_dir => Probe::Healthy(Box::new(readiness)),
        Ok(dir) => Probe::Stale(format!(
            "the socket answers for {}, not {}",
            readiness.data_dir.display(),
            dir.display()
        )),
        Err(e) => Probe::Stale(e.to_string()),
    }
}

/// Send `op` and decode its result.
///
/// # Errors
/// [`Error::Unavailable`] when no verified coordinator answers within
/// `bound`, or the coordinator refuses the op.
pub fn call<T: DeserializeOwned>(paths: &Paths, op: &Op, bound: Duration) -> Result<T> {
    let deadline = Instant::now() + bound;
    let link = Link::open(paths, deadline).map_err(Refusal::into_error)?;
    link.call(op, deadline)
}

/// Queue a wake.
///
/// # Errors
/// As [`call`].
pub fn wake(paths: &Paths, wake: Wake, bound: Duration) -> Result<()> {
    call::<serde_json::Value>(paths, &Op::Wake(wake), bound).map(drop)
}

/// Wait for one complete catch-up and return its last pass.
///
/// # Errors
/// As [`call`].
pub fn catch_up(paths: &Paths, bound: Duration) -> Result<PassSummary> {
    call(paths, &Op::CatchUp {}, bound)
}

/// Ask the coordinator to stop gracefully.
///
/// # Errors
/// As [`call`].
pub fn shutdown(paths: &Paths, bound: Duration) -> Result<()> {
    call::<serde_json::Value>(paths, &Op::Shutdown {}, bound).map(drop)
}

/// A held event stream.
pub struct Subscription {
    reader: BufReader<UnixStream>,
}

/// Subscribe to the coordinator's events.
///
/// # Errors
/// As [`call`].
pub fn subscribe(paths: &Paths, bound: Duration) -> Result<Subscription> {
    let deadline = Instant::now() + bound;
    let mut link = Link::open(paths, deadline).map_err(Refusal::into_error)?;
    link.send(&Op::Subscribe {}, deadline)?;
    let response: Response = read_frame(&mut link.reader, deadline)?;
    if !response.ok {
        return Err(Error::Unavailable(response.error.unwrap_or_default()));
    }
    link.reader.get_ref().set_read_timeout(None)?;
    Ok(Subscription {
        reader: link.reader,
    })
}

impl Subscription {
    /// The next event, waiting at most `wait` (forever with `None`).
    /// `Ok(None)` means the coordinator closed the stream or `wait` passed.
    ///
    /// # Errors
    /// A malformed frame or a read failure other than a timeout.
    pub fn next(&mut self, wait: Option<Duration>) -> Result<Option<Event>> {
        self.reader.get_ref().set_read_timeout(wait)?;
        let mut line = String::new();
        match (&mut self.reader)
            .take(MAX_FRAME as u64)
            .read_line(&mut line)
        {
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(serde_json::from_str(&line)?)),
            Err(e) if is_timeout(&e) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

impl Refusal {
    fn into_error(self) -> Error {
        match self {
            Self::Absent(why) => {
                Error::Unavailable(format!("the sync daemon is not running: {why}"))
            }
            Self::Foreign(why) => {
                Error::Unavailable(format!("the sync daemon did not verify: {why}"))
            }
        }
    }
}

impl Link {
    /// Find, check, connect and authenticate.
    fn open(paths: &Paths, deadline: Instant) -> std::result::Result<Self, Refusal> {
        let canonical =
            identity::canonical_data_dir(paths).map_err(|e| Refusal::Absent(e.to_string()))?;
        let token = match handshake::load(paths) {
            Ok(Some(token)) => token,
            Ok(None) => return Err(Refusal::Absent("no coordinator has run here".into())),
            Err(e) => return Err(Refusal::Foreign(e.to_string())),
        };
        let uid =
            socket_path::owner_uid(&canonical).map_err(|e| Refusal::Foreign(e.to_string()))?;
        let mut last = Refusal::Absent("no socket".into());
        for socket in candidates(paths, &canonical) {
            match connect(&socket, uid) {
                Ok(stream) => return Self::greet(stream, token, deadline),
                Err(refusal) => last = refusal,
            }
        }
        Err(last)
    }

    /// The hello exchange; the client proves nothing until the server has.
    fn greet(
        stream: UnixStream,
        token: String,
        deadline: Instant,
    ) -> std::result::Result<Self, Refusal> {
        let foreign = |e: Error| Refusal::Foreign(e.to_string());
        let writer = stream.try_clone().map_err(|e| foreign(e.into()))?;
        let mut link = Self {
            reader: BufReader::new(stream),
            writer,
            token,
            server_nonce: String::new(),
        };
        let nonce = handshake::nonce().map_err(foreign)?;
        let hello = Hello {
            hello: PROTOCOL,
            nonce: nonce.clone(),
            proof: None,
        };
        link.write(&hello, deadline).map_err(foreign)?;
        let answer: Hello = read_frame(&mut link.reader, deadline).map_err(foreign)?;
        if answer.hello != PROTOCOL {
            return Err(Refusal::Foreign(format!(
                "it speaks control protocol {}",
                answer.hello
            )));
        }
        let expected = handshake::proof(handshake::Side::Server, &nonce, &link.token);
        if !answer
            .proof
            .as_deref()
            .is_some_and(|p| handshake::matches(&expected, p))
        {
            return Err(Refusal::Foreign("it failed the identity proof".into()));
        }
        link.server_nonce = answer.nonce;
        Ok(link)
    }

    fn send(&mut self, op: &Op, deadline: Instant) -> Result<()> {
        let request = Request {
            proof: handshake::proof(handshake::Side::Client, &self.server_nonce, &self.token),
            op: op.clone(),
        };
        self.write(&request, deadline)
    }

    fn call<T: DeserializeOwned>(mut self, op: &Op, deadline: Instant) -> Result<T> {
        self.send(op, deadline)?;
        let response: Response = read_frame(&mut self.reader, deadline)?;
        if !response.ok {
            return Err(Error::Unavailable(format!(
                "the sync daemon refused: {}",
                response.error.unwrap_or_default()
            )));
        }
        Ok(serde_json::from_value(
            response.result.unwrap_or(serde_json::Value::Null),
        )?)
    }

    fn write(&mut self, frame: &impl Serialize, deadline: Instant) -> Result<()> {
        self.writer.set_write_timeout(Some(remaining(deadline)?))?;
        let mut line = serde_json::to_vec(frame)?;
        line.push(b'\n');
        self.writer.write_all(&line)?;
        Ok(())
    }
}

/// The record's socket first (it may have moved to the runtime directory),
/// then the computed one.
fn candidates(paths: &Paths, canonical: &std::path::Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = runtime_record::read(paths)
        .map(|r| r.socket)
        .into_iter()
        .collect();
    if let Ok(expected) = socket_path::expected(canonical)
        && !found.contains(&expected)
    {
        found.push(expected);
    }
    found
}

fn connect(socket: &std::path::Path, uid: u32) -> std::result::Result<UnixStream, Refusal> {
    match std::fs::symlink_metadata(socket) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Refusal::Absent(format!(
                "{} does not exist",
                socket.display()
            )));
        }
        Err(e) => return Err(Refusal::Foreign(e.to_string())),
        Ok(_) => {}
    }
    socket_path::check_socket(socket, uid).map_err(|e| Refusal::Foreign(e.to_string()))?;
    UnixStream::connect(socket).map_err(|e| match e.kind() {
        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound => {
            Refusal::Absent(format!("nothing listens on {}", socket.display()))
        }
        _ => Refusal::Foreign(format!("{}: {e}", socket.display())),
    })
}

fn read_frame<T: DeserializeOwned>(
    reader: &mut BufReader<UnixStream>,
    deadline: Instant,
) -> Result<T> {
    reader
        .get_ref()
        .set_read_timeout(Some(remaining(deadline)?))?;
    let mut line = String::new();
    match reader.take(MAX_FRAME as u64).read_line(&mut line) {
        Ok(0) => Err(Error::Unavailable(
            "the socket closed without answering".into(),
        )),
        Ok(_) => Ok(serde_json::from_str(&line)?),
        Err(e) if is_timeout(&e) => Err(Error::Unavailable("no answer within the bound".into())),
        Err(e) => Err(e.into()),
    }
}

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| Error::Unavailable("no answer within the bound".into()))
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
#[path = "tests/client.rs"]
mod tests;
