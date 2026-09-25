//! The coordinator's shared state: the readiness it answers with, kept
//! current by the worker, the ticker and the channel thread, plus the event
//! stream `subscribe` relays. Answering `status` reads only this — never the
//! store, never the network.

use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::broadcast;

use crate::config::Paths;
use crate::domains::sync::AuthFile;
use crate::domains::sync::auth_barrier;
use crate::domains::sync::daemon::control::Event;
use crate::domains::sync::daemon::readiness::{
    AuthState, AuthView, ChannelState, PassSummary, Readiness, StoreState, SyncState,
};

/// Events buffered for a slow subscriber before it starts losing them.
const EVENT_BUFFER: usize = 256;

/// Shared coordinator state.
pub struct State {
    readiness: Mutex<Readiness>,
    events: broadcast::Sender<Event>,
}

impl State {
    /// State seeded with the coordinator's identity.
    #[must_use]
    pub fn new(readiness: Readiness) -> Arc<Self> {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Arc::new(Self {
            readiness: Mutex::new(readiness),
            events,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Readiness> {
        // A panic while holding this lock cannot leave a torn value: every
        // writer assigns whole fields.
        self.readiness
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The current readiness.
    #[must_use]
    pub fn readiness(&self) -> Readiness {
        self.lock().clone()
    }

    /// A new subscriber's receiver.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn publish(&self, event: Event) {
        // No subscriber is not an error.
        let _ = self.events.send(event);
    }

    /// The reconciliation interval in force.
    pub fn set_interval(&self, secs: u64) {
        self.lock().sync.interval_secs = secs;
    }

    /// Whether work waits for the next pass.
    pub fn set_queued(&self, queued: bool) {
        self.lock().sync.queued = queued;
    }

    /// The worker began a pass.
    pub fn pass_started(&self, trigger: crate::domains::sync::daemon::readiness::Trigger) {
        self.lock().sync.state = SyncState::Running;
        self.publish(Event::PassStarted { trigger });
    }

    /// The worker finished a pass.
    pub fn pass_finished(&self, summary: &PassSummary) {
        {
            let mut r = self.lock();
            if r.sync.state == SyncState::Running {
                r.sync.state = SyncState::Idle;
            }
            r.sync.passes += 1;
            r.store = summary.store;
            r.sync.last_pass = Some(summary.clone());
        }
        self.publish(Event::PassFinished(summary.clone()));
    }

    /// A verify succeeded at `at`.
    pub fn verified(&self, at: String) {
        self.lock().sync.last_verify_at = Some(at);
    }

    /// The store's state, probed outside a pass.
    pub fn set_store(&self, store: StoreState) {
        self.lock().store = store;
    }

    /// The channel's new state.
    pub fn set_channel(&self, channel: ChannelState) {
        let changed = {
            let mut r = self.lock();
            let changed = r.sync.channel != channel;
            r.sync.channel = channel;
            changed
        };
        if changed {
            self.publish(Event::Channel { state: channel });
        }
    }

    /// Stopping: no new pass starts.
    pub fn set_stopping(&self) {
        self.lock().sync.state = SyncState::Stopping;
    }

    /// Re-read the credential; `true` when its public face changed.
    pub fn refresh_auth(&self, paths: &Paths) -> bool {
        let view = auth_view(paths);
        let mut r = self.lock();
        let changed = r.auth != view;
        r.auth = view;
        changed
    }
}

/// The credential's public face — never the secret.
#[must_use]
pub fn auth_view(paths: &Paths) -> AuthView {
    if auth_barrier::active(paths) {
        return AuthView {
            state: AuthState::LoggedOutBarrier,
            ..AuthView::default()
        };
    }
    match AuthFile::load(paths) {
        Ok(Some(auth)) => AuthView {
            state: AuthState::Authenticated,
            api_url: Some(auth.api_url),
            organization_slug: Some(auth.organization_slug),
            workspace_id: Some(auth.workspace_id),
            key_prefix: Some(auth.key_prefix),
        },
        Ok(None) => AuthView::default(),
        Err(_) => AuthView {
            state: AuthState::Unusable,
            ..AuthView::default()
        },
    }
}
