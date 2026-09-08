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
import sys
from pathlib import Path

import run as harness


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, default=harness.REPO / "target/release/runyte")
    parser.add_argument("--runs", type=harness.positive_run_count, default=10)
    parser.add_argument("--idle-runs", type=harness.idle_run_count, default=3)
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
    try:
        for name, binary, enabled in [
            ("base-disabled", binaries[0], False),
            ("branch-disabled", binaries[1], False),
            ("branch-example", binaries[1], True),
        ]:
            config.write_text(example_config if enabled else baseline)
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
    finally:
        config.write_text(baseline)


if __name__ == "__main__":
    main()
