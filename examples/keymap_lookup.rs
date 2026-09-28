// SPDX-License-Identifier: MPL-2.0

//! Run with `cargo run --release --locked --example keymap_lookup`.

use std::{hint::black_box, time::Instant};

use runyte::{
    command::Mode,
    input::KeyStroke,
    keymap::{BindingScope, KeySequence, default_keymap},
};

fn median_lookup_ns(scope: BindingScope, sequence: &KeySequence) -> f64 {
    const SAMPLES: usize = 1_000;
    const LOOKUPS_PER_SAMPLE: usize = 1_000;

    let keymap = default_keymap();
    for _ in 0..10 * LOOKUPS_PER_SAMPLE {
        black_box(keymap.lookup_in(Mode::Normal, scope, black_box(sequence)));
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        for _ in 0..LOOKUPS_PER_SAMPLE {
            black_box(keymap.lookup_in(Mode::Normal, scope, black_box(sequence)));
        }
        samples.push(start.elapsed().as_nanos() as f64 / LOOKUPS_PER_SAMPLE as f64);
    }
    samples.sort_by(f64::total_cmp);
    samples[SAMPLES / 2]
}

fn main() {
    let sequence = KeySequence::from(KeyStroke::char('l'));
    for scope in [BindingScope::Global, BindingScope::Markdown] {
        println!(
            "{scope:?}: {:.1} ns/lookup",
            median_lookup_ns(scope, &sequence)
        );
    }
}
