# SPDX-License-Identifier: MPL-2.0

"""Regression coverage for the fuzzy-matching comparison harness."""

from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import corpus
import fuzzy


class EnvironmentTests(unittest.TestCase):
    def test_personal_fzf_options_are_removed(self) -> None:
        """A personal FZF_DEFAULT_OPTS can change scheme, tiebreak and algorithm."""
        with mock.patch.dict(
            "os.environ",
            {
                "FZF_DEFAULT_OPTS": "--algo=v1",
                "FZF_DEFAULT_COMMAND": "fd",
                "PATH": "/usr/bin",
            },
            clear=True,
        ):
            environment = fuzzy.clean_environment()
        self.assertNotIn("FZF_DEFAULT_OPTS", environment)
        self.assertNotIn("FZF_DEFAULT_COMMAND", environment)
        self.assertEqual(environment["PATH"], "/usr/bin")


class CommandTests(unittest.TestCase):
    def test_a_query_reaches_the_filter_unsplit(self) -> None:
        """Terms are separated by whitespace, so the query is one argument."""
        self.assertIn("parser test", fuzzy.runyte_command("parser test"))

    def test_timing_is_off_unless_asked_for(self) -> None:
        self.assertNotIn("--time", fuzzy.runyte_command("src"))
        self.assertIn("--time", fuzzy.runyte_command("src", time_it=True))

    def test_fzf_is_given_the_scheme(self) -> None:
        self.assertEqual(
            fuzzy.fzf_command("src", "path"),
            ["fzf", "--filter=src", "--scheme=path"],
        )

    def test_an_empty_query_is_still_passed(self) -> None:
        self.assertIn("--filter=", fuzzy.fzf_command("", "path"))


class RunTests(unittest.TestCase):
    """A filter that did not answer must not be measured.

    The failure this guards is silent: a crashed filter, or an fzf that
    rejected an option, exits fast and writes nothing, so timing would record
    the speed of the failure and agreement would read the empty output as a
    legitimate empty result set.
    """

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.candidates = Path(self.directory.name) / "corpus.txt"
        self.candidates.write_text("alpha\nbeta\n", encoding="utf-8")
        self.addCleanup(self.directory.cleanup)

    def test_an_answering_status_returns_the_process(self) -> None:
        completed = fuzzy.run(
            ["cat"], self.candidates, dict(os.environ), True, fuzzy.RUNYTE_ANSWERED
        )
        self.assertEqual(completed.stdout, b"alpha\nbeta\n")

    def test_fzfs_no_match_status_is_an_answer_not_a_failure(self) -> None:
        completed = fuzzy.run(
            ["sh", "-c", "exit 1"],
            self.candidates,
            dict(os.environ),
            False,
            fuzzy.FZF_ANSWERED,
        )
        self.assertEqual(completed.returncode, 1)

    def test_the_same_status_from_the_runyte_filter_is_a_failure(self) -> None:
        with self.assertRaises(fuzzy.FilterFailed):
            fuzzy.run(
                ["sh", "-c", "exit 1"],
                self.candidates,
                dict(os.environ),
                False,
                fuzzy.RUNYTE_ANSWERED,
            )

    def test_a_rejected_option_is_refused_and_its_message_surfaced(self) -> None:
        with self.assertRaises(fuzzy.FilterFailed) as failure:
            fuzzy.run(
                ["sh", "-c", "echo 'unknown option: --scheme' >&2; exit 2"],
                self.candidates,
                dict(os.environ),
                False,
                fuzzy.FZF_ANSWERED,
            )
        message = str(failure.exception)
        self.assertIn("exited 2", message)
        self.assertIn("unknown option: --scheme", message)

    def test_a_failure_is_not_timed(self) -> None:
        with self.assertRaises(fuzzy.FilterFailed):
            fuzzy.median_wall(
                ["sh", "-c", "exit 3"],
                self.candidates,
                dict(os.environ),
                2,
                fuzzy.FZF_ANSWERED,
            )

    def test_a_failure_is_not_read_as_an_empty_result_set(self) -> None:
        with self.assertRaises(fuzzy.FilterFailed):
            fuzzy.results(
                ["sh", "-c", "exit 3"],
                self.candidates,
                dict(os.environ),
                fuzzy.FZF_ANSWERED,
            )


class AgreementTests(unittest.TestCase):
    def test_identical_answers_agree_completely(self) -> None:
        ranked = ["a", "b", "c"]
        measured = fuzzy.agreement(ranked, list(ranked), top=10)
        self.assertTrue(measured["same_matches"])
        self.assertTrue(measured["same_first"])
        self.assertEqual(measured["shared_top"], 3)
        self.assertEqual(measured["only_ours"], 0)
        self.assertEqual(measured["only_theirs"], 0)

    def test_the_same_matches_in_a_different_order_are_not_a_filter_difference(
        self,
    ) -> None:
        """Ranking and filtering fail separately and are counted separately."""
        measured = fuzzy.agreement(["a", "b"], ["b", "a"], top=10)
        self.assertTrue(measured["same_matches"])
        self.assertFalse(measured["same_first"])
        self.assertEqual(measured["shared_top"], 2)

    def test_extra_matches_on_each_side_are_counted_separately(self) -> None:
        measured = fuzzy.agreement(["a", "b"], ["b", "c", "d"], top=10)
        self.assertFalse(measured["same_matches"])
        self.assertEqual(measured["only_ours"], 1)
        self.assertEqual(measured["only_theirs"], 2)

    def test_only_the_top_counts_towards_the_shared_figure(self) -> None:
        ours = [str(index) for index in range(10)]
        measured = fuzzy.agreement(ours, list(reversed(ours)), top=3)
        self.assertEqual(measured["shared_top"], 0)
        self.assertTrue(measured["same_matches"])

    def test_no_match_on_either_side_has_no_first_result(self) -> None:
        measured = fuzzy.agreement([], [], top=10)
        self.assertFalse(measured["same_first"])
        self.assertIsNone(measured["ours_first"])
        self.assertIsNone(measured["theirs_first"])


class QueryTests(unittest.TestCase):
    def test_query_names_are_unique(self) -> None:
        names = [name for name, _, _ in fuzzy.QUERIES]
        self.assertEqual(len(names), len(set(names)))

    def test_the_floor_and_the_rejection_path_are_both_measured(self) -> None:
        typed = {query for _, query, _ in fuzzy.QUERIES}
        self.assertIn("", typed)
        self.assertIn("zzqx", typed)

    def test_no_query_uses_an_fzf_extended_operator(self) -> None:
        """`^ $ ! ' |` change what fzf matches and have no Runyte equivalent.

        A query carrying one would compare fzf's extended search syntax against
        Runyte reading the character literally, which is not a disagreement
        about fuzzy matching.
        """
        for name, query, _ in fuzzy.QUERIES:
            for operator in "^$!'|":
                self.assertNotIn(operator, query, f"{name} uses {operator}")

    def test_the_no_match_query_really_matches_nothing(self) -> None:
        """A rejection row that quietly started matching would measure nothing."""
        rejected = next(query for name, query, _ in fuzzy.QUERIES if name == "no match")
        for candidate in corpus.paths(corpus.SIZES["medium"]):
            self.assertFalse(
                _is_subsequence(rejected, candidate), f"{rejected} matches {candidate}"
            )


def _is_subsequence(query: str, candidate: str) -> bool:
    wanted = iter(candidate.lower())
    return all(character in wanted for character in query.lower())


class TableTests(unittest.TestCase):
    def test_a_table_has_a_header_rule_per_column(self) -> None:
        rendered = fuzzy.table(["a", "b"], [["1", "2"]]).splitlines()
        self.assertEqual(rendered[0], "| a | b |")
        self.assertEqual(rendered[1], "| --- | --- |")
        self.assertEqual(rendered[2], "| 1 | 2 |")

    def test_alignments_are_used_when_given(self) -> None:
        rendered = fuzzy.table(["a", "b"], [], ["---", "---:"]).splitlines()
        self.assertEqual(rendered[1], "| --- | ---: |")


class ArgumentTests(unittest.TestCase):
    def test_defaults_measure_every_size(self) -> None:
        options = fuzzy.parse_arguments([])
        self.assertEqual(options.sizes, ",".join(corpus.SIZES))
        self.assertEqual(options.scheme, "path")

    def test_fzf_can_be_left_out(self) -> None:
        self.assertTrue(fuzzy.parse_arguments(["--no-fzf"]).no_fzf)


class MainTests(unittest.TestCase):
    def test_an_unknown_size_is_refused(self) -> None:
        with mock.patch.object(fuzzy.Path, "exists", return_value=True):
            self.assertEqual(fuzzy.main(["--sizes", "enormous"]), 1)

    def test_an_unknown_query_is_refused(self) -> None:
        with mock.patch.object(fuzzy.Path, "exists", return_value=True):
            self.assertEqual(fuzzy.main(["--queries", "nonsense"]), 1)

    def test_a_missing_filter_binary_is_reported_rather_than_measured(self) -> None:
        with mock.patch.object(fuzzy.Path, "exists", return_value=False):
            self.assertEqual(fuzzy.main([]), 1)


if __name__ == "__main__":
    unittest.main()
