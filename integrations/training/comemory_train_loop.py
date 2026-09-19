"""The seeded training loop, the validation pass, and checkpoint selection.

Three properties of this file matter more than the arithmetic in it.

**Selection reads validation and nothing else.** The held-out split was never
loaded — `comemory_train_data` cannot open it — so there is no code path by
which the final test set could influence which epoch is kept. The manifest
records `holdout_used: false` beside the per-epoch numbers, so the claim is
auditable rather than asserted.

**The run is seeded end to end.** One seed initializes Python's global RNG,
NumPy's and torch_lib's, and each epoch's shuffle draws from its own
`random.Random(seed + epoch)` — a separate stream per epoch, so an epoch's order
depends on the seed and the epoch number and on nothing that ran before it.
Deterministic kernels are requested unless an operator explicitly opts out, and
the batching is plain index slicing rather than a DataLoader with worker
processes, because worker scheduling is a source of nondeterminism nobody needs
here.

**The base is proven frozen.** The digest over every non-adapter parameter is
taken before the first optimizer step and again after the last, and a difference
refuses the run rather than warning about it.
"""

from __future__ import annotations

import math
import os
import random
import resource
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))

import comemory_train_manifest as manifest  # noqa: E402
import comemory_train_model as model  # noqa: E402
import comemory_train_pins as pins  # noqa: E402

_BACKEND = os.path.join(os.path.dirname(os.path.dirname(os.path.realpath(__file__))), "reranker")
if _BACKEND not in sys.path:
    sys.path.insert(0, _BACKEND)

import comemory_rerank_compat as compat  # noqa: E402


def run(dataset, args, overrides: dict) -> int:
    """Fit an adapter over `dataset` and package it into `args.out`."""
    recipe = manifest.recipe(overrides)
    started = time.perf_counter()
    config = model.backend_config(recipe["precision"]["device"], args.allow_download)
    torch_lib = model.torch_module()
    tokenizer, base, _head = model.load_base(config)
    _seed_everything(torch_lib, recipe)
    adapted = model.attach_adapter(base)
    audit = model.audit(adapted)
    frozen_before = model.frozen_digest(torch_lib, adapted)
    device = compat.resolve_device(torch_lib, recipe["precision"]["device"])
    adapted.to(device)

    selection, best_state = _fit(torch_lib, tokenizer, adapted, dataset, recipe, device)
    frozen_after = model.frozen_digest(torch_lib, adapted)
    if frozen_after["sha256"] != frozen_before["sha256"]:
        raise pins.RecipeError(
            "the frozen base moved during training: " + frozen_before["sha256"]
            + " became " + frozen_after["sha256"] + ". An adapter whose base changed is "
            "not an adapter, and nothing was saved.",
            pins.EX_SOFTWARE,
        )
    _restore(adapted, best_state)
    adapted.eval()
    probe = model.parity_pairs(pins.PARITY_PAIRS)
    before = model.score_pairs(adapted, tokenizer, torch_lib, probe)
    files = model.save(adapted, args.out)
    reload_check = model.reload_and_compare(
        args.out, before, recipe["precision"]["device"], args.allow_download
    )
    _package(args, dataset, recipe, audit, frozen_before, selection, reload_check, files, started)
    return pins.EX_OK


def _package(args, dataset, recipe, audit, frozen, selection, reload_check, files, started):
    """Write the immutable manifest that closes the adapter package."""
    label = args.adapter_label or pins.adapter_label_default(args.out)
    document = manifest.build(
        adapter_label=label,
        recipe=recipe,
        dataset=manifest.dataset_block(dataset),
        base_checkpoint={
            "model_id": pins.MODEL_ID,
            "model_revision": pins.MODEL_REVISION,
            "frozen_parameters_sha256": frozen["sha256"],
            "frozen_parameters": frozen["parameters"],
        },
        trainable=audit,
        selection=selection,
        reload_check=reload_check,
        hardware=manifest.hardware(
            recipe["precision"]["device"],
            resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
            time.perf_counter() - started,
        ),
        libraries=compat.library_versions(True),
        files=files,
    )
    manifest.write(os.path.join(args.out, pins.ADAPTER_MANIFEST_FILE), document)
    compat.diag("wrote " + document["adapter_id"] + " to " + args.out)


def _seed_everything(torch_lib, recipe: dict) -> None:
    """One seed for every stream, and deterministic kernels unless opted out."""
    seed = int(recipe["seed"])
    random.seed(seed)
    torch_lib.manual_seed(seed)
    try:
        import numpy

        numpy.random.seed(seed)
    except ImportError:
        pass
    if recipe["deterministic_algorithms"]:
        torch_lib.use_deterministic_algorithms(True)


def _fit(torch_lib, tokenizer, adapted, dataset, recipe: dict, device) -> tuple:
    """Run every epoch, score validation after each, and keep the best.

    Returns the selection record and the selected epoch's trainable tensors,
    as two values rather than one, so the checkpoint never has to travel inside
    the object that is about to be serialized into the adapter manifest.
    """
    schedule = recipe["schedule"]
    epochs = int(schedule["epochs"])
    batch = int(schedule["train_batch_size"])
    steps = max(1, math.ceil(len(dataset.train) / batch)) * epochs
    optimizer, scheduler = _optimizer(torch_lib, adapted, schedule, steps)
    loss_fn = torch_lib.nn.BCEWithLogitsLoss()
    groups = _validation_groups(dataset)
    per_epoch, best, state = [], None, None
    for epoch in range(1, epochs + 1):
        train_loss = _one_epoch(
            torch_lib, tokenizer, adapted, dataset, recipe, device, optimizer, scheduler, loss_fn, epoch
        )
        scored = _validate(torch_lib, tokenizer, adapted, groups, loss_fn, device)
        row = {"epoch": epoch, "train_loss": train_loss}
        row.update(scored)
        per_epoch.append(row)
        rank = _selection_key(row)
        if best is None or rank > best:
            best, state = rank, _trainable_state(adapted)
    metric = pins.SELECTION_METRIC if _has_ranking(per_epoch) else pins.SELECTION_FALLBACK_METRIC
    selection = {
        "metric": metric,
        "k": pins.SELECTION_K,
        "holdout_used": False,
        "splits_consulted": [pins.VALIDATION_SPLIT],
        "selected_epoch": max(per_epoch, key=_selection_key)["epoch"],
        "validation_observations": len(groups),
        "per_epoch": per_epoch,
    }
    return selection, state


def _one_epoch(torch_lib, tokenizer, adapted, dataset, recipe, device, optimizer, scheduler, loss_fn, epoch):
    """One pass over the shuffled training rows; returns the mean loss."""
    adapted.train()
    order = list(range(len(dataset.train)))
    random.Random(int(recipe["seed"]) + epoch).shuffle(order)
    batch = int(recipe["schedule"]["train_batch_size"])
    total, seen = 0.0, 0
    for start in range(0, len(order), batch):
        window = [dataset.train[index] for index in order[start : start + batch]]
        encoded = model.encode(
            tokenizer, [e.query for e in window], [e.text for e in window], device
        )
        targets = torch_lib.tensor([e.target for e in window], dtype=torch_lib.float32, device=device)
        logits = adapted(**encoded).logits[:, pins.SCORE_COLUMN]
        loss = loss_fn(logits.to(torch_lib.float32), targets)
        loss.backward()
        torch_lib.nn.utils.clip_grad_norm_(
            [p for p in adapted.parameters() if p.requires_grad],
            float(recipe["schedule"]["max_grad_norm"]),
        )
        optimizer.step()
        scheduler.step()
        optimizer.zero_grad(set_to_none=True)
        total += float(loss.detach()) * len(window)
        seen += len(window)
    return total / seen if seen else 0.0


def _optimizer(torch_lib, adapted, schedule: dict, steps: int):
    """AdamW over the trainable parameters, with linear warmup and decay."""
    parameters = [p for p in adapted.parameters() if p.requires_grad]
    optimizer = torch_lib.optim.AdamW(
        parameters,
        lr=float(schedule["learning_rate"]),
        betas=tuple(schedule["adam_betas"]),
        eps=float(schedule["adam_eps"]),
        weight_decay=float(schedule["weight_decay"]),
    )
    warmup = max(1, int(steps * float(schedule["warmup_ratio"])))

    def factor(step: int) -> float:
        if step < warmup:
            return (step + 1) / warmup
        remaining = max(1, steps - warmup)
        return max(0.0, (steps - step) / remaining)

    return optimizer, torch_lib.optim.lr_scheduler.LambdaLR(optimizer, factor)


def _validation_groups(dataset) -> list:
    """Validation rows grouped by observation, which is one ranking task each."""
    grouped: dict = {}
    for example in dataset.validation:
        grouped.setdefault(example.observation_id, []).append(example)
    return [grouped[key] for key in sorted(grouped)]


def _validate(torch_lib, tokenizer, adapted, groups: list, loss_fn, device) -> dict:
    """Validation loss, and nDCG over every observation with a real ordering."""
    adapted.eval()
    losses, gains = [], []
    for group in groups:
        pairs = [(e.query, e.text) for e in group]
        scores = model.score_pairs(adapted, tokenizer, torch_lib, pairs)
        targets = torch_lib.tensor([e.target for e in group], dtype=torch_lib.float32)
        logits = torch_lib.tensor(scores, dtype=torch_lib.float32)
        losses.append(float(loss_fn(logits, targets)) * len(group))
        # Two or more candidates AND at least one of them graded above zero.
        # A group of reviewed hard negatives has an ideal DCG of zero, so every
        # ordering of it scores 0.0 — counting that as a rankable observation
        # would dilute the selection metric with tasks that cannot express a
        # preference.
        if len(group) >= 2 and any(e.relevance > 0 for e in group):
            gains.append(_ndcg([e.relevance for e in group], scores, pins.SELECTION_K))
    rows = sum(len(group) for group in groups)
    return {
        "validation_loss": sum(losses) / rows if rows else 0.0,
        "validation_ndcg_at_" + str(pins.SELECTION_K): (
            sum(gains) / len(gains) if gains else None
        ),
        "validation_rankable_observations": len(gains),
    }


def _ndcg(relevances: list, scores: list, k: int) -> float:
    """nDCG@k with the gain and discount #208's benchmark metrics use."""
    order = sorted(range(len(scores)), key=lambda i: (-scores[i], i))
    dcg = sum(
        (2 ** relevances[index] - 1) / math.log2(rank + 2)
        for rank, index in enumerate(order[:k])
    )
    ideal = sorted(relevances, reverse=True)[:k]
    idcg = sum((2 ** grade - 1) / math.log2(rank + 2) for rank, grade in enumerate(ideal))
    return dcg / idcg if idcg > 0 else 0.0


def _selection_key(row: dict):
    """Higher is better: nDCG first, then the negated loss as the tie-break."""
    ndcg = row.get("validation_ndcg_at_" + str(pins.SELECTION_K))
    return (ndcg if ndcg is not None else -1.0, -row.get("validation_loss", 0.0))


def _has_ranking(per_epoch: list) -> bool:
    """Whether any epoch produced a real nDCG, which decides what selected."""
    key = "validation_ndcg_at_" + str(pins.SELECTION_K)
    return any(row.get(key) is not None for row in per_epoch)


def _trainable_state(adapted) -> dict:
    """A detached copy of every trainable tensor: this epoch's checkpoint."""
    return {
        name: parameter.detach().clone()
        for name, parameter in adapted.named_parameters()
        if parameter.requires_grad
    }


def _restore(adapted, state: dict) -> None:
    """Put the selected epoch's trainable tensors back before saving."""
    if not state:
        return
    for name, parameter in adapted.named_parameters():
        saved = state.get(name)
        if saved is not None:
            parameter.data.copy_(saved)
