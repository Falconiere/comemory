//! The control server: accept on the Unix socket, check the peer's uid,
//! run the two-way handshake under a deadline, then serve one op. Answers
//! come from shared state or the queue, never from the store or the network,
//! so the socket stays responsive while a pass blocks.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, mpsc};

use crate::domains::sync::daemon::control::{Hello, MAX_FRAME, Op, PROTOCOL, Request, Response};
use crate::domains::sync::daemon::handshake;
use crate::domains::sync::daemon::readiness::Trigger;
use crate::domains::sync::daemon::state::State;
use crate::domains::sync::daemon::worker::Queue;
use crate::prelude::*;

/// How long a client has for the hello and its request.
const HANDSHAKE: Duration = Duration::from_secs(2);

/// What every connection needs.
pub struct Ctx {
    /// The control token.
    pub token: String,
    /// The uid a peer must run as.
    pub uid: u32,
    /// Shared state.
    pub state: Arc<State>,
    /// The pass queue.
    pub queue: Arc<Queue>,
    /// Raised by a `shutdown` op.
    pub shutdown: Arc<Notify>,
    /// Raised by a `reload` op.
    pub reload: Arc<Notify>,
}

/// Accept until the process exits, swapping in a re-bound listener whenever
/// the watchdog sends one.
pub async fn serve(
    mut listener: UnixListener,
    ctx: Arc<Ctx>,
    mut rebound: mpsc::Receiver<UnixListener>,
) {
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let ctx = Arc::clone(&ctx);
                    tokio::spawn(async move {
                        if let Err(e) = handle(stream, &ctx).await {
                            tracing::debug!(error = %e, "control connection ended");
                        }
                    });
                }
                Err(e) => tracing::warn!(error = %e, "control socket accept failed"),
            },
            next = rebound.recv() => match next {
                Some(fresh) => listener = fresh,
                None => return,
            },
        }
    }
}

async fn handle(stream: UnixStream, ctx: &Ctx) -> Result<()> {
    if stream.peer_cred()?.uid() != ctx.uid {
        return Err(Error::Forbidden("control peer runs as another user".into()));
    }
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let request = tokio::time::timeout(HANDSHAKE, greet(&mut reader, &mut write, &ctx.token))
        .await
        .map_err(|_| Error::Unavailable("control handshake timed out".into()))??;
    dispatch(request.op, &mut write, ctx).await
}

/// Prove ourselves first, then check the client's proof.
async fn greet(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
    write: &mut OwnedWriteHalf,
    token: &str,
) -> Result<Request> {
    let hello: Hello = read_frame(reader).await?;
    let nonce = handshake::nonce()?;
    let answer = Hello {
        hello: PROTOCOL,
        nonce: nonce.clone(),
        proof: Some(handshake::server_proof(token, &hello.nonce)),
    };
    write_frame(write, &answer).await?;
    let request: Request = read_frame(reader).await?;
    if !handshake::matches(&handshake::client_proof(token, &nonce), &request.proof) {
        return Err(Error::Forbidden(
            "control client failed the identity proof".into(),
        ));
    }
    Ok(request)
}

async fn dispatch(op: Op, write: &mut OwnedWriteHalf, ctx: &Ctx) -> Result<()> {
    match op {
        Op::Status {} => reply(write, &ctx.state.readiness()).await,
        Op::Wake(wake) => {
            ctx.queue.wake(wake.reason, wake.checkout);
            ctx.state.set_queued(true);
            reply(write, &serde_json::json!({ "queued": true })).await
        }
        Op::CatchUp {} => {
            let summary = ctx.queue.catch_up();
            ctx.state.set_queued(true);
            match summary.await {
                Ok(summary) => reply(write, &summary).await,
                Err(_) => refuse(write, "the sync daemon stopped before the pass ran").await,
            }
        }
        Op::Reload {} => {
            ctx.reload.notify_one();
            ctx.queue.wake(Trigger::Reload, None);
            reply(write, &ctx.state.readiness()).await
        }
        Op::Subscribe {} => {
            let mut events = ctx.state.subscribe();
            reply(write, &serde_json::json!({ "subscribed": true })).await?;
            loop {
                match events.recv().await {
                    Ok(event) => write_frame(write, &event).await?,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
        Op::Shutdown {} => {
            reply(write, &serde_json::json!({ "stopping": true })).await?;
            ctx.shutdown.notify_one();
            Ok(())
        }
    }
}

async fn reply(write: &mut OwnedWriteHalf, result: &impl Serialize) -> Result<()> {
    let response = Response {
        ok: true,
        result: Some(serde_json::to_value(result)?),
        error: None,
    };
    write_frame(write, &response).await
}

async fn refuse(write: &mut OwnedWriteHalf, why: &str) -> Result<()> {
    let response = Response {
        ok: false,
        result: None,
        error: Some(why.to_string()),
    };
    write_frame(write, &response).await
}

async fn read_frame<T: DeserializeOwned>(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
) -> Result<T> {
    let mut line = String::new();
    let read = (&mut *reader)
        .take(MAX_FRAME as u64)
        .read_line(&mut line)
        .await?;
    if read == 0 {
        return Err(Error::Unavailable("control client hung up".into()));
    }
    Ok(serde_json::from_str(&line)?)
}

async fn write_frame(write: &mut OwnedWriteHalf, frame: &impl Serialize) -> Result<()> {
    let mut line = serde_json::to_vec(frame)?;
    line.push(b'\n');
    write.write_all(&line).await?;
    Ok(())
}
