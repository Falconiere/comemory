#!/usr/bin/env python3
"""The comemory adapter qualification harness: build a set, score arms, decide.

    comemory_qualify.py build-set --dataset DIR --out set.yaml --provenance p.json
    comemory_qualify.py score --report run.json --arm lora --out lora.json \\
                              --sidecar lora.scoring.json --model M [--adapter A] \\
                              -- python3 .../comemory_rerank.py score --adapter ...
    comemory_qualify.py decide --report qualification.json --provenance p.json \\
                               --scoring base=base.scoring.json \\
                               --scoring lora=lora.scoring.json \\
                               --adapter-manifest comemory_adapter.json \\
                               --out decision.json --markdown decision.md

The harness supplies ARMS; it does not compute retrieval metrics. `comemory
benchmark` already captures one candidate pool per task, scores every arm over
that one snapshot, measures pool recall apart from recall@k, MRR and nDCG@k per
domain, bootstraps the paired per-task delta and reads a verdict off the
interval rather than off the point estimate. Re-implementing any of that here
would create a second definition of this repository's own quality metrics, free
to drift from the one its tests cover.

Every subcommand is the standard library alone. `score` runs whatever scorer the
operator names as a child process, so the only thing that needs torch is the
scorer — and the deterministic `lexical-overlap` mode of the reference backend
needs nothing at all, which is what lets comemory's normal test run exercise
this whole path end to end without a download.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))

# Deliberately after that line, because it is what makes the siblings importable.
import comemory_qualify_decide as decide_module  # noqa: E402
import comemory_qualify_render as render  # noqa: E402
import comemory_qualify_score as score_module  # noqa: E402
import comemory_qualify_set as set_module  # noqa: E402
import comemory_train_pins as pins  # noqa: E402


def main(argv: list) -> int:
    """Run one subcommand and return its exit code."""
    args = _parser().parse_args(argv)
    try:
        return _dispatch(args)
    except (pins.RecipeError, pins.BackendError) as exc:
        pins.diag(str(exc))
        return exc.code
    except BrokenPipeError:
        pins.diag("stdout closed before the output was delivered")
        return pins.EX_UNAVAILABLE


def _dispatch(args: argparse.Namespace) -> int:
    """Route to the selected subcommand."""
    if args.command == "build-set":
        return _build_set(args)
    if args.command == "score":
        return _score(args)
    return _decide(args)


def _build_set(args: argparse.Namespace) -> int:
    """Generate a reviewed benchmark set from the withheld holdout split."""
    document, provenance = set_module.build(
        args.dataset,
        {
            "name": args.name,
            "k": args.k,
            "max_text_bytes": args.max_text_bytes,
            "decay": args.decay,
            "pin_content_version": not args.no_pin_content_version,
            "budgets": {
                "min_tasks": args.min_tasks,
                "min_ndcg_gain": args.min_ndcg_gain,
                "max_ndcg_regression": args.max_ndcg_regression,
                "max_p95_task_ms": args.max_p95_task_ms,
            },
        },
    )
    body = set_module.to_yaml(document)
    _write(args.out, body)
    provenance["set_sha256"] = _digest(body)
    _write_json(args.provenance, provenance)
    pins.diag(
        "wrote " + str(provenance["tasks"]) + " task(s) and "
        + str(provenance["judgments"]) + " judgment(s) to " + args.out
    )
    return pins.EX_OK


def _score(args: argparse.Namespace) -> int:
    """Score one arm over a captured artifact, through a real child process."""
    # `argparse.REMAINDER` hands back the `--` separator itself, which would be
    # spawned as the program. Dropping exactly one leading separator keeps a
    # scorer whose own first argument is `--` expressible.
    command = list(args.command_argv)
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        raise pins.RecipeError(
            "no scorer command; put it after `--`, exactly as `[rerank] command` does",
            pins.EX_USAGE,
        )
    if args.arm == pins.BASELINE_ARM:
        raise pins.RecipeError(
            "arm name " + repr(pins.BASELINE_ARM) + " is reserved for comemory's own "
            "deterministic ranking, which `comemory benchmark` supplies itself",
            pins.EX_USAGE,
        )
    scores, sidecar = score_module.score(
        args.report,
        {
            "arm": args.arm,
            "model": args.model,
            "adapter": args.adapter,
            "command": command,
            "timeout_ms": args.timeout_ms,
        },
    )
    _write_json(args.out, scores)
    _write_json(args.sidecar, sidecar)
    pins.diag(
        args.arm + ": " + str(sidecar["candidates_scored"]) + " candidate(s) over "
        + str(sidecar["invocations"]) + " invocation(s), " + str(sidecar["failures"])
        + " failure(s), p95 " + str(sidecar["latency_ms"]["p95"]) + "ms"
    )
    return pins.EX_OK


def _decide(args: argparse.Namespace) -> int:
    """Record the outcome of one qualification run."""
    decision = decide_module.decide(
        {
            "report": args.report,
            "provenance": args.provenance,
            "scoring": dict(_pairs(args.scoring)),
            "adapter_manifest": args.adapter_manifest,
            "lora_arm": args.lora_arm,
            "base_arm": args.base_arm,
        }
    )
    _write_json(args.out, decision)
    if args.markdown:
        _write(args.markdown, render.markdown(decision))
    sys.stdout.write(decision["outcome"] + "\n")
    for reason in decision["reasons"]:
        sys.stdout.write("  " + reason + "\n")
    return pins.EX_OK


def _positive(value: str) -> int:
    """A millisecond budget argparse accepts only when it can bound a child.

    `subprocess.run` treats a zero or negative timeout as already expired, so an
    unvalidated value would make every task record a spurious `timed_out`
    fallback instead of reporting the operator's mistake.
    """
    try:
        number = int(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(str(exc)) from exc
    if number < 1:
        raise argparse.ArgumentTypeError("must be at least 1 millisecond")
    return number


def _pairs(values: list) -> list:
    """Decode every `--scoring <arm>=<path>` pair, or refuse naming the value."""
    decoded = []
    for value in values or []:
        name, separator, path = value.partition("=")
        if not separator or not name.strip() or not path.strip():
            raise pins.RecipeError(
                "--scoring expects <arm>=<path>, found " + repr(value), pins.EX_USAGE
            )
        decoded.append((name.strip(), path.strip()))
    return decoded


def _write(path: str, body: str) -> None:
    """Write a text artifact, creating its parent directory."""
    parent = os.path.dirname(os.path.abspath(path))
    os.makedirs(parent, exist_ok=True)
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(body)


def _write_json(path: str, document: dict) -> None:
    """Write a JSON artifact as stable, key-sorted, pretty-printed text."""
    _write(path, json.dumps(document, indent=2, sort_keys=True, ensure_ascii=False) + "\n")


def _digest(body: str) -> str:
    """Hex SHA-256 of a text artifact, over its UTF-8 bytes."""
    return hashlib.sha256(body.encode("utf-8")).hexdigest()


def _parser() -> argparse.ArgumentParser:
    """The command line."""
    parser = argparse.ArgumentParser(
        prog="comemory_qualify.py", description=__doc__.splitlines()[0]
    )
    sub = parser.add_subparsers(dest="command", required=True)
    _build_set_flags(sub.add_parser("build-set", help="holdout split -> benchmark set"))
    _score_flags(sub.add_parser("score", help="captured artifact -> one arm's scores"))
    _decide_flags(sub.add_parser("decide", help="record go / no-go / insufficient-evidence"))
    return parser


def _build_set_flags(parser: argparse.ArgumentParser) -> None:
    """`build-set`, whose budget defaults are #208's documented example."""
    parser.add_argument("--dataset", required=True)
    parser.add_argument("--out", required=True, help="the benchmark set YAML to write")
    parser.add_argument("--provenance", required=True, help="the provenance sidecar to write")
    parser.add_argument("--name", default="comemory-holdout-v1")
    parser.add_argument("--k", type=int, default=5)
    parser.add_argument("--max-text-bytes", type=int, default=4096)
    parser.add_argument("--min-tasks", type=int, default=8)
    parser.add_argument("--min-ndcg-gain", type=float, default=0.02)
    parser.add_argument("--max-ndcg-regression", type=float, default=0.01)
    parser.add_argument("--max-p95-task-ms", type=int, default=1500)
    parser.add_argument(
        "--decay",
        type=float,
        default=set_module.FROZEN_DECAY,
        help="ACT-R decay for the pinned ranking; 0.0 freezes activation",
    )
    parser.add_argument(
        "--no-pin-content-version",
        action="store_true",
        help="omit the content-version pin, so a judgment matches any version",
    )


def _score_flags(parser: argparse.ArgumentParser) -> None:
    """`score`, whose scorer command comes after `--` and from nowhere else."""
    parser.add_argument("--report", required=True, help="a `comemory benchmark --report` artifact")
    parser.add_argument("--arm", required=True)
    parser.add_argument("--out", required=True, help="the scores file to write")
    parser.add_argument("--sidecar", required=True, help="the operational sidecar to write")
    parser.add_argument("--model", required=True, help="the model label the scorer answers to")
    parser.add_argument("--adapter", default=None, help="the adapter label, or omit for base")
    parser.add_argument(
        "--timeout-ms",
        type=_positive,
        default=pins.DEFAULT_TIMEOUT_MS,
        help="end-to-end budget for one scorer child, in milliseconds",
    )
    parser.add_argument("command_argv", nargs=argparse.REMAINDER, metavar="-- PROGRAM [ARGS…]")


def _decide_flags(parser: argparse.ArgumentParser) -> None:
    """`decide`, which reads artifacts and writes a record, never a model."""
    parser.add_argument("--report", required=True, help="the qualification artifact")
    parser.add_argument("--provenance", required=True, help="the set's provenance sidecar")
    parser.add_argument(
        "--scoring", action="append", default=[], metavar="ARM=PATH",
        help="one arm's operational sidecar; repeatable",
    )
    parser.add_argument("--adapter-manifest", default=None)
    parser.add_argument("--out", required=True, help="the decision JSON to write")
    parser.add_argument("--markdown", default=None, help="also render the decision as markdown")
    parser.add_argument("--lora-arm", default="lora")
    parser.add_argument("--base-arm", default="base")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
