# SPDX-License-Identifier: MPL-2.0

"""Regression coverage for aggregation in the pseudo-terminal benchmark."""

from __future__ import annotations

import sys
import unittest
from unittest import mock

import ptybench


class MedianStartupTests(unittest.TestCase):
    def test_ready_and_quit_completeness_are_counted_independently(self) -> None:
        samples = [
            {"first_paint": 0.001, "ready": 0.003, "quit": 0.007, "bytes": 100},
            {"first_paint": 0.002, "ready": 0.004, "quit": 0.005, "bytes": 120},
            {"first_paint": 0.003, "ready": 0.005, "quit": None, "bytes": 140},
            {"first_paint": 0.004, "ready": None, "quit": None, "bytes": 160},
        ]

        with mock.patch("ptybench.measure_startup", side_effect=samples):
            result = ptybench.median_startup(["editor"], {}, runs=4)

        self.assertEqual(result["first_paint_ms"], 2.5)
        self.assertEqual(result["ready_ms"], 4.0)
        self.assertEqual(result["quit_ms"], 6.0)
        self.assertEqual(result["bytes"], 130)
        self.assertEqual(result["runs"], 4)
        self.assertEqual(result["complete"], 3)
        self.assertEqual(result["quit_complete"], 2)


class QuitValidityTests(unittest.TestCase):
    def test_exit_before_the_quit_command_is_not_a_quit_sample(self) -> None:
        result = ptybench.measure_startup([sys.executable, "-c", "pass"], {})

        self.assertIsNone(result["quit"])


if __name__ == "__main__":
    unittest.main()
