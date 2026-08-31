# SPDX-License-Identifier: MPL-2.0

"""Regression coverage for benchmark command selection."""

from __future__ import annotations

from pathlib import Path
import tempfile
import unittest
from unittest import mock

import run as benchmark_run


class DiscoveryTests(unittest.TestCase):
    def test_runyte_only_needs_no_external_editor(self) -> None:
        with mock.patch("run.shutil.which", return_value=None):
            with mock.patch("run.runyte_binary", return_value="/path/to/runyte"):
                editors = benchmark_run.discover(["runyte"])

        self.assertEqual(editors, [("runyte", ["/path/to/runyte"])])

    def test_neovim_disables_swap_and_shada(self) -> None:
        def which(name: str) -> str | None:
            return "/usr/bin/nvim" if name == "nvim" else None

        with mock.patch("run.shutil.which", side_effect=which):
            with mock.patch("run.runyte_binary", return_value=None):
                editors = benchmark_run.discover(["neovim"])

        self.assertEqual(editors, [("neovim", ["/usr/bin/nvim", "-n", "-i", "NONE"])])


class EnvironmentTests(unittest.TestCase):
    def test_every_personal_storage_root_is_isolated(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            replacements = {
                "EMPTY_CONFIG": root / "config",
                "EMPTY_CACHE": root / "cache",
                "EMPTY_STATE": root / "state",
                "EMPTY_DATA": root / "data",
                "EMPTY_HOME": root / "home",
            }
            with mock.patch.multiple(benchmark_run, **replacements):
                environment = benchmark_run.environment()

        self.assertEqual(
            environment,
            {
                "XDG_CONFIG_HOME": str(root / "config"),
                "XDG_CACHE_HOME": str(root / "cache"),
                "XDG_STATE_HOME": str(root / "state"),
                "XDG_DATA_HOME": str(root / "data"),
                "HOME": str(root / "home"),
            },
        )

    def test_prepare_creates_every_isolated_storage_root(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            replacements = {
                "FIXTURES": root / "fixtures",
                "EMPTY_CONFIG": root / "config",
                "EMPTY_CACHE": root / "cache",
                "EMPTY_STATE": root / "state",
                "EMPTY_DATA": root / "data",
                "EMPTY_HOME": root / "home",
            }
            with mock.patch.multiple(benchmark_run, **replacements):
                with mock.patch("run.fixtures.ensure", return_value={}):
                    with mock.patch("run.subprocess.run"):
                        benchmark_run.prepare()

            for name, path in replacements.items():
                if name != "FIXTURES":
                    with self.subTest(name=name):
                        self.assertTrue(path.is_dir())

    def test_version_probe_uses_the_isolated_environment_and_working_directory(self) -> None:
        completed = mock.Mock(stdout="editor 1.0\n")
        isolated = {"XDG_STATE_HOME": "/isolated/state"}
        with mock.patch("run.subprocess.run", return_value=completed) as subprocess_run:
            version = benchmark_run.version_of(["/usr/bin/editor"], isolated, "/work")

        self.assertEqual(version, "editor 1.0")
        kwargs = subprocess_run.call_args.kwargs
        self.assertEqual(kwargs["env"]["XDG_STATE_HOME"], "/isolated/state")
        self.assertEqual(kwargs["cwd"], "/work")


class RunCountTests(unittest.TestCase):
    def test_idle_aggregation_requires_at_least_three_windows(self) -> None:
        for value in ("0", "1", "2"):
            with self.subTest(value=value):
                with self.assertRaisesRegex(
                    benchmark_run.argparse.ArgumentTypeError,
                    "idle run count must be at least 3",
                ):
                    benchmark_run.idle_run_count(value)

        self.assertEqual(benchmark_run.idle_run_count("3"), 3)


class IdleCellTests(unittest.TestCase):
    def test_incomplete_result_set_discloses_its_sample_count(self) -> None:
        result = {
            "cpu_percent": None,
            "cpu_min": None,
            "cpu_max": None,
            "writes": None,
            "writes_min": None,
            "writes_max": None,
            "runs": 5,
            "complete": 4,
        }

        self.assertEqual(
            benchmark_run.idle_cells(result),
            ("incomplete (4/5)", "incomplete (4/5)"),
        )

    def test_missing_proc_keeps_the_portable_write_median(self) -> None:
        result = {
            "cpu_percent": None,
            "cpu_min": None,
            "cpu_max": None,
            "writes": 1,
            "writes_min": 0,
            "writes_max": 2,
            "runs": 5,
            "complete": 5,
        }

        self.assertEqual(
            benchmark_run.idle_cells(result), ("unavailable", "1 (0–2)")
        )

    def test_idle_cells_show_median_and_range(self) -> None:
        result = {
            "cpu_percent": 0.2,
            "cpu_min": 0.1,
            "cpu_max": 0.3,
            "writes": 1,
            "writes_min": 0,
            "writes_max": 2,
            "runs": 5,
            "complete": 5,
        }

        self.assertEqual(
            benchmark_run.idle_cells(result), ("0.20 % (0.10–0.30)", "1 (0–2)")
        )


if __name__ == "__main__":
    unittest.main()
