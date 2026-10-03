#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

"""Compare actual URL inference on growing unmatched punctuation suffixes."""

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
fn main() {
    let runs: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let cases = [4096, 16384, 65536].into_iter().map(|length| (
        format!("suffix_{length}"), format!("https://example.test/{}", ")".repeat(length)), 1, Some("https://example.test/")
    )).chain([128, 512, 2048].into_iter().map(|length| {
        let link = format!("{}{}", "https://a,".repeat(length), ")".repeat(16));
        let caret = link.len() - 1;
        (format!("prefixes_{length}"), link, caret, None)
    }));
    for (case, link, caret, expected) in cases {
        for run in 0..=runs {
            let old = || { let start = std::time::Instant::now(); let result = before::under_cursor(std::hint::black_box(&link), caret); (start.elapsed().as_nanos(), result) };
            let new = || { let start = std::time::Instant::now(); let result = after::under_cursor(std::hint::black_box(&link), caret); (start.elapsed().as_nanos(), result) };
            let (before, after) = if run % 2 == 0 { (old(), new()) } else { let after = new(); (old(), after) };
            assert_eq!(before.1, after.1);
            assert_eq!(after.1.as_deref(), expected);
            if run > 0 { println!("{case} {} {}", before.0, after.0); }
        }
    }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", default="1778900", help="Git revision before optimization")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    root = Path(__file__).resolve().parents[1]
    revision = subprocess.check_output(
        ["git", "rev-parse", "--verify", "--end-of-options", args.baseline + "^{commit}"],
        cwd=root, text=True,
    ).strip()
    before = subprocess.check_output(["git", "show", revision + ":src/navigation_target.rs"], cwd=root)
    after = (root / "src/navigation_target.rs").read_bytes()
    samples = {case: {version: [] for version in ["before_ms", "after_ms"]}
               for case in ["suffix_4096", "suffix_16384", "suffix_65536", "prefixes_128", "prefixes_512", "prefixes_2048"]}
    with tempfile.TemporaryDirectory(prefix="runyte-navigation-") as temporary:
        directory = Path(temporary)
        (directory / "before.rs").write_bytes(before)
        (directory / "after.rs").write_bytes(after)
        (directory / "main.rs").write_text(PROGRAM)
        binary = directory / ("navigation-bench.exe" if platform.system() == "Windows" else "navigation-bench")
        subprocess.run([
            "rustc", "--edition=2024", "-O", "-A", "dead_code", str(directory / "main.rs"),
            "-o", str(binary),
        ], check=True, cwd=root)
        output = subprocess.check_output([str(binary), str(args.runs)], text=True)
    for line in output.splitlines():
        case, old, new = line.split()
        samples[case]["before_ms"].append(int(old) / 1_000_000)
        samples[case]["after_ms"].append(int(new) / 1_000_000)
    report = {
        "baseline": revision,
        "baseline_navigation_sha256": hashlib.sha256(before).hexdigest(),
        "current_navigation_sha256": hashlib.sha256(after).hexdigest(),
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
