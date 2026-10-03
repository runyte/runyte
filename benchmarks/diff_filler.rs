// SPDX-License-Identifier: MPL-2.0

//! Run after `cargo build --locked --lib`:
//! rustc --edition=2024 -O -A dead_code benchmarks/diff_filler.rs \
//!   --extern ropey=$(ls -t target/debug/deps/libropey-*.rlib | head -1) \
//!   -L dependency=target/debug/deps -o /tmp/runyte-diff-filler
//! /tmp/runyte-diff-filler
//!
//! Isolated mapping stress case, not an end-to-end scrolling benchmark.

#[path = "../src/diff.rs"]
mod diff;
#[path = "../src/diff_view.rs"]
mod diff_view;
#[path = "../src/text.rs"]
mod text;

use diff::Side;
use diff_view::{DiffSession, DiffSide};
use std::{hint::black_box, time::Instant};

fn scan(session: &DiffSession, aligned: usize) -> usize {
    // The algorithm used before the filler lookup optimization.
    (0..=aligned)
        .rev()
        .find_map(|row| session.alignment().row_at(Side::Left, row))
        .unwrap_or(0)
}

fn measure(session: &DiffSession, baseline: bool) -> u128 {
    let start = Instant::now();
    for _ in 0..100 {
        let row = if baseline {
            scan(black_box(session), black_box(1_000_000))
        } else {
            black_box(session).row_at_or_above(Side::Left, black_box(1_000_000))
        };
        assert_eq!(black_box(row), 0);
    }
    start.elapsed().as_nanos()
}

fn main() {
    let right = format!("head\n{}tail\n", "x\n".repeat(1_000_000));
    let session = DiffSession::new(
        DiffSide { pane: 0, buffer: 0 },
        DiffSide { pane: 1, buffer: 1 },
        "head\ntail\n",
        &right,
    );
    for sample in 0..6 {
        let (before, after) = if sample % 2 == 0 {
            (measure(&session, true), measure(&session, false))
        } else {
            let after = measure(&session, false);
            (measure(&session, true), after)
        };
        if sample > 0 {
            println!(
                "scan_ms={:.6} lookup_ms={:.6}",
                before as f64 / 1_000_000.0,
                after as f64 / 1_000_000.0
            );
        }
    }
}
