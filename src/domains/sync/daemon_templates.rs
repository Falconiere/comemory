//! Plist / systemd unit file text for the required sync daemon, one per
//! canonical data directory ([`crate::domains::sync::daemon::identity`]).

use std::path::Path;

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render the LaunchAgent plist body for `label`.
///
/// `KeepAlive.SuccessfulExit = false`: a graceful stop (exit 0, e.g.
/// `comemory sync daemon stop`) is not respawned; a crash is. The next
/// ordinary CLI command brings a stopped coordinator back (D10).
pub fn render_launch_agent_plist(label: &str, exe: &Path, data_dir: &Path) -> String {
    let label_s = xml_escape(label);
    let exe_s = xml_escape(&exe.display().to_string());
    let data_s = xml_escape(&data_dir.display().to_string());
    let log_dir = data_dir.join("logs");
    let stdout = xml_escape(&log_dir.join("sync-daemon.out.log").display().to_string());
    let stderr = xml_escape(&log_dir.join("sync-daemon.err.log").display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label_s}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe_s}</string>
    <string>--data-dir</string>
    <string>{data_s}</string>
    <string>sync</string>
    <string>daemon</string>
    <string>run</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>WorkingDirectory</key>
  <string>{data_s}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>COMEMORY_DATA_DIR</key>
    <string>{data_s}</string>
  </dict>
  <key>StandardOutPath</key>
  <string>{stdout}</string>
  <key>StandardErrorPath</key>
  <string>{stderr}</string>
</dict>
</plist>
"#
    )
}

/// Render the systemd user unit body for `unit_name`'s description.
pub fn render_systemd_unit(exe: &Path, data_dir: &Path) -> String {
    // systemd treats `%` as a specifier; double it in paths.
    let exe_s = exe.display().to_string().replace('%', "%%");
    let data_s = data_dir.display().to_string().replace('%', "%%");
    format!(
        r"[Unit]
Description=comemory sync daemon ({data_s})
Documentation=https://comemory.io/docs

[Service]
Type=simple
ExecStart={exe_s} --data-dir {data_s} sync daemon run
WorkingDirectory={data_s}
Environment=COMEMORY_DATA_DIR={data_s}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"
    )
}

#[cfg(test)]
#[path = "tests/daemon_templates.rs"]
mod tests;
