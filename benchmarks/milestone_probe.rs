// SPDX-License-Identifier: MPL-2.0

// Included only in disposable measurement builds, never in shipped editors.
pub fn mark(name: &str) {
    use std::{io::Write, time::SystemTime};
    let at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("benchmark clock precedes Unix epoch")
        .as_nanos();
    let Ok(path) = std::env::var("RUNYTE_BENCH_EVENTS") else {
        return;
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open benchmark event file");
    writeln!(file, "{name} {at}").expect("write benchmark event");
}
