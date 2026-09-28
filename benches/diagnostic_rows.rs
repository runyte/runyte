// SPDX-License-Identifier: MPL-2.0

//! Publication and cursor-only frame cost as diagnostic batches grow.
//!
//! Run with `cargo bench --bench diagnostic_rows`. Sparse cases keep the
//! visible-row diagnostic count fixed while adding distinct offscreen rows.
//! The dense case adds all diagnostics to one visible row, so lookup cost can
//! grow with the number of results. Publication excludes fixture construction;
//! each frame moves the caret between columns 0 and 1 of a fixed viewport.

use std::{
    fs,
    hint::black_box,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use lsp_types::{DiagnosticSeverity, Position, Range};
use runyte::{
    app::{App, FrameGeometry},
    config::Config,
    input::KeyStroke,
    layout::Rect,
    lsp::{Diagnostic, DiagnosticStore},
};

fn diagnostic(row: u32, severity: DiagnosticSeverity) -> Diagnostic {
    Diagnostic::new(lsp_types::Diagnostic {
        range: Range::new(Position::new(row, 0), Position::new(row, 1)),
        severity: Some(severity),
        message: "diagnostic".to_owned(),
        ..Default::default()
    })
}

fn batch(additional: usize, dense: bool) -> Vec<Diagnostic> {
    let mut diagnostics = vec![
        diagnostic(2, DiagnosticSeverity::WARNING),
        diagnostic(2, DiagnosticSeverity::ERROR),
        diagnostic(17, DiagnosticSeverity::HINT),
    ];
    diagnostics.extend((0..additional).map(|index| {
        let row = if dense { 2 } else { 100 + index as u32 };
        diagnostic(row, DiagnosticSeverity::WARNING)
    }));
    diagnostics
}

fn median(mut body: impl FnMut() -> Duration) -> Duration {
    body();
    let mut samples = (0..7).map(|_| body()).collect::<Vec<_>>();
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn editor_fixture() -> (PathBuf, PathBuf, App) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "runyte-diagnostic-rows-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let path = root.join("diagnostic-rows.txt");
    // Every indexed row is valid, while only the first viewport is rendered.
    fs::write(&path, "abcdefgh\n".repeat(10_120)).unwrap();
    let path = path.canonicalize().unwrap();
    let app = App::new_in_project(Config::default(), Some(path.clone()), &root).unwrap();
    (root, path, app)
}

fn geometry() -> FrameGeometry {
    FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 40,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 38,
        },
        status: Rect {
            x: 0,
            y: 38,
            width: 120,
            height: 1,
        },
        message: Rect {
            x: 0,
            y: 39,
            width: 120,
            height: 1,
        },
    }
}

fn main() {
    let (root, path, mut app) = editor_fixture();
    // Check that every measured frame changes only the caret, never the text
    // or viewport. An even number of motions returns it to the first column.
    app.handle_key(KeyStroke::char('l')).unwrap();
    assert_eq!(app.cursor_position().col, 1);
    app.handle_key(KeyStroke::char('h')).unwrap();
    assert_eq!(app.cursor_position().col, 0);
    println!(
        "shape     added  insert ms  replace ms  40-row lookup us  40-row severity us  120x40 frame ms"
    );
    for (shape, additional, dense) in [
        ("sparse", 0, false),
        ("sparse", 100, false),
        ("sparse", 1_000, false),
        ("sparse", 10_000, false),
        ("dense", 10_000, true),
    ] {
        let diagnostics = batch(additional, dense);
        let insertion = median(|| {
            let mut store = DiagnosticStore::default();
            let input = diagnostics.clone();
            let start = Instant::now();
            store.set("rust", path.clone(), input);
            let elapsed = start.elapsed();
            black_box(store);
            elapsed
        });
        let mut replacing_store = DiagnosticStore::default();
        replacing_store.set("rust", path.clone(), diagnostics.clone());
        let replacement = median(|| {
            let input = diagnostics.clone();
            let start = Instant::now();
            replacing_store.set("rust", path.clone(), input);
            start.elapsed()
        });
        black_box(replacing_store);

        let mut store = DiagnosticStore::default();
        store.set("rust", path.clone(), diagnostics.clone());
        let lookup = median(|| {
            let start = Instant::now();
            for _ in 0..1_000 {
                for row in 0..40 {
                    black_box(store.for_row(&path, row));
                }
            }
            start.elapsed() / 1_000
        });
        let severity = median(|| {
            let start = Instant::now();
            for _ in 0..1_000 {
                for row in 0..40 {
                    black_box(store.severity_for_row(&path, row));
                }
            }
            start.elapsed() / 1_000
        });
        app.diagnostics.set("rust", path.clone(), diagnostics);
        let frame = median(|| {
            let start = Instant::now();
            for index in 0..100 {
                let motion = if index % 2 == 0 { 'l' } else { 'h' };
                app.handle_key(KeyStroke::char(motion)).unwrap();
                let prepared = app.prepare_view(geometry());
                black_box(app.snapshot(&prepared));
            }
            let elapsed = start.elapsed() / 100;
            assert_eq!(app.cursor_position().col, 0);
            elapsed
        });
        println!(
            "{shape:<6}  {additional:>6}  {:>9.3}  {:>10.3}  {:>16.3}  {:>18.3}  {:>16.3}",
            insertion.as_secs_f64() * 1_000.0,
            replacement.as_secs_f64() * 1_000.0,
            lookup.as_secs_f64() * 1_000_000.0,
            severity.as_secs_f64() * 1_000_000.0,
            frame.as_secs_f64() * 1_000.0,
        );
    }
    drop(app);
    fs::remove_dir_all(root).unwrap();
}
