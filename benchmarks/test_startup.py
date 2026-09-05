# SPDX-License-Identifier: MPL-2.0

"""Behavior checks for readiness evidence and instrumented milestone reports."""

import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import startup


class ScreenTests(unittest.TestCase):
    def test_a_gutter_space_cannot_masquerade_as_the_inserted_space(self):
        terminal = startup.Terminal()
        terminal.feed(b" 1  local function scan_0")
        position = terminal.position(startup.DOCUMENT_TEXT)
        self.assertEqual(position, (0, 4))
        self.assertFalse(terminal.at(position, " " + startup.DOCUMENT_TEXT))
        terminal.feed(b"\x1b[1;5H local function scan_0")
        self.assertTrue(terminal.at(position, " " + startup.DOCUMENT_TEXT))

    def test_expired_stagger_deadline_uses_a_nonblocking_poll(self):
        with mock.patch("startup.select.select", return_value=([], [], [])) as poll:
            startup.pump(7, None, -0.0001)
        self.assertEqual(poll.call_args.args[-1], 0.0)

    def test_cursor_addressed_edits_count_only_after_sync_frame_ends(self):
        terminal = startup.Terminal()
        terminal.feed(b"\x1b[2J\x1b[Hlocal function scan_0")
        self.assertTrue(terminal.contains("local function scan_0"))
        terminal.feed(b"\x1b[?2026h\x1b[HBENCHREADYlocal function scan_0")
        self.assertFalse(terminal.contains("BENCHREADYlocal function scan_0"))
        terminal.feed(b"\x1b[?2026l")
        self.assertTrue(terminal.contains("BENCHREADYlocal function scan_0"))

    def test_title_and_erased_output_are_not_document_evidence(self):
        terminal = startup.Terminal()
        terminal.feed(b"\x1b]0;BENCHREADYlocal function scan_0\x07")
        self.assertFalse(terminal.contains("BENCHREADYlocal function scan_0"))
        terminal.feed(b"BENCHREADYlocal function scan_0\x1b[2J")
        self.assertFalse(terminal.contains("BENCHREADYlocal function scan_0"))

    def test_split_capability_query_is_answered_exactly_once(self):
        terminal = startup.Terminal()
        self.assertEqual(terminal.feed(b"\x1b[?"), b"")
        self.assertEqual(terminal.feed(b"u"), b"\x1b[?0u")
        self.assertEqual(terminal.feed(b"next frame"), b"")

    def test_control_string_payload_cannot_overwrite_document_cells(self):
        for control in (b"P+q4D73", b"_kitty payload", b"^private", b"Xstring"):
            with self.subTest(control=control):
                terminal = startup.Terminal()
                terminal.feed(b"local function scan_0\x1b[H\x1b")
                terminal.feed(control)
                terminal.feed(b"\x1b")
                terminal.feed(b"\\")
                self.assertTrue(terminal.contains("local function scan_0"))


class EventTests(unittest.TestCase):
    def read(self, content, syntax=True):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "events"
            path.write_text(content)
            return startup.read_events(path, 1_000_000_000, syntax)

    def test_milestones_share_the_parent_clock_and_plaintext_has_no_syntax(self):
        content = "file_loaded 1002000000\nsyntax_ready 1009000000\n"
        self.assertEqual(self.read(content), {"file_loaded": 2, "syntax_ready": 9})
        self.assertEqual(self.read(content, False), {"file_loaded": 2})

    def test_missing_duplicate_reversed_and_negative_events_are_invalid(self):
        for content in ("", "file_loaded 1002000000\n",
                        "file_loaded 1002000000\nfile_loaded 1003000000\n",
                        "file_loaded 1002000000\nsyntax_ready 1001000000\n",
                        "file_loaded 900000000\nsyntax_ready 1001000000\n"):
            with self.subTest(content=content), self.assertRaises(ValueError):
                self.read(content)

    def test_failed_sample_is_not_hidden_by_a_successful_subset(self):
        self.assertEqual(startup.cell([{"ready": 10}, {}], "ready", 2),
                         "incomplete (1/2)")
        self.assertEqual(startup.cell([{"ready": 10}, {"ready": 30}], "ready", 2),
                         "20.0 (10.0–30.0)")


class ReadinessTests(unittest.TestCase):
    # The executable is Python itself; only fixture data is written by tests.
    SCRIPT = r'''
import os, sys, tty
tty.setraw(0)
original = open(sys.argv[1], 'rb').read()
os.write(1, b'\x1b[2J\x1b[H' + original)
pending = b''
while b'i ' not in pending:
    pending += os.read(0, 4096)
os.write(1, b'\x1b[H' + b' ' + original)
pending = b''
while True:
    pending += os.read(0, 4096)
    if b'\r' not in pending:
        continue
    command, pending = pending.split(b'\r', 1)
    if b':write! ' in command:
        path = command.split(b':write! ', 1)[1].decode()
        with open(path, 'wb') as saved:
            saved.write(PAYLOAD)
    elif b':q!' in command:
        sys.exit(0)
'''

    def run_editor(self, payload):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "short.txt"
            fixture.write_bytes(b"local function scan_0\nrest of file\n")
            script = self.SCRIPT.replace("PAYLOAD", payload)
            return startup.measure([sys.executable, "-c", script], {}, fixture)

    def test_readiness_requires_a_verified_edit_to_the_complete_file(self):
        result = self.run_editor("b' ' + original")
        self.assertGreater(result["ready_to_edit"], 0)

    def test_displaying_a_marker_without_changing_the_buffer_fails(self):
        with mock.patch.object(startup, "TIMEOUT", 0.4):
            with self.assertRaises(TimeoutError):
                self.run_editor("original")


@unittest.skipUnless(os.environ.get("RUNYTE_BENCH_BINARY"),
                     "set RUNYTE_BENCH_BINARY to a built Runyte executable")
class RunyteReadinessTests(unittest.TestCase):
    def test_first_open_with_empty_storage_accepts_and_saves_the_edit(self):
        binary = str(Path(os.environ["RUNYTE_BENCH_BINARY"]).resolve())
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            replacements = {name: root / name.lower() for name in (
                "FIXTURES", "EMPTY_CONFIG", "EMPTY_CACHE", "EMPTY_STATE",
                "EMPTY_DATA", "EMPTY_HOME",
            )}
            with mock.patch.multiple(startup.baseline, **replacements):
                fixtures = startup.baseline.prepare()
                for name in ("short.txt", "short.lua"):
                    with self.subTest(fixture=name):
                        result = startup.measure(
                            [binary], startup.baseline.environment(), fixtures[name])
                        self.assertGreater(result["ready_to_edit"], 0)


if __name__ == "__main__":
    unittest.main()
