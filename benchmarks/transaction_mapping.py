#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

"""Compare actual transaction mapping across growing cursor counts."""

import argparse
import hashlib
import json
import platform
import statistics
import subprocess
import tempfile
from pathlib import Path


PROGRAM = r'''
#[path = "before.rs"] mod before;
#[path = "after.rs"] mod after;
macro_rules! measure {
    ($name:ident, $module:ident) => {
        fn $name(count: usize) -> u128 {
            let transaction = $module::Transaction::new((0..count)
                .map(|i| $module::Change::new(i * 2, i * 2, "α")).collect());
            let start = std::time::Instant::now();
            for i in 0..count {
                let offset = std::hint::black_box(i * 2);
                assert_eq!(transaction.map_offset(offset, $module::Assoc::After), i * 3 + 1);
                assert_eq!(transaction.map_offset(offset, $module::Assoc::Before), i * 3);
            }
            start.elapsed().as_nanos()
        }
    };
}
measure!(old, before);
measure!(new, after);
fn main() {
    let runs: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    for count in [2000, 10000, 20000] {
        for run in 0..=runs {
            let (before, after) = if run % 2 == 0 { (old(count), new(count)) }
                else { let after = new(count); (old(count), after) };
            if run > 0 { println!("{count} {before} {after}"); }
        }
    }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", default="02c1338", help="Git revision before optimization")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    root = Path(__file__).resolve().parents[1]
    dependencies = root / "target/debug/deps"
    libraries = list(dependencies.glob("libropey-*.rlib"))
    if not libraries:
        parser.error("run cargo build --locked --lib first")
    ropey = max(libraries, key=lambda path: path.stat().st_mtime_ns)
    revision = subprocess.check_output(
        ["git", "rev-parse", "--verify", "--end-of-options", args.baseline + "^{commit}"],
        cwd=root, text=True,
    ).strip()
    before = subprocess.check_output(["git", "show", revision + ":src/text.rs"], cwd=root)
    after = (root / "src/text.rs").read_bytes()
    samples = {str(count): {name: [] for name in ["before_ms", "after_ms"]}
               for count in [2000, 10000, 20000]}
    with tempfile.TemporaryDirectory(prefix="runyte-offsets-") as temporary:
        directory = Path(temporary)
        (directory / "before.rs").write_bytes(before)
        (directory / "after.rs").write_bytes(after)
        (directory / "main.rs").write_text(PROGRAM)
        binary = directory / ("offsets.exe" if platform.system() == "Windows" else "offsets")
        subprocess.run([
            "rustc", "--edition=2024", "-O", "-A", "dead_code", str(directory / "main.rs"),
            "--extern", f"ropey={ropey}", "-L", f"dependency={dependencies}", "-o", str(binary),
        ], check=True, cwd=root)
        output = subprocess.check_output([str(binary), str(args.runs)], text=True)
    for line in output.splitlines():
        case, old, new = line.split()
        samples[case]["before_ms"].append(int(old) / 1_000_000)
        samples[case]["after_ms"].append(int(new) / 1_000_000)
    report = {
        "baseline": revision,
        "baseline_text_sha256": hashlib.sha256(before).hexdigest(),
        "current_text_sha256": hashlib.sha256(after).hexdigest(),
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "compiler_flags": ["--edition=2024", "-O", "-A", "dead_code"],
        "dependency_profile": "debug",
        "platform": platform.platform(),
        "samples": samples,
    }
    for case, versions in samples.items():
        print(case + ": " + "; ".join(
            f"{name}={statistics.median(values):.6f} ({min(values):.6f}–{max(values):.6f})"
            for name, values in versions.items()
        ))
    if args.json:
        args.json.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
