"""Every pinned value of the LoRA recipe and of the qualification, in one file.

An adapter is only reproducible if everything that can change it is pinned
somewhere a reader can find at once: the base model and its immutable revision,
the tokenizer, the seed, the LoRA rank, alpha, dropout and target modules, the
learning schedule, the numeric precision, the objective, the scoring-head
behavior and the library versions. They all live here, they all appear in the
adapter manifest the recipe writes, and nothing else in this directory
hard-codes one of them.

The base identity is IMPORTED from `integrations/reranker/comemory_rerank_pins`
rather than restated. A drift between the trainer's idea of the base model and
the scorer's is undetectable from either side: the adapter would load and
produce confidently wrong numbers. `tests/test_offline_recipe.py` asserts the
agreement, so this import cannot silently become a copy.

Contract: `docs/designs/2026-09-18-lora-adapter-training.md`.
"""

from __future__ import annotations

import os
import sys

# The reference backend is a sibling directory, resolved from this file's own
# real path so the recipe runs correctly from anywhere and through a symlink.
_HERE = os.path.dirname(os.path.realpath(__file__))
_BACKEND_DIR = os.path.join(os.path.dirname(_HERE), "reranker")
if _BACKEND_DIR not in sys.path:
    sys.path.insert(0, _BACKEND_DIR)

# Deliberately after that line, because it is what makes the import resolve.
import comemory_rerank_pins as backend  # noqa: E402

# --- Identity, imported from the reference backend (#212)

MODEL_ID = backend.MODEL_ID
MODEL_REVISION = backend.MODEL_REVISION
TOKENIZER_ID = backend.TOKENIZER_ID
TOKENIZER_REVISION = backend.TOKENIZER_REVISION
EXPECTED_ARCHITECTURES = backend.EXPECTED_ARCHITECTURES
EXPECTED_NUM_LABELS = backend.EXPECTED_NUM_LABELS
HEAD_MODULES = backend.HEAD_MODULES
MAX_LENGTH = backend.MAX_LENGTH
TRUNCATION = backend.TRUNCATION
PADDING = backend.PADDING
SCORE_COLUMN = backend.SCORE_COLUMN
DTYPE = backend.DTYPE
DEVICES = backend.DEVICES
ADAPTER_PEFT_TYPE = backend.ADAPTER_PEFT_TYPE
ADAPTER_TASK_TYPE = backend.ADAPTER_TASK_TYPE
ADAPTER_CONFIG_FILE = backend.ADAPTER_CONFIG_FILE
ADAPTER_WEIGHT_FILES = backend.ADAPTER_WEIGHT_FILES
PROTOCOL_VERSION = backend.PROTOCOL_VERSION
SCORE_DIRECTION = backend.SCORE_DIRECTION
LEXICAL_MODEL_LABEL = backend.LEXICAL_MODEL_LABEL
MAX_REQUEST_BYTES = backend.MAX_REQUEST_BYTES

# --- Exit codes, following sysexits.h as the rest of this repository does

EX_OK = backend.EX_OK
EX_USAGE = backend.EX_USAGE
EX_DATAERR = backend.EX_DATAERR
EX_UNAVAILABLE = backend.EX_UNAVAILABLE
EX_SOFTWARE = backend.EX_SOFTWARE

TOOL_NAME = "comemory-train"
DIAG_PREFIX = TOOL_NAME + ": "

# --- Recipe version
#
# Bumped whenever any pinned value below changes. Two adapters trained at the
# same recipe version under the same seed on the same dataset are the same
# experiment; two at different versions are not comparable.

RECIPE_VERSION = 1
ADAPTER_MANIFEST_VERSION = 1
ADAPTER_MANIFEST_FILE = "comemory_adapter.json"
DEFAULT_ADAPTER_LABEL = "lora-v1"

# --- The data contract this recipe consumes (#210)

DATASET_MANIFEST_FILE = "manifest.json"
SUPPORTED_MANIFEST_VERSION = 1
SUPPORTED_RECORD_VERSION = 1
SUPPORTED_OBSERVATION_VERSION = 1

TRAIN_SPLIT = "train"
VALIDATION_SPLIT = "validation"
HOLDOUT_SPLIT = "holdout"

# The whole allowlist of files the trainer may open, and the structural
# expression of "final test data stays out of training, mining and selection".
# `holdout.jsonl` and every `.implicit.jsonl` file are never read: the holdout
# because it is the final test set, the implicit files because this recipe
# begins with reviewed labels and an implicit signal is a different experiment.
READABLE_FILES = (DATASET_MANIFEST_FILE, "train.jsonl", "validation.jsonl")
HOLDOUT_FILE = "holdout.jsonl"
REVIEWED_PROVENANCE = "manual"

# --- Determinism

SEED = 20260918
DETERMINISTIC_ALGORITHMS = True

# --- LoRA
#
# `("query", "value")` is PEFT's own BERT target mapping: the attention query
# and value projections, which is where a rank-16 adapter has been shown to
# carry a reranking objective on an encoder of this size. A bias term is not
# adapted, because a trainable bias outside the adapter would change the base
# model that `frozen_parameters_sha256` promises did not move.

LORA_R = 16
LORA_ALPHA = 32
LORA_DROPOUT = 0.05
LORA_TARGET_MODULES = ("query", "value")
LORA_BIAS = "none"
MODULES_TO_SAVE = tuple(HEAD_MODULES)

# --- Objective
#
# One relevance logit per pair, trained with binary cross entropy over the
# graded label rescaled into [0, 1]. This is the single-score objective the
# ms-marco cross-encoder family was itself trained with; pairwise and listwise
# losses are later experiments the issue defers until measurements justify them.

OBJECTIVE = "bce-with-logits"
MAX_RELEVANCE = 3
POS_WEIGHT = None

# --- Schedule

EPOCHS = 3
LEARNING_RATE = 2e-4
WEIGHT_DECAY = 0.01
WARMUP_RATIO = 0.1
LR_SCHEDULE = "linear"
MAX_GRAD_NORM = 1.0
TRAIN_BATCH_SIZE = 16
EVAL_BATCH_SIZE = 32
OPTIMIZER = "adamw"
ADAM_BETAS = (0.9, 0.999)
ADAM_EPS = 1e-8

# --- Precision
#
# float32 throughout, and no autocast. The serving path is pinned to float32,
# so a head trained in bf16 and scored in fp32 would be a second difference
# between the two arms beside the adapter itself — which is exactly what the
# comparison exists to isolate.

TRAIN_DTYPE = DTYPE
AUTOCAST = False

# --- Checkpoint selection
#
# Validation only. The holdout is never loaded, never scored and never consulted
# by the selection rule, and `selection.holdout_used` records that fact in the
# manifest so a reader does not have to take the code's word for it.

SELECTION_METRIC = "ndcg_at_10"
SELECTION_K = 10
SELECTION_FALLBACK_METRIC = "validation_loss"

# --- The save/reload parity probe

PARITY_PAIRS = 32
PARITY_QUERY = "bounded subprocess deadline and candidate reranking"
PARITY_TEXT = (
    "Candidate {index}: the bounded process runner holds one child under a "
    "single end-to-end deadline covering startup, concurrent stdin and stdout "
    "handling, and exit."
)
# float32 carries roughly 1e-7 of relative precision, so a logit under 20 in
# magnitude has about 2e-6 of representational slack, and a reduction-order
# difference across a reload accumulates a few multiples of that. This pin sits
# two orders above that floor and far below any gap that could reorder a pool.
# The observed delta is recorded in the manifest, so it can be tightened from
# evidence rather than from argument.
RELOAD_MAX_ABS_DELTA = 1e-4

# --- Audit ceilings

TRAINABLE_FRACTION_MAX = 0.05

# --- Qualification budgets owned by this harness
#
# #208's `budgets` block already declares the quality and per-task latency
# thresholds, and a verdict is read off them by `comemory benchmark`. These two
# are the operational budgets the binary cannot see, because they belong to the
# scorer child rather than to retrieval.

# Production tolerates a fallback by design: a failed scorer leaves comemory's
# own ranking in place, which is the guaranteed no-regression path. A
# QUALIFICATION run is different. A run that could not score every held-out task
# has not measured the arm it is about to claim a number for, so any fallback at
# all withholds a go.
MAX_FALLBACK_RATE = 0.0

# `utilities::rerank_runner::DEFAULT_RERANK_TIMEOUT`. A scorer whose p95 exceeds
# the production deadline would fall back in production, so its measured quality
# would not be the quality served.
MAX_SCORING_P95_MS = 20_000
DEFAULT_TIMEOUT_MS = MAX_SCORING_P95_MS

# A deterministic scorer measures no relevance quality whatsoever — #212 says so
# in its own fingerprint, which reports `scoring_is_neural: false`. It exists to
# prove protocol conformance without a download, and it can never earn a go.
NON_NEURAL_MODEL_LABELS = (LEXICAL_MODEL_LABEL,)

OUTCOME_GO = "go"
OUTCOME_NO_GO = "no-go"
OUTCOME_INSUFFICIENT = "insufficient-evidence"

BASELINE_ARM = "deterministic"
SUPPORTED_ARTIFACT_VERSION = 1

# --- The pinned dependency set, mirrored from requirements.txt
#
# Identical to the reference backend's, because training and serving must run
# the same libraries or the save/reload parity check compares two builds rather
# than one. `run-training-tests.sh` reads this map and refuses to run the opt-in
# suite against anything else.

REQUIREMENTS = dict(backend.REQUIREMENTS)
REQUIREMENTS_FILE = "integrations/training/requirements.txt"
INSTALL_HINT = "pip install -r " + REQUIREMENTS_FILE
DOWNLOAD_HINT = backend.DOWNLOAD_HINT


# The shared compatibility module raises the reference backend's own refusal
# type, carrying the same `.code` contract. It is re-exported here so a caller
# can catch both without importing the backend, and so a refusal that comes from
# the shared code still leaves this process with a documented exit status rather
# than a traceback and a bare `1`.
BackendError = backend.RerankError


class RecipeError(Exception):
    """A refusal, carrying the exit code the process must terminate with.

    Raising this rather than letting a traceback escape is what keeps a refusal
    a documented number instead of a `1`, exactly as the reference backend's
    `RerankError` does for the scoring path.
    """

    def __init__(self, message: str, code: int = EX_DATAERR) -> None:
        super().__init__(message)
        self.code = code


def diag(message: str) -> None:
    """Write one diagnostic line to stderr, which no caller parses."""
    sys.stderr.write(DIAG_PREFIX + message + "\n")
    sys.stderr.flush()


def adapter_label_default(path: str) -> str:
    """The label an adapter directory answers to when none is given.

    The directory's own basename, which is the same default
    `comemory_rerank.py --adapter` applies, so a directory trained here and
    served there answers to one name without either side being told it twice.
    """
    return os.path.basename(os.path.normpath(path))
