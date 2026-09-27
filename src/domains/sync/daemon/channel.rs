//! The workspace channel inside the coordinator: its own thread and
//! current-thread runtime, so the blocking ticket mint never stalls the
//! control socket. A nudge becomes a wake; the channel follows only while a
//! usable credential stands, and restarts whenever the control generation
//! moves (a reload, a login, a logout, shutdown).

use std::sync::Arc;
use std::thread::JoinHandle;

use tokio::sync::watch as signal;

use crate::config::Paths;
use crate::domains::sync::AuthFile;
use crate::domains::sync::daemon::readiness::{ChannelState, Trigger};
use crate::domains::sync::daemon::state::State;
use crate::domains::sync::daemon::worker::Queue;
use crate::domains::sync::drain::network;
use crate::domains::sync::watch::{self, Frame, OffRuntime};
use crate::prelude::*;
use crate::store::connection;
use crate::store::readiness::{self, StoreReadiness};
use crate::store::sync_exchange::{self, ExchangeKey};
use crate::utilities::digest::sha256_hex;

/// The generation value that tells the channel thread to exit.
pub const STOP: u64 = u64::MAX;

/// How often a followable-but-suspended credential is rechecked.
const SUSPENDED_RECHECK: std::time::Duration = std::time::Duration::from_secs(5);

/// Runs a blocking platform call on a scoped thread with no runtime in scope.
struct PlainThread;

impl OffRuntime for PlainThread {
    fn off<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T> + Send,
        T: Send,
    {
        std::thread::scope(|scope| scope.spawn(f).join())
            .map_err(|_| Error::Other("platform request thread panicked".into()))?
    }
}

/// Start the channel thread.
///
/// # Errors
/// The thread or its runtime cannot be created.
pub fn spawn(
    paths: Paths,
    state: Arc<State>,
    queue: Arc<Queue>,
    control: signal::Receiver<u64>,
) -> Result<JoinHandle<()>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    Ok(std::thread::Builder::new()
        .name("comemory-sync-channel".into())
        .spawn(move || runtime.block_on(follow(&paths, &state, &queue, control)))?)
}

/// Whether `auth`'s key is suspended on the same signal the exchange drain
/// gates on — a channel follower must not hammer the ticket endpoint for a
/// credential the platform already told us is invalid.
fn suspended(paths: &Paths, auth: &AuthFile) -> bool {
    let Ok(conn) = connection::open_read_only(paths.db_path()) else {
        return false;
    };
    let key = ExchangeKey::new(&auth.api_url, &auth.workspace_id);
    let Ok(Some(row)) = sync_exchange::load(&conn, &key) else {
        return false;
    };
    if row.network_state != "auth_suspended" {
        return false;
    }
    let secret = auth.effective_secret();
    let fingerprint =
        sha256_hex(format!("{}\0{}\0{secret}", key.api_url, key.workspace_id).as_bytes());
    network::gated(&row, &fingerprint, false).unwrap_or(false)
}

async fn follow(paths: &Paths, state: &State, queue: &Queue, mut control: signal::Receiver<u64>) {
    loop {
        if *control.borrow_and_update() == STOP {
            return;
        }
        let usable = AuthFile::load_usable(paths).ok().flatten();
        let followable = usable.filter(|auth| {
            matches!(
                readiness::probe(&paths.db_path()),
                Ok(StoreReadiness::Ready)
            ) && !suspended(paths, auth)
        });
        let Some(auth) = followable else {
            state.set_channel(ChannelState::Off);
            tokio::select! {
                changed = control.changed() => if changed.is_err() { return; },
                () = tokio::time::sleep(SUSPENDED_RECHECK) => {}
            }
            continue;
        };
        let mut on_frame = |frame: Frame| {
            match frame {
                Frame::Connecting => state.set_channel(ChannelState::Connecting),
                Frame::Connected => state.set_channel(ChannelState::Connected),
                Frame::Retrying => state.set_channel(ChannelState::Backoff),
                Frame::Nudge => queue.wake(Trigger::Channel, None),
            }
            Ok(false)
        };
        tokio::select! {
            ended = watch::channel_loop(&auth, false, &PlainThread, &mut on_frame) => {
                if let Err(e) = ended {
                    tracing::debug!(error = %e, "workspace channel loop ended");
                }
            }
            changed = control.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}
