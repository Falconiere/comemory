"""The opt-in model suite: what only the pinned weights can prove.

Run it with `bash integrations/reranker/run-model-tests.sh`, which refuses to
start unless every pinned library and the pinned snapshot are present. Nothing
here skips: if this file runs at all, the prerequisites were satisfied, so a
failure is a real failure.

What it establishes that the protocol conformance suite cannot:

- the pinned revision really is a `BertForSequenceClassification` with one
  relevance logit, and its relevance head really came from the checkpoint;
- a query's answering passage really outranks its distractors;
- batching does not change a score, so `--batch-size` is a throughput knob and
  not a scoring knob;
- a PEFT `SEQ_CLS` LoRA adapter round trips through the backend — built and
  saved here, never trained, so this needs no second download;
- and an adapter that omits the relevance head from `modules_to_save` is
  refused with the diagnostic that names the failure, rather than silently
  scoring with a freshly initialized head.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.realpath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, ROOT)

import comemory_rerank_compat as compat
import comemory_rerank_model as model_mod
import comemory_rerank_pins as pins

ENTRY = os.path.join(ROOT, "comemory_rerank.py")
REQUEST_ID = "rr-20260918-1a2b3c4d"

QUERY = "how does a bounded process runner stop a child that never exits"
ANSWER = (
    "The runner computes a deadline before spawning and polls the child; when the "
    "deadline passes it kills the process and reaps it, so no zombie is left behind."
)
DISTRACTORS = (
    "Markdown files with YAML frontmatter are the source of truth for every memory.",
    "The FTS5 tokenizer splits camelCase and snake_case identifiers into subtokens.",
    "Homebrew publishes the release tap from the cargo-dist workflow on every tag.",
)


def _base_config(**overrides) -> dict:
    """The pinned configuration the entry point would build."""
    config = {
        "model_id": pins.MODEL_ID,
        "model_revision": pins.MODEL_REVISION,
        "tokenizer_id": pins.TOKENIZER_ID,
        "tokenizer_revision": pins.TOKENIZER_REVISION,
        "model_label": None,
        "adapter": None,
        "adapter_label": None,
        "dtype": pins.DTYPE,
        "device": pins.DEVICE,
        "max_length": pins.MAX_LENGTH,
        "truncation": pins.TRUNCATION,
        "padding": pins.PADDING,
        "batch_size": pins.BATCH_SIZE,
        "score_column": pins.SCORE_COLUMN,
        "head_modules": pins.HEAD_MODULES,
        "allow_download": False,
        "allow_base_mismatch": False,
        "digest_base_weights": False,
    }
    config.update(overrides)
    return config


def _request(model_label: str, adapter_label, texts) -> bytes:
    """One protocol request over the answer passage and its distractors."""
    return json.dumps(
        {
            "protocol_version": pins.PROTOCOL_VERSION,
            "request_id": REQUEST_ID,
            "model": model_label,
            "adapter": adapter_label,
            "query": QUERY,
            "candidates": [
                {"id": "candidate:%d" % index, "rank": index, "text": text}
                for index, text in enumerate(texts)
            ],
        }
    ).encode("utf-8")


def _run_backend(body: bytes, *args) -> subprocess.CompletedProcess:
    """Drive the real entry point as a real child over real pipes."""
    return subprocess.run(
        [sys.executable, ENTRY, "score", "--scoring", pins.SCORING_CROSS_ENCODER, *args],
        input=body,
        capture_output=True,
        check=False,
    )


class PinnedModelTest(unittest.TestCase):
    """The pinned base model, loaded and scored in process."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.scorer = model_mod.CrossEncoderScorer(**_base_config())
        cls.scorer.load()

    def test_architecture_and_head_match_the_pins(self) -> None:
        """The pinned revision is the shape this backend was configured for."""
        print_out = self.scorer.fingerprint()
        self.assertEqual(print_out["architectures"], list(pins.EXPECTED_ARCHITECTURES))
        self.assertEqual(print_out["num_labels"], pins.EXPECTED_NUM_LABELS)
        self.assertEqual(print_out["model_revision"], pins.MODEL_REVISION)
        self.assertEqual(print_out["score_direction"], pins.SCORE_DIRECTION)
        self.assertTrue(print_out["scoring_is_neural"])

    def test_relevance_head_came_from_the_checkpoint(self) -> None:
        """Every head parameter is present in the pinned weight file itself."""
        fingerprint = self.scorer.fingerprint()
        head = fingerprint["head_parameters"]
        self.assertTrue(head, "the model declares no relevance-head parameters")
        keys = compat.checkpoint_keys(compat.base_weights_path(_base_config()))
        for name in head:
            self.assertIn(name, keys, "%s was newly initialized, not loaded" % name)

    def test_answer_outranks_every_distractor(self) -> None:
        """The passage that answers the query scores highest, over real text."""
        texts = [ANSWER, *DISTRACTORS]
        scores = self.scorer.score(QUERY, texts)
        self.assertEqual(len(scores), len(texts))
        self.assertGreater(scores[0], max(scores[1:]))

    def test_batching_does_not_change_a_score(self) -> None:
        """`--batch-size` is a throughput knob, never a scoring knob."""
        texts = [ANSWER, *DISTRACTORS]
        one_at_a_time = model_mod.CrossEncoderScorer(**_base_config(batch_size=1))
        one_at_a_time.load()
        for whole, single in zip(self.scorer.score(QUERY, texts),
                                 one_at_a_time.score(QUERY, texts)):
            self.assertAlmostEqual(whole, single, places=4)

    def test_real_protocol_round_trip(self) -> None:
        """The real child answers the real wire protocol with a valid response."""
        label = pins.model_label(pins.MODEL_ID, pins.MODEL_REVISION)
        done = _run_backend(_request(label, None, [ANSWER, *DISTRACTORS]))
        self.assertEqual(done.returncode, 0, done.stderr.decode("utf-8", "replace"))
        body = json.loads(done.stdout)
        self.assertEqual(
            sorted(body),
            ["adapter", "model", "protocol_version", "request_id", "score_direction", "scores"],
        )
        self.assertEqual(body["request_id"], REQUEST_ID)
        self.assertEqual(body["model"], label)
        self.assertIsNone(body["adapter"])
        top = max(body["scores"], key=lambda row: row["score"])
        self.assertEqual(top["id"], "candidate:0")


class AdapterTest(unittest.TestCase):
    """A PEFT sequence-classification adapter, saved without training."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.workspace = tempfile.mkdtemp(prefix="comemory-rerank-adapter-")
        cls.adapter = os.path.join(cls.workspace, "lora-v1")
        _save_untrained_adapter(cls.adapter)

    @classmethod
    def tearDownClass(cls) -> None:
        shutil.rmtree(cls.workspace, ignore_errors=True)

    def test_adapter_loads_and_scores_through_the_real_protocol(self) -> None:
        """A well-formed adapter is accepted and echoes its own identity."""
        done = _run_backend(
            _request(pins.model_label(pins.MODEL_ID, pins.MODEL_REVISION), "lora-v1",
                     [ANSWER, *DISTRACTORS]),
            "--adapter",
            self.adapter,
        )
        self.assertEqual(done.returncode, 0, done.stderr.decode("utf-8", "replace"))
        body = json.loads(done.stdout)
        self.assertEqual(body["adapter"], "lora-v1")
        self.assertEqual(len(body["scores"]), 1 + len(DISTRACTORS))

    def test_adapter_without_a_saved_head_is_refused(self) -> None:
        """Dropping the head from modules_to_save is refused, never scored with."""
        broken = os.path.join(self.workspace, "lora-no-head")
        shutil.copytree(self.adapter, broken)
        config_path = os.path.join(broken, pins.ADAPTER_CONFIG_FILE)
        with open(config_path, "r", encoding="utf-8") as handle:
            config = json.load(handle)
        config["modules_to_save"] = []
        with open(config_path, "w", encoding="utf-8") as handle:
            json.dump(config, handle)
        done = _run_backend(
            _request(pins.model_label(pins.MODEL_ID, pins.MODEL_REVISION), "lora-no-head",
                     [ANSWER]),
            "--adapter",
            broken,
        )
        self.assertEqual(done.returncode, pins.EX_DATAERR)
        self.assertEqual(done.stdout, b"")
        self.assertIn("modules_to_save", done.stderr.decode("utf-8", "replace"))
        self.assertIn("freshly initialized", done.stderr.decode("utf-8", "replace"))

    def test_adapter_for_another_base_is_refused(self) -> None:
        """An adapter naming a different base model does not silently load."""
        retargeted = os.path.join(self.workspace, "lora-other-base")
        shutil.copytree(self.adapter, retargeted)
        config_path = os.path.join(retargeted, pins.ADAPTER_CONFIG_FILE)
        with open(config_path, "r", encoding="utf-8") as handle:
            config = json.load(handle)
        config["base_model_name_or_path"] = "BAAI/bge-reranker-base"
        with open(config_path, "w", encoding="utf-8") as handle:
            json.dump(config, handle)
        done = _run_backend(
            _request(pins.model_label(pins.MODEL_ID, pins.MODEL_REVISION), "lora-other-base",
                     [ANSWER]),
            "--adapter",
            retargeted,
        )
        self.assertEqual(done.returncode, pins.EX_DATAERR)
        self.assertIn("adapter/base mismatch", done.stderr.decode("utf-8", "replace"))


def _save_untrained_adapter(destination: str) -> None:
    """Build and save a PEFT LoRA adapter with the relevance head included.

    Nothing is trained: the point is the checkpoint's shape, not its quality,
    and training here would make the suite depend on data it does not have.
    """
    from peft import LoraConfig, TaskType, get_peft_model
    from transformers import AutoModelForSequenceClassification

    base = AutoModelForSequenceClassification.from_pretrained(
        pins.MODEL_ID, revision=pins.MODEL_REVISION, local_files_only=True
    )
    config = LoraConfig(
        task_type=TaskType.SEQ_CLS,
        r=4,
        lora_alpha=8,
        lora_dropout=0.0,
        target_modules=["query", "value"],
        modules_to_save=list(pins.HEAD_MODULES),
    )
    adapted = get_peft_model(base, config)
    adapted.save_pretrained(destination)


if __name__ == "__main__":
    unittest.main()
