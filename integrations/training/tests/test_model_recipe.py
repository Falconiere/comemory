"""What only the pinned weights can prove about the recipe.

Opt-in, and run exclusively by `run-training-tests.sh`, which refuses to exit 0
unless every pinned library is installed at its pinned version and the pinned
model snapshot is already in the local cache. Nothing here is reachable from
`cargo nextest`, and nothing here downloads anything.

These are the assertions the design document lists as implemented and
**unmeasured** until someone runs this script on a machine that has the weights:
which parameters actually train, that the base is byte-identical after a real
optimizer step, that a saved adapter reloads to the same predictions within the
pinned tolerance, and that the resulting package scores a real request through
the reference backend without the identity gate refusing it.
"""

from __future__ import annotations

import json
import math
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
BACKEND = os.path.join(os.path.dirname(HERE), "reranker")
sys.path.insert(0, HERE)
sys.path.insert(0, BACKEND)

import comemory_rerank_compat as compat
import comemory_train_data as data
import comemory_train_model as model
import comemory_train_pins as pins

EXAMPLES = (
    ("activation decay", "Only a zero decay exponent removes the wall clock.", 3),
    ("activation decay", "The shared splitter keeps a heading breadcrumb.", 0),
    ("bounded subprocess", "One child under a single end-to-end deadline.", 2),
    ("bounded subprocess", "Markdown frontmatter carries the content hash.", 0),
)


def examples() -> list:
    """A handful of real reviewed rows, in the shape the loop consumes."""
    return [
        data.Example(
            observation_id="o-fixture",
            group_id="g-fixture",
            domain="memory",
            candidate_ref="memory:" + format(index, "08x") + ":" + "0" * 64,
            query=query,
            text=text,
            relevance=relevance,
            target=relevance / float(pins.MAX_RELEVANCE),
        )
        for index, (query, text, relevance) in enumerate(EXAMPLES)
    ]


class Recipe(unittest.TestCase):
    """The parameter audit, the frozen base and the save/reload guarantee."""

    @classmethod
    def setUpClass(cls) -> None:
        """Load the pinned base once; every test below adapts a fresh copy."""
        cls.config = model.backend_config("cpu", False)

    def adapted(self):
        """A freshly built LoRA model over a freshly loaded frozen base."""
        tokenizer, base, _head = model.load_base(self.config)
        return model.torch_module(), tokenizer, model.attach_adapter(base)

    def test_only_the_adapter_and_the_head_train(self) -> None:
        _torch, _tokenizer, adapted = self.adapted()
        audit = model.audit(adapted)
        self.assertGreater(audit["lora_parameters"], 0)
        self.assertGreater(audit["head_parameters"], 0)
        self.assertLessEqual(audit["fraction"], pins.TRAINABLE_FRACTION_MAX)
        for name in audit["names"]:
            self.assertTrue(
                ".lora_" in name or any(m in name.split(".") for m in pins.HEAD_MODULES), name
            )

    def test_one_optimizer_step_leaves_the_base_byte_identical(self) -> None:
        torch, tokenizer, adapted = self.adapted()
        before = model.frozen_digest(torch, adapted)
        rows = examples()
        optimizer = torch.optim.AdamW(
            [p for p in adapted.parameters() if p.requires_grad], lr=pins.LEARNING_RATE
        )
        adapted.train()
        encoded = model.encode(
            tokenizer, [e.query for e in rows], [e.text for e in rows], "cpu"
        )
        targets = torch.tensor([e.target for e in rows], dtype=torch.float32)
        logits = adapted(**encoded).logits[:, pins.SCORE_COLUMN]
        torch.nn.BCEWithLogitsLoss()(logits.to(torch.float32), targets).backward()
        optimizer.step()
        after = model.frozen_digest(torch, adapted)
        self.assertEqual(before["sha256"], after["sha256"])
        self.assertGreater(before["parameters"], 0)

    def test_a_saved_adapter_reloads_within_the_pinned_tolerance(self) -> None:
        torch, tokenizer, adapted = self.adapted()
        adapted.eval()
        probe = model.parity_pairs(pins.PARITY_PAIRS)
        before = model.score_pairs(adapted, tokenizer, torch, probe)
        with tempfile.TemporaryDirectory() as root:
            package = os.path.join(root, "lora-test")
            files = model.save(adapted, package)
            self.assertTrue(any(f["path"] == pins.ADAPTER_CONFIG_FILE for f in files))
            facts = compat.validate_adapter(
                model.backend_config("cpu", False, adapter=package)
            )
            self.assertEqual(facts["modules_to_save"], sorted(pins.MODULES_TO_SAVE))
            result = model.reload_and_compare(package, before, "cpu", False)
        self.assertTrue(result["passed"])
        self.assertLessEqual(result["max_abs_delta"], pins.RELOAD_MAX_ABS_DELTA)

    def test_the_reference_backend_scores_through_the_saved_adapter(self) -> None:
        _torch, _tokenizer, adapted = self.adapted()
        adapted.eval()
        with tempfile.TemporaryDirectory() as root:
            package = os.path.join(root, "lora-v1")
            files = model.save(adapted, package)
            saved = {entry["path"]: entry["sha256"] for entry in files}
            response, fingerprint = self.score_through_backend(package)
        self.assertEqual(response["adapter"], "lora-v1")
        self.assertEqual(response["model"], pins.MODEL_ID + "@" + pins.MODEL_REVISION)
        # The echoed label alone proves only that the process was CONFIGURED for
        # this adapter — the identity gate makes it exit 65 otherwise. The
        # fingerprint proves it LOADED this package: the digests it reports are
        # taken from the two files on disk, so they can only agree with the ones
        # recorded at save time if the weights that scored are these weights.
        self.assertEqual(
            fingerprint["adapter_weights_sha256"], saved["adapter_model.safetensors"]
        )
        self.assertEqual(
            fingerprint["adapter_config_sha256"], saved[pins.ADAPTER_CONFIG_FILE]
        )
        self.assertEqual(fingerprint["adapter_path"], package)
        self.assertEqual(fingerprint["adapter_modules_to_save"], sorted(pins.MODULES_TO_SAVE))
        self.assertTrue(fingerprint["scoring_is_neural"])
        self.assertEqual(len(response["scores"]), 2)
        self.assertEqual(response["score_direction"], pins.SCORE_DIRECTION)
        self.assertEqual(sorted(row["id"] for row in response["scores"]), ["1", "2"])
        for row in response["scores"]:
            self.assertIsInstance(row["score"], float)
            self.assertTrue(math.isfinite(row["score"]), row)

    def score_through_backend(self, package: str) -> tuple:
        """One real request to the backend; returns its response and fingerprint.

        `--verbose` makes the backend write the complete scoring identity to
        stderr before it scores, which is the only channel that reports what was
        actually loaded rather than what was asked for.
        """
        request = {
            "protocol_version": pins.PROTOCOL_VERSION,
            "request_id": "rr-20260918-0000abcd",
            "model": pins.MODEL_ID + "@" + pins.MODEL_REVISION,
            "adapter": "lora-v1",
            "query": "activation decay",
            "candidates": [
                {"id": "1", "rank": 1, "text": "Only a zero decay exponent removes the clock."},
                {"id": "2", "rank": 2, "text": "The splitter keeps a heading breadcrumb."},
            ],
        }
        done = subprocess.run(
            [
                sys.executable,
                os.path.join(BACKEND, "comemory_rerank.py"),
                "score",
                "--scoring",
                "cross-encoder",
                "--adapter",
                package,
                "--verbose",
            ],
            input=json.dumps(request).encode("utf-8"),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=300,
            check=False,
        )
        self.assertEqual(
            done.returncode, 0, done.stderr.decode("utf-8", "replace")
        )
        return (
            json.loads(done.stdout.decode("utf-8")),
            self.fingerprint_of(done.stderr.decode("utf-8", "replace")),
        )

    def fingerprint_of(self, stderr: str) -> dict:
        """The one `comemory-rerank: fingerprint {...}` line the backend wrote."""
        marker = "fingerprint "
        for line in stderr.splitlines():
            if marker in line:
                return json.loads(line.split(marker, 1)[1])
        raise AssertionError("no fingerprint line on stderr:\n" + stderr)


if __name__ == "__main__":
    unittest.main()
