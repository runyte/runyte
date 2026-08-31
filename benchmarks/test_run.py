# SPDX-License-Identifier: MPL-2.0

"""Regression coverage for benchmark command selection."""

from __future__ import annotations

import unittest
from unittest import mock

import run as benchmark_run


class DiscoveryTests(unittest.TestCase):
    def test_runyte_only_needs_no_external_editor(self) -> None:
        with mock.patch("run.shutil.which", return_value=None):
            with mock.patch("run.runyte_binary", return_value="/path/to/runyte"):
                editors = benchmark_run.discover(["runyte"])

        self.assertEqual(editors, [("runyte", ["/path/to/runyte"])])


if __name__ == "__main__":
    unittest.main()
