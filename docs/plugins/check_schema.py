# SPDX-License-Identifier: MPL-2.0
"""Check the plugin schema and examples: python3 docs/plugins/check_schema.py.

Requires the development-only jsonschema package. Writes no files and launches
only the existing example script, with bounded input and a process timeout.
"""
import copy
import json
from pathlib import Path
import re
import subprocess
import sys
import unittest

from jsonschema import Draft202012Validator


DIRECTORY = Path(__file__).resolve().parent
SCHEMA = json.loads((DIRECTORY / "runyte-experimental-1.schema.json").read_text())
DEFINITIONS = SCHEMA["$defs"]
VALIDATOR = Draft202012Validator(SCHEMA)


def validator_for(name):
    return Draft202012Validator({
        "$schema": SCHEMA["$schema"],
        "$defs": DEFINITIONS,
        "$ref": f"#/$defs/{name}",
    })


def example(name):
    return copy.deepcopy(DEFINITIONS[name]["examples"][0])


class PluginSchemaTests(unittest.TestCase):
    def test_schema_and_all_message_examples(self):
        Draft202012Validator.check_schema(SCHEMA)
        for direction in ("hostMessage", "pluginMessage"):
            for reference in DEFINITIONS[direction]["oneOf"]:
                name = reference["$ref"].rsplit("/", 1)[1]
                self.assertTrue(DEFINITIONS[name]["examples"], name)
                for message in DEFINITIONS[name]["examples"]:
                    with self.subTest(direction=direction, message=message):
                        VALIDATOR.validate(message)
                        validator_for(direction).validate(message)
                        validator_for(name).validate(message)
                        other = ("pluginMessage" if direction == "hostMessage"
                                 else "hostMessage")
                        self.assertFalse(validator_for(other).is_valid(message))

    def test_guide_json_blocks(self):
        guide = (DIRECTORY.parent / "plugins.md").read_text()
        blocks = re.findall(r"^```json\n(.*?)^```", guide, re.M | re.S)
        self.assertTrue(blocks)
        for block in blocks:
            for line in block.splitlines():
                with self.subTest(line=line):
                    VALIDATOR.validate(json.loads(line))

    def test_required_fields_and_unknown_field_policy(self):
        for name, definition in DEFINITIONS.items():
            if "examples" not in definition:
                continue
            for field in definition["required"]:
                message = example(name)
                del message[field]
                with self.subTest(name=name, missing=field):
                    self.assertFalse(VALIDATOR.is_valid(message))
            message = example(name)
            message["future_field"] = True
            with self.subTest(name=name, extra=True):
                self.assertEqual(VALIDATOR.is_valid(message),
                                 definition["additionalProperties"])
        message = example("register")
        message["commands"][0]["future_field"] = True
        self.assertFalse(VALIDATOR.is_valid(message))
        message = example("invoke")
        message["selections"][0]["future_field"] = True
        VALIDATOR.validate(message)
        self.assertFalse(VALIDATOR.is_valid({"type": "future_message"}))

    def test_registration_and_failure_constraints(self):
        for name in ("", "Uppercase", "has.dot", "stop", "a" * 49, "a\n"):
            message = example("register")
            message["commands"][0]["name"] = name
            with self.subTest(name=name):
                self.assertFalse(VALIDATOR.is_valid(message))
        for count in (0, 17):
            message = example("register")
            message["commands"] *= count
            self.assertFalse(VALIDATOR.is_valid(message))
        message = example("register")
        message["version"] = "runyte-experimental-2"
        self.assertFalse(VALIDATOR.is_valid(message))
        for text in ("line\n", "\x00", "\x85", "a" * 161):
            message = example("register")
            message["commands"][0]["description"] = text
            self.assertFalse(VALIDATOR.is_valid(message))
        for text in ("line\n", "\x7f", "a" * 1025):
            message = example("fail")
            message["message"] = text
            self.assertFalse(VALIDATOR.is_valid(message))

    def test_completion_status_and_revision(self):
        for status in DEFINITIONS["complete"]["properties"]["status"]["enum"]:
            message = example("complete")
            message["status"] = status
            message["revision"] = "revision-token" if status == "applied" else None
            VALIDATOR.validate(message)
            message["revision"] = None if status == "applied" else "revision-token"
            self.assertFalse(VALIDATOR.is_valid(message))
        message = example("complete")
        message["status"] = "success"
        self.assertFalse(VALIDATOR.is_valid(message))
        message = example("complete")
        message["message"] = "unexpected success message"
        self.assertFalse(VALIDATOR.is_valid(message))

    def test_coordinates_tokens_and_replacement_types(self):
        for value in (-1, 1.5, "1", None, True):
            message = example("invoke")
            message["selections"][0]["from"] = value
            self.assertFalse(VALIDATOR.is_valid(message))
        message = example("invoke")
        message["primary"] = 1024
        self.assertFalse(VALIDATOR.is_valid(message))
        message = example("replace")
        message["invocation"] = 1
        self.assertFalse(VALIDATOR.is_valid(message))
        message["invocation"] = "opaque-token"
        VALIDATOR.validate(message)
        message["replacements"] = [None]
        self.assertFalse(VALIDATOR.is_valid(message))

    def test_schema_does_not_claim_stateful_or_byte_validation(self):
        # These pass the structural schema but can be rejected by the host.
        message = example("subscribe")
        message["request"] = "😀" * 64
        self.assertGreater(len(message["request"].encode("utf-8")), 64)
        VALIDATOR.validate(message)
        message["request"] = "a" * 65
        self.assertFalse(VALIDATOR.is_valid(message))
        message = example("replace")
        message["replacements"] = []
        VALIDATOR.validate(message)
        message = example("invoke")
        message["primary"] = len(message["selections"])
        VALIDATOR.validate(message)

    def test_running_example_handles_repeated_unicode_invocations(self):
        first = example("invoke")
        second = copy.deepcopy(first)
        second.update(invocation="second", text="e\u0301ß😀")
        second["selections"] = [
            {"anchor": 2, "head": 0, "from": 0, "to": 3},
            {"anchor": 3, "head": 3, "from": 3, "to": 4},
        ]
        messages = [example("hello"), example("registered"), first,
                    example("complete"), second]
        for message in messages:
            validator_for("hostMessage").validate(message)
        result = subprocess.run(
            [sys.executable, str(DIRECTORY / "uppercase.py")],
            input="".join(json.dumps(message) + "\n" for message in messages),
            text=True, encoding="utf-8", capture_output=True, timeout=5, check=True,
        )
        self.assertTrue(result.stdout.endswith("\n"))
        replies = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(len(replies), 3)
        for reply in replies:
            validator_for("pluginMessage").validate(reply)
        self.assertEqual(replies[0]["type"], "register")
        self.assertEqual(replies[1], example("replace"))
        self.assertEqual(replies[2], {
            "type": "replace", "invocation": "second",
            "replacements": ["E\u0301SS", "😀"],
        })


if __name__ == "__main__":
    unittest.main(verbosity=2)
