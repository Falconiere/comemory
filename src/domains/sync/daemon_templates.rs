//! Plist / systemd unit file text for the sync daemon.

use std::path::Path;

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render the LaunchAgent plist body for label `io.comemory.sync`.
pub fn render_launch_agent_plist(exe: &Path, data_dir: &Path) -> String {
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
  <string>io.comemory.sync</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe_s}</string>
    <string>sync</string>
    <string>daemon</string>
    <string>run</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
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

/// Render the systemd user unit body.
pub fn render_systemd_unit(exe: &Path, data_dir: &Path) -> String {
    // systemd treats `%` as a specifier; double it in paths.
    let exe_s = exe.display().to_string().replace('%', "%%");
    let data_s = data_dir.display().to_string().replace('%', "%%");
    format!(
        r"[Unit]
Description=comemory organization sync daemon
Documentation=https://comemory.io/docs

[Service]
Type=simple
ExecStart={exe_s} sync daemon run
WorkingDirectory={data_s}
Environment=COMEMORY_DATA_DIR={data_s}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"
    )
}
