//! Opt-in regression against the real managed platform and its repository policy gate.
//! Supply `COMEMORY_MANAGED_AUTH_FILE` and run this target with `--ignored`.

use std::error::Error;
use std::fs;
use std::time::Duration;

use assert_cmd::Command;
use comemory::domains::sync::AuthFile;
use comemory::domains::sync::client_protocol::{PROTOCOL_HEADER, REVISION_HEADER, SYNC_PROTOCOL};
use reqwest::blocking::{Client, Response};
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

fn manifest(
    client: &Client,
    auth: &AuthFile,
    protocol: &str,
    revision: i64,
) -> reqwest::Result<Response> {
    client
        .get(format!(
            "{}/v1/sync/replica/manifest",
            auth.api_url.trim_end_matches('/')
        ))
        .bearer_auth(auth.effective_secret())
        .header(PROTOCOL_HEADER, protocol)
        .header(REVISION_HEADER, revision.to_string())
        .send()
}

#[test]
#[ignore = "requires a real managed platform organization-scoped credential"]
fn managed_replica_verify_obeys_the_repository_policy_contract() -> TestResult {
    let auth_bytes = fs::read(std::env::var("COMEMORY_MANAGED_AUTH_FILE")?)?;
    let auth: AuthFile = serde_json::from_slice(&auth_bytes)?;
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
    let status: Value = client
        .get(format!(
            "{}/v1/sync/status",
            auth.api_url.trim_end_matches('/')
        ))
        .bearer_auth(auth.effective_secret())
        .send()?
        .error_for_status()?
        .json()?;
    assert_eq!(status["data"]["sync_protocol"], SYNC_PROTOCOL);
    let revision = status["data"]["policy_revision"]
        .as_i64()
        .ok_or("status has no policy revision")?;

    let response = manifest(&client, &auth, SYNC_PROTOCOL, revision)?.error_for_status()?;
    assert_eq!(response.headers()[PROTOCOL_HEADER], SYNC_PROTOCOL);
    assert_eq!(response.headers()[REVISION_HEADER], revision.to_string());
    let body: Value = response.json()?;
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["protocol"], "replica-v1");
    assert!(
        body["data"]["capabilities"]
            .as_array()
            .ok_or("manifest has no capabilities")?
            .iter()
            .any(|capability| capability == "replica-v1")
    );

    for (protocol, sent_revision, code) in [
        ("replica-v1", revision, "sync_upgrade_required"),
        (SYNC_PROTOCOL, revision - 1, "sync_policy_changed"),
    ] {
        let response = manifest(&client, &auth, protocol, sent_revision)?;
        assert_eq!(response.status().as_u16(), 409);
        let body: Value = response.json()?;
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"]["code"], code);
    }

    // Only the credential is copied: no local outbox or test memories can be uploaded.
    let home = tempfile::tempdir()?;
    fs::write(home.path().join("auth.json"), auth_bytes)?;
    let output = Command::new(env!("CARGO_BIN_EXE_comemory"))
        .timeout(Duration::from_secs(120))
        .env("COMEMORY_SYNC_DAEMON", "0")
        .env("COMEMORY_INDEXING_AUTO_REINDEX", "off")
        .args(["--data-dir"])
        .arg(home.path())
        .args(["sync", "--action", "verify", "--json"])
        .output()?;
    assert!(
        output.status.success(),
        "verify failed with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)?;
    let kinds = report["kinds"]
        .as_array()
        .ok_or("no replica verification report")?;
    assert!(report["held_positions"].is_u64());
    for kind in kinds {
        assert!(kind["kind"].is_string());
        assert!(kind["differing_buckets"].is_u64());
        assert!(kind["repaired"].is_boolean());
    }
    let mut expected: Vec<&str> = body["data"]["capabilities"]
        .as_array()
        .ok_or("manifest has no capabilities")?
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|capability| capability.split_once('@').map(|(kind, _)| kind))
        .collect();
    let mut reported: Vec<&str> = kinds
        .iter()
        .filter_map(|kind| kind["kind"].as_str())
        .collect();
    expected.sort_unstable();
    reported.sort_unstable();
    assert_eq!(reported, expected);
    Ok(())
}
