#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

"""Compare actual no-change directory planning after snapshot acquisition (Unix)."""

import argparse
import hashlib
import json
import platform
import statistics
import subprocess
import tempfile
from pathlib import Path


PROGRAM = r'''
#[path = "before/mod.rs"] mod before;
#[path = "after/mod.rs"] mod after;
#[path = "@SOURCE@/hash.rs"] mod hash;
#[path = "private_storage/mod.rs"] mod private_storage;
macro_rules! measure {
    ($name:ident, $module:ident) => {
        fn $name(root: &std::path::Path, snapshot: &$module::DirectorySnapshot) -> u128 {
            let desired = snapshot.entries().iter().map(|entry| $module::DesiredEntry::existing(entry, entry.path.clone())).collect();
            let snapshot = snapshot.clone();
            let root = root.to_path_buf();
            let start = std::time::Instant::now();
            let plan = $module::FsPlan::build(root, snapshot, desired).unwrap();
            let elapsed = start.elapsed().as_nanos();
            assert!(plan.is_empty());
            std::hint::black_box(plan);
            elapsed
        }
    };
}
measure!(old, before);
measure!(new, after);
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let runs: usize = args[1].parse().unwrap();
    let base = std::path::Path::new(&args[2]);
    for length in [2048, 8192, 16384] {
        let root = base.join(length.to_string());
        std::fs::create_dir(&root).unwrap();
        for n in 0..length { std::fs::write(root.join(format!("file-{n:05}")), b"").unwrap(); }
        let before = before::DirectorySnapshot::read(&root).unwrap();
        let after = after::DirectorySnapshot::read(&root).unwrap();
        for run in 0..=runs {
            let (before, after) = if run % 2 == 0 { (old(&root, &before), new(&root, &after)) } else { let after = new(&root, &after); (old(&root, &before), after) };
            if run > 0 { println!("{length} {before} {after}"); }
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
    if platform.system() not in ["Linux", "Darwin"]:
        parser.error("this harness compiles the Unix filesystem implementation")
    root = Path(__file__).resolve().parents[1]
    dependencies = root / "target/debug/deps"
    externs = []
    for name in ["anyhow", "libc", "trash"]:
        libraries = list(dependencies.glob(f"lib{name}-*.rlib"))
        if not libraries:
            parser.error("run cargo build --locked --lib first")
        externs += ["--extern", f"{name}={max(libraries, key=lambda path: path.stat().st_mtime_ns)}"]
    revision = subprocess.check_output(
        ["git", "rev-parse", "--verify", "--end-of-options", args.baseline + "^{commit}"],
        cwd=root, text=True,
    ).strip()
    before = subprocess.check_output(["git", "show", revision + ":src/fs_plan.rs"], cwd=root)
    after = (root / "src/fs_plan.rs").read_bytes()
    samples = {case: {version: [] for version in ["before_ms", "after_ms"]}
               for case in ["2048", "8192", "16384"]}
    with tempfile.TemporaryDirectory(prefix="runyte-directory-plan-") as temporary:
        directory = Path(temporary)
        for version, source in [("before", before), ("after", after)]:
            destination = directory / version
            destination.mkdir()
            (destination / "mod.rs").write_bytes(source)
            for name in ["platform.rs", "staging.rs"]:
                child = subprocess.check_output(["git", "show", revision + ":src/fs_plan/" + name], cwd=root) if version == "before" else (root / "src/fs_plan" / name).read_bytes()
                (destination / name).write_bytes(child)
        storage = directory / "private_storage"
        storage.mkdir()
        (storage / "mod.rs").write_bytes((root / "src/private_storage.rs").read_bytes())
        (storage / "owned_file.rs").write_bytes((root / "src/private_storage/owned_file.rs").read_bytes())
        (directory / "main.rs").write_text(PROGRAM.replace("@SOURCE@", str(root / "src")))
        binary = directory / ("directory-plan-bench.exe" if platform.system() == "Windows" else "directory-plan-bench")
        subprocess.run([
            "rustc", "--edition=2024", "-O", "-A", "dead_code", str(directory / "main.rs"),
            *externs, "-L", f"dependency={dependencies}", "-o", str(binary),
        ], check=True, cwd=root)
        output = subprocess.check_output([str(binary), str(args.runs), str(directory)], text=True)
    for line in output.splitlines():
        case, old, new = line.split()
        samples[case]["before_ms"].append(int(old) / 1_000_000)
        samples[case]["after_ms"].append(int(new) / 1_000_000)
    report = {
        "baseline": revision,
        "baseline_fs_plan_sha256": hashlib.sha256(before).hexdigest(),
        "current_fs_plan_sha256": hashlib.sha256(after).hexdigest(),
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
