// SPDX-License-Identifier: MPL-2.0

//! What path completion costs on directory trees large enough to feel it.
//!
//! Both path popups do their filesystem work on the input thread, between the
//! keystroke and the redraw that answers it, so the number that matters is the
//! cost of one keystroke rather than throughput. Each row below is one
//! keystroke's worth of work: reading a directory, matching the typed prefix
//! against every name in it, and keeping the bounded best.
//!
//! Run with `cargo bench --bench path_completion`, and prefer a release build:
//! a debug build is an order of magnitude slower and says nothing about what a
//! person feels. `RUNYTE_BENCH_SCALE=n` multiplies every directory size, for
//! looking at how the cost grows.
//!
//! This is a measurement, not a limit. `tests/performance.rs` holds the
//! budgets that fail when one of these becomes pathological.

use std::{
    fs,
    hint::black_box,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use runyte::{
    app::App,
    config::Config,
    input::{KeyCode, KeyStroke, Modifiers},
};

fn scale() -> usize {
    std::env::var("RUNYTE_BENCH_SCALE")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1)
}

fn temporary(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "runyte-bench-{}-{nanos}-{name}",
        std::process::id()
    ))
}

/// A directory of `files` files and `files / 10` subdirectories.
fn wide_directory(path: &Path, files: usize) {
    fs::create_dir_all(path).unwrap();
    for index in 0..files {
        fs::write(path.join(format!("file_{index:06}.txt")), "").unwrap();
        if index % 10 == 0 {
            fs::create_dir_all(path.join(format!("dir_{index:06}"))).unwrap();
        }
    }
}

/// A chain of `depth` nested directories, each holding `files` files.
fn deep_directory(root: &Path, depth: usize, files: usize) -> PathBuf {
    let mut path = root.to_path_buf();
    for level in 0..depth {
        path = path.join(format!("level_{level:02}"));
        fs::create_dir_all(&path).unwrap();
        for index in 0..files {
            fs::write(path.join(format!("noise_{index:05}.txt")), "").unwrap();
        }
    }
    path
}

fn press(app: &mut App, character: char) {
    app.handle_key(KeyStroke::new(KeyCode::Char(character), Modifiers::NONE))
        .unwrap();
}

fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        press(app, character);
    }
}

fn editor(root: &Path) -> App {
    let active = root.join("note.txt");
    if !active.exists() {
        fs::write(&active, "").unwrap();
    }
    App::new_in_project(Config::default(), Some(active.clone()), root).unwrap()
}

/// The median of `runs` timings of `body`, warm-up discarded, so one unlucky
/// scheduling delay does not become the reported number.
fn measure(runs: usize, mut body: impl FnMut() -> Duration) -> Duration {
    body();
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        samples.push(body());
    }
    samples.sort_unstable();
    samples[samples.len() / 2]
}

/// One keystroke typed into a prepared editor, timed on its own so that
/// building the editor and typing the path leading up to it are not counted.
fn keystroke(root: &Path, prefix: &str, character: char) -> Duration {
    let mut app = editor(root);
    press(&mut app, 'i');
    type_text(&mut app, prefix);
    let start = Instant::now();
    press(&mut app, character);
    let elapsed = start.elapsed();
    black_box(app.completion.is_some());
    elapsed
}

struct Report {
    rows: Vec<(String, String, Duration)>,
}

impl Report {
    fn row(&mut self, shape: &str, operation: &str, elapsed: Duration) {
        self.rows
            .push((shape.to_owned(), operation.to_owned(), elapsed));
        let (shape, operation, elapsed) = self.rows.last().expect("the row just pushed");
        eprintln!(
            "  {shape} · {operation}: {:.3} ms",
            elapsed.as_secs_f64() * 1_000.0
        );
    }

    fn print(&self) {
        let shape_width = self
            .rows
            .iter()
            .map(|(shape, _, _)| shape.len())
            .chain([5])
            .max()
            .unwrap_or(5);
        let operation_width = self
            .rows
            .iter()
            .map(|(_, operation, _)| operation.len())
            .chain([9])
            .max()
            .unwrap_or(9);
        println!(
            "\n{:shape_width$}  {:operation_width$}  {:>12}",
            "shape", "operation", "per keystroke"
        );
        println!(
            "{}  {}  {}",
            "-".repeat(shape_width),
            "-".repeat(operation_width),
            "-".repeat(12)
        );
        for (shape, operation, elapsed) in &self.rows {
            println!(
                "{shape:shape_width$}  {operation:operation_width$}  {:>9.3} ms",
                elapsed.as_secs_f64() * 1_000.0
            );
        }
        println!();
    }
}

fn main() {
    let scale = scale();
    let mut report = Report { rows: Vec::new() };
    let root = temporary("path-completion");
    fs::create_dir_all(&root).unwrap();

    for entries in [1_000, 10_000, 100_000].map(|entries| entries * scale) {
        let shape = format!("{entries} entries");
        eprintln!("building {shape}…");
        let wide = root.join(format!("wide_{entries}"));
        wide_directory(&wide, entries);
        let base = wide.display().to_string();

        // A listing whose directory was written to moments ago is reused for
        // a short window and then read again, because its modification time
        // cannot yet vouch for it. The tree was written a moment ago; waiting
        // that window out means the warm rows below are warm because the
        // listing is kept, not because a timer happened to still be running.
        std::thread::sleep(Duration::from_secs(3));

        // Opening the popup on the slash, with nothing kept: the whole
        // directory is read, and every name matches the empty prefix.
        let opening = measure(5, || keystroke(&root, &base, '/'));
        report.row(&shape, "insert, first /", opening);

        // One more character. The prefix is answered from the directory
        // again, which is the steady-state cost of typing a path.
        // The slash that opened the popup read the directory, so this is the
        // keystroke after it rather than the first one.
        let narrowing = measure(5, || keystroke(&root, &format!("{base}/file_0999"), '9'));
        report.row(&shape, "insert, next key", narrowing);

        // The palette's first rows, again with nothing kept.
        let palette_cold = measure(5, || {
            let mut app = editor(&root);
            press(&mut app, ':');
            type_text(&mut app, &format!("open {base}/"));
            let start = Instant::now();
            black_box(app.matching_path_hints().map(|hints| hints.len()));
            start.elapsed()
        });
        report.row(&shape, "palette, first rows", palette_cold);

        // The same rows recomputed, which is what every redraw asks for.
        let mut app = editor(&root);
        press(&mut app, ':');
        type_text(&mut app, &format!("open {base}/"));
        let palette_redraw = measure(5, || {
            let start = Instant::now();
            black_box(app.matching_path_hints().map(|hints| hints.len()));
            start.elapsed()
        });
        report.row(&shape, "palette, redraw", palette_redraw);

        type_text(&mut app, "file_099999");
        let palette_narrow = measure(5, || {
            let start = Instant::now();
            black_box(app.matching_path_hints().map(|hints| hints.len()));
            start.elapsed()
        });
        report.row(&shape, "palette, one match", palette_narrow);

        fs::remove_dir_all(&wide).unwrap();
    }

    // Descending a deep tree, where every level is itself a large listing.
    let levels = 12;
    let per_level = 500 * scale;
    eprintln!("building {levels} levels of {per_level} entries…");
    let deepest = deep_directory(&root.join("deep"), levels, per_level);
    let relative = deepest.strip_prefix(&root).unwrap().display().to_string();
    let descending = measure(5, || keystroke(&root, &relative, '/'));
    report.row(
        &format!("{levels} levels x {per_level}"),
        "insert, descend",
        descending,
    );

    report.print();
    fs::remove_dir_all(&root).unwrap();
}
