#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::domains::sync::daemon::client`] against real Unix
//! sockets: nothing there, a dead leftover, a foreign listener, a listener
//! that never answers, another protocol, another directory, and a listener
//! that proves the token and answers for this directory.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use comemory::config::paths::Paths;
use comemory::domains::sync::daemon::client::{Probe, probe};
use comemory::domains::sync::daemon::control::{Hello, Request, Response};
use comemory::domains::sync::daemon::handshake;
use comemory::domains::sync::daemon::readiness::{AuthView, Readiness, StoreState, SyncView};
use comemory::domains::sync::daemon::socket_path::SOCKET_FILE;

const BOUND: Duration = Duration::from_secs(2);

struct Home {
    _dir: tempfile::TempDir,
    paths: Paths,
    canonical: PathBuf,
}

fn home() -> Home {
    let dir = tempfile::Builder::new()
        .prefix("cm")
        .tempdir_in("/tmp")
        .unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    Home {
        paths: Paths::new(&canonical),
        canonical,
        _dir: dir,
    }
}

fn readiness_for(data_dir: &Path) -> Readiness {
    Readiness {
        protocol: 1,
        version: env!("CARGO_PKG_VERSION").into(),
        binary: "/usr/bin/comemory".into(),
        pid: std::process::id(),
        instance: "0123456789abcdef".into(),
        started_at: "2026-09-25T10:00:00Z".into(),
        data_dir: data_dir.to_path_buf(),
        socket: data_dir.join(SOCKET_FILE),
        supervisor: "foreground".into(),
        store: StoreState::Absent,
        auth: AuthView::default(),
        sync: SyncView::default(),
    }
}

fn send(stream: &mut UnixStream, frame: &impl serde::Serialize) {
    let mut line = serde_json::to_vec(frame).unwrap();
    line.push(b'\n');
    stream.write_all(&line).unwrap();
}

/// Accept one connection; answer the hello with `server_hello(client_nonce)`;
/// return every further line the client sent (until it hangs up), so a test
/// can prove the client revealed nothing to an unverified server.
fn listen(
    socket: &Path,
    server_hello: impl FnOnce(&str) -> Hello + Send + 'static,
) -> std::thread::JoinHandle<Vec<String>> {
    let listener = UnixListener::bind(socket).unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let hello: Hello = serde_json::from_str(&line).unwrap();
        send(&mut writer, &server_hello(&hello.nonce));
        reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        reader.lines().map_while(std::result::Result::ok).collect()
    })
}

#[test]
fn no_token_and_no_socket_mean_not_running() {
    let h = home();
    assert!(matches!(probe(&h.paths, BOUND), Probe::NotRunning(_)));
    handshake::load_or_create(&h.paths).unwrap();
    assert!(matches!(probe(&h.paths, BOUND), Probe::NotRunning(_)));
}

#[test]
fn a_dead_leftover_socket_means_not_running() {
    let h = home();
    handshake::load_or_create(&h.paths).unwrap();
    drop(UnixListener::bind(h.canonical.join(SOCKET_FILE)).unwrap());
    assert!(
        h.canonical.join(SOCKET_FILE).exists(),
        "the file outlives its listener"
    );
    let Probe::NotRunning(why) = probe(&h.paths, BOUND) else {
        panic!("expected not running")
    };
    assert!(why.contains("nothing listens"), "{why}");
}

#[test]
fn a_foreign_listener_fails_the_proof_and_learns_nothing() {
    let h = home();
    handshake::load_or_create(&h.paths).unwrap();
    let seen = listen(&h.canonical.join(SOCKET_FILE), |_| Hello {
        hello: 1,
        nonce: "feedface".into(),
        proof: Some("0".repeat(64)),
    });
    let Probe::Stale(why) = probe(&h.paths, BOUND) else {
        panic!("expected stale")
    };
    assert!(why.contains("identity proof"), "{why}");
    assert!(
        seen.join().unwrap().is_empty(),
        "the client sent no proof and no op"
    );
}

#[test]
fn another_protocol_is_stale() {
    let h = home();
    handshake::load_or_create(&h.paths).unwrap();
    let seen = listen(&h.canonical.join(SOCKET_FILE), |_| Hello {
        hello: 2,
        nonce: "n".into(),
        proof: None,
    });
    let Probe::Stale(why) = probe(&h.paths, BOUND) else {
        panic!("expected stale")
    };
    assert!(why.contains("protocol 2"), "{why}");
    assert!(seen.join().unwrap().is_empty());
}

#[test]
fn a_listener_that_never_answers_costs_the_bound_and_is_stale() {
    let h = home();
    handshake::load_or_create(&h.paths).unwrap();
    let listener = UnixListener::bind(h.canonical.join(SOCKET_FILE)).unwrap();
    let started = Instant::now();
    let outcome = probe(&h.paths, Duration::from_millis(500));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    let Probe::Stale(why) = outcome else {
        panic!("expected stale")
    };
    assert!(why.contains("within the bound"), "{why}");
    drop(listener);
}

fn answering(h: &Home, answer_for: PathBuf) -> std::thread::JoinHandle<()> {
    let token = handshake::load_or_create(&h.paths).unwrap();
    let listener = UnixListener::bind(h.canonical.join(SOCKET_FILE)).unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let hello: Hello = serde_json::from_str(&line).unwrap();
        send(
            &mut writer,
            &Hello {
                hello: 1,
                nonce: "servernonce".into(),
                proof: Some(handshake::server_proof(&token, &hello.nonce)),
            },
        );
        line.clear();
        reader.read_line(&mut line).unwrap();
        let request: Request = serde_json::from_str(&line).unwrap();
        assert!(handshake::matches(
            &handshake::client_proof(&token, "servernonce"),
            &request.proof
        ));
        send(
            &mut writer,
            &Response {
                ok: true,
                result: Some(serde_json::to_value(readiness_for(&answer_for)).unwrap()),
                error: None,
            },
        );
    })
}

#[test]
fn a_verified_answer_for_this_directory_is_healthy() {
    let h = home();
    let server = answering(&h, h.canonical.clone());
    let Probe::Healthy(readiness) = probe(&h.paths, BOUND) else {
        panic!("expected healthy")
    };
    assert_eq!(readiness.data_dir, h.canonical);
    server.join().unwrap();
}

#[test]
fn a_verified_answer_for_another_directory_is_stale() {
    let h = home();
    let server = answering(&h, PathBuf::from("/somewhere/else"));
    let Probe::Stale(why) = probe(&h.paths, BOUND) else {
        panic!("expected stale")
    };
    assert!(why.contains("/somewhere/else"), "{why}");
    server.join().unwrap();
}
