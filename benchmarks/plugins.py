#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

"""Compare release startup/idle before plugins, with plugins off, and with the example.

Uses the existing first-document-output and descendant-aware idle measurements.
Run after builds and tests have finished, with a separately retained base binary:

    python3 benchmarks/plugins.py --before /path/to/base/runyte
"""

import argparse
import json
import os
import select
import time
import sys
from pathlib import Path

import run as harness
from ptybench import terminal_replies, COLUMNS, ROWS


def prepare_task_view(_pid, fd, initial):
    """Require an actual view, then settle before measuring its idle window."""
    import pyte
    screen = pyte.Screen(COLUMNS, ROWS)
    stream = pyte.Stream(screen)
    stream.feed(initial.decode("utf-8", errors="replace"))
    try:
        os.write(fd, b":plugin.tasks.open")
    except OSError:
        return False
    started = time.monotonic()
    shown = None
    submitted = False
    last_output = started
    while time.monotonic() - started < 10:
        if shown is not None and time.monotonic() - shown >= 2.5:
            return True
        readable, _, _ = select.select([fd], [], [], 0.05)
        if not readable:
            # Palette acceptance is tied to the displayed query. Submit only
            # after it has been rendered, as a person using the command would.
            if (not submitted and time.monotonic() - last_output >= 0.1
                    and ":plugin.tasks.open" in "\n".join(screen.display)):
                try:
                    os.write(fd, b"\r")
                except OSError:
                    return False
                submitted = True
            continue
        try:
            data = os.read(fd, 65536)
        except OSError:
            return False
        if not data:
            return False
        stream.feed(data.decode("utf-8", errors="replace"))
        last_output = time.monotonic()
        reply = terminal_replies(data)
        if reply:
            try:
                os.write(fd, reply)
            except OSError:
                return False
        if shown is None and "Read the plugin guide" in "\n".join(screen.display):
            shown = time.monotonic()
    return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, default=harness.REPO / "target/release/runyte")
    parser.add_argument("--runs", type=harness.positive_run_count, default=10)
    parser.add_argument("--idle-runs", type=harness.idle_run_count, default=3)
    parser.add_argument("--applications", action="store_true", help="also measure quiescent epoch 2 and a visible native task list")
    options = parser.parse_args()
    binaries = [options.before.resolve(), options.after.resolve()]
    for binary in binaries:
        if not binary.is_file():
            parser.error(f"missing release binary: {binary}")
    os.environ.pop("RUNYTE_PARENT_CONTEXT", None)
    harness.prepare()
    environment = harness.environment()
    config = harness.EMPTY_CONFIG / "runyte/config.yaml"
    baseline = "lsp:\n  enable: false\n"
    # JSON strings are valid YAML scalars, including paths containing spaces.
    example_config = (
        baseline + "plugins:\n  - id: case\n    enabled: true\n"
        f"    executable: {json.dumps(sys.executable)}\n"
        f"    args: [{json.dumps(str(harness.REPO / 'docs/plugins/uppercase.py'))}]\n"
    )
    def application_config(plugin_id, script, capability):
        return (baseline + f"plugins:\n  - id: {plugin_id}\n    enabled: true\n"
                "    api: runyte-experimental-2\n"
                f"    capabilities: [{capability}]\n"
                f"    executable: {json.dumps(sys.executable)}\n"
                f"    args: [{json.dumps(str(harness.REPO / 'docs/plugins' / script))}]\n")
    cases = [
        ("base-disabled", binaries[0], baseline),
        ("branch-disabled", binaries[1], baseline),
        ("branch-example", binaries[1], example_config),
    ]
    if options.applications:
        cases.append(("branch-applications-quiescent", binaries[1], application_config("jobs", "jobs.py", "jobs")))
    try:
        for name, binary, contents in cases:
            config.write_text(contents)
            for fixture in ["short.txt", "medium.lua", "long.lua"]:
                result = harness.median_startup(
                    [str(binary), fixture], environment, harness.fixtures.DOCUMENT_MARKER,
                    cwd=str(harness.FIXTURES), runs=options.runs,
                )
                print(name, fixture, json.dumps(result), flush=True)
            result = harness.median_idle(
                [str(binary), "medium.lua"], environment,
                cwd=str(harness.FIXTURES), runs=options.idle_runs,
            )
            print(name, "idle", json.dumps(result), flush=True)
        if options.applications:
            config.write_text(application_config("tasks", "tasks.py", "views"))
            result = harness.median_idle(
                [str(binaries[1]), "medium.lua"], environment,
                cwd=str(harness.FIXTURES), runs=options.idle_runs,
                prepare=prepare_task_view,
            )
            print("branch-native-view", "idle", json.dumps(result), flush=True)
    finally:
        config.write_text(baseline)


if __name__ == "__main__":
    main()
