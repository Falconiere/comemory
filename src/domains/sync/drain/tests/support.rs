#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! A REAL engine for the drain module's tests: the production router over a
//! real temp data directory, served on a loopback socket so the client's own
//! HTTP stack talks to it exactly as it talks to a hub.

use std::net::SocketAddr;

use crate::test_common::serve_state::{self, Session};

/// An in-process engine listening on `127.0.0.1:<port>`.
pub struct LiveEngine {
    /// The session (its data dir and token).
    pub session: Session,
    /// `http://127.0.0.1:<port>/api` — the `api_url` a client names.
    pub api_url: String,
    /// The bound address.
    pub addr: SocketAddr,
}

impl LiveEngine {
    /// Start one over a fresh data directory.
    pub fn start() -> Self {
        let session = serve_state::session(false);
        let router = session.router.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        listener.set_nonblocking(true).expect("nonblocking");
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
                axum::serve(listener, router).await.expect("serve");
            });
        });
        Self {
            session,
            api_url: format!("http://{addr}/api"),
            addr,
        }
    }

    /// The session token — the credential a client presents.
    pub fn token(&self) -> String {
        self.session.token.clone()
    }
}

impl LiveEngine {
    /// The key a client of this engine files everything under.
    pub fn key(&self) -> crate::store::sync_exchange::ExchangeKey {
        crate::store::sync_exchange::ExchangeKey::new(&self.api_url, "ws")
    }

    /// A plain transport to this engine (it is unmanaged: no policy headers).
    pub fn transport(&self) -> crate::domains::sync::drain::transport::Transport {
        crate::domains::sync::drain::transport::Transport::new(
            &self.api_url,
            &self.token(),
            std::time::Duration::from_secs(10),
        )
        .expect("transport")
    }

    /// The engine's feed above `since` — exactly what a client pull reads.
    pub fn changes(
        &self,
        since: i64,
        kind: Option<&str>,
    ) -> crate::domains::sync::replica::contract_views::ChangesResponse {
        let mut query = vec![("since", since.to_string()), ("limit", "500".to_string())];
        if let Some(kind) = kind {
            query.push(("kind", kind.to_string()));
        }
        self.transport()
            .get("/v1/sync/replica/changes", &query)
            .expect("changes")
    }

    /// Operation ids in the engine's feed, in position order.
    pub fn feed(&self) -> Vec<String> {
        let conn =
            rusqlite::Connection::open(self.session.home.path().join("comemory.db")).expect("hub");
        let mut statement = conn
            .prepare("SELECT operation_id FROM replica_feed ORDER BY sequence")
            .expect("prepare");
        statement
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect")
    }
}

/// Persist the policy snapshot a policy load would for `key` — approving
/// `repos` — and return the policy it resolves to.
pub fn approve(
    conn: &rusqlite::Connection,
    key: &crate::store::sync_exchange::ExchangeKey,
    repos: &[&str],
) -> crate::domains::sync::repository_policy::RepositoryPolicy {
    crate::store::sync_policy_snapshot::save(
        conn,
        key,
        &crate::store::sync_policy_snapshot::PolicySnapshot {
            revision: 1,
            fingerprint: format!("fp-{}", repos.join(",")),
            allowlist: repos.iter().map(|r| (*r).to_string()).collect(),
            mappings: std::collections::BTreeMap::new(),
            loaded_at: "t".into(),
        },
    )
    .expect("snapshot");
    crate::domains::sync::repository_policy::RepositoryPolicy::from_snapshot(conn, key)
        .expect("policy")
}

/// Push every eligible operation of `home` to `engine`, as a pass would.
pub fn push_all(home: &crate::domains::sync::replica::test_support::Home, engine: &LiveEngine) {
    let key = engine.key();
    let policy = approve(&home.conn, &key, &["falconiere/comemory"]);
    let transport = engine.transport();
    let push = crate::domains::sync::drain::push::Push {
        key: &key,
        transport: &transport,
        policy: &policy,
        cursor: None,
        max_bytes: 4 << 20,
        epoch: None,
        resting: &std::collections::BTreeSet::new(),
    };
    let pushed = crate::domains::sync::drain::push::batch(&home.conn, &push, "t").expect("push");
    assert!(pushed.failure.is_none(), "{pushed:?}");
}

/// A logged-in client home for `engine`: `auth.json` names it, `REPO` is
/// approved for its key.
pub fn client_of(engine: &LiveEngine) -> crate::domains::sync::replica::test_support::Home {
    let home = crate::domains::sync::replica::test_support::Home::new();
    crate::test_common::auth_fixture::seed_org_auth(
        &home.paths,
        &engine.api_url,
        &engine.token(),
        "ws",
    );
    approve(&home.conn, &engine.key(), &["falconiere/comemory"]);
    home
}

/// A writer whose history gives one memory two feed positions: `m` saved,
/// `n` saved, then `m` re-tagged — pushed to `engine` in that order.
/// Returns `(m, n)`.
pub fn history_of_two(engine: &LiveEngine) -> (String, String) {
    let mut writer = crate::domains::sync::replica::test_support::Home::new();
    let m = writer.save(
        crate::domains::sync::replica::test_support::BODY,
        &["first"],
    );
    let n = writer.save("A second memory with a single position.", &["sync"]);
    let retag = crate::domains::memories::update::Request {
        kind: None,
        repo: None,
        tags: Some(vec!["second".to_string()]),
        quality: None,
        body: None,
        title: None,
    };
    let mut ctx = writer.ctx();
    crate::domains::memories::update::run(&mut ctx, &m, retag).expect("retag");
    push_all(&writer, engine);
    (m, n)
}

/// Post `home`'s pending memory operation for `entity_key` straight to the
/// engine under `operation_id` (its own, or another the way a legacy delivery
/// lands the same bytes), recording nothing locally.
pub fn deliver(
    engine: &LiveEngine,
    home: &crate::domains::sync::replica::test_support::Home,
    entity_key: &str,
    operation_id: Option<&str>,
) {
    use crate::domains::sync::replica::contract::{ImportRequest, ImportResponse, PROTOCOL};
    use crate::store::replica_outbox::{self, Scope};
    let row = replica_outbox::read(&home.conn, Scope::Entity("memory", entity_key), 1)
        .expect("row")
        .remove(0);
    let policy = approve(&home.conn, &engine.key(), &["falconiere/comemory"]);
    let mut prepared = crate::domains::sync::drain::push_batch::prepare(
        &home.conn,
        &engine.key(),
        &policy,
        row,
        "t",
    )
    .expect("prepare")
    .expect("payload held");
    if let Some(id) = operation_id {
        prepared.operation.operation_id = id.to_string();
    }
    let request = ImportRequest {
        protocol: PROTOCOL.to_string(),
        cursor: None,
        operations: vec![prepared.operation],
        workspace_id: None,
    };
    let answer: ImportResponse = engine
        .transport()
        .post("/v1/sync/replica/import", &request)
        .expect("import");
    assert_eq!(answer.results.len(), 1);
}
