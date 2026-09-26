#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! Real-process harness for the daemon suites (#257): a private `HOME`, a
//! short data directory under `/tmp` (so the socket fits beside the data),
//! and the real CLI binary run with the harness switch removed and the
//! `process` supervisor forced — never the developer's launchd or systemd.
//!
//! Every coordinator a home started is stopped when the home drops: SIGTERM
//! to the pid it answers with, SIGKILL if it lingers, and the data directory
//! removed, which the coordinator's own guard also watches for.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin;
use serde_json::Value;

use comemory::config::paths::Paths;
use comemory::domains::sync::daemon::client::{self, Probe};
use comemory::domains::sync::daemon::readiness::Readiness;

/// A home: `HOME`, a data directory, and a short private `TMPDIR`.
pub struct DaemonHome {
    root: tempfile::TempDir,
    data: PathBuf,
    extra_env: Vec<(String, String)>,
}

impl Default for DaemonHome {
    fn default() -> Self {
        Self::new()
    }
}

impl DaemonHome {
    /// A home whose data directory is `<root>/d`.
    pub fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("cmd")
            .tempdir_in("/tmp")
            .expect("tempdir");
        let data = root.path().join("d");
        std::fs::create_dir_all(&data).expect("data dir");
        std::fs::create_dir_all(root.path().join("h")).expect("home dir");
        std::fs::create_dir_all(root.path().join("t")).expect("tmp dir");
        Self {
            root,
            data,
            extra_env: Vec::new(),
        }
    }

    /// A home whose data directory is `name` under the root (a long path).
    pub fn with_data_named(name: &str) -> Self {
        let mut home = Self::new();
        home.data = home.root.path().join(name);
        std::fs::create_dir_all(&home.data).expect("data dir");
        home
    }

    /// Add an environment variable to every command this home runs.
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.extra_env.push((key.into(), value.into()));
        self
    }

    /// The root every private directory lives under.
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    /// The data directory as given.
    pub fn data_dir(&self) -> PathBuf {
        self.data.clone()
    }

    /// The canonical data directory.
    pub fn canonical(&self) -> PathBuf {
        std::fs::canonicalize(&self.data).expect("canonical")
    }

    /// `Paths` over the data directory.
    pub fn paths(&self) -> Paths {
        Paths::new(&self.data)
    }

    /// The private `HOME`.
    pub fn home_dir(&self) -> PathBuf {
        self.root.path().join("h")
    }

    /// The CLI with this home's environment.
    pub fn command(&self) -> Command {
        self.command_with_binary(&cargo_bin("comemory"))
    }

    /// A `<root>/bin/comemory` symlink to the binary under test, so a real
    /// git hook's `command -v comemory` (or its `$HOME/.cargo/bin` etc.
    /// fallback search) finds this build rather than a host install.
    pub fn hook_bin_dir(&self) -> PathBuf {
        let dir = self.root.path().join("bin");
        if !dir.join("comemory").exists() {
            std::fs::create_dir_all(&dir).expect("hook bin dir");
            std::os::unix::fs::symlink(cargo_bin("comemory"), dir.join("comemory"))
                .expect("symlink comemory");
        }
        dir
    }

    /// A bare `PATH` naming only this home's `comemory` and the system git —
    /// never the host's own installed `comemory` (tests-firing-git-hooks
    /// note).
    pub fn hook_path(&self) -> std::ffi::OsString {
        let mut path = self.hook_bin_dir().into_os_string();
        path.push(":/usr/bin:/bin");
        path
    }

    /// Like [`Self::command`], running `bin` instead of the binary under
    /// test — a copy at another path, to prove identity-based replacement.
    pub fn command_with_binary(&self, bin: &Path) -> Command {
        let mut cmd = Command::new(bin);
        cmd.env("COMEMORY_DATA_DIR", &self.data)
            .env("HOME", self.home_dir())
            .env("TMPDIR", self.root.path().join("t"))
            .env("COMEMORY_DAEMON_SUPERVISOR", "process")
            .env("COMEMORY_INDEXING_AUTO_REINDEX", "off")
            .env_remove("COMEMORY_SYNC_DAEMON")
            .env_remove("XDG_RUNTIME_DIR")
            .env_remove("COMEMORY_API")
            .env_remove("COMEMORY_API_KEY");
        for (k, v) in &self.extra_env {
            cmd.env(k, v);
        }
        cmd
    }

    /// Run `comemory <args>`: `(exit code, stdout, stderr)`.
    pub fn run(&self, args: &[&str]) -> (i32, String, String) {
        self.run_with_env(args, &[])
    }

    /// Like [`Self::run`], with extra environment variables set.
    pub fn run_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
        let mut cmd = self.command();
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.args(args).output().expect("run comemory");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// Run `comemory --json <args>`, asserting success, and parse stdout.
    pub fn json(&self, args: &[&str]) -> Value {
        self.json_with_env(args, &[])
    }

    /// Like [`Self::json`], with extra environment variables set. A command
    /// this asserts success on may legitimately exit nonzero (`ensure` when
    /// it cannot reach readiness); callers that expect that parse `run`'s
    /// stdout themselves instead.
    pub fn json_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> Value {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        let (code, stdout, stderr) = self.run_with_env(&full, env);
        assert_eq!(code, 0, "comemory {args:?} failed: {stderr}\n{stdout}");
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout}"))
    }

    /// A foreground `comemory sync daemon run`, stderr to `<root>/fg-<n>.log`.
    pub fn spawn_foreground(&self, env: &[(&str, &str)]) -> Foreground {
        self.spawn_foreground_with_binary(&cargo_bin("comemory"), env)
    }

    /// Like [`Self::spawn_foreground`], running `bin` instead of the binary
    /// under test.
    pub fn spawn_foreground_with_binary(&self, bin: &Path, env: &[(&str, &str)]) -> Foreground {
        let log = self.root.path().join(format!("fg-{}.log", rand_suffix()));
        let mut cmd = self.command_with_binary(bin);
        cmd.env_remove("COMEMORY_DAEMON_SUPERVISOR");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = cmd
            .args(["sync", "daemon", "run"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).expect("log"))
            .spawn()
            .expect("spawn daemon run");
        Foreground { child, log }
    }

    /// A background `comemory serve` (ephemeral port), killed by the caller.
    pub fn spawn_serve(&self) -> std::process::Child {
        self.command()
            .args(["serve", "--port", "0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn serve")
    }

    /// A background `comemory mcp --read-only`, killed by the caller.
    pub fn spawn_mcp_read_only(&self) -> std::process::Child {
        self.command()
            .args(["mcp", "--read-only"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn mcp")
    }

    /// Probe the coordinator from this process.
    pub fn probe(&self) -> Probe {
        client::probe(&self.paths(), client::PROBE_BOUND)
    }

    /// Wait until a verified coordinator answers, or panic after `within`.
    pub fn wait_ready(&self, within: Duration) -> Readiness {
        let deadline = Instant::now() + within;
        loop {
            match self.probe() {
                Probe::Healthy(r) => return *r,
                other if Instant::now() >= deadline => panic!("no coordinator: {other:?}"),
                _ => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    /// Wait until `pred` holds for the coordinator's readiness.
    pub fn wait_for(&self, within: Duration, pred: impl Fn(&Readiness) -> bool) -> Readiness {
        let deadline = Instant::now() + within;
        loop {
            if let Probe::Healthy(r) = self.probe()
                && pred(&r)
            {
                return *r;
            }
            assert!(
                Instant::now() < deadline,
                "condition never held: {:?}",
                self.probe()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Every live `sync daemon run` process serving this data directory.
    pub fn coordinator_pids(&self) -> Vec<u32> {
        coordinator_pids_for(&self.canonical())
    }
}

impl Drop for DaemonHome {
    fn drop(&mut self) {
        if let Ok(canonical) = std::fs::canonicalize(&self.data) {
            for pid in coordinator_pids_for(&canonical) {
                terminate(pid);
            }
        }
        if let Probe::Healthy(r) = client::probe(&self.paths(), Duration::from_millis(500)) {
            terminate(r.pid);
        }
    }
}

/// A foreground coordinator child, killed and reaped on drop.
pub struct Foreground {
    /// The child.
    pub child: Child,
    /// Its stderr log.
    pub log: PathBuf,
}

impl Foreground {
    /// Its pid.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Wait up to `within` for it to exit; its code, or `None` if it lingers.
    pub fn wait_exit(&mut self, within: Duration) -> Option<i32> {
        let deadline = Instant::now() + within;
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return Some(status.code().unwrap_or(-1));
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Its log so far.
    pub fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Foreground {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Send `signal` (`TERM`, `KILL`, `STOP`, `CONT`) to `pid`.
pub fn signal(pid: u32, signal: &str) {
    let _ = Command::new("kill")
        .args([format!("-{signal}"), pid.to_string()])
        .status();
}

/// Whether `pid` is alive.
pub fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// SIGTERM, then SIGKILL after two seconds.
pub fn terminate(pid: u32) {
    signal(pid, "TERM");
    let deadline = Instant::now() + Duration::from_secs(2);
    while alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    if alive(pid) {
        signal(pid, "KILL");
    }
}

/// Live `… --data-dir <dir> sync daemon run` processes (plus foreground
/// ones started through `COMEMORY_DATA_DIR`, which `ps` cannot attribute).
pub fn coordinator_pids_for(canonical: &Path) -> Vec<u32> {
    let out = Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .expect("ps");
    let needle = format!("--data-dir {} sync daemon run", canonical.display());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(&needle))
        .filter_map(|l| l.split_whitespace().next()?.parse().ok())
        .collect()
}

fn rand_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("{nanos:09}")
}
