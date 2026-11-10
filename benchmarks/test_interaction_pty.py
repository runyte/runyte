# SPDX-License-Identifier: MPL-2.0
"""Checks for repeated-interaction evidence and percentile conventions."""

import os
import select
import sys
import time
import unittest
from unittest import mock

import interaction_pty as interaction
import ptybench
import startup


class InteractionTests(unittest.TestCase):
    def test_exact_spawn_environment_does_not_restore_omitted_ambient_values(self):
        with mock.patch.dict(os.environ, {"RUNYTE_BENCH_UNEXPECTED": "ambient"}):
            pid, fd = ptybench._spawn(
                [sys.executable, "-c",
                 "import os; print(os.environ.get('RUNYTE_BENCH_UNEXPECTED', 'absent'))"],
                {}, None, exact_environment=True)
            try:
                data = b""
                deadline = time.monotonic() + 3
                while b"\n" not in data and time.monotonic() < deadline:
                    if select.select([fd], [], [], .05)[0]:
                        data += os.read(fd, 4096)
                self.assertEqual(data.strip(), b"absent")
            finally:
                ptybench._reap(pid)
                os.close(fd)

    def test_fixture_is_shared_ascii_contract(self):
        lines = interaction.fixture_text().splitlines()
        self.assertEqual(len(lines), 100)
        self.assertEqual(lines[0], "interaction latency marker")
        self.assertEqual(lines[-1], "line 0099: repeated editing fixture")
        self.assertTrue(interaction.fixture_text().isascii())

    def test_nearest_rank_small_sample_tail_is_explicit(self):
        self.assertEqual(interaction.summary(list(range(1, 41))),
                         {"p50": 20, "p95": 38, "p99": 40, "max": 40})
        with self.assertRaises(ValueError):
            interaction.summary([])
        for value in (float("nan"), float("inf"), -1):
            with self.assertRaises(ValueError):
                interaction.summary([value])

    def test_exact_cells_require_completed_frame_and_unchanged_companion(self):
        terminal = startup.Terminal()
        terminal.feed(("\x1b[H" + interaction.MARKER + "\x1b[1;65H"
                       + interaction.SIDE_MARKER).encode())
        position, companion = (0, 0), (0, 64)
        expected = " " + interaction.MARKER
        self.assertFalse(interaction.frame_matches(terminal, position, expected, companion))
        terminal.feed(("\x1b[?2026h\x1b[H" + expected).encode())
        self.assertFalse(interaction.frame_matches(terminal, position, expected, companion))
        terminal.feed(b"\x1b[?2026l")
        self.assertTrue(interaction.frame_matches(terminal, position, expected, companion))
        terminal.feed(b"\x1b[1;65HX")
        self.assertFalse(interaction.frame_matches(terminal, position, expected, companion))

    def test_backspace_requires_removal_of_trailing_marker_character(self):
        terminal = startup.Terminal()
        terminal.feed((" " + interaction.MARKER + "\x1b[1;65H"
                       + interaction.SIDE_MARKER).encode())
        # A shortened partial rewrite leaves the old final 'r' behind.
        terminal.feed(("\x1b[H" + interaction.MARKER).encode())
        self.assertFalse(interaction.frame_matches(
            terminal, (0, 0), interaction.MARKER + " ", (0, 64)))
        terminal.feed(b" ")
        self.assertTrue(interaction.frame_matches(
            terminal, (0, 0), interaction.MARKER + " ", (0, 64)))


if __name__ == "__main__":
    unittest.main()
