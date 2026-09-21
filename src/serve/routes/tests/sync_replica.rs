#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The `replica-v1` route table: which routes are mounted, and which of them
//! the read-only gate must refuse. The over-the-wire behavior is proven in
//! `tests/replica_contract.rs` against a real `comemory serve`.

use comemory::serve::routes::table;

use super::table_entries;

#[test]
fn every_replica_route_is_in_the_shared_table_exactly_once() {
    let mounted = table();
    for entry in table_entries() {
        let matches: Vec<_> = mounted
            .iter()
            .filter(|e| e.path == entry.path && e.method == entry.method)
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "{} {} must appear once in the route table",
            entry.method,
            entry.path
        );
        assert_eq!(matches[0].command, entry.command);
        assert_eq!(matches[0].mutating, entry.mutating);
    }
}

#[test]
fn reads_are_read_class_and_writes_are_mutating() {
    let read: Vec<&str> = table_entries()
        .iter()
        .filter(|e| !e.mutating)
        .map(|e| e.path)
        .collect();
    let write: Vec<&str> = table_entries()
        .iter()
        .filter(|e| e.mutating)
        .map(|e| e.path)
        .collect();

    assert_eq!(
        read,
        vec![
            "/sync/replica/changes",
            "/sync/replica/manifest",
            "/sync/replica/events"
        ]
    );
    assert_eq!(
        write,
        vec![
            "/sync/replica/import",
            "/sync/replica/stage",
            "/sync/replica/activate"
        ],
        "staging and activation move state, so they take the write gate"
    );
}

#[test]
fn every_replica_command_is_namespaced_under_sync_replica() {
    for entry in table_entries() {
        assert!(
            entry.command.starts_with("sync.replica."),
            "{} should be namespaced: {}",
            entry.path,
            entry.command
        );
    }
}
