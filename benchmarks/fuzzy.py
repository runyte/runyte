#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

"""Fuzzy path matching in Runyte, measured against fzf.

Run from anywhere:

    benchmarks/fuzzy.py                    every size and query
    benchmarks/fuzzy.py --sizes medium     one corpus size
    benchmarks/fuzzy.py --runs 15          more timing samples; default 7
    benchmarks/fuzzy.py --no-fzf           Runyte alone, no comparison
    benchmarks/fuzzy.py --scheme default   fzf's default scoring scheme

Output is Markdown. Two things are reported and they answer different
questions. The timing table asks what one query costs; the agreement table asks
whether the two programs put the same candidates at the top of the list, which
is the part a person actually notices.

Both programs are given byte-identical candidates on standard input and write
their matches to standard output, best first, so the comparison is between two
complete filters rather than between two functions chosen for similarity. That
also means the headline timing includes process start, reading the corpus and
writing the answer for both — see the README for why no attempt is made to
subtract those, and what the Runyte-only rank column is for.
"""

from __future__ import annotations

import argparse
import os
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import corpus  # noqa: E402

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
WORK = HERE / ".work"
CORPUS = WORK / "fuzzy"
FILTER = REPO / "target" / "release" / "examples" / "fuzzy_filter"

# The query shapes a picker is asked for, chosen to separate the parts of a
# score rather than to flatter either program. Each name is what the row means;
# the query is what is typed.
QUERIES = (
    ("empty", "", "no query: the floor, where both programs emit every candidate"),
    ("one character", "s", "the widest possible match set"),
    ("segment", "src", "a short directory name, matching a large fraction"),
    ("acronym", "fpr", "scattered characters, where alignment choice decides the order"),
    ("across a separator", "keymap", "spans the underscore in key_map"),
    ("basename", "file_picker.rs", "a whole file name, typed out"),
    ("path", "src/parser", "a directory and a name, with the separator typed"),
    ("two terms", "parser test", "whitespace-separated terms, matched in order"),
    ("no match", "zzqx", "the rejection path, where nothing survives the filter"),
)


def parse_arguments(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Compare Runyte's fuzzy path matching against fzf.",
    )
    parser.add_argument(
        "--sizes",
        default=",".join(corpus.SIZES),
        help=f"comma-separated corpus sizes; one or more of {', '.join(corpus.SIZES)}",
    )
    parser.add_argument(
        "--queries",
        default=None,
        help="comma-separated query names to measure; every one by default",
    )
    parser.add_argument("--runs", type=int, default=7, help="timing samples per cell")
    parser.add_argument(
        "--repeat",
        type=int,
        default=5,
        help="ranking passes inside one Runyte process, for the rank-only column",
    )
    parser.add_argument(
        "--scheme",
        default="path",
        help="fzf scoring scheme; 'path' matches what the corpus is",
    )
    parser.add_argument(
        "--top",
        type=int,
        default=10,
        help="how many results the agreement table compares",
    )
    parser.add_argument(
        "--agreement-size",
        default="medium",
        help="the corpus size the agreement table is computed on",
    )
    parser.add_argument("--no-fzf", action="store_true", help="measure Runyte alone")
    return parser.parse_args(argv)


def clean_environment() -> dict[str, str]:
    """The environment both programs run in.

    A personal ``FZF_DEFAULT_OPTS`` can change fzf's scheme, tiebreak and even
    its matching algorithm, so a result measured with one in place would not be
    reproducible anywhere else. It is removed rather than merged.
    """
    environment = dict(os.environ)
    for name in ("FZF_DEFAULT_OPTS", "FZF_DEFAULT_OPTS_FILE", "FZF_DEFAULT_COMMAND"):
        environment.pop(name, None)
    return environment


def runyte_command(query: str, repeat: int = 1, time_it: bool = False) -> list[str]:
    command = [str(FILTER), "--query", query, "--repeat", str(repeat)]
    if time_it:
        command.append("--time")
    return command


def fzf_command(query: str, scheme: str) -> list[str]:
    return ["fzf", f"--filter={query}", f"--scheme={scheme}"]


# Exit statuses that are answers rather than failures. fzf reports 1 when
# nothing matched, which is a result the agreement table needs; anything else
# from either program is a failure whose empty output must not be measured.
RUNYTE_ANSWERED = (0,)
FZF_ANSWERED = (0, 1)


class FilterFailed(Exception):
    """A filter exited with a status that is not an answer.

    Worth its own type because the failure it guards against is silent. A
    filter that crashes, or an fzf that rejects an option such as an
    unsupported `--scheme`, exits fast and writes nothing: the timing table
    would record the speed of the failure and the agreement table would read
    the empty output as a legitimate empty result set.
    """


def run(command: list[str], candidates: Path, environment: dict[str, str],
        capture: bool, answered: tuple[int, ...]) -> subprocess.CompletedProcess:
    """One filter run, with the corpus on standard input.

    Raises `FilterFailed` unless the process exits with one of `answered`.
    """
    with candidates.open("rb") as stdin:
        completed = subprocess.run(
            command,
            stdin=stdin,
            stdout=subprocess.PIPE if capture else subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            env=environment,
        )
    if completed.returncode not in answered:
        message = completed.stderr.decode("utf-8", "replace").strip()
        raise FilterFailed(
            f"{' '.join(command)}\n  exited {completed.returncode}, "
            f"expected one of {', '.join(str(status) for status in answered)}"
            + (f"\n  {message}" if message else "")
        )
    return completed


def median_wall(command: list[str], candidates: Path, environment: dict[str, str],
                runs: int, answered: tuple[int, ...]) -> float:
    """Median wall time of `runs` complete runs, in milliseconds.

    One warm-up run is discarded so that the first read of a corpus file, which
    comes off the disk rather than the page cache, is not one of the samples.
    """
    run(command, candidates, environment, False, answered)
    samples = []
    for _ in range(runs):
        start = time.perf_counter()
        run(command, candidates, environment, False, answered)
        samples.append((time.perf_counter() - start) * 1000.0)
    return statistics.median(samples)


def rank_microseconds(query: str, candidates: Path, environment: dict[str, str],
                      repeat: int) -> float | None:
    """Runyte's ranking time alone, as the filter itself reports it."""
    completed = run(
        runyte_command(query, repeat=repeat, time_it=True),
        candidates,
        environment,
        False,
        RUNYTE_ANSWERED,
    )
    for line in completed.stderr.decode("utf-8", "replace").splitlines():
        if line.startswith("rank_us "):
            return float(line.split(" ", 2)[1])
    return None


def results(command: list[str], candidates: Path, environment: dict[str, str],
            answered: tuple[int, ...]) -> list[str]:
    completed = run(command, candidates, environment, True, answered)
    return completed.stdout.decode("utf-8", "replace").splitlines()


def agreement(ours: list[str], theirs: list[str], top: int) -> dict:
    """How far two ranked answers to the same query agree.

    Three separate questions, because they fail separately. Whether the two
    programs consider the same candidates to match at all is a property of the
    filter; whether the same candidates reach the visible part of the list is a
    property of the ranking; whether the same one is first is what a person who
    presses Enter without looking gets.
    """
    ours_set, theirs_set = set(ours), set(theirs)
    ours_top, theirs_top = ours[:top], theirs[:top]
    shared = len(set(ours_top) & set(theirs_top))
    return {
        "ours": len(ours),
        "theirs": len(theirs),
        "only_ours": len(ours_set - theirs_set),
        "only_theirs": len(theirs_set - ours_set),
        "same_matches": ours_set == theirs_set,
        "top": top,
        "shared_top": shared,
        "same_first": bool(ours_top and theirs_top and ours_top[0] == theirs_top[0]),
        "ours_first": ours_top[0] if ours_top else None,
        "theirs_first": theirs_top[0] if theirs_top else None,
        "ours_top": ours_top,
        "theirs_top": theirs_top,
    }


def table(headers: list[str], rows: list[list[str]],
          alignments: list[str] | None = None) -> str:
    alignments = alignments or ["---"] * len(headers)
    lines = [
        "| " + " | ".join(headers) + " |",
        "| " + " | ".join(alignments) + " |",
    ]
    lines += ["| " + " | ".join(row) + " |" for row in rows]
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    options = parse_arguments(sys.argv[1:] if argv is None else argv)

    if not FILTER.exists():
        print(
            f"{FILTER.relative_to(REPO)} is missing. Build it first:\n"
            "    cargo build --release --example fuzzy_filter",
            file=sys.stderr,
        )
        return 1

    sizes = [name.strip() for name in options.sizes.split(",") if name.strip()]
    unknown = [name for name in sizes if name not in corpus.SIZES]
    if unknown:
        print(f"unknown corpus size: {', '.join(unknown)}", file=sys.stderr)
        return 1

    queries = list(QUERIES)
    if options.queries:
        wanted = [name.strip() for name in options.queries.split(",") if name.strip()]
        queries = [entry for entry in QUERIES if entry[0] in wanted]
        missing = set(wanted) - {name for name, _, _ in queries}
        if missing:
            print(f"unknown query: {', '.join(sorted(missing))}", file=sys.stderr)
            return 1

    have_fzf = not options.no_fzf and shutil.which("fzf") is not None
    if not options.no_fzf and not have_fzf:
        print("fzf is not on PATH; measuring Runyte alone.", file=sys.stderr)

    environment = clean_environment()
    files = corpus.write(CORPUS, {name: corpus.SIZES[name] for name in sizes})

    print("# Fuzzy path matching against fzf\n")
    print(f"Runyte {version()} · {fzf_version(environment) if have_fzf else 'fzf absent'}"
          f" · {machine()}\n")
    print(
        f"{options.runs} samples per cell, median reported. fzf runs with "
        f"`--scheme={options.scheme}`, and once more under `GOMAXPROCS=1`, "
        "because fzf matches on every core and Runyte's picker ranks on one "
        "worker thread.\n"
    )

    for size in sizes:
        candidates = files[size]
        count = corpus.SIZES[size]
        print(f"## {size} · {count:,} candidates\n")
        rows = []
        for name, query, _ in queries:
            row = [name, f"`{query}`" if query else "(empty)"]
            ours = median_wall(
                runyte_command(query), candidates, environment,
                options.runs, RUNYTE_ANSWERED,
            )
            row.append(f"{ours:.1f}")
            rank = rank_microseconds(query, candidates, environment, options.repeat)
            row.append("—" if rank is None else f"{rank / 1000.0:.1f}")
            if have_fzf:
                theirs = median_wall(
                    fzf_command(query, options.scheme), candidates, environment,
                    options.runs, FZF_ANSWERED,
                )
                row.append(f"{theirs:.1f}")
                single = dict(environment, GOMAXPROCS="1")
                alone = median_wall(
                    fzf_command(query, options.scheme), candidates, single,
                    options.runs, FZF_ANSWERED,
                )
                row.append(f"{alone:.1f}")
            rows.append(row)
        headers = ["query", "typed", "runyte", "runyte rank only", "fzf", "fzf, one thread"]
        alignments = ["---", "---", "---:", "---:", "---:", "---:"]
        if not have_fzf:
            headers, alignments = headers[:4], alignments[:4]
        print(table(headers, rows, alignments))
        print("\nMilliseconds. Every column but *rank only* is a whole process: "
              "start, read the corpus, filter, write the answer, exit.\n")

    if not have_fzf:
        return 0

    size = options.agreement_size
    if size not in files:
        size = sizes[0]
    candidates = files[size]
    print(f"## Agreement · {size}, {corpus.SIZES[size]:,} candidates\n")
    rows = []
    divergent = []
    for name, query, _ in queries:
        # Neither program ranks an empty query: fzf echoes its input in order
        # and Runyte's picker sorts by path. Comparing those two orders would
        # report a disagreement about ranking where no ranking happened. The
        # row stays in the timing table above, which is what it is for.
        if not query:
            continue
        ours = results(runyte_command(query), candidates, environment, RUNYTE_ANSWERED)
        theirs = results(
            fzf_command(query, options.scheme), candidates, environment, FZF_ANSWERED
        )
        measured = agreement(ours, theirs, options.top)
        rows.append([
            name,
            f"`{query}`" if query else "(empty)",
            f"{measured['ours']:,}",
            f"{measured['theirs']:,}",
            "same" if measured["same_matches"]
            else f"+{measured['only_ours']:,} / −{measured['only_theirs']:,}",
            f"{measured['shared_top']}/{min(options.top, measured['ours'], measured['theirs'])}"
            if measured["ours"] and measured["theirs"] else "—",
            "yes" if measured["same_first"] else "no",
        ])
        if not measured["same_first"] and measured["ours"] and measured["theirs"]:
            divergent.append((name, query, measured))
    print(table(
        ["query", "typed", "runyte matched", "fzf matched", "match set",
         f"top {options.top} shared", "same first"],
        rows,
        ["---", "---", "---:", "---:", "---", "---:", "---"],
    ))
    print(
        "\n*match set* compares what each program considers a match at all, "
        "independent of order: `+n / −m` is candidates only Runyte accepted "
        "and candidates only fzf accepted. The empty query is left out: "
        "neither program ranks it, so the two orders are not answers to "
        "the same question.\n"
    )

    if divergent:
        print("### Where the first result differs\n")
        for name, query, measured in divergent:
            print(f"**{name}** — `{query}`\n")
            print(table(
                ["rank", "runyte", "fzf"],
                [
                    [
                        str(index + 1),
                        f"`{measured['ours_top'][index]}`"
                        if index < len(measured["ours_top"]) else "—",
                        f"`{measured['theirs_top'][index]}`"
                        if index < len(measured["theirs_top"]) else "—",
                    ]
                    for index in range(min(5, options.top))
                ],
                ["---:", "---", "---"],
            ))
            print()
    return 0


def version() -> str:
    """The crate version the filter was built from.

    Read from the manifest rather than from `target/release/runyte --version`,
    because the benchmark builds the example and never the editor binary; a
    stale editor binary would otherwise put the wrong version on the result.
    """
    manifest = (REPO / "Cargo.toml").read_text(encoding="utf-8")
    for line in manifest.splitlines():
        if line.startswith("version = "):
            return "runyte " + line.split("=", 1)[1].strip().strip('"')
    return "runyte (version unknown)"


def fzf_version(environment: dict[str, str]) -> str:
    completed = subprocess.run(
        ["fzf", "--version"], capture_output=True, text=True, env=environment
    )
    return f"fzf {completed.stdout.strip()}" if completed.returncode == 0 else "fzf"


def machine() -> str:
    try:
        count = len(os.sched_getaffinity(0))
    except AttributeError:
        count = os.cpu_count() or 0
    return f"{os.uname().sysname} {os.uname().machine}, {count} cores"


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except FilterFailed as failure:
        print(f"\nfuzzy.py: a filter did not answer:\n{failure}", file=sys.stderr)
        raise SystemExit(1) from failure
