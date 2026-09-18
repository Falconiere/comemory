"""The warm local inference option: a socket server and its thin client.

A cold cross-encoder start pays interpreter startup, a torch import and weight
loading on every request. The wire protocol is deliberately one child process
per request and this module does not change that. Instead the heavy work moves
behind a local Unix socket: `serve` loads the model once and keeps it, and
`client` is a child process that imports nothing heavier than `socket`, so the
#211 contract the Rust runner drives — one JSON object in on stdin, one JSON
object out on stdout, exit 0 — holds exactly as before.

The socket carries a small framed transport that is an implementation detail of
this option and explicitly **not** the wire protocol. Framing is needed because
a stream socket has no end-of-message the way a closed stdin does, and a
distinct error frame is needed because the protocol's response object has no
error representation: a refusal has to reach the Rust caller as an exit code,
and the error frame is how the server tells the client which code to exit with.

    frame := kind:u8 | code:u8 | length:u32 big-endian | payload[length]
    kind 1   request / response   code 0, payload is the protocol JSON object
    kind 2   error                code is the sysexits code, payload is a message

Requests are served one at a time. The model is the bottleneck, so concurrency
would multiply resident memory without shortening the queue, and comemory
issues one rerank per search.
"""

from __future__ import annotations

import json
import os
import socket
import struct
import sys
import time

import comemory_rerank_pins as pins
import comemory_rerank_protocol as wire

KIND_PAYLOAD = 1
KIND_ERROR = 2

_HEADER = struct.Struct(">BBI")

# A frame claiming more than the pinned request ceiling is refused before a
# single byte of it is allocated.
MAX_FRAME_BYTES = pins.MAX_REQUEST_BYTES

DEFAULT_CONNECT_TIMEOUT = 2.0
# Below the Rust runner's 20-second `DEFAULT_RERANK_TIMEOUT`, so a stalled
# server makes the client produce a diagnostic rather than being killed
# mid-write with nothing to show for it.
DEFAULT_REQUEST_TIMEOUT = 15.0

_BACKLOG = 16


def serve(scorer, path: str, ready_file: str | None, idle_timeout: float) -> int:
    """Load once, then answer framed requests on `path` until stopped.

    Returns the exit code. The socket file is always removed on the way out,
    including when loading fails, so a failed start never leaves a path behind
    that the next start would have to decide whether to reclaim.
    """
    server = _bind(path)
    try:
        load_ms = scorer.load()
        _diag("fingerprint " + json.dumps(scorer.fingerprint(), separators=(",", ":")))
        if ready_file:
            _write_ready(ready_file, path)
        _diag("ready socket=" + path + " load_ms=" + str(load_ms))
        _accept_loop(server, scorer, idle_timeout)
    finally:
        server.close()
        _unlink_quietly(path)
    return pins.EX_OK


def client(path: str, connect_timeout: float, request_timeout: float) -> int:
    """Forward one request over `path` and write the response to stdout."""
    raw = sys.stdin.buffer.read()
    if not raw.strip():
        raise pins.RerankError("empty stdin; expected exactly one JSON request object")
    with _connect(path, connect_timeout) as sock:
        sock.settimeout(request_timeout)
        try:
            _write_frame(sock, KIND_PAYLOAD, 0, raw)
            kind, code, payload = _read_frame(sock)
        except socket.timeout as exc:
            raise pins.RerankError(
                "reranker server at " + path + " did not answer within "
                + str(request_timeout) + "s",
                pins.EX_UNAVAILABLE,
            ) from exc
        except OSError as exc:
            raise pins.RerankError(
                "reranker server at " + path + " went away: " + str(exc),
                pins.EX_UNAVAILABLE,
            ) from exc
    if kind == KIND_ERROR:
        raise pins.RerankError(payload.decode("utf-8", "replace"), code or pins.EX_DATAERR)
    sys.stdout.buffer.write(payload)
    sys.stdout.buffer.flush()
    return pins.EX_OK


def _accept_loop(server: socket.socket, scorer, idle_timeout: float) -> None:
    """Answer one connection at a time until the idle budget expires."""
    server.settimeout(idle_timeout if idle_timeout > 0 else None)
    while True:
        try:
            conn, _ = server.accept()
        except socket.timeout:
            _diag("idle timeout reached, stopping")
            return
        except InterruptedError:
            continue
        with conn:
            conn.settimeout(DEFAULT_REQUEST_TIMEOUT)
            _serve_one(conn, scorer)


def _serve_one(conn: socket.socket, scorer) -> None:
    """Score one framed request, answering with a payload or an error frame."""
    try:
        kind, _, raw = _read_frame(conn)
        if kind != KIND_PAYLOAD:
            raise pins.RerankError("client sent frame kind " + str(kind) + ", expected 1")
        request = wire.decode(raw)
        wire.check_identity(request, scorer.model_label, scorer.adapter_label)
        started = time.perf_counter()
        scores = scorer.score(request.query, [c.text for c in request.candidates])
        body = wire.encode(request, scores)
        elapsed_ms = int((time.perf_counter() - started) * 1000)
        _write_frame(conn, KIND_PAYLOAD, 0, body.encode("utf-8"))
        _diag(
            "served request_id=" + request.request_id
            + " candidates=" + str(len(request.candidates))
            + " score_ms=" + str(elapsed_ms)
        )
    except pins.RerankError as exc:
        _answer_error(conn, str(exc), exc.code)
    except OSError as exc:
        # The connection itself failed, so there is nowhere to answer. Report
        # it and keep serving: one broken client must not stop the server.
        _diag("connection failed: " + str(exc))


def _answer_error(conn: socket.socket, message: str, code: int) -> None:
    """Tell the client which exit code to use, if the connection still allows it."""
    _diag("refused: " + message)
    try:
        _write_frame(conn, KIND_ERROR, code, message.encode("utf-8"))
    except OSError as exc:
        _diag("could not deliver the refusal: " + str(exc))


def _bind(path: str) -> socket.socket:
    """Take ownership of `path`, refusing to displace a live server on it."""
    _clear_stale(path)
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    # Both, and in this order. The umask is what makes the socket private at
    # the instant it is created, leaving no window in which it is world
    # reachable; the chmod then states the intended mode explicitly and does
    # not depend on a process-global that something else could have changed.
    # `_bind` runs once, before any worker exists, so the global is not shared
    # with anything here — but it is restored immediately regardless.
    previous = os.umask(0o077)
    try:
        server.bind(path)
        os.chmod(path, 0o600)
    except OSError as exc:
        server.close()
        _unlink_quietly(path)
        raise pins.RerankError(
            "cannot bind " + path + ": " + str(exc), pins.EX_CANTCREAT
        ) from exc
    finally:
        os.umask(previous)
    server.listen(_BACKLOG)
    return server


def _clear_stale(path: str) -> None:
    """Remove a leftover socket file, but never one a live server is using.

    The probe and the unlink are two steps, so a server that binds this exact
    path in between would have its socket removed. Unix domain sockets offer no
    atomic create-or-fail, so the race is narrow rather than absent: it is
    stated here instead of being papered over, and it needs two servers racing
    for one path, which is a configuration mistake in its own right.
    """
    if not os.path.exists(path):
        return
    probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    probe.settimeout(DEFAULT_CONNECT_TIMEOUT)
    try:
        probe.connect(path)
    except OSError:
        _unlink_quietly(path)
        return
    finally:
        probe.close()
    raise pins.RerankError(
        "another reranker server is already listening on " + path,
        pins.EX_CANTCREAT,
    )


def _connect(path: str, connect_timeout: float) -> socket.socket:
    """Open the client end, mapping every failure onto `EX_UNAVAILABLE`."""
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(connect_timeout)
    try:
        sock.connect(path)
    except OSError as exc:
        sock.close()
        raise pins.RerankError(
            "no reranker server at " + path + ": " + str(exc),
            pins.EX_UNAVAILABLE,
        ) from exc
    return sock


def _write_frame(sock: socket.socket, kind: int, code: int, payload: bytes) -> None:
    """Send one complete frame."""
    sock.sendall(_HEADER.pack(kind, code, len(payload)) + payload)


def _read_frame(sock: socket.socket) -> tuple:
    """Read one complete frame, refusing an oversized or truncated one."""
    header = _read_exactly(sock, _HEADER.size)
    kind, code, length = _HEADER.unpack(header)
    if length > MAX_FRAME_BYTES:
        raise pins.RerankError(
            "frame of " + str(length) + " bytes exceeds the limit of "
            + str(MAX_FRAME_BYTES),
            pins.EX_DATAERR,
        )
    return kind, code, _read_exactly(sock, length)


def _read_exactly(sock: socket.socket, count: int) -> bytes:
    """Read exactly `count` bytes, or fail: a short frame is never parsed."""
    chunks = []
    remaining = count
    while remaining > 0:
        chunk = sock.recv(min(remaining, 1 << 16))
        if not chunk:
            raise pins.RerankError(
                "connection closed after " + str(count - remaining) + " of "
                + str(count) + " bytes",
                pins.EX_UNAVAILABLE,
            )
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def _write_ready(ready_file: str, path: str) -> None:
    """Publish readiness atomically, so a watcher never sees a half-written file."""
    temporary = ready_file + ".tmp"
    with open(temporary, "w", encoding="utf-8") as handle:
        handle.write(path + "\n")
    os.replace(temporary, ready_file)


def _unlink_quietly(path: str) -> None:
    """Remove `path` if it is there, tolerating the race where it is not."""
    try:
        os.unlink(path)
    except FileNotFoundError:
        return
    except OSError as exc:
        _diag("could not remove " + path + ": " + str(exc))


def _diag(message: str) -> None:
    """Write one diagnostic line. stderr is never parsed by the Rust caller."""
    sys.stderr.write(pins.DIAG_PREFIX + message + "\n")
    sys.stderr.flush()
