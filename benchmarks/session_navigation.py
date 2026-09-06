#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Measure persistent attachment and unchanged-screen cost with session observation.

Uses ptybench's fixed terminal geometry and capability replies. All hosts,
configuration, inventories and documents are isolated under a temporary root.
CPU covers the attached client plus persistent hosts, excluding shell children.
Linux supplies CPU ticks; other platforms report CPU unavailable.
"""
import argparse
import json
import os
from pathlib import Path
import select
import statistics
import subprocess
import tempfile
import time

from ptybench import _spawn, _reap, terminal_replies


def own_cpu_ticks(pid):
    """Linux user/system ticks for this process, excluding terminal children."""
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[-1].split()
    return int(fields[11]) + int(fields[12])


def drain(fd, duration, marker=None):
    deadline = time.monotonic() + duration
    writes = total = 0
    accumulated = b""
    while time.monotonic() < deadline:
        readable, _, _ = select.select([fd], [], [], min(.05, max(0, deadline-time.monotonic())))
        if not readable:
            continue
        data = os.read(fd, 65536)
        if not data:
            raise RuntimeError("TUI closed during observation")
        writes += 1
        total += len(data)
        reply = terminal_replies(data)
        if reply:
            os.write(fd, reply)
        accumulated = (accumulated + data)[-65536:]
        if marker and marker in accumulated:
            return writes, total
    if marker:
        raise RuntimeError("persistent attachment did not display its document marker")
    return writes, total


def command(fd, text):
    os.write(fd, b"\x1b")
    drain(fd, .08)
    os.write(fd, b":" + text.encode() + b"\r")


def wait_for_detach(pid):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        reaped, status = os.waitpid(pid, os.WNOHANG)
        if reaped:
            if status != 0:
                raise RuntimeError("benchmark client failed while detaching")
            return
        time.sleep(.01)
    raise RuntimeError("benchmark client did not detach")


def sample(binary, sessions, hidden, noisy, window):
    with tempfile.TemporaryDirectory(prefix="runyte-nav-bench-") as temporary:
        root = Path(temporary)
        env = dict(os.environ)
        env.pop("RUNYTE_PARENT_CONTEXT", None)
        for name, suffix in [("XDG_CONFIG_HOME", "config"), ("XDG_CACHE_HOME", "cache"),
                             ("XDG_STATE_HOME", "state"), ("XDG_DATA_HOME", "data"),
                             ("XDG_RUNTIME_DIR", "runtime"), ("RUNYTE_ALL_HOSTS_DIR", "inventory")]:
            path = root / suffix
            path.mkdir(mode=0o700)
            env[name] = str(path)
        config = root / "config" / "runyte"
        config.mkdir()
        (config / "config.yaml").write_text("lsp:\n  enable: false\nworkspace:\n  session_strip: " + ("hidden" if hidden else "auto") + "\n")
        projects = []
        clients = []
        try:
            for index in range(sessions):
                project = root / f"project-{index}"
                project.mkdir()
                # Keep the marker beyond the selected first character, whose
                # terminal styling splits the first rendered text run.
                (project / "navigation-marker.txt").write_text("x navbench-content-ready\n")
                projects.append(project)
                cold_start = time.monotonic()
                pid, fd = _spawn([binary, "-a", str(project)], env, str(project))
                clients.append((pid, fd))
                drain(fd, 15, b"[about]")
                cold_start_ms = (time.monotonic() - cold_start) * 1000
                drain(fd, .4)
                command(fd, "open navigation-marker.txt")
                drain(fd, .4)
                if noisy and index == sessions - 1:
                    command(fd, "terminal /bin/sh -c 'while :; do printf noise\\\\n; sleep 0.02; done'")
                    drain(fd, .5)
                    os.write(fd, b"\x1c")
                    drain(fd, .1)
                command(fd, "detach")
                wait_for_detach(pid)
                os.close(fd)
                clients.pop()
            started = time.monotonic()
            pid, fd = _spawn([binary, "-a", str(projects[0])], env, str(projects[0]))
            clients.append((pid, fd))
            drain(fd, 15, b"navbench-content-ready")
            attachment_ms = (time.monotonic() - started) * 1000
            # Auto visibility can need discovery followed by a scalar poll.
            # Allow both 15-second cycles before measuring unchanged output.
            drain(fd, 32)
            pids = {pid}
            for path in root.rglob("endpoint.json"):
                metadata = json.loads(path.read_text())
                if isinstance(metadata.get("pid"), int):
                    pids.add(metadata["pid"])
            cpu_supported = Path("/proc").is_dir()
            before = sum(own_cpu_ticks(process) for process in pids) if cpu_supported else 0
            start = time.monotonic()
            writes, byte_count = drain(fd, window)
            elapsed = time.monotonic() - start
            after = sum(own_cpu_ticks(process) for process in pids) if cpu_supported else 0
            return {"cold_start_ms": cold_start_ms, "attachment_ms": attachment_ms, "cpu_percent": (after-before)/os.sysconf("SC_CLK_TCK")/elapsed*100 if cpu_supported else None,
                    "screen_writes": writes, "screen_bytes": byte_count, "observed_processes": len(pids), "window_seconds": elapsed}
        finally:
            for pid, fd in clients:
                _reap(pid)
                os.close(fd)
            for project in projects:
                subprocess.run([binary, "--session-stop", "--force", str(project)], env=env, cwd=project,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15, check=False)


def main():
    # ptybench overlays the inherited environment when forking its child.
    os.environ.pop("RUNYTE_PARENT_CONTEXT", None)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/runyte")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--window", type=float, default=16)
    parser.add_argument("--json", type=Path, required=True)
    args = parser.parse_args()
    if args.runs < 1 or args.window < 15:
        parser.error("runs must be positive and window must span at least one 15-second observation")
    binary = str(Path(args.binary).resolve())
    results = {}
    for label, sessions, hidden, noisy in [("one", 1, False, False), ("several", 3, False, False),
                                          ("hidden", 3, True, False), ("remote-noisy", 3, False, True)]:
        results[label] = [sample(binary, sessions, hidden, noisy, args.window) for _ in range(args.runs)]
        print(label, json.dumps(results[label]), flush=True)
        args.json.write_text(json.dumps({"binary": binary, "results": results}, indent=2) + "\n")
    print("| Scenario | Host start ms (median) | Attach ms (median) | CPU % (median) | Screen writes |")
    print("| --- | ---: | ---: | ---: | --- |")
    for label, rows in results.items():
        cpu = [row["cpu_percent"] for row in rows if row["cpu_percent"] is not None]
        print(f"| {label} | {statistics.median(row['cold_start_ms'] for row in rows):.2f} | {statistics.median(row['attachment_ms'] for row in rows):.2f} | {statistics.median(cpu) if cpu else 'unavailable'} | {[row['screen_writes'] for row in rows]} |")


if __name__ == "__main__":
    main()
