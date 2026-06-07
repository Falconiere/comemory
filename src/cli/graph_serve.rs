//! `comemory graph serve` — local HTTP viewer for the property graph.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::Graph;
use crate::prelude::*;
use crate::serve;

const EXAMPLES: &str = "\
Examples:
  # Open the viewer in the default browser
  comemory graph serve

  # Headless / over SSH
  comemory graph serve --no-open

  # Pin a port
  comemory graph serve --port 7878";

/// Arguments to `comemory graph serve`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Override the bind port. `0` lets the kernel pick a free port.
    #[arg(long, default_value_t = 0)]
    pub port: u16,
    /// Skip auto-opening the URL in the system browser.
    #[arg(long)]
    pub no_open: bool,
    /// Bind address. Loopback by default.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// Required when `--host` is non-loopback. Acknowledges the network
    /// exposure: the viewer is read-only but unauthenticated.
    #[arg(long)]
    pub bind_public: bool,
}

/// Spin up the local HTTP viewer for the property graph.
pub async fn run(a: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let host: IpAddr = a
        .host
        .parse()
        .map_err(|e| Error::Other(format!("--host {host}: {e}", host = a.host)))?;
    if !host.is_loopback() && !a.bind_public {
        return Err(Error::Other(
            "non-loopback --host requires --bind-public".into(),
        ));
    }
    if !host.is_loopback() {
        tracing::warn!(%host, "comemory graph serve is binding to a public address");
    }

    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let graph = Graph::open(paths.graph_dir())?;
    let state = serve::ServerState::new(graph, paths);
    let addr = SocketAddr::new(host, a.port);
    serve::router::run(state, addr, !a.no_open).await
}
