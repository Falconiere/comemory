#!/usr/bin/env python3
"""A loopback stand-in for GitHub Releases, for scripts/test-daemon-install.sh.

Mirrors tests/common/release_server.rs's three request shapes exactly:
  - GET/HEAD /latest             -> 302 to /tag/<latest>
  - GET/HEAD /tag/<t>             -> 200 (a stub body)
  - GET/HEAD /download/<tag>/<f>  -> the file's bytes, from `root`

Usage: release_server.py <root-dir> <latest-tag>
Binds 127.0.0.1 on an ephemeral port, prints the port on stdout (alone, on
its own line, then flushed) and serves until killed.
"""

import http.server
import os
import sys
import threading


def make_handler(root: str, latest: str):
    class Handler(http.server.BaseHTTPRequestHandler):
        server_version = "comemory-release-fixture/1"

        def log_message(self, fmt, *args):  # noqa: A002 - stdlib override
            pass

        def _route(self):
            path = self.path
            if path == "/latest":
                return (302, [("Location", f"/tag/{latest}")], b"")
            if path.startswith("/tag/"):
                return (200, [("Content-Type", "text/html")], b"<html>release</html>")
            prefix = "/download/"
            if path.startswith(prefix):
                rel = path[len(prefix):]
                full = os.path.normpath(os.path.join(root, rel))
                if not full.startswith(os.path.normpath(root) + os.sep):
                    return (404, [], b"not found")
                try:
                    with open(full, "rb") as handle:
                        body = handle.read()
                except OSError:
                    return (404, [], b"not found")
                return (200, [("Content-Type", "application/octet-stream")], body)
            return (404, [], b"not found")

        def _respond(self, send_body: bool):
            status, headers, body = self._route()
            self.send_response(status)
            for key, value in headers:
                self.send_header(key, value)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if send_body:
                self.wfile.write(body)

        def do_GET(self):  # noqa: N802 - stdlib override
            self._respond(True)

        def do_HEAD(self):  # noqa: N802 - stdlib override
            self._respond(False)

    return Handler


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: release_server.py <root-dir> <latest-tag>", file=sys.stderr)
        return 2
    root, latest = sys.argv[1], sys.argv[2]
    handler = make_handler(root, latest)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    port = server.server_address[1]
    print(port, flush=True)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    thread.join()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
