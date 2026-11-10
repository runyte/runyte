#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Settled editing latency to decoded PTY cells, not GPU/pixel presentation.

Each mode owns a fresh isolated workspace. Samples are repeated space/backspace
pairs in one process, not independent launches. Timing includes Python decoding.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

import editions
import ptybench
import run
import session_navigation
import startup

MARKER = "interaction latency marker"
SIDE_MARKER = "unchanged companion pane"
SETTLE = 0.2


def fixture_text():
    return MARKER + "\n" + "".join(
        f"line {index:04d}: repeated editing fixture\n" for index in range(1, 100))


def summary(values):
    if any(not math.isfinite(value) or value < 0 for value in values):
        raise ValueError("samples must be finite nonnegative milliseconds")
    ordered = sorted(values)
    if not ordered:
        raise ValueError("no samples")
    # Nearest-rank empirical percentiles; with 40 observations p99 is the max.
    return {name: ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]
            for name, fraction in (("p50", .5), ("p95", .95), ("p99", .99), ("max", 1))}


def settle(fd, terminal):
    end = time.perf_counter() + SETTLE
    while time.perf_counter() < end:
        startup.pump(fd, terminal, min(.005, max(0, end - time.perf_counter())))


def command(fd, terminal, text):
    startup.keys(fd, terminal, ((b"\x1b", .08),
                               ((":" + text + "\r").encode(), .0)))


def verify_save(fd, terminal, path, expected):
    command(fd, terminal, f"write! {path}")

    def correct():
        return path.exists() and path.read_bytes() == expected

    startup.until(fd, terminal, correct, time.perf_counter() + 10, "exact saved contents")
    settle(fd, terminal)


def frame_matches(terminal, position, expected, companion):
    # Fixed positions prevent a gutter space or shifted marker from passing.
    # Terminal.at also rejects an unfinished synchronized-update frame.
    return (terminal.at(position, expected)
            and terminal.at(companion, SIDE_MARKER))


def measure(binary, mode, runs):
    with tempfile.TemporaryDirectory(prefix="runyte-interaction-pty-") as temporary:
        root = Path(temporary)
        env = session_navigation.environment(root)
        selectors = {"RUNYTE_ALL_HOSTS_DIR", "RUNYTE_TEST_STATE_HOME",
                     "RUNYTE_TEST_ACCOUNT_HOME", "RUNYTE_TEST_LOCAL_APP_DATA",
                     "RUNYTE_TEST_SOCKET_HOME", "RUNYTE_CONTEXT_HOME"}
        env = {key: value for key, value in env.items()
               if not key.startswith("RUNYTE_") or key in selectors}
        # The socket namespace is a private storage reservation; it cannot be
        # an ancestor of the fixture workspace's .runyte directory.
        socket_home = root / "sockets"
        socket_home.mkdir(mode=0o700)
        env["RUNYTE_TEST_SOCKET_HOME"] = str(socket_home)
        project = root / "project"
        project.mkdir()
        subprocess.run(["git", "init", "-q", str(project)], env=env, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        original = fixture_text().encode()
        fixture = project / "interaction.txt"
        fixture.write_bytes(original)
        side = project / "side.txt"
        side_original = fixture_text().replace(MARKER, SIDE_MARKER, 1)
        side.write_text(side_original)
        config = root / "config" / "runyte" / "config.yaml"
        config.parent.mkdir()
        config.write_text("lsp:\n  enable: false\nworkspace:\n  session_strip: hidden\n")
        terminal = startup.Terminal()
        argv = [binary, "--config", str(config)] + editions.launch_arguments(
            "terminal", mode, fixture.name)
        pid, fd = ptybench._spawn(argv, env, str(project), exact_environment=True)
        try:
            deadline = time.perf_counter() + 40
            if mode == "mux":
                editions.terminal_open(fd, terminal, fixture.name, deadline)
            startup.until(fd, terminal, lambda: terminal.contains(MARKER), deadline, "document")
            command(fd, terminal, "vsplit side.txt")
            startup.until(fd, terminal, lambda: terminal.contains(SIDE_MARKER), deadline, "second pane")
            startup.keys(fd, terminal, ((b" wh", .2), (b"gg0i", .2)))
            position = terminal.position(MARKER)
            companion = terminal.position(SIDE_MARKER)
            if position is None or companion is None or position[1] >= companion[1]:
                raise RuntimeError("expected document on left and companion on right")
            # Verify a real intermediate edit before sampling, then restore the
            # saved baseline. Saving between timed actions would change which
            # action pays same-content dirty checking and rename the buffer.
            os.write(fd, b" ")
            startup.until(fd, terminal,
                          lambda: frame_matches(terminal, position, " " + MARKER, companion),
                          time.perf_counter() + 10, "warmup inserted frame")
            verify_save(fd, terminal, project / "intermediate.txt", b" " + original)
            startup.keys(fd, terminal, ((b"gg0li\x7f", .2),))
            startup.until(fd, terminal,
                          lambda: frame_matches(terminal, position, MARKER + " ", companion),
                          time.perf_counter() + 10, "warmup restored frame")
            verify_save(fd, terminal, fixture, original)
            startup.keys(fd, terminal, ((b"gg0i", .2),))
            samples = {"insert": [], "backspace": []}
            for _ in range(runs):
                for name, key, expected in (("insert", b" ", " " + MARKER),
                                            ("backspace", b"\x7f", MARKER + " ")):
                    settle(fd, terminal)
                    before = MARKER + " " if name == "insert" else " " + MARKER
                    if not frame_matches(terminal, position, before, companion):
                        raise RuntimeError("unexpected pre-input frame for " + name)
                    started = time.perf_counter()
                    os.write(fd, key)
                    startup.until(fd, terminal,
                                  lambda: frame_matches(terminal, position, expected, companion),
                                  started + 10, name + " complete decoded frame")
                    samples[name].append((time.perf_counter() - started) * 1000)
            verify_save(fd, terminal, project / "final.txt", original)
            if side.read_text() != side_original:
                raise RuntimeError("companion file changed")
            return {"raw_ms": samples, "summary_ms": {k: summary(v) for k, v in samples.items()},
                    "verified_intermediate_and_final": True}
        except (OSError, EOFError, TimeoutError) as error:
            # Only this synthetic fixture's decoded screen is retained, bounded
            # independently of any diagnostic output the child might produce.
            lines = terminal.screen.display
            visible = "\n".join(line.rstrip() for line in lines[:6] + lines[-3:]).strip()
            raise RuntimeError(f"{mode} interaction failed: {error}; "
                               f"decoded screen tail: {visible[-4096:]!r}") from error
        finally:
            active_error = sys.exception()
            ptybench._reap(pid)
            os.close(fd)
            if mode == "mux":
                stopped = subprocess.run([binary, "--session-stop", "--force", str(project)],
                                         env=env, cwd=project, stdout=subprocess.DEVNULL,
                                         stderr=subprocess.PIPE, timeout=20)
                if stopped.returncode:
                    detail = "host cleanup failed: " + stopped.stderr[-2048:].decode(errors="replace")
                    if active_error is not None:
                        active_error.add_note(detail)
                    else:
                        raise RuntimeError(detail)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/runyte"))
    parser.add_argument("--runs", type=run.positive_run_count, default=40)
    parser.add_argument("--modes", default="ide,mux", help="ordered modes for alternating runs")
    parser.add_argument("--max-busy-cores", type=float, default=1.0)
    parser.add_argument("--quiet-timeout", type=float, default=60.0)
    parser.add_argument("--json", type=Path, required=True)
    args = parser.parse_args()
    modes = args.modes.split(',')
    if set(modes) - {'ide', 'mux'} or len(set(modes)) != len(modes):
        parser.error('modes must be a nonrepeating list of ide,mux')
    if (not math.isfinite(args.max_busy_cores) or args.max_busy_cores <= 0
            or not math.isfinite(args.quiet_timeout) or args.quiet_timeout <= 0):
        parser.error("quiet-check limits must be finite and positive")
    binary = str(args.binary.resolve(strict=True))
    report = {"metric": "input write to exact completed decoded PTY frame; includes Python decoding; not GPU/pixel latency",
              "binary_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
              "runs_per_action_per_mode": args.runs, "settle_seconds": SETTLE,
              "geometry": [ptybench.COLUMNS, ptybench.ROWS],
              "percentiles": "nearest rank; repeated actions within one process per mode",
              "complete": False, "modes": {}}
    args.json.parent.mkdir(parents=True, exist_ok=True)
    try:
        for mode in modes:
            quiet = editions.wait_until_quiet(args.max_busy_cores, args.quiet_timeout)
            if not quiet['quiet']:
                raise RuntimeError(f"Machine did not become quiet: {quiet}")
            report['modes'][mode] = {'quiet_before': quiet, **measure(binary, mode, args.runs)}
        report['complete'] = True
    finally:
        args.json.write_text(json.dumps(report, indent=2) + "\n")
    for mode, result in report["modes"].items():
        print(mode, json.dumps(result["summary_ms"]))


if __name__ == "__main__":
    main()
