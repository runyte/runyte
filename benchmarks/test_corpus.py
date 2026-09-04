# SPDX-License-Identifier: MPL-2.0

"""Regression coverage for the generated path corpora."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import corpus


class SizeTests(unittest.TestCase):
    def test_sizes_are_the_documented_candidate_counts(self) -> None:
        self.assertEqual(
            corpus.SIZES, {"small": 1_000, "medium": 10_000, "large": 100_000}
        )


class PathTests(unittest.TestCase):
    def test_one_seed_gives_one_corpus(self) -> None:
        self.assertEqual(corpus.paths(500), corpus.paths(500))

    def test_a_different_seed_gives_a_different_corpus(self) -> None:
        self.assertNotEqual(corpus.paths(500), corpus.paths(500, seed=1))

    def test_candidates_are_unique(self) -> None:
        generated = corpus.paths(5_000)
        self.assertEqual(len(generated), 5_000)
        self.assertEqual(len(set(generated)), 5_000)

    def test_the_largest_size_is_reachable(self) -> None:
        """The vocabulary has to be able to spell 100,000 distinct paths.

        Generation loops until the set is full, so a vocabulary too small for
        the requested count would hang rather than fail. This is the guard.
        """
        generated = corpus.paths(corpus.SIZES["large"])
        self.assertEqual(len(set(generated)), corpus.SIZES["large"])

    def test_order_is_not_sorted(self) -> None:
        generated = corpus.paths(1_000)
        self.assertNotEqual(generated, sorted(generated))

    def test_an_empty_corpus_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            corpus.paths(0)

    def test_depth_and_shape_look_like_a_repository(self) -> None:
        generated = corpus.paths(2_000)
        depths = {candidate.count("/") + 1 for candidate in generated}
        self.assertEqual(depths, {1, 2, 3, 4, 5})
        for candidate in generated:
            self.assertFalse(candidate.startswith("/"))
            self.assertFalse(candidate.endswith("/"))

    def test_directories_are_candidates_in_their_own_right(self) -> None:
        """The picker ranks directories, so a corpus without them measures
        a picker nobody uses. Leaving them out once produced a recorded result
        claiming Runyte ranked `src` badly, when the editor puts `src` first.
        """
        generated = corpus.paths(2_000)
        directories = [
            candidate for candidate in generated if "." not in _basename(candidate)
        ]
        self.assertTrue(directories)
        # Every directory that appears in a path is offered as a candidate too.
        for candidate in generated:
            parent = candidate.rsplit("/", 1)[0] if "/" in candidate else None
            if parent is not None:
                self.assertIn(parent, set(generated))

    def test_files_outnumber_directories_the_way_a_repository_does(self) -> None:
        generated = corpus.paths(10_000)
        directories = sum(
            1 for candidate in generated if "." not in _basename(candidate)
        )
        share = directories / len(generated)
        self.assertGreater(share, 0.03)
        self.assertLess(share, 0.20)

    def test_directories_are_reused_rather_than_unique_per_file(self) -> None:
        """A generator that gave each file a fresh chain would make every
        directory query match one file, which is not what a repository is.
        """
        generated = corpus.paths(2_000)
        holders = [
            candidate.rsplit("/", 1)[0] for candidate in generated if "/" in candidate
        ]
        self.assertGreater(len(holders) / len(set(holders)), 3.0)


def _basename(candidate: str) -> str:
    return candidate.rsplit("/", 1)[-1]


class WriteTests(unittest.TestCase):
    def test_every_size_is_written_and_returned(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            written = corpus.write(Path(directory), {"tiny": 10})
            self.assertEqual(set(written), {"tiny"})
            lines = written["tiny"].read_text(encoding="utf-8").splitlines()
            self.assertEqual(lines, corpus.paths(10))

    def test_rewriting_reproduces_the_same_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            first = corpus.write(Path(directory), {"tiny": 10})["tiny"].read_bytes()
            second = corpus.write(Path(directory), {"tiny": 10})["tiny"].read_bytes()
            self.assertEqual(first, second)


if __name__ == "__main__":
    unittest.main()
