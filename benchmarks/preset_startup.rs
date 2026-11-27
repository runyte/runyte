// SPDX-License-Identifier: MPL-2.0
//! Cold semantic keymap initialization, one process per measured sample.
fn main() {
    let start = std::time::Instant::now();
    std::hint::black_box(runyte::keymap::default_keymap());
    let total = start.elapsed().as_nanos();
    let [yaml, program, index] = runyte::keymap::presets::initialization_timings_ns();
    println!(
        "{{\"yaml_ns\":{yaml},\"program_ns\":{program},\"index_ns\":{index},\"total_ns\":{total}}}"
    );
}
