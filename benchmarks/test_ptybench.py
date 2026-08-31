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


class MedianIdleTests(unittest.TestCase):
    def test_complete_windows_are_aggregated_by_median(self) -> None:
        samples = [
            {"cpu_percent": 0.3, "writes": 3, "bytes": 30, "complete": True},
            {"cpu_percent": 0.1, "writes": 1, "bytes": 10, "complete": True},
            {"cpu_percent": 0.2, "writes": 2, "bytes": 20, "complete": True},
        ]

        with mock.patch("ptybench.measure_idle", side_effect=samples):
            result = ptybench.median_idle(["editor"], {}, runs=3)

        self.assertEqual(result["cpu_percent"], 0.2)
        self.assertEqual(result["cpu_min"], 0.1)
        self.assertEqual(result["cpu_max"], 0.3)
        self.assertEqual(result["writes"], 2)
        self.assertEqual(result["writes_min"], 1)
        self.assertEqual(result["writes_max"], 3)
        self.assertEqual(result["bytes"], 20)
        self.assertEqual(result["runs"], 3)
        self.assertEqual(result["complete"], 3)

    def test_an_incomplete_window_suppresses_the_whole_result_set(self) -> None:
        samples = [
            {"cpu_percent": 0.1, "writes": 0, "bytes": 0, "complete": True},
            {"cpu_percent": None, "writes": 0, "bytes": 0, "complete": False},
            {"cpu_percent": 0.2, "writes": 0, "bytes": 0, "complete": True},
        ]

        with mock.patch("ptybench.measure_idle", side_effect=samples):
            result = ptybench.median_idle(["editor"], {}, runs=3)

        self.assertIsNone(result["cpu_percent"])
        self.assertIsNone(result["cpu_min"])
        self.assertIsNone(result["cpu_max"])
        self.assertIsNone(result["writes"])
        self.assertIsNone(result["writes_min"])
        self.assertIsNone(result["writes_max"])
        self.assertIsNone(result["bytes"])
        self.assertEqual(result["complete"], 2)

    def test_cpu_unavailable_does_not_suppress_portable_screen_writes(self) -> None:
        samples = [
            {"cpu_percent": None, "writes": 0, "bytes": 0, "complete": True},
            {"cpu_percent": None, "writes": 2, "bytes": 20, "complete": True},
            {"cpu_percent": None, "writes": 1, "bytes": 10, "complete": True},
        ]

        with mock.patch("ptybench.measure_idle", side_effect=samples):
            result = ptybench.median_idle(["editor"], {}, runs=3)

        self.assertIsNone(result["cpu_percent"])
        self.assertIsNone(result["cpu_min"])
        self.assertIsNone(result["cpu_max"])
        self.assertEqual(result["writes"], 1)
        self.assertEqual(result["writes_min"], 0)
        self.assertEqual(result["writes_max"], 2)
        self.assertEqual(result["bytes"], 10)
        self.assertEqual(result["complete"], 3)

    def test_process_that_exits_during_settle_is_incomplete(self) -> None:
        result = ptybench.measure_idle(
            [sys.executable, "-c", "pass"], {}, settle=0.1, window=0.1
        )

        self.assertFalse(result["complete"])
        self.assertIsNone(result["cpu_percent"])


if __name__ == "__main__":
    unittest.main()
