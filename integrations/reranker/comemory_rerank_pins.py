"""Every pinned identity, inference constant and exit code, in one file.

A reranker score is only reproducible if everything that can change it is
pinned somewhere a reader can find in one place: the model repository and its
immutable revision, the tokenizer, the dtype, the maximum length, the
truncation and padding policy, the batch size, the logit column and the
library versions. They all live here, they all appear in the fingerprint the
backend emits, and nothing else in the backend hard-codes one of them.

The exit-code vocabulary lives here too, together with the one error type that
carries a code, because a refusal is only useful to the Rust caller as an exit
status: `utilities::rerank_runner` reports a non-zero exit as
`RerankFailure::NonZeroExit { code }` even when stdout parsed cleanly, so
exiting non-zero is how this backend refuses.

The wire contract these constants serve is
`docs/designs/2026-09-18-reranker-command-protocol.md` section "Wire protocol".
"""

from __future__ import annotations

# --- The wire contract (docs/designs/2026-09-18-reranker-command-protocol.md)

# The protocol version this backend speaks. Any additive field, relaxed rule or
# new score direction is a bump, never a silent extension.
PROTOCOL_VERSION = 1

BACKEND_NAME = "comemory-rerank"
BACKEND_VERSION = 1

# The largest request this backend will read from stdin, matching the Rust
# side's `RerankLimits::max_request_bytes`. The runner already refuses a larger
# one before spawning, but the backend is also driven by hand and by the
# training work in #214, so it enforces its own ceiling rather than trusting a
# caller it cannot see.
MAX_REQUEST_BYTES = 8 << 20

# The request id shape minted by `utilities::dated_id` under the `rr` prefix.
REQUEST_ID_PATTERN = r"\Arr-[0-9]{8}-[0-9a-f]{8}\Z"

# Exactly these keys, no more and no fewer, on each object. Both Rust types
# carry `deny_unknown_fields`, so the backend mirrors it rather than ignoring
# a field a caller believed was being honoured.
REQUEST_KEYS = ("protocol_version", "request_id", "model", "adapter", "query", "candidates")
CANDIDATE_KEYS = ("id", "rank", "text")

# Declared on every response; the Rust validator never assumes it. Both scoring
# modes here rank the largest score first.
SCORE_DIRECTION = "higher_is_better"

# --- Scoring modes

SCORING_CROSS_ENCODER = "cross-encoder"
SCORING_LEXICAL = "lexical-overlap"
SCORING_MODES = (SCORING_CROSS_ENCODER, SCORING_LEXICAL)

# --- The pinned base reranker
#
# Selected on published Hub metadata verified on 2026-09-18: a genuine trained
# reranker rather than a large generative model, `BertForSequenceClassification`
# with a single relevance logit, roughly 22.7M parameters, a 512-token context,
# Apache-2.0, and standard BERT module names so PEFT infers LoRA targets without
# a custom map. See the design document for the full comparison.

MODEL_ID = "cross-encoder/ms-marco-MiniLM-L6-v2"
MODEL_REVISION = "233902d25c440f23af6f7d6e94d2946bac0bee0a"
TOKENIZER_ID = MODEL_ID
TOKENIZER_REVISION = MODEL_REVISION

EXPECTED_ARCHITECTURES = ("BertForSequenceClassification",)
EXPECTED_NUM_LABELS = 1

# The modules whose parameters constitute the relevance head. Every one of them
# must be present in the checkpoint that is loaded; a head that arrives newly
# initialized is refused rather than scored with.
HEAD_MODULES = ("classifier",)

# --- Pinned inference configuration

DTYPE = "float32"
DEVICE = "cpu"
DEVICES = ("cpu", "cuda", "mps", "auto")
MAX_LENGTH = 512
TRUNCATION = "longest_first"
TRUNCATIONS = ("longest_first", "only_second")
PADDING = "longest"
PADDINGS = ("longest", "max_length")
BATCH_SIZE = 16
SCORE_COLUMN = 0

# --- The deterministic, non-neural mode
#
# This exists so that the real subprocess, the real JSON on real pipes and the
# real Rust runner are exercised without downloading anything. It measures no
# relevance quality whatsoever, and its model label is deliberately one no
# request expecting a real model can carry.

LEXICAL_VERSION = 1
LEXICAL_MODEL_LABEL = "lexical-overlap@1"
LEXICAL_NOTE = (
    "Deterministic Jaccard overlap of lowercased alphanumeric token sets. "
    "This mode exists to prove protocol conformance without a model download; "
    "it measures no relevance quality and must never be read as a baseline."
)
CROSS_ENCODER_NOTE = (
    "Pinned Transformers sequence-classification cross-encoder. One relevance "
    "logit per query/passage pair, read from the configured score column."
)

# --- Adapter expectations

ADAPTER_PEFT_TYPE = "LORA"
ADAPTER_TASK_TYPE = "SEQ_CLS"
ADAPTER_WEIGHT_FILES = ("adapter_model.safetensors", "adapter_model.bin")
BASE_WEIGHT_FILES = ("model.safetensors", "pytorch_model.bin")
ADAPTER_CONFIG_FILE = "adapter_config.json"

# --- The pinned dependency set, mirrored from requirements.txt
#
# `run-model-tests.sh` reads this map and refuses to run the opt-in suite when
# an installed version differs, for the same reason `scripts/dup-check.sh`
# refuses a `similarity-rs` it did not pin: a result from another build is not
# the result this configuration produces.

REQUIREMENTS = {
    "torch": "2.14.0",
    "transformers": "5.17.0",
    "peft": "0.21.0",
    "tokenizers": "0.23.2",
    "safetensors": "0.8.0",
    "huggingface_hub": "1.32.0",
    "numpy": "2.5.3",
}
REQUIREMENTS_FILE = "integrations/reranker/requirements.txt"
INSTALL_HINT = "pip install -r " + REQUIREMENTS_FILE
DOWNLOAD_HINT = (
    "huggingface-cli download " + MODEL_ID + " --revision " + MODEL_REVISION
)

# --- Exit codes, following sysexits.h as the rest of this repository does

EX_OK = 0
EX_USAGE = 64
EX_DATAERR = 65
EX_UNAVAILABLE = 69
EX_SOFTWARE = 70
EX_CANTCREAT = 73

# Every diagnostic line this backend writes to stderr starts with this, so an
# operator reading an interleaved log can tell which process spoke.
DIAG_PREFIX = BACKEND_NAME + ": "


class RerankError(Exception):
    """A refusal, carrying the exit code the process must terminate with.

    The Rust runner learns nothing from stderr, which it never parses; the exit
    code is the whole signal. Raising this rather than letting a traceback
    escape is what keeps the signal a documented number instead of `1`.
    """

    def __init__(self, message: str, code: int = EX_DATAERR) -> None:
        super().__init__(message)
        self.code = code


def model_label(model_id: str, revision: str) -> str:
    """The default identity a cross-encoder process answers to.

    Repository plus immutable revision, because two revisions of one repository
    are two different scorers and a label that could not tell them apart would
    let one silently answer for the other.
    """
    return model_id + "@" + revision
