"""What the standard library alone can prove about the recipe and the harness.

This suite runs on every `cargo nextest run`, driven from
`tests/lora_qualification.rs`, and it needs no model, no download and none of
the pinned libraries. What it proves is the part of the recipe that is a
contract rather than a number.

The centrepiece is the adapter round trip. A safetensors file is written here
with `struct`, `json` and `open` — an eight-byte little-endian header length
followed by that many bytes of JSON — carrying exactly the tensor names PEFT
produces for a `modules_to_save` head. That synthetic package is then handed to
`comemory_rerank_compat.validate_adapter`: the function that will refuse a real
adapter in production. Accepting the shape this recipe saves, and refusing each
way of breaking it, is the strongest evidence available without weights that
what the trainer writes is what the scorer expects to find.
"""

from __future__ import annotations

import json
import os
import struct
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(os.path.dirname(HERE), "reranker"))

import comemory_qualify_yaml as yaml_out
import comemory_rerank_compat as compat
import comemory_rerank_pins as backend
import comemory_train_manifest as manifest
import comemory_train_model as model
import comemory_train_pins as pins

HEAD_TENSORS = (
    "base_model.model.classifier.modules_to_save.default.weight",
    "base_model.model.classifier.modules_to_save.default.bias",
    "base_model.model.classifier.original_module.weight",
)
LORA_TENSORS = (
    "base_model.model.bert.encoder.layer.0.attention.self.query.lora_A.default.weight",
    "base_model.model.bert.encoder.layer.0.attention.self.query.lora_B.default.weight",
)


def write_safetensors(path: str, names) -> None:
    """A minimal, readable safetensors file carrying exactly `names`."""
    header = {
        name: {"dtype": "F32", "shape": [1], "data_offsets": [index * 4, index * 4 + 4]}
        for index, name in enumerate(names)
    }
    body = json.dumps(header, separators=(",", ":")).encode("utf-8")
    with open(path, "wb") as handle:
        handle.write(struct.pack("<Q", len(body)))
        handle.write(body)
        handle.write(b"\0" * (4 * len(header)))


def write_adapter(directory: str, **changes) -> str:
    """The adapter package this recipe saves, with optional deliberate damage."""
    os.makedirs(directory, exist_ok=True)
    config = {
        "peft_type": pins.ADAPTER_PEFT_TYPE,
        "task_type": pins.ADAPTER_TASK_TYPE,
        "base_model_name_or_path": pins.MODEL_ID,
        "revision": pins.MODEL_REVISION,
        "r": pins.LORA_R,
        "lora_alpha": pins.LORA_ALPHA,
        "lora_dropout": pins.LORA_DROPOUT,
        "target_modules": list(pins.LORA_TARGET_MODULES),
        "bias": pins.LORA_BIAS,
        "modules_to_save": list(pins.MODULES_TO_SAVE),
    }
    config.update(changes.get("config", {}))
    with open(os.path.join(directory, pins.ADAPTER_CONFIG_FILE), "w", encoding="utf-8") as h:
        json.dump(config, h, indent=2, sort_keys=True)
    names = changes.get("tensors", list(LORA_TENSORS) + list(HEAD_TENSORS))
    write_safetensors(os.path.join(directory, "adapter_model.safetensors"), names)
    return directory


class AdapterShape(unittest.TestCase):
    """The package this recipe saves is the package the backend will load."""

    def validate(self, directory: str) -> dict:
        """Run the production compatibility check over a package."""
        return compat.validate_adapter(model.backend_config("cpu", False, adapter=directory))

    def test_the_saved_shape_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            facts = self.validate(write_adapter(os.path.join(root, "lora-v1")))
        self.assertEqual(facts["task_type"], pins.ADAPTER_TASK_TYPE)
        self.assertEqual(facts["peft_type"], pins.ADAPTER_PEFT_TYPE)
        self.assertEqual(facts["modules_to_save"], sorted(pins.MODULES_TO_SAVE))
        self.assertEqual(facts["base_model_name_or_path"], pins.MODEL_ID)
        self.assertEqual(len(facts["weights_sha256"]), 64)

    def test_a_head_missing_from_modules_to_save_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            package = write_adapter(os.path.join(root, "a"), config={"modules_to_save": []})
            with self.assertRaises(backend.RerankError) as caught:
                self.validate(package)
        self.assertEqual(caught.exception.code, pins.EX_DATAERR)
        self.assertIn("modules_to_save", str(caught.exception))

    def test_a_head_missing_from_the_weight_file_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            package = write_adapter(os.path.join(root, "a"), tensors=list(LORA_TENSORS))
            with self.assertRaises(backend.RerankError) as caught:
                self.validate(package)
        self.assertEqual(caught.exception.code, pins.EX_DATAERR)
        self.assertIn("no tensor for it", str(caught.exception))

    def test_another_base_model_or_revision_is_refused(self) -> None:
        for change in ({"base_model_name_or_path": "other/model"}, {"revision": "deadbeef"}):
            with tempfile.TemporaryDirectory() as root:
                package = write_adapter(os.path.join(root, "a"), config=change)
                with self.assertRaises(backend.RerankError) as caught:
                    self.validate(package)
            self.assertEqual(caught.exception.code, pins.EX_DATAERR)
            self.assertIn("adapter/base mismatch", str(caught.exception))


class PinAgreement(unittest.TestCase):
    """The recipe and the scorer share one identity, not two copies of one."""

    def test_the_base_identity_is_imported_not_restated(self) -> None:
        for name in (
            "MODEL_ID", "MODEL_REVISION", "TOKENIZER_ID", "TOKENIZER_REVISION",
            "HEAD_MODULES", "MAX_LENGTH", "TRUNCATION", "PADDING", "SCORE_COLUMN",
            "EXPECTED_ARCHITECTURES", "EXPECTED_NUM_LABELS", "ADAPTER_TASK_TYPE",
            "ADAPTER_PEFT_TYPE", "PROTOCOL_VERSION", "SCORE_DIRECTION",
        ):
            self.assertEqual(getattr(pins, name), getattr(backend, name), name)

    def test_the_head_is_what_the_adapter_saves(self) -> None:
        self.assertEqual(tuple(pins.MODULES_TO_SAVE), tuple(backend.HEAD_MODULES))

    def test_requirements_agree_with_the_backend_and_with_the_file(self) -> None:
        self.assertEqual(pins.REQUIREMENTS, backend.REQUIREMENTS)
        self.assertEqual(self.pinned("training"), self.pinned("reranker"))
        declared = {name.replace("-", "_"): version for name, version in self.pinned("training")}
        self.assertEqual(declared, pins.REQUIREMENTS)

    def pinned(self, directory: str) -> list:
        """Every `name==version` line of one requirements file."""
        path = os.path.join(os.path.dirname(HERE), directory, "requirements.txt")
        with open(path, "r", encoding="utf-8") as handle:
            rows = [line.strip() for line in handle if line.strip() and not line.startswith("#")]
        return sorted(tuple(row.split("==")) for row in rows)


class SplitBoundary(unittest.TestCase):
    """The held-out split is unreachable by construction, not by convention."""

    def test_the_readable_allowlist_excludes_every_held_out_file(self) -> None:
        self.assertNotIn(pins.HOLDOUT_FILE, pins.READABLE_FILES)
        for name in pins.READABLE_FILES:
            self.assertNotIn("holdout", name)
            self.assertNotIn("implicit", name)

    def test_selection_consults_validation_only(self) -> None:
        recipe = manifest.recipe()
        self.assertEqual(recipe["selection"]["splits_consulted"], [pins.VALIDATION_SPLIT])


class AdapterManifest(unittest.TestCase):
    """The manifest digests its own contents, so tampering is detectable."""

    def test_the_id_recomputes_and_moves_with_its_contents(self) -> None:
        document = {"manifest_version": 1, "adapter_label": "lora-v1", "recipe": manifest.recipe()}
        first = manifest.adapter_id(document)
        document["adapter_id"] = first
        self.assertEqual(manifest.adapter_id(document), first)
        document["adapter_label"] = "lora-v2"
        self.assertNotEqual(manifest.adapter_id(document), first)

    def test_limitations_are_written_out(self) -> None:
        self.assertGreaterEqual(len(manifest.LIMITATIONS), 4)
        for line in manifest.LIMITATIONS:
            self.assertGreater(len(line), 60, line)


class YamlEmitter(unittest.TestCase):
    """Every scalar is typed, so no value can be reinterpreted on the way out."""

    def test_strings_are_quoted_and_numbers_are_not(self) -> None:
        body = yaml_out.dump({"name": "no", "k": 5, "decay": 0.0, "on": True, "gone": None})
        self.assertIn('name: "no"', body)
        self.assertIn("k: 5", body)
        self.assertIn("decay: 0.0", body)
        self.assertIn("on: true", body)
        self.assertIn("gone: null", body)

    def test_a_mapping_inside_a_sequence_indents_under_its_dash(self) -> None:
        body = yaml_out.dump({"tasks": [{"id": "a", "judgments": [{"relevance": 3}]}]})
        self.assertEqual(
            body,
            'tasks:\n  - id: "a"\n    judgments:\n      - relevance: 3\n',
        )

    def test_the_same_input_emits_the_same_bytes(self) -> None:
        document = {"version": 1, "tasks": [{"id": "a", "query": "x"}]}
        self.assertEqual(yaml_out.dump(document), yaml_out.dump(document))


if __name__ == "__main__":
    unittest.main()
