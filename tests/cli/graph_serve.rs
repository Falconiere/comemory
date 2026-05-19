//! Spawn `qwick-memory graph serve --port 0 --no-open`, parse the URL
//! from stdout (tracing-subscriber writes to stdout by default), hit
//! `/api/seed`, and shut down.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

#[test]
fn graph_serve_starts_and_serves_seed() {
    let tmp = TempDir::new().expect("tempdir");

    let bin = env!("CARGO_BIN_EXE_qwick-memory");
    let mut child = Command::new(bin)
        .args(["graph", "serve", "--port", "0", "--no-open"])
        .env("QWICK_MEMORY_DATA_DIR", tmp.path())
        .env("RUST_LOG", "qwick_memory=info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn qwick-memory");

    // tracing-subscriber's default formatter writes to stdout (not stderr).
    // Verified empirically: redirecting fd-1 suppresses the tracing line;
    // redirecting fd-2 has no effect on it.
    let stdout = child.stdout.take().expect("child stdout");
    let mut reader = BufReader::new(stdout);

    // Matches the URL that tracing logs in the startup message, e.g.:
    // "... INFO qwick_memory::serve::router: qwick-memory graph viewer listening
    //  on http://127.0.0.1:12345; ... url=http://127.0.0.1:12345"
    // ANSI escape codes may appear around the `=` separator in the structured
    // field but the URL itself is emitted as a clean substring.
    let url_re = regex::Regex::new(r"http://[0-9a-fA-F.\[\]]+:[0-9]+").expect("regex");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut url: Option<String> = None;
    while Instant::now() < deadline {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        if let Some(m) = url_re.find(&line) {
            url = Some(m.as_str().to_string());
            break;
        }
    }
    let url = url.expect("did not see listening line with url within timeout");

    let body: serde_json::Value = reqwest::blocking::get(format!("{url}/api/seed?layer=memory"))
        .expect("GET /api/seed")
        .json()
        .expect("parse json");
    assert!(body.get("nodes").is_some(), "response missing 'nodes' key");
    assert!(body.get("edges").is_some(), "response missing 'edges' key");

    #[cfg(unix)]
    {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(child.id() as i32),
            nix::sys::signal::Signal::SIGINT,
        )
        .expect("sigint");
        let status = child.wait().expect("wait");
        assert!(
            status.success() || {
                use std::os::unix::process::ExitStatusExt;
                status.signal() == Some(libc::SIGINT)
            },
            "unexpected exit: {status:?}"
        );
    }
    #[cfg(not(unix))]
    {
        child.kill().expect("kill");
        let _ = child.wait();
    }
}
