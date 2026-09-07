#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

"""Quit immediately after the first document frame, without a quiet/parse wait."""

import argparse
import json
import os
import statistics
import tempfile
import time
from pathlib import Path

import ptybench
import run
import startup


def measure(binary, fixture, env, *, instrumented=False):
    with tempfile.TemporaryDirectory(prefix="runyte-initial-quit-") as directory:
        event_file = Path(directory) / "events"
        environment = dict(env)
        if instrumented:
            environment["RUNYTE_BENCH_EVENTS"] = str(event_file)
        terminal = startup.Terminal()
        pid, fd = ptybench._spawn([str(binary), fixture.name], environment, str(fixture.parent))
        try:
            startup.until(fd, terminal, lambda: terminal.contains(startup.DOCUMENT_TEXT),
                          time.perf_counter() + startup.TIMEOUT, "first document frame")
            # A fresh file starts in Normal mode, so no Escape delay is needed.
            started = time.perf_counter()
            os.write(fd, b":q\r")
            while True:
                exited, status = os.waitpid(pid, os.WNOHANG)
                if exited:
                    if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
                        raise RuntimeError("editor exited unsuccessfully")
                    break
                if time.perf_counter() - started >= ptybench.QUIT_TIMEOUT_SECONDS:
                    raise TimeoutError("editor did not quit")
                try:
                    startup.pump(fd, terminal)
                except (EOFError, OSError):
                    time.sleep(0.001)
            elapsed = (time.perf_counter() - started) * 1000
            parsed = None
            if instrumented:
                milestones = event_file.read_text().splitlines()
                if not any(line.startswith("file_loaded ") for line in milestones):
                    raise ValueError("probe did not report file loading")
                parsed = any(line.startswith("syntax_ready ") for line in milestones)
            return {"quit_ms": elapsed, "syntax_completed": parsed}
        finally:
            ptybench._reap(pid)
            os.close(fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=run.positive_run_count, default=10)
    parser.add_argument("--runyte-probe", type=Path)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    binary = run.runyte_binary()
    if not binary:
        parser.error("Runyte release binary not found")
    builds = [("stock", Path(binary))]
    if args.runyte_probe:
        builds.append(("probe", args.runyte_probe.resolve()))
    fixture = run.prepare()["long.lua"]
    samples = []
    for round_index in range(args.runs + 1):
        for name, binary in builds:
            sample = measure(binary, fixture, run.environment(), instrumented=name == "probe")
            if round_index:
                samples.append(dict(build=name, round=round_index, **sample))
    if args.json:
        args.json.write_text(json.dumps(samples, indent=2) + "\n")
    for name, _ in builds:
        rows = [row for row in samples if row["build"] == name]
        values = [row["quit_ms"] for row in rows]
        completion = (f"; {sum(row['syntax_completed'] for row in rows)}/{len(rows)} "
                      "initial parses completed before exit") if name == "probe" else ""
        print(f"{name}: {statistics.median(values):.1f} "
              f"({min(values):.1f}–{max(values):.1f}) ms{completion}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
