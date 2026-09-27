#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Coordinator health checks against real OS threads that exit or panic.

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use super::{Tasks, initial_readiness, wait_for_stop};
use crate::config::Paths;
use crate::domains::sync::daemon::state::State;
use crate::domains::sync::daemon::worker::Queue;

fn waiting_thread(panics: bool) -> (JoinHandle<()>, Sender<()>) {
    let (tx, rx) = channel();
    let thread = std::thread::spawn(move || {
        rx.recv().expect("release thread");
        assert!(!panics, "intentional background-thread panic");
    });
    (thread, tx)
}

fn tasks(worker_panics: bool, channel_panics: bool) -> (Tasks, Sender<()>, Sender<()>) {
    let (worker, release_worker) = waiting_thread(worker_panics);
    let (channel, release_channel) = waiting_thread(channel_panics);
    let (gen_tx, _) = tokio::sync::watch::channel(0);
    (
        Tasks {
            worker,
            channel,
            queue: Queue::new(),
            shutdown: Arc::new(tokio::sync::Notify::new()),
            reload: Arc::new(tokio::sync::Notify::new()),
            gen_tx,
        },
        release_worker,
        release_channel,
    )
}

#[tokio::test]
async fn an_unexpected_worker_or_channel_exit_stops_the_coordinator() {
    for (name, panics) in [
        ("pass worker", false),
        ("workspace channel", false),
        ("pass worker", true),
        ("workspace channel", true),
    ] {
        let home = tempfile::tempdir().expect("home");
        let paths = Paths::new(home.path());
        let socket = home.path().join("daemon.sock");
        let state = State::new(initial_readiness(&paths, home.path(), &socket).expect("readiness"));
        let worker_panics = name == "pass worker" && panics;
        let channel_panics = name == "workspace channel" && panics;
        let (tasks, worker, channel) = tasks(worker_panics, channel_panics);
        let (failed, remaining) = if name == "pass worker" {
            (worker, channel)
        } else {
            (channel, worker)
        };
        failed.send(()).expect("exit selected thread");
        let error = tokio::time::timeout(
            Duration::from_secs(5),
            wait_for_stop(&paths, &state, &tasks),
        )
        .await
        .expect("coordinator must observe the exit")
        .expect_err("an unexpectedly exited thread is unhealthy");
        assert!(error.to_string().contains(name), "{error}");
        remaining.send(()).expect("release remaining thread");
        assert_eq!(tasks.worker.join().is_err(), worker_panics);
        assert_eq!(tasks.channel.join().is_err(), channel_panics);
    }
}

#[tokio::test]
async fn a_requested_shutdown_succeeds_while_background_threads_are_alive() {
    let home = tempfile::tempdir().expect("home");
    let paths = Paths::new(home.path());
    let socket = home.path().join("daemon.sock");
    let state = State::new(initial_readiness(&paths, home.path(), &socket).expect("readiness"));
    let (tasks, worker, channel) = tasks(false, false);
    tasks.shutdown.notify_one();
    tokio::time::timeout(
        Duration::from_secs(5),
        wait_for_stop(&paths, &state, &tasks),
    )
    .await
    .expect("bounded shutdown")
    .expect("normal shutdown");
    worker.send(()).expect("release worker");
    channel.send(()).expect("release channel");
    tasks.worker.join().expect("join worker");
    tasks.channel.join().expect("join channel");
}
