#!/usr/bin/env python3
"""The comemory reranker backend: one child process, one scoring request.

Speaks the versioned JSON protocol defined by
`docs/designs/2026-09-18-reranker-command-protocol.md` section "Wire protocol".
One UTF-8 JSON object arrives on stdin followed by end of file; one UTF-8 JSON
object goes out on stdout followed by end of file, with exit status 0. stderr
is diagnostic only and is never parsed by the caller.

    comemory_rerank.py score       scoring one request and exiting
    comemory_rerank.py serve       holding the model behind a local socket
    comemory_rerank.py client      forwarding one request to that socket
    comemory_rerank.py fingerprint printing the complete scoring identity
    comemory_rerank.py benchmark   measuring load cost and query latency

Nothing here is wired into comemory's search path. `comemory` is a standalone
Rust binary with no Python dependency; this backend is an optional repository
asset a caller opts into. See `integrations/reranker/README.md`.
"""

from __future__ import annotations

import argparse
import json
import os
import resource
import sys
import time

# The sibling modules are resolved from this file's own directory rather than
# from the working directory, so the backend runs correctly from anywhere and
# through a symlink. `realpath` matters: `sys.path[0]` would otherwise be the
# symlink's directory, which need not hold the modules.
sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))

# The four imports below are deliberately after that line, because they are the
# siblings it makes importable.
import comemory_rerank_lexical as lexical
import comemory_rerank_pins as pins
import comemory_rerank_protocol as wire
import comemory_rerank_warm as warm

_BENCHMARK_QUERY = "bounded subprocess deadline and candidate reranking"
_BENCHMARK_TEXT = (
    "Candidate {index}: the bounded process runner holds one child under a "
    "single end-to-end deadline covering startup, concurrent stdin and stdout "
    "handling, and exit."
)


def main(argv: list) -> int:
    """Run one subcommand and return its exit code."""
    args = _parser().parse_args(argv)
    try:
        return _dispatch(args)
    except pins.RerankError as exc:
        _diag(str(exc))
        return exc.code
    except BrokenPipeError:
        # The caller stopped reading. Nothing is wrong with the answer, but it
        # was not delivered, so this is not a success.
        _diag("stdout closed before the response was delivered")
        return pins.EX_UNAVAILABLE


def _dispatch(args: argparse.Namespace) -> int:
    """Route to the selected subcommand."""
    if args.command == "client":
        return warm.client(args.socket, args.connect_timeout, args.request_timeout)
    scorer = _build_scorer(args)
    if args.command == "serve":
        return warm.serve(scorer, args.socket, args.ready_file, args.idle_timeout)
    if args.command == "score":
        return _score(scorer, args.verbose)
    if args.command == "fingerprint":
        scorer.load()
        sys.stdout.write(json.dumps(scorer.fingerprint(), indent=2, sort_keys=True) + "\n")
        return pins.EX_OK
    return _benchmark(scorer, args.candidates, args.repeat)


def _score(scorer, verbose: bool) -> int:
    """Read one request from stdin, score it, and write one response."""
    request = wire.decode(_read_request())
    wire.check_identity(request, scorer.model_label, scorer.adapter_label)
    scorer.load()
    if verbose:
        _diag("fingerprint " + json.dumps(scorer.fingerprint(), separators=(",", ":")))
    scores = scorer.score(request.query, [c.text for c in request.candidates])
    sys.stdout.write(wire.encode(request, scores))
    sys.stdout.flush()
    return pins.EX_OK


def _read_request() -> bytes:
    """Read the request body, refusing one larger than the pinned ceiling.

    One byte past the ceiling is read deliberately: it is how an over-cap body
    is distinguished from one that exactly fills the budget, without buffering
    the whole oversized payload to find out.
    """
    raw = sys.stdin.buffer.read(pins.MAX_REQUEST_BYTES + 1)
    if len(raw) > pins.MAX_REQUEST_BYTES:
        raise pins.RerankError(
            "request exceeds the limit of " + str(pins.MAX_REQUEST_BYTES) + " bytes"
        )
    return raw


def _benchmark(scorer, candidates: int, repeat: int) -> int:
    """Measure load cost, per-query latency and peak resident memory.

    The request is generated here from a fixed query and a fixed template
    numbered by index, so it reads nothing from stdin and two runs on one
    machine measure the same work.
    """
    if candidates < 1 or repeat < 1:
        raise pins.RerankError("--candidates and --repeat must both be at least 1", pins.EX_USAGE)
    load_ms = scorer.load()
    texts = [_BENCHMARK_TEXT.format(index=index) for index in range(candidates)]
    samples = []
    for _ in range(repeat):
        mark = time.perf_counter()
        scorer.score(_BENCHMARK_QUERY, texts)
        samples.append((time.perf_counter() - mark) * 1000.0)
    samples.sort()
    report = {
        "scoring": scorer.fingerprint()["scoring"],
        "model_label": scorer.model_label,
        "adapter_label": scorer.adapter_label,
        "candidates": candidates,
        "repeat": repeat,
        "load_ms": load_ms,
        "p50_ms": round(_percentile(samples, 0.50), 3),
        "p95_ms": round(_percentile(samples, 0.95), 3),
        "max_ms": round(samples[-1], 3),
        "peak_rss_bytes": _peak_rss_bytes(),
    }
    sys.stdout.write(json.dumps(report, indent=2, sort_keys=True) + "\n")
    return pins.EX_OK


def _percentile(sorted_samples: list, fraction: float) -> float:
    """Nearest-rank percentile over an already sorted, non-empty sample list."""
    index = int(round(fraction * (len(sorted_samples) - 1)))
    return sorted_samples[index]


def _peak_rss_bytes() -> int:
    """Peak resident set size of this process, normalized to bytes.

    `ru_maxrss` is kilobytes on Linux and bytes on macOS, which is exactly the
    kind of silent factor-of-1024 that makes a benchmark table wrong without
    ever looking wrong.
    """
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return peak if sys.platform == "darwin" else peak * 1024


def _build_scorer(args: argparse.Namespace):
    """Construct the selected scorer, importing torch only when it is needed."""
    if args.scoring == pins.SCORING_LEXICAL:
        if args.adapter:
            raise pins.RerankError(
                "--adapter is not valid with --scoring " + pins.SCORING_LEXICAL
                + "; that mode has no model to adapt",
                pins.EX_USAGE,
            )
        return lexical.LexicalScorer(model_label=args.model_label)
    # Imported here, not at module scope: a `lexical-overlap` process, a
    # `client` process and `--help` must never pay for a torch import, and on a
    # machine without torch they must still work.
    import comemory_rerank_model as model

    return model.CrossEncoderScorer(
        model_id=args.model_id,
        model_revision=args.model_revision,
        tokenizer_id=args.tokenizer_id or args.model_id,
        tokenizer_revision=args.tokenizer_revision or args.model_revision,
        model_label=args.model_label,
        adapter=args.adapter,
        adapter_label=args.adapter_label,
        dtype=args.dtype,
        device=args.device,
        max_length=args.max_length,
        truncation=args.truncation,
        padding=args.padding,
        batch_size=args.batch_size,
        score_column=args.score_column,
        head_modules=tuple(args.head_module or pins.HEAD_MODULES),
        allow_download=args.allow_download,
        allow_base_mismatch=args.allow_base_mismatch,
        digest_base_weights=args.digest_base_weights,
    )


def _parser() -> argparse.ArgumentParser:
    """Build the command line. Every default is a pinned constant."""
    parser = argparse.ArgumentParser(
        prog="comemory_rerank.py",
        description="Optional external relevance scorer for comemory (protocol version "
        + str(pins.PROTOCOL_VERSION)
        + ").",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    score = subparsers.add_parser("score", help="score one request from stdin and exit")
    _add_scorer_arguments(score)
    score.add_argument(
        "--verbose",
        action="store_true",
        help="write the fingerprint to stderr before scoring",
    )

    serve = subparsers.add_parser("serve", help="hold the model behind a local socket")
    _add_scorer_arguments(serve)
    serve.add_argument("--socket", required=True, help="Unix socket path to listen on")
    serve.add_argument("--ready-file", help="file to create once the model is loaded")
    serve.add_argument(
        "--idle-timeout",
        type=float,
        default=0.0,
        help="stop after this many idle seconds; 0 never stops (default: 0)",
    )

    client = subparsers.add_parser("client", help="forward one request to a warm server")
    client.add_argument("--socket", required=True, help="Unix socket path to connect to")
    client.add_argument(
        "--connect-timeout",
        type=float,
        default=warm.DEFAULT_CONNECT_TIMEOUT,
        help="seconds to wait for the socket (default: %(default)s)",
    )
    client.add_argument(
        "--request-timeout",
        type=float,
        default=warm.DEFAULT_REQUEST_TIMEOUT,
        help="seconds to wait for the response (default: %(default)s)",
    )

    fingerprint = subparsers.add_parser(
        "fingerprint", help="print the complete scoring identity and exit"
    )
    _add_scorer_arguments(fingerprint)

    benchmark = subparsers.add_parser(
        "benchmark", help="measure load cost, query latency and peak memory"
    )
    _add_scorer_arguments(benchmark)
    benchmark.add_argument("--candidates", type=int, default=32, help="default: %(default)s")
    benchmark.add_argument("--repeat", type=int, default=5, help="default: %(default)s")
    return parser


def _add_scorer_arguments(parser: argparse.ArgumentParser) -> None:
    """Attach the scorer selection and every pinned inference knob."""
    parser.add_argument(
        "--scoring",
        choices=pins.SCORING_MODES,
        default=pins.SCORING_CROSS_ENCODER,
        help="default: %(default)s",
    )
    parser.add_argument("--model-id", default=pins.MODEL_ID, help="default: %(default)s")
    parser.add_argument(
        "--model-revision", default=pins.MODEL_REVISION, help="default: the pinned revision"
    )
    parser.add_argument("--tokenizer-id", help="default: the model id")
    parser.add_argument("--tokenizer-revision", help="default: the model revision")
    parser.add_argument(
        "--model-label",
        help="the model identity this process answers to; "
        "default: <model-id>@<model-revision>, or "
        + pins.LEXICAL_MODEL_LABEL
        + " for "
        + pins.SCORING_LEXICAL,
    )
    parser.add_argument("--adapter", help="directory holding a PEFT adapter checkpoint")
    parser.add_argument(
        "--adapter-label",
        help="the adapter identity this process answers to; default: the directory name",
    )
    parser.add_argument("--dtype", default=pins.DTYPE, help="default: %(default)s")
    parser.add_argument(
        "--device", choices=pins.DEVICES, default=pins.DEVICE, help="default: %(default)s"
    )
    parser.add_argument(
        "--max-length", type=int, default=pins.MAX_LENGTH, help="default: %(default)s"
    )
    parser.add_argument(
        "--truncation",
        choices=pins.TRUNCATIONS,
        default=pins.TRUNCATION,
        help="default: %(default)s",
    )
    parser.add_argument(
        "--padding", choices=pins.PADDINGS, default=pins.PADDING, help="default: %(default)s"
    )
    parser.add_argument(
        "--batch-size", type=int, default=pins.BATCH_SIZE, help="default: %(default)s"
    )
    parser.add_argument(
        "--score-column", type=int, default=pins.SCORE_COLUMN, help="default: %(default)s"
    )
    parser.add_argument(
        "--head-module",
        action="append",
        help="a module whose parameters form the relevance head; repeatable, default: "
        + ", ".join(pins.HEAD_MODULES),
    )
    parser.add_argument(
        "--allow-download",
        action="store_true",
        help="permit fetching the model; off by default so a request never stalls on a download",
    )
    parser.add_argument(
        "--allow-base-mismatch",
        action="store_true",
        help="downgrade an adapter/base identity disagreement to a warning",
    )
    parser.add_argument(
        "--digest-base-weights",
        action="store_true",
        help="hash the base weight file into the fingerprint; the pinned revision already "
        "identifies it, and hashing costs a pass over the whole file",
    )


def _diag(message: str) -> None:
    """Write one diagnostic line to stderr, which the caller never parses."""
    sys.stderr.write(pins.DIAG_PREFIX + message + "\n")
    sys.stderr.flush()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
