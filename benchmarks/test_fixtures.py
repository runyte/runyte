# SPDX-License-Identifier: MPL-2.0

"""Regression coverage for generated benchmark fixtures."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import fixtures


class MatrixTests(unittest.TestCase):
    def test_matrix_is_every_size_in_every_suffix(self) -> None:
        self.assertEqual(
            fixtures.FIXTURES,
            (
                "short.txt",
                "medium.txt",
                "long.txt",
                "short.lua",
                "medium.lua",
                "long.lua",
            ),
        )

    def test_sizes_are_the_documented_line_counts(self) -> None:
        self.assertEqual(fixtures.SIZES, {"short": 500, "medium": 5_000, "long": 50_000})

    def test_split_rejects_a_name_outside_the_matrix(self) -> None:
        for name in ("large.rs", "small.txt", "long.md", "long"):
            with self.subTest(name=name):
                with self.assertRaises(ValueError):
                    fixtures.split(name)


class LuaFixtureTests(unittest.TestCase):
    def test_every_size_is_deterministic_complete_source(self) -> None:
        for size, lines in fixtures.SIZES.items():
            with self.subTest(size=size):
                source = fixtures._lua_source(lines)

                self.assertEqual(source, fixtures._lua_source(lines))
                self.assertEqual(source.count("\n"), lines)
                self.assertTrue(source.endswith("\n\n"))
                self.assertEqual(
                    source.count("local function "), source.count("\nend\n")
                )

    def test_sizes_avoid_every_editors_injection_triggers(self) -> None:
        for size, lines in fixtures.SIZES.items():
            source = fixtures._lua_source(lines)
            for trigger in ("--", "[[", "]]", "cdef(", "exec_lua(", "vim."):
                with self.subTest(size=size, trigger=trigger):
                    self.assertNotIn(trigger, source)


class EnsureTests(unittest.TestCase):
    def test_ensure_generates_the_whole_matrix(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            paths = fixtures.ensure(Path(directory))

            self.assertEqual(tuple(paths), fixtures.FIXTURES)
            for name, path in paths.items():
                size, _ = fixtures.split(name)
                self.assertEqual(
                    path.read_text(), fixtures._lua_source(fixtures.SIZES[size])
                )

    def test_each_pair_of_a_size_is_byte_identical(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            paths = fixtures.ensure(Path(directory))

            for size in fixtures.SIZES:
                with self.subTest(size=size):
                    self.assertEqual(
                        paths[f"{size}.txt"].read_bytes(),
                        paths[f"{size}.lua"].read_bytes(),
                    )

    def test_ensure_generates_a_requested_subset_only(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = fixtures.ensure(root, ("long.lua",))

            self.assertEqual(tuple(paths), ("long.lua",))
            self.assertEqual(
                sorted(path.name for path in root.iterdir()), ["long.lua"]
            )

    def test_ensure_rejects_a_name_outside_the_matrix(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                fixtures.ensure(Path(directory), ("large.rs",))


class DescribeTests(unittest.TestCase):
    def test_describe_reports_kilobytes_below_a_megabyte(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "short.txt"
            path.write_bytes(b"x" * 17_400)

            self.assertEqual(fixtures.describe(path), "17 kB")

    def test_describe_reports_megabytes_at_and_above_one(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "long.txt"
            path.write_bytes(b"x" * 1_740_000)

            self.assertEqual(fixtures.describe(path), "1.7 MB")


if __name__ == "__main__":
    unittest.main()
