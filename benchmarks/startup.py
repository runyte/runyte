#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

"""Readiness on stock editors; loaded/parsed milestones on observation builds.

See README.md for the distinct contracts and instrumentation setup. Never
substitute first output or terminal silence for an internal milestone.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import select
import statistics
import subprocess
import sys
import tempfile
import time
from datetime import date
from pathlib import Path

import fixtures
import ptybench
import run as baseline

TIMEOUT = 40.0
# One whitespace insertion keeps Lua valid and does not accumulate a burst of
# per-keystroke parsing work. Verify its position, not a substring in a gutter.
EDIT = b" "
DOCUMENT_TEXT = "local function scan_0"
CONTROL = re.compile(rb"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|P[^\x1b]*\x1b\\)")


class Terminal:
    """Decode real terminal cells, including edits made by cursor-addressed diffs."""

    def __init__(self):
        import pyte
        self.screen = pyte.Screen(ptybench.COLUMNS, ptybench.ROWS)
        self.stream = pyte.ByteStream(self.screen)
        self.tail = b""
        self.strings = b""

    def display_bytes(self, data):
        # pyte does not implement DCS/APC/PM/SOS and can paint their payloads
        # over document cells. Consume terminal control strings, including OSC,
        # before decoding display output. Preserve incomplete sequences.
        pending = self.strings + data
        display = bytearray()
        position = 0
        while position < len(pending):
            escape = pending.find(b"\x1b", position)
            if escape < 0:
                display.extend(pending[position:])
                position = len(pending)
                break
            display.extend(pending[position:escape])
            if escape + 1 == len(pending):
                position = escape
                break
            kind = pending[escape + 1:escape + 2]
            if kind not in (b"P", b"]", b"_", b"^", b"X"):
                display.extend(pending[escape:escape + 2])
                position = escape + 2
                continue
            end = pending.find(b"\x1b\\", escape + 2)
            width = 2
            if kind == b"]":
                bell = pending.find(b"\x07", escape + 2)
                if bell >= 0 and (end < 0 or bell < end):
                    end, width = bell, 1
            if end < 0:
                position = escape
                break
            position = end + width
        self.strings = pending[position:]
        if len(self.strings) > 65536:
            raise ValueError("unterminated terminal control string")
        return bytes(display)

    def feed(self, data):
        self.stream.feed(self.display_bytes(data))
        # Preserve split capability requests, but never reply twice to one.
        joined = self.tail + data
        replies = b""
        for match in CONTROL.finditer(joined):
            if match.end() > len(self.tail):
                replies += ptybench.terminal_replies(match.group())
        self.tail = joined[-256:]
        return replies

    def contains(self, text):
        return self.position(text) is not None

    def position(self, text):
        # Do not count a partially received synchronized-update frame.
        if (2026 << 5) in self.screen.mode:
            return None
        for row, line in enumerate(self.screen.display):
            column = line.find(text)
            if column >= 0:
                return row, column
        return None

    def at(self, position, text):
        row, column = position
        return ((2026 << 5) not in self.screen.mode
                and self.screen.display[row][column:column + len(text)] == text)


def pump(fd, terminal, timeout=0.005):
    if select.select([fd], [], [], max(0.0, timeout))[0]:
        data = os.read(fd, 65536)
        if not data:
            raise EOFError("editor closed the terminal")
        reply = terminal.feed(data)
        if reply:
            os.write(fd, reply)


def until(fd, terminal, predicate, deadline, observation="observation"):
    while not predicate():
        if time.perf_counter() >= deadline:
            raise TimeoutError(f"editor did not complete {observation}")
        pump(fd, terminal)


def keys(fd, terminal, sequence):
    for part, gap in sequence:
        os.write(fd, part)
        deadline = time.perf_counter() + gap
        while time.perf_counter() < deadline:
            pump(fd, terminal, min(0.005, deadline - time.perf_counter()))


def finish(pid, fd, terminal):
    keys(fd, terminal, ptybench.QUIT_SEQUENCE)
    deadline = time.perf_counter() + 10
    while time.perf_counter() < deadline:
        exited, status = os.waitpid(pid, os.WNOHANG)
        if exited:
            if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
                raise RuntimeError("editor exited unsuccessfully")
            return
        try:
            pump(fd, terminal)
        except (EOFError, OSError):
            time.sleep(0.001)
    raise TimeoutError("editor did not quit")


def read_events(path, origin_ns, syntax):
    values = {}
    for line in path.read_text().splitlines():
        name, timestamp = line.split()
        if name not in ("file_loaded", "syntax_ready") or name in values:
            raise ValueError("duplicate or unknown milestone")
        values[name] = (int(timestamp) - origin_ns) / 1e6
    if "file_loaded" not in values or values["file_loaded"] < 0:
        raise ValueError("missing or invalid file-loaded milestone")
    if syntax and ("syntax_ready" not in values
                   or values["syntax_ready"] < values["file_loaded"]):
        raise ValueError("missing or invalid syntax-ready milestone")
    if not syntax:
        values.pop("syntax_ready", None)
    return values


def measure(argv, env, fixture, *, instrumented=False):
    """One fresh process; a failed verification invalidates the whole sample."""
    with tempfile.TemporaryDirectory(prefix="runyte-startup-") as directory:
        root = Path(directory)
        saved = root / "verified.txt"
        events = root / "events"
        original = fixture.read_bytes()
        environment = dict(env)
        if instrumented:
            environment["RUNYTE_BENCH_EVENTS"] = str(events)
        terminal = Terminal()
        origin_ns = time.time_ns()
        start = time.perf_counter()
        pid, fd = ptybench._spawn(argv + [fixture.name], environment, str(fixture.parent))
        try:
            deadline = start + TIMEOUT
            until(fd, terminal, lambda: terminal.contains(DOCUMENT_TEXT),
                  deadline, "initial document display")
            if instrumented:
                syntax = fixture.suffix == ".lua"

                def complete():
                    try:
                        read_events(events, origin_ns, syntax)
                        return True
                    except (OSError, ValueError):
                        return False

                until(fd, terminal, complete, deadline, "internal milestones")
                result = read_events(events, origin_ns, syntax)
                # Internal probes use the shared wall clock. Reject clock jumps.
                drift = (time.time_ns() - origin_ns) / 1e9 - (time.perf_counter() - start)
                if abs(drift) > 0.005:
                    raise ValueError("wall clock changed during the measurement")
            else:
                # Input begins at observed document content, without a quiet wait.
                position = terminal.position(DOCUMENT_TEXT)
                os.write(fd, b"i" + EDIT)
                until(fd, terminal,
                      lambda: terminal.at(position, EDIT.decode() + DOCUMENT_TEXT),
                      deadline, "edited document display")
                result = {"ready_to_edit": (time.perf_counter() - start) * 1000}
                # Outside the timed interval, prove the marker was an actual
                # buffer edit and the rest of the complete file stayed intact.
                keys(fd, terminal, ((b"\x1b", 0.08),
                     (f":write! {saved}\r".encode(), 0.0)))

                def saved_correctly():
                    try:
                        return saved.read_bytes() == EDIT + original
                    except OSError:
                        return False

                try:
                    until(fd, terminal, saved_correctly, deadline, "saved edit verification")
                except TimeoutError as error:
                    preview = saved.read_bytes()[:100] if saved.exists() else b"<missing>"
                    raise TimeoutError(f"{error}; saved prefix {preview!r}; "
                                       f"screen tail {terminal.screen.display[-3:]!r}") from error
            finish(pid, fd, terminal)
            return result
        except TimeoutError as error:
            edges = terminal.screen.display[:2] + terminal.screen.display[-2:]
            raise TimeoutError(f"{error}; screen edges={edges!r}") from error
        finally:
            ptybench._reap(pid)
            os.close(fd)


def cell(samples, metric, runs):
    values = [sample[metric] for sample in samples if metric in sample]
    if len(values) != runs:
        return f"incomplete ({len(values)}/{runs})"
    return f"{statistics.median(values):.1f} ({min(values):.1f}–{max(values):.1f})"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=baseline.positive_run_count, default=10)
    parser.add_argument("--only", help="comma-separated editor names")
    parser.add_argument("--fixtures", default=",".join(fixtures.FIXTURES))
    parser.add_argument("--helix-probe", type=Path)
    parser.add_argument("--runyte-probe", type=Path)
    parser.add_argument("--helix-runtime", type=Path)
    parser.add_argument("--json", type=Path, help="retain all samples and binary hashes")
    args = parser.parse_args()
    editors = baseline.discover(args.only.split(",") if args.only else None)
    if not editors:
        parser.error("no editors found")
    paths = baseline.prepare()
    env = baseline.environment()
    if args.helix_runtime:
        env["HELIX_RUNTIME"] = str(args.helix_runtime.resolve())
    probes = {"neovim": None, "helix": args.helix_probe, "runyte": args.runyte_probe}
    samples = {}
    records = []
    binaries = {}
    failed = False
    print(f"## {date.today().isoformat()} — startup milestones\n", flush=True)
    print(f"{args.runs} measured runs after one discarded warm-up per cell; "
          "median (min–max), milliseconds from before PTY fork. "
          "Editor order rotates each round. 120×40 PTY, isolated home/XDG.\n", flush=True)
    print("Machine: " + subprocess.check_output(["uname", "-srmo"], text=True).strip())
    for name, argv in editors:
        version = baseline.version_of(argv, env, str(baseline.FIXTURES))
        digest = hashlib.sha256(Path(argv[0]).read_bytes()).hexdigest()
        binaries[name] = {"version": version, "sha256": digest}
        print(f"- {name}: `{version}`")
        print(f"  binary SHA-256: `{digest}`")
        if probes[name]:
            probe_digest = hashlib.sha256(probes[name].read_bytes()).hexdigest()
            binaries[name]["probe_sha256"] = probe_digest
            print(f"  probe SHA-256: `{probe_digest}`")
    for fixture_name in args.fixtures.split(","):
        fixtures.split(fixture_name)
        for mode in ("ready", "internal"):
            active = [(name, argv) for name, argv in editors
                      if mode == "ready" or name == "neovim" or probes[name]]
            for round_index in range(args.runs + 1):
                for offset in range(len(active)):
                    name, argv = active[(offset + round_index) % len(active)]
                    command = list(argv)
                    if mode == "internal":
                        if name == "neovim":
                            script = Path(__file__).with_name("neovim_milestones.lua").resolve()
                            command += ["--cmd", f"lua dofile({str(script)!r})"]
                        else:
                            command = [str(probes[name].resolve())]
                    try:
                        sample = measure(command, env, paths[fixture_name],
                                         instrumented=mode == "internal")
                    except (OSError, EOFError, TimeoutError, ValueError, RuntimeError) as error:
                        print(f"{name} {fixture_name} {mode}: {error}", file=sys.stderr, flush=True)
                        sample = {}
                        failed = True
                    if round_index:
                        samples.setdefault((fixture_name, name, mode), []).append(sample)
                        records.append(dict(fixture=fixture_name, editor=name,
                                            mode=mode, round=round_index, **sample))
    if args.json:
        args.json.write_text(json.dumps({"binaries": binaries, "samples": records}, indent=2) + "\n")
    for metric, title, mode in (("ready_to_edit", "Ready to edit (stock binaries)", "ready"),
                                ("file_loaded", "File loaded (instrumented)", "internal"),
                                ("syntax_ready", "Syntax ready (instrumented)", "internal")):
        print(f"\n### {title}\n")
        print("| Fixture | " + " | ".join(name for name, _ in editors) + " |")
        print("| --- | " + " | ".join("---:" for _ in editors) + " |")
        for fixture_name in args.fixtures.split(","):
            cells = []
            for name, _ in editors:
                if metric == "syntax_ready" and fixture_name.endswith(".txt"):
                    value = "not applicable"
                elif (fixture_name, name, mode) not in samples:
                    value = "unavailable (probe build required)"
                else:
                    value = cell(samples[fixture_name, name, mode], metric, args.runs)
                cells.append(value)
            print(f"| `{fixture_name}` | " + " | ".join(cells) + " |")
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
