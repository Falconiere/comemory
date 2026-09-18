//! The three worker threads and the polling loop behind
//! [`crate::utilities::process_runner::ProcessRunner`].
//!
//! Three detached threads service stdin, stdout and stderr at once while the
//! calling thread keeps `&mut Child` so it can still `kill()`; `std` has no
//! `wait_with_timeout`, so it polls `try_wait` every [`POLL_INTERVAL`].
//!
//! The workers are never joined, and the input is copied into the writer
//! rather than borrowed through `thread::scope`, because a join is the
//! unbounded wait the deadline exists to prevent — a descendant holding a pipe
//! can block a reader indefinitely.

use std::io::{ErrorKind, Read, Write};
use std::process::{Child, ChildStdin, ExitStatus};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

/// How often the calling thread re-checks the child and the three channels.
const POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Buffer size of one `read` syscall against a child pipe.
const READ_CHUNK: usize = 64 * 1024;

/// What a worker reports when its channel closed without a message — only
/// reachable if the thread itself died, which none of them can do without a
/// panic they contain no source of.
const LOST: &str = "worker thread ended without reporting";

/// How one pipe reader finished.
#[derive(Debug)]
pub(crate) enum ReadDone {
    /// The pipe reached EOF. Carries the retained (byte-capped) payload.
    Eof(Vec<u8>),
    /// More bytes arrived than the cap allows, and overflow is fatal for this
    /// stream. The reader stops immediately rather than draining to EOF.
    Overflow,
    /// The read itself failed.
    Failed(String),
}

/// How the stdin writer finished. `Ok(true)` means the child closed its read
/// end before the whole payload was written — its choice, not a failure.
pub(crate) type WriteDone = std::result::Result<bool, String>;

/// The parent's ends of one child's three standard streams, each serviced by
/// its own detached thread.
pub(crate) struct Pipes {
    writer: Receiver<WriteDone>,
    stdout: Receiver<ReadDone>,
    stderr: Receiver<ReadDone>,
}

/// Everything one [`Pipes::drain`] call collected before it stopped.
#[derive(Debug, Default)]
pub(crate) struct Drained {
    /// The child's exit status, once it has exited.
    pub(crate) status: Option<ExitStatus>,
    /// How the stdout reader finished.
    pub(crate) stdout: Option<ReadDone>,
    /// How the stderr reader finished.
    pub(crate) stderr: Option<ReadDone>,
    /// How the stdin writer finished.
    pub(crate) write: Option<WriteDone>,
    /// `try_wait` itself failed; the run cannot be trusted.
    pub(crate) wait_error: Option<String>,
    /// The deadline passed before every stream and the exit were accounted for.
    pub(crate) timed_out: bool,
}

impl Pipes {
    /// Take the child's three pipe handles and start a worker on each.
    ///
    /// `input` is moved into the writer thread, which closes stdin when it is
    /// done so the child sees EOF.
    pub(crate) fn start(
        child: &mut Child,
        input: Vec<u8>,
        max_stdout: usize,
        max_stderr: usize,
    ) -> std::result::Result<Self, String> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "stdin unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "stdout unavailable".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "stderr unavailable".to_string())?;
        Ok(Self {
            writer: spawn_writer(stdin, input),
            stdout: spawn_reader(stdout, max_stdout, true),
            stderr: spawn_reader(stderr, max_stderr, false),
        })
    }

    /// Poll the child and the three channels until the run is fully accounted
    /// for, an unrecoverable condition appears, or `deadline` passes.
    pub(crate) fn drain(&self, child: &mut Child, deadline: Instant) -> Drained {
        let mut drained = Drained::default();
        loop {
            self.poll_once(child, &mut drained);
            if drained.wait_error.is_some() || matches!(drained.stdout, Some(ReadDone::Overflow)) {
                return drained;
            }
            if drained.status.is_some()
                && drained.stdout.is_some()
                && drained.stderr.is_some()
                && drained.write.is_some()
            {
                return drained;
            }
            if Instant::now() >= deadline {
                drained.timed_out = true;
                return drained;
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// One non-blocking pass over the child and every channel still pending.
    fn poll_once(&self, child: &mut Child, drained: &mut Drained) {
        if drained.status.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => drained.status = Some(status),
                Ok(None) => {}
                Err(e) => drained.wait_error = Some(e.to_string()),
            }
        }
        poll(&self.stdout, &mut drained.stdout, || {
            ReadDone::Failed(LOST.to_string())
        });
        poll(&self.stderr, &mut drained.stderr, || {
            ReadDone::Failed(LOST.to_string())
        });
        poll(&self.writer, &mut drained.write, || Err(LOST.to_string()));
    }
}

/// Fill `slot` if the worker has reported, folding a closed channel into the
/// value `on_lost` produces so a dead worker cannot spin the loop to the
/// deadline.
fn poll<T>(rx: &Receiver<T>, slot: &mut Option<T>, on_lost: impl FnOnce() -> T) {
    if slot.is_some() {
        return;
    }
    match rx.try_recv() {
        Ok(value) => *slot = Some(value),
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => *slot = Some(on_lost()),
    }
}

/// Read `src` to EOF on a detached thread, retaining at most `cap` bytes.
///
/// When `fatal_overflow` is set the reader stops as soon as more than `cap`
/// bytes have arrived, so an oversized payload is refused promptly instead of
/// at the deadline. When it is clear the reader keeps draining to EOF and
/// merely discards the excess — that is what stops a chatty stderr from
/// filling its pipe and blocking the child that writes to it.
fn spawn_reader<R: Read + Send + 'static>(
    mut src: R,
    cap: usize,
    fatal_overflow: bool,
) -> Receiver<ReadDone> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut kept: Vec<u8> = Vec::new();
        let mut buf = vec![0_u8; READ_CHUNK];
        let mut total: usize = 0;
        let done = loop {
            match src.read(&mut buf) {
                Ok(0) => break ReadDone::Eof(kept),
                Ok(n) => {
                    total = total.saturating_add(n);
                    if kept.len() < cap {
                        let room = cap - kept.len();
                        kept.extend_from_slice(&buf[..n.min(room)]);
                    }
                    if fatal_overflow && total > cap {
                        break ReadDone::Overflow;
                    }
                }
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => break ReadDone::Failed(e.to_string()),
            }
        };
        drop(src);
        let _ = tx.send(done);
    });
    rx
}

/// Write the whole payload to the child's stdin on a detached thread, then
/// close it so the child sees EOF.
fn spawn_writer(mut sink: ChildStdin, input: Vec<u8>) -> Receiver<WriteDone> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let done = write_all(&mut sink, &input);
        drop(sink);
        let _ = tx.send(done);
    });
    rx
}

/// `write_all` + `flush`, treating a broken pipe as a truncated write rather
/// than an error.
///
/// A child may close its read end, or exit, before the whole payload is
/// written — a canned-answer script, or one that stops after the part it
/// needed. `write_all` loops over write syscalls and the first to meet the
/// closed read end fails with `EPIPE` no matter how many bytes went through,
/// so the only honest report is "we could not deliver all of it". Whether that
/// matters is the caller's call: the child's stdout and exit status still
/// decide the outcome.
fn write_all(sink: &mut ChildStdin, input: &[u8]) -> WriteDone {
    let wrote = sink.write_all(input).and_then(|()| sink.flush());
    match wrote {
        Ok(()) => Ok(false),
        Err(ref e) if e.kind() == ErrorKind::BrokenPipe => Ok(true),
        Err(e) => Err(e.to_string()),
    }
}
