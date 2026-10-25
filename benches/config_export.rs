// SPDX-License-Identifier: MPL-2.0

//! In-process configuration load, keymap compilation and export cost.
//! Run `cargo bench --bench config_export`; timings are medians of three runs
//! after a warm-up. This measures components, not complete editor readiness.

use runyte::{
    config::Config,
    keymap::{actions::MAX_BINDINGS, configured, keymap_for},
    settings::write_config,
};
use serde_yaml::Value;
use std::{
    fs,
    hint::black_box,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "runyte-config-bench-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn section(count: usize, conflicting: bool) -> Value {
    let mut source = String::from("bind:\n  normal:\n");
    for n in 0..count {
        let index = if conflicting { n / 2 } else { n };
        let letters = format!("{index:05}")
            .bytes()
            .map(|digit| char::from(b'a' + digit - b'0').to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let suffix = if conflicting && n % 2 != 0 { " z" } else { "" };
        source.push_str(&format!("    F12 {letters}{suffix}: save\n"));
    }
    serde_yaml::from_str(&source).unwrap()
}

fn median(mut run: impl FnMut()) -> f64 {
    run();
    let mut samples = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        run();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    samples[1]
}

fn main() {
    let defaults = [keymap_for(false), keymap_for(true)];
    println!("assignments,load_ms,compile_both_ms,export_ms");
    for count in [0, 1, 1400, MAX_BINDINGS] {
        let root = Root::new();
        let path = root.0.join("config.yaml");
        let config = Config {
            keys: (count > 0).then(|| section(count, false)),
            ..Default::default()
        };
        let compile = median(|| {
            if let Some(keys) = &config.keys {
                let compiled = defaults
                    .iter()
                    .map(|base| {
                        let compiled = configured::compile(black_box(keys), base);
                        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
                        compiled
                    })
                    .collect::<Vec<_>>();
                black_box(compiled);
            }
        });
        let export = median(|| {
            black_box(write_config(&path, &config, true).unwrap());
        });
        let load = median(|| {
            black_box(Config::load(Some(&path)).unwrap());
        });
        println!("{count},{load:.3},{compile:.3},{export:.3}");
    }
    let keys = section(MAX_BINDINGS, true);
    let reject = median(|| {
        for base in &defaults {
            let compiled = configured::compile(black_box(&keys), base);
            assert_eq!(compiled.errors.len(), MAX_BINDINGS);
            black_box(compiled);
        }
    });
    println!("conflicting_{MAX_BINDINGS},compile_both_ms={reject:.3}");
}
