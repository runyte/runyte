#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

"""Startup, quit and idle benchmark for Runyte, Neovim and Helix.

Run from anywhere:

    benchmarks/run.py                     every editor found on PATH
    benchmarks/run.py --only runyte       Runyte alone, no external editors
    benchmarks/run.py --runs 20           more startup and quit samples
    benchmarks/run.py --idle-runs 7       more idle windows than the default 5
    benchmarks/run.py --no-idle           skip the idle window
    benchmarks/run.py --fixtures long.txt,long.lua

Output is Markdown, shaped for `context/reference/startup-performance.md`.

Every editor runs with isolated XDG storage and home directories, so no
personal configuration, plugin set, cache, or state is measured. Comparisons
across editors are only meaningful when each is doing the same work; the fixture
table in the report says which rows satisfy that and which do not.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from datetime import date
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import fixtures  # noqa: E402
from ptybench import median_idle, median_startup  # noqa: E402

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
WORK = HERE / ".work"
FIXTURES = WORK / "fixtures"
EMPTY_CONFIG = WORK / "empty-config"
EMPTY_CACHE = WORK / "empty-cache"
EMPTY_STATE = WORK / "empty-state"
EMPTY_DATA = WORK / "empty-data"
EMPTY_HOME = WORK / "empty-home"

IDLE_FIXTURE = "medium.lua"


def positive_run_count(value: str) -> int:
    count = int(value)
    if count < 1:
        raise argparse.ArgumentTypeError("run count must be at least 1")
    return count


def idle_run_count(value: str) -> int:
    count = int(value)
    if count < 3:
        raise argparse.ArgumentTypeError("idle run count must be at least 3")
    return count


def idle_cells(result) -> tuple[str, str]:
    if result["complete"] < result["runs"]:
        incomplete = f"incomplete ({result['complete']}/{result['runs']})"
        return incomplete, incomplete
    cpu = (
        "unavailable"
        if result["cpu_percent"] is None
        else (
            f"{result['cpu_percent']:.2f} % "
            f"({result['cpu_min']:.2f}–{result['cpu_max']:.2f})"
        )
    )
    writes = (
        f"{result['writes']:.0f} "
        f"({result['writes_min']}–{result['writes_max']})"
    )
    return cpu, writes


def first_document_output_cell(result) -> str:
    complete = result["first_document_output_complete"]
    if complete < result["runs"]:
        return f"incomplete ({complete}/{result['runs']})"
    return f"{result['first_document_output_ms']:.0f} ms"


def first_byte_cell(result) -> str:
    complete = result["first_byte_complete"]
    if complete < result["runs"]:
        return f"incomplete ({complete}/{result['runs']})"
    return f"{result['first_byte_ms']:.0f} ms"


def runyte_binary() -> str | None:
    local = REPO / "target" / "release" / "runyte"
    if local.exists():
        return str(local)
    return shutil.which("runyte")


def discover(requested: list[str] | None) -> list[tuple[str, list[str]]]:
    """Resolve editor names to argv prefixes, skipping any that are absent."""
    nvim = shutil.which("nvim")
    helix = shutil.which("hx")
    runyte = runyte_binary()
    candidates: list[tuple[str, list[str] | None]] = [
        # -n excludes swap and -i NONE excludes shada. Both are persistent
        # state rather than editor startup.
        ("neovim", [nvim, "-n", "-i", "NONE"] if nvim else None),
        ("helix", [helix] if helix else None),
        ("runyte", [runyte] if runyte else None),
    ]
    found: list[tuple[str, list[str]]] = []
    for name, argv in candidates:
        if requested and name not in requested:
            continue
        if argv is None:
            print(f"note: {name} not found, skipping", file=sys.stderr)
            continue
        found.append((name, argv))
    return found


def version_of(argv: list[str], env: dict[str, str], cwd: str) -> str:
    try:
        probe_env = os.environ.copy()
        probe_env.update(env)
        out = subprocess.run(
            [argv[0], "--version"],
            capture_output=True,
            text=True,
            timeout=15,
            env=probe_env,
            cwd=cwd,
        ).stdout.strip().splitlines()
        return out[0] if out else "unknown"
    except (OSError, subprocess.SubprocessError):
        return "unknown"


def prepare() -> dict[str, Path]:
    """Generate fixtures and make their directory a workspace every editor accepts.

    Runyte prompts for a project directory when it is opened somewhere that is
    neither a Git repository nor an existing workspace, which would block the
    harness. Initialising a repository here also makes the idle measurement
    represent the realistic case, since a Git repository is what an editor is
    normally opened inside.
    """
    for directory in (
        EMPTY_CONFIG,
        EMPTY_CACHE,
        EMPTY_STATE,
        EMPTY_DATA,
        EMPTY_HOME,
    ):
        directory.mkdir(parents=True, exist_ok=True)
    # Keep editor readiness independent of workspace permission prompts and
    # installed language servers. Syntax highlighting stays enabled.
    runyte_config = EMPTY_CONFIG / "runyte"
    runyte_config.mkdir(exist_ok=True)
    (runyte_config / "config.yaml").write_text("lsp:\n  enable: false\n")
    paths = fixtures.ensure(FIXTURES)
    if not (FIXTURES / ".git").exists():
        subprocess.run(
            ["git", "init", "-q"], cwd=FIXTURES, check=True, capture_output=True
        )
    return paths


def environment() -> dict[str, str]:
    return {
        "XDG_CONFIG_HOME": str(EMPTY_CONFIG),
        "XDG_CACHE_HOME": str(EMPTY_CACHE),
        "XDG_STATE_HOME": str(EMPTY_STATE),
        "XDG_DATA_HOME": str(EMPTY_DATA),
        "HOME": str(EMPTY_HOME),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", help="comma-separated subset: neovim,helix,runyte")
    parser.add_argument(
        "--runs",
        type=positive_run_count,
        default=10,
        help="startup and quit samples per figure (default 10)",
    )
    parser.add_argument(
        "--idle-runs",
        type=idle_run_count,
        default=5,
        help="independent idle windows per editor (default 5)",
    )
    parser.add_argument("--no-idle", action="store_true", help="skip the idle window")
    parser.add_argument(
        "--fixtures", help=f"comma-separated subset of {','.join(fixtures.FIXTURES)}"
    )
    args = parser.parse_args()

    requested = args.only.split(",") if args.only else None
    editors = discover(requested)
    if not editors:
        print("error: no editors to measure", file=sys.stderr)
        return 1

    names = args.fixtures.split(",") if args.fixtures else list(fixtures.FIXTURES)
    for name in names:
        try:
            fixtures.split(name)
        except ValueError as error:
            print(f"error: {error}", file=sys.stderr)
            return 1
    paths = prepare()
    env = environment()

    commit = subprocess.run(
        ["git", "-C", str(REPO), "rev-parse", "--short", "HEAD"],
        capture_output=True, text=True,
    ).stdout.strip() or "unknown"
    # HEAD names the harness only when the harness is actually committed. Say so
    # rather than printing a commit that does not contain the code that ran.
    modified = subprocess.run(
        ["git", "-C", str(REPO), "status", "--porcelain", "--", str(HERE)],
        capture_output=True, text=True,
    ).stdout.strip()
    provenance = f"`{commit}` with uncommitted changes under `benchmarks/`" if modified else f"`{commit}`"

    print(f"## {date.today().isoformat()}")
    print()
    print(
        f"Harness {provenance}. Startup and quit are medians of {args.runs} runs, "
        "120x40 pty, isolated home and XDG storage."
    )
    print()
    for name, argv in editors:
        print(f"- {name}: `{version_of(argv, env, str(FIXTURES))}`")
    print()

    print("### Startup: first document content emitted")
    print()
    print(
        "Elapsed time is measured from immediately before process launch until a "
        "shared token from the first document line is emitted in the raw terminal "
        "stream. This does not prove that the terminal has presented the whole "
        "screen, input is accepted, or background work is complete. A sample "
        "counts only if the process subsequently reaches terminal-output quiet."
    )
    print()
    print("| Fixture | Size | " + " | ".join(n for n, _ in editors) + " |")
    print("| --- | --- | " + " | ".join("---:" for _ in editors) + " |")
    startup_results = {}
    for fixture in names:
        path = paths[fixture]
        cells = []
        for _, argv in editors:
            result = median_startup(
                argv + [fixture],
                env,
                fixtures.DOCUMENT_MARKER,
                cwd=str(FIXTURES),
                runs=args.runs,
            )
            startup_results.setdefault(fixture, []).append(result)
            cells.append(first_document_output_cell(result))
        print(f"| `{fixture}` | {fixtures.describe(path)} | " + " | ".join(cells) + " |")
    print()

    print("#### First terminal byte (diagnostic only)")
    print()
    print(
        "This may be an invisible capability query, terminal setup, a loading "
        "presentation, or document drawing. It is not an editor-readiness metric "
        "and must not be used to rank the editors."
    )
    print()
    print("| Fixture | Size | " + " | ".join(n for n, _ in editors) + " |")
    print("| --- | --- | " + " | ".join("---:" for _ in editors) + " |")
    for fixture in names:
        cells = [first_byte_cell(result) for result in startup_results[fixture]]
        print(
            f"| `{fixture}` | {fixtures.describe(paths[fixture])} | "
            + " | ".join(cells)
            + " |"
        )
    print()

    print("### Quit")
    print()
    print(
        "Time from the final force-quit keystroke until the editor process exits; "
        "the harness's staggered-key delay is excluded."
    )
    print()
    print("| Fixture | Size | " + " | ".join(n for n, _ in editors) + " |")
    print("| --- | --- | " + " | ".join("---:" for _ in editors) + " |")
    for fixture in names:
        cells = []
        for result in startup_results[fixture]:
            quit_ms = result["quit_ms"]
            if quit_ms is None or result["quit_complete"] < result["runs"]:
                cells.append("no measured exit")
            else:
                cells.append(f"{quit_ms:.0f} ms")
        print(
            f"| `{fixture}` | {fixtures.describe(paths[fixture])} | "
            + " | ".join(cells)
            + " |"
        )
    print()

    if not args.no_idle:
        print(
            f"### Idle cost, `{IDLE_FIXTURE}` open in a Git repository, "
            f"median of {args.idle_runs} independent 10 s windows"
        )
        print()
        print("| Editor | Idle CPU median (range) | Screen writes median (range) |")
        print("| --- | ---: | ---: |")
        for name, argv in editors:
            idle = median_idle(
                argv + [IDLE_FIXTURE],
                env,
                cwd=str(FIXTURES),
                runs=args.idle_runs,
            )
            cpu, writes = idle_cells(idle)
            print(f"| {name} | {cpu} | {writes} |")
        print()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
