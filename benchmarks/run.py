#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

"""Startup and idle benchmark for Runyte, Neovim and Helix.

Run from anywhere:

    benchmarks/run.py                     every editor found on PATH
    benchmarks/run.py --only runyte       Runyte alone, no external editors
    benchmarks/run.py --runs 9            more samples per figure
    benchmarks/run.py --no-idle           skip the idle window

Output is Markdown, shaped for `context/reference/startup-performance.md`.

Every editor runs against an empty ``XDG_CONFIG_HOME``, so no personal
configuration or plugin set is measured. Comparisons across editors are only
meaningful when each is doing the same work; the fixture table in the report
says which rows satisfy that and which do not.
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
from ptybench import measure_idle, median_startup  # noqa: E402

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
WORK = HERE / ".work"
FIXTURES = WORK / "fixtures"
EMPTY_CONFIG = WORK / "empty-config"

IDLE_FIXTURE = "medium.rs"


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
        # -i NONE keeps Neovim from reading or writing a shada file, which is
        # state rather than editor startup.
        ("neovim", [nvim, "-i", "NONE"] if nvim else None),
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


def version_of(argv: list[str]) -> str:
    try:
        out = subprocess.run(
            [argv[0], "--version"], capture_output=True, text=True, timeout=15
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
    EMPTY_CONFIG.mkdir(parents=True, exist_ok=True)
    paths = fixtures.ensure(FIXTURES)
    if not (FIXTURES / ".git").exists():
        subprocess.run(
            ["git", "init", "-q"], cwd=FIXTURES, check=True, capture_output=True
        )
    return paths


def environment() -> dict[str, str]:
    return {"XDG_CONFIG_HOME": str(EMPTY_CONFIG), "HOME": os.environ.get("HOME", str(WORK))}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", help="comma-separated subset: neovim,helix,runyte")
    parser.add_argument("--runs", type=int, default=5, help="samples per figure (default 5)")
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
    print(f"Harness {provenance}. Median of {args.runs} runs, 120x40 pty, empty config.")
    print()
    for name, argv in editors:
        print(f"- {name}: `{version_of(argv)}`")
    print()

    print("### Startup")
    print()
    print("First paint is the first byte of output; ready is when drawing goes quiet.")
    print()
    print("| Fixture | Size | " + " | ".join(f"{n} first / ready" for n, _ in editors) + " |")
    print("| --- | --- | " + " | ".join("---:" for _ in editors) + " |")
    for fixture in names:
        path = paths[fixture]
        cells = []
        for _, argv in editors:
            result = median_startup(
                argv + [fixture], env, cwd=str(FIXTURES), runs=args.runs
            )
            first, ready = result["first_paint_ms"], result["ready_ms"]
            if ready is None or result["complete"] < result["runs"]:
                cells.append("no settled frame")
            else:
                cells.append(f"{first:.0f} / {ready:.0f} ms")
        print(f"| `{fixture}` | {fixtures.describe(path)} | " + " | ".join(cells) + " |")
    print()

    if not args.no_idle:
        print(f"### Idle cost, `{IDLE_FIXTURE}` open in a Git repository, 10 s")
        print()
        print("| Editor | Idle CPU | Screen writes |")
        print("| --- | ---: | ---: |")
        for name, argv in editors:
            idle = measure_idle(argv + [IDLE_FIXTURE], env, cwd=str(FIXTURES))
            print(f"| {name} | {idle['cpu_percent']:.2f} % | {idle['writes']} |")
        print()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
