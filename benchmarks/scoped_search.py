#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

"""Compare actual scoped-search functions across growing selection counts."""

import argparse
import hashlib
import json
import platform
import statistics
import subprocess
import tempfile
from pathlib import Path


PROGRAM = r'''
#[path = "text.rs"] mod text;
#[path = "selection.rs"] mod selection;
#[path = "before.rs"] mod before;
#[path = "after.rs"] mod after;
fn main() {
    let runs: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    for count in [2000, 10000, 20000] {
        let text = "aa ".repeat(count);
        let spans: Vec<_> = (0..count).map(|i| (3 * i, 3 * i + 2)).collect();
        for run in 0..=runs {
            let old = || { let start = std::time::Instant::now(); let answer = before::search(std::hint::black_box(&text), &spans); (start.elapsed().as_nanos(), answer) };
            let new = || { let start = std::time::Instant::now(); let answer = after::search(std::hint::black_box(&text), &spans); (start.elapsed().as_nanos(), answer) };
            let (before, after) = if run % 2 == 0 { (old(), new()) }
                else { let after = new(); (old(), after) };
            assert_eq!(before.1, after.1);
            assert_eq!(after.1.len(), count * 2);
            if run > 0 { println!("{count} {} {}", before.0, after.0); }
        }
    }
}
'''


def search_source(source):
    mode_start = source.index("pub enum SearchMode {")
    mode = source[mode_start:source.index("/// The committed search", mode_start)]
    start = source.index("fn text_matches(")
    function = source[start:source.index("/// The character spans", start)]
    return (
        "use regex::{Regex, RegexBuilder};\n"
        "use crate::{selection::Range, text::Offset};\n" + mode + function
        + "pub fn search(text: &str, spans: &[(Offset, Offset)]) -> Vec<Range> {\n"
        + 'text_matches(text, "a", SearchMode::Sensitive, Some(spans)).unwrap()\n}\n'
    ).replace("    #[default]\n", "")


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
    externs = []
    for name in ["ropey", "regex"]:
        libraries = list(dependencies.glob(f"lib{name}-*.rlib"))
        if not libraries:
            parser.error("run cargo build --locked --lib first")
        library = max(libraries, key=lambda path: path.stat().st_mtime_ns)
        externs += ["--extern", f"{name}={library}"]
    revision = subprocess.check_output(
        ["git", "rev-parse", "--verify", "--end-of-options", args.baseline + "^{commit}"],
        cwd=root, text=True,
    ).strip()
    before = search_source(subprocess.check_output(
        ["git", "show", revision + ":src/app.rs"], cwd=root, text=True,
    ))
    after = search_source((root / "src/app.rs").read_text())
    samples = {str(count): {name: [] for name in ["before_ms", "after_ms"]}
               for count in [2000, 10000, 20000]}
    with tempfile.TemporaryDirectory(prefix="runyte-scoped-search-") as temporary:
        directory = Path(temporary)
        (directory / "before.rs").write_text(before)
        (directory / "after.rs").write_text(after)
        for name in ["text.rs", "selection.rs"]:
            (directory / name).write_bytes((root / "src" / name).read_bytes())
        (directory / "main.rs").write_text(PROGRAM)
        binary = directory / ("search.exe" if platform.system() == "Windows" else "search")
        subprocess.run([
            "rustc", "--edition=2024", "-O", "-A", "dead_code", str(directory / "main.rs"),
            *externs, "-L", f"dependency={dependencies}", "-o", str(binary),
        ], check=True, cwd=root)
        output = subprocess.check_output([str(binary), str(args.runs)], text=True)
    for line in output.splitlines():
        case, old, new = line.split()
        samples[case]["before_ms"].append(int(old) / 1_000_000)
        samples[case]["after_ms"].append(int(new) / 1_000_000)
    report = {
        "baseline": revision,
        "baseline_search_sha256": hashlib.sha256(before.encode()).hexdigest(),
        "current_search_sha256": hashlib.sha256(after.encode()).hexdigest(),
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
