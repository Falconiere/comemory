#!/usr/bin/env python3
"""The comemory LoRA training recipe: plan it, run it, or verify a saved one.

    comemory_train.py plan   --dataset DIR    resolve the recipe and print it
    comemory_train.py train  --dataset DIR --out DIR   fit and package an adapter
    comemory_train.py verify --adapter DIR    re-establish the reload guarantee

`plan` is the standard library alone. It reads a `comemory export-dataset`
directory, applies every gate the trainer applies, and prints the resolved
recipe together with the dataset's provenance — without importing torch, without
touching a checkpoint, and without ever opening `holdout.jsonl`. That is what
makes it runnable in comemory's normal test path, and it is the dry run an
operator should always do first.

`train` and `verify` need the pinned libraries and the pinned weights. They
import them lazily, so a machine with neither still gets a working `plan` and a
useful `--help` rather than an ImportError.

Nothing here runs automatically. There is no hook, no watcher and no save-time
trigger anywhere in this directory: training is a command an operator types,
against an export an operator produced, and #213 makes capture and reranking
mutually exclusive precisely so the collected pool is never the model's own
output. Contract: `docs/designs/2026-09-18-lora-adapter-training.md`.
"""

from __future__ import annotations

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))

# Deliberately after that line, because it is what makes the siblings importable.
import comemory_train_data as data  # noqa: E402
import comemory_train_manifest as manifest  # noqa: E402
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
    if args.command == "plan":
        return _plan(args)
    if args.command == "train":
        return _train(args)
    return _verify(args)


def _plan(args: argparse.Namespace) -> int:
    """Resolve the recipe against a dataset and print it. Reads no weights."""
    resolved = _resolve(args.dataset, _overrides(args))
    if args.json:
        sys.stdout.write(
            json.dumps(resolved, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
        )
    else:
        _write_plan(resolved)
    return pins.EX_OK


def _resolve(directory: str, overrides: dict) -> dict:
    """The recipe plus the dataset provenance, as one printable object."""
    dataset = data.load(directory)
    return {
        "recipe": manifest.recipe(overrides),
        "dataset": manifest.dataset_block(dataset),
        "limitations": list(manifest.LIMITATIONS),
    }


def _write_plan(resolved: dict) -> None:
    """The human summary: what would be fitted, to what, under which pins."""
    out = sys.stdout
    dataset = resolved["dataset"]
    recipe = resolved["recipe"]
    out.write(
        "dataset " + str(dataset["dataset_id"]) + "  snapshot "
        + _short(str(dataset["snapshot_digest"])) + "\n"
    )
    out.write(
        "rows " + json.dumps(dataset["rows_by_split"], sort_keys=True)
        + "  by domain " + json.dumps(dataset["rows_by_domain"], sort_keys=True)
        + "  by grade " + json.dumps(dataset["rows_by_relevance"], sort_keys=True) + "\n"
    )
    out.write(
        "holdout " + str(dataset["holdout"]["path"]) + " sha256 "
        + _short(str(dataset["holdout"]["sha256"])) + " read="
        + str(dataset["holdout"]["read"]).lower()
        + "  (recorded, never opened)\n"
    )
    out.write(
        "base " + recipe["base"]["model_id"] + "@"
        + _short(recipe["base"]["model_revision"]) + "  head "
        + ", ".join(recipe["base"]["head_modules"]) + "\n"
    )
    out.write(
        "lora r=" + str(recipe["lora"]["r"]) + " alpha=" + str(recipe["lora"]["alpha"])
        + " dropout=" + str(recipe["lora"]["dropout"]) + " targets "
        + ", ".join(recipe["lora"]["target_modules"]) + " modules_to_save "
        + ", ".join(recipe["lora"]["modules_to_save"]) + "\n"
    )
    out.write(
        "schedule " + str(recipe["schedule"]["epochs"]) + " epochs lr "
        + str(recipe["schedule"]["learning_rate"]) + " " + recipe["schedule"]["lr_schedule"]
        + " warmup " + str(recipe["schedule"]["warmup_ratio"]) + " batch "
        + str(recipe["schedule"]["train_batch_size"]) + "  precision "
        + recipe["precision"]["dtype"] + "\n"
    )
    out.write(
        "selection " + recipe["selection"]["metric"] + " on "
        + ", ".join(recipe["selection"]["splits_consulted"]) + " only  seed "
        + str(recipe["seed"]) + "\n"
    )


def _train(args: argparse.Namespace) -> int:
    """Fit an adapter and package it. Needs the pinned libraries and weights."""
    overrides = _overrides(args)
    dataset = data.load(args.dataset)
    _prepare_output(args.out, args.force)
    import comemory_train_loop as loop

    return loop.run(dataset=dataset, args=args, overrides=overrides)


def _verify(args: argparse.Namespace) -> int:
    """Reload a saved adapter and re-check the parity guarantee."""
    import comemory_train_model as model

    return model.verify(args.adapter, args.candidates, args.allow_download)


def _prepare_output(directory: str, force: bool) -> None:
    """Refuse to mix a new adapter into a directory that already holds one."""
    if os.path.isdir(directory) and os.listdir(directory) and not force:
        raise pins.RecipeError(
            directory + " is not empty; an adapter silently mixed with an older "
            "one cannot be qualified. Pass --force to overwrite it.",
            pins.EX_DATAERR,
        )
    os.makedirs(directory, exist_ok=True)


def _overrides(args: argparse.Namespace) -> dict:
    """Every explicit deviation from the pins, for the record."""
    applied = {}
    for key in ("seed", "epochs", "device"):
        value = getattr(args, key, None)
        if value is not None:
            applied[key] = value
    if getattr(args, "allow_nondeterministic_algorithms", False):
        applied["deterministic_algorithms"] = False
    return applied


def _short(digest: str) -> str:
    """The first twelve characters of a digest, enough to compare by eye."""
    return digest[:12] if digest else "-"


def _parser() -> argparse.ArgumentParser:
    """The command line."""
    parser = argparse.ArgumentParser(
        prog="comemory_train.py", description=__doc__.splitlines()[0]
    )
    sub = parser.add_subparsers(dest="command", required=True)

    plan = sub.add_parser("plan", help="resolve the recipe against a dataset")
    plan.add_argument("--dataset", required=True, help="a `comemory export-dataset` directory")
    plan.add_argument("--json", action="store_true", help="print the resolved object")
    _recipe_flags(plan)

    train = sub.add_parser("train", help="fit and package an adapter")
    train.add_argument("--dataset", required=True)
    train.add_argument("--out", required=True, help="the adapter directory to write")
    train.add_argument("--adapter-label", default=None, help="the label the backend answers to")
    train.add_argument("--force", action="store_true", help="overwrite a non-empty --out")
    train.add_argument("--allow-download", action="store_true", help="fetch the pinned weights")
    _recipe_flags(train)

    verify = sub.add_parser("verify", help="re-check a saved adapter's reload guarantee")
    verify.add_argument("--adapter", required=True)
    verify.add_argument("--candidates", type=int, default=pins.PARITY_PAIRS)
    verify.add_argument("--allow-download", action="store_true")
    return parser


def _recipe_flags(parser: argparse.ArgumentParser) -> None:
    """The overrides both `plan` and `train` accept, so a dry run matches a run."""
    parser.add_argument("--seed", type=int, default=None, help="override the pinned seed")
    parser.add_argument("--epochs", type=int, default=None, help="override the pinned epochs")
    parser.add_argument("--device", choices=list(pins.DEVICES), default=None)
    parser.add_argument(
        "--allow-nondeterministic-algorithms",
        action="store_true",
        help="permit torch kernels with no deterministic implementation",
    )


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
