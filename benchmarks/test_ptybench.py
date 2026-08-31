# SPDX-License-Identifier: MPL-2.0

"""Regression coverage for aggregation in the pseudo-terminal benchmark."""

from __future__ import annotations

import sys
import time
import unittest
from unittest import mock

import ptybench


class MedianStartupTests(unittest.TestCase):
    def test_startup_metrics_and_quit_completeness_are_counted_independently(
        self,
    ) -> None:
        samples = [
            {
                "first_byte": 0.001,
                "first_document_output": 0.002,
                "settled_output": 0.003,
                "quit": 0.007,
                "bytes": 100,
            },
            {
                "first_byte": 0.002,
                "first_document_output": 0.003,
                "settled_output": 0.004,
                "quit": 0.005,
                "bytes": 120,
            },
            {
                "first_byte": 0.003,
                "first_document_output": 0.004,
                "settled_output": 0.005,
                "quit": None,
                "bytes": 140,
            },
            {
                "first_byte": 0.004,
                "first_document_output": 0.001,
                "settled_output": None,
                "quit": None,
                "bytes": 160,
            },
        ]

        with mock.patch("ptybench.measure_startup", side_effect=samples):
            result = ptybench.median_startup(["editor"], {}, b"DOC", runs=4)

        self.assertEqual(result["first_byte_ms"], 2.0)
        self.assertEqual(result["first_document_output_ms"], 3.0)
        self.assertEqual(result["settled_output_ms"], 4.0)
        self.assertEqual(result["quit_ms"], 6.0)
        self.assertEqual(result["bytes"], 130)
        self.assertEqual(result["runs"], 4)
        self.assertEqual(result["first_byte_complete"], 3)
        self.assertEqual(result["first_document_output_complete"], 3)
        self.assertEqual(result["settled_output_complete"], 3)
        self.assertEqual(result["quit_complete"], 2)


class QuitValidityTests(unittest.TestCase):
    def test_exit_before_the_quit_command_is_not_a_quit_sample(self) -> None:
        result = ptybench.measure_startup(
            [sys.executable, "-c", "pass"], {}, b"DOC"
        )

        self.assertIsNone(result["quit"])

    def test_marker_followed_by_process_exit_is_not_complete_startup(self) -> None:
        script = "import os; os.write(1,b'DOC')"
        sample = ptybench.measure_startup(
            [sys.executable, "-c", script], {}, b"DOC"
        )

        with mock.patch("ptybench.measure_startup", return_value=sample):
            result = ptybench.median_startup(
                [sys.executable, "-c", script], {}, b"DOC", runs=1
            )

        self.assertIsNotNone(sample["first_document_output"])
        self.assertIsNone(sample["settled_output"])
        self.assertIsNone(result["first_document_output_ms"])
        self.assertEqual(result["first_document_output_complete"], 0)

    def test_startup_clock_includes_spawn_time(self) -> None:
        script = "import os; os.write(1,b'DOC'); os.read(0,5)"
        real_spawn = ptybench._spawn

        def delayed_spawn(*args, **kwargs):
            process = real_spawn(*args, **kwargs)
            time.sleep(0.1)
            return process

        with mock.patch("ptybench._spawn", side_effect=delayed_spawn):
            result = ptybench.measure_startup(
                [sys.executable, "-c", script], {}, b"DOC"
            )

        self.assertGreaterEqual(result["first_document_output"], 0.08)

    def test_loading_presentation_is_not_first_document_content(self) -> None:
        script = (
            "import os,time; "
            "os.write(1,b'L'*17); time.sleep(0.6); "
            "os.write(1,b'DOC'+b'F'*300); os.read(0,5)"
        )

        result = ptybench.measure_startup(
            [sys.executable, "-c", script], {}, b"DOC"
        )

        self.assertGreater(result["settled_output"], 0.5)
        self.assertGreater(result["first_document_output"], 0.5)
        self.assertGreaterEqual(result["bytes"], 317)

    def test_loading_output_larger_than_the_old_threshold_cannot_settle(self) -> None:
        script = "import os; os.write(1,b'L'*300)"

        result = ptybench.measure_startup(
            [sys.executable, "-c", script], {}, b"DOC"
        )

        self.assertIsNone(result["settled_output"])
        self.assertIsNone(result["first_document_output"])

    def test_document_marker_can_span_pty_reads(self) -> None:
        script = (
            "import os,time; os.write(1,b'D'); time.sleep(0.05); "
            "os.write(1,b'OC'+b'F'*300); os.read(0,5)"
        )

        result = ptybench.measure_startup(
            [sys.executable, "-c", script], {}, b"DOC"
        )

        self.assertIsNotNone(result["settled_output"])
        self.assertIsNotNone(result["first_document_output"])

    def test_small_output_after_document_moves_settlement(self) -> None:
        script = (
            "import os,time; os.write(1,b'DOC'+b'F'*300); time.sleep(0.15); "
            "os.write(1,b'x'); os.read(0,5)"
        )

        result = ptybench.measure_startup(
            [sys.executable, "-c", script], {}, b"DOC"
        )

        self.assertGreater(result["settled_output"], 0.1)

    def test_empty_document_marker_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "must not be empty"):
            ptybench.measure_startup([sys.executable, "-c", "pass"], {}, b"")


class CpuAccountingTests(unittest.TestCase):
    def test_proc_stat_includes_reaped_child_ticks(self) -> None:
        # Process names may contain spaces and parentheses. The fields after
        # the final `)` begin with state; positions 11-14 are fields 14-17.
        fields = ["S", *["0"] * 10, "2", "3", "5", "7", "0"]
        stat = f"123 (helper (finished)) {' '.join(fields)}"

        self.assertEqual(ptybench._stat_cpu_ticks(stat), 17)


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
