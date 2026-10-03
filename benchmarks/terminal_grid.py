#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

"""Compare actual Grid implementations on extreme supported terminal shapes."""

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
        fn $name(characters: bool) -> (u128, String) {
            let (columns, rows) = if characters { (32768, 1) } else { (1, 32768) };
            let mut grid = $module::Grid::new(columns, rows, false);
            let pen = $module::Pen::default();
            for _ in 0..32768 { grid.write('x', pen, true); }
            grid.move_to(0, 0);
            let start = std::time::Instant::now();
            if characters {
                for _ in 0..4 {
                    grid.insert_characters(16000, pen);
                    grid.delete_characters(16000, pen);
                    std::hint::black_box(&grid);
                }
            } else {
                grid.scroll_up(16000, pen);
                grid.scroll_down(16000, pen);
                grid.insert_lines(16000, pen);
                grid.delete_lines(16000, pen);
                std::hint::black_box(&grid);
            }
            (start.elapsed().as_nanos(), grid.plain_text())
        }
    };
}
measure!(old, before);
measure!(new, after);

fn main() {
    let runs: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    for characters in [true, false] {
        // Discard one warm-up, alternate measurement order, and compare the
        // complete final text outside the measured intervals on every sample.
        for run in 0..=runs {
            let (before, after) = if run % 2 == 0 {
                (old(characters), new(characters))
            } else {
                let after = new(characters);
                (old(characters), after)
            };
            assert_eq!(before.1, after.1);
            if run > 0 {
                println!("{} {} {}", if characters { "characters" } else { "rows" }, before.0, after.0);
            }
        }
    }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", default="d26624a^", help="Git revision before optimization")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    root = Path(__file__).resolve().parents[1]
    dependencies = root / "target/debug/deps"
    libraries = list(dependencies.glob("libunicode_width-*.rlib"))
    if not libraries:
        parser.error("run cargo build --locked --lib first (target/debug/deps is required)")
    library = max(libraries, key=lambda path: path.stat().st_mtime_ns)
    revision = subprocess.check_output(
        ["git", "rev-parse", "--verify", "--end-of-options", args.baseline + "^{commit}"],
        cwd=root, text=True,
    ).strip()
    before = subprocess.check_output(["git", "show", revision + ":src/terminal/grid.rs"], cwd=root)
    after = (root / "src/terminal/grid.rs").read_bytes()
    samples = {case: {version: [] for version in ["before_ms", "after_ms"]}
               for case in ["characters", "rows"]}
    with tempfile.TemporaryDirectory(prefix="runyte-terminal-grid-") as temporary:
        directory = Path(temporary)
        (directory / "before.rs").write_bytes(before)
        (directory / "after.rs").write_bytes(after)
        (directory / "main.rs").write_text(PROGRAM)
        binary = directory / ("grid-bench.exe" if platform.system() == "Windows" else "grid-bench")
        subprocess.run([
            "rustc", "--edition=2024", "-O", "-A", "dead_code", str(directory / "main.rs"),
            "--extern", f"unicode_width={library}", "-L", f"dependency={dependencies}",
            "-o", str(binary),
        ], check=True, cwd=root)
        output = subprocess.check_output([str(binary), str(args.runs)], text=True)
    for line in output.splitlines():
        case, old, new = line.split()
        samples[case]["before_ms"].append(int(old) / 1_000_000)
        samples[case]["after_ms"].append(int(new) / 1_000_000)
    report = {
        "baseline": revision,
        "baseline_grid_sha256": hashlib.sha256(before).hexdigest(),
        "current_grid_sha256": hashlib.sha256(after).hexdigest(),
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "compiler_flags": ["--edition=2024", "-O", "-A", "dead_code"],
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
