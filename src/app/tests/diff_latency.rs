// SPDX-License-Identifier: MPL-2.0

//! Manual, complete edit-to-prepared-frame measurements for large diffs.

use super::*;
use crate::git::BaseContent;
use std::time::{Duration, Instant};

fn frame(app: &mut App) {
    app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 100,
            height: 28,
            ..Rect::default()
        },
        editor: Rect {
            width: 100,
            height: 26,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    });
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

#[test]
#[ignore = "manual timing, run with --nocapture"]
fn large_diff_edit_to_frame_latency() {
    let base = (0..50_000)
        .map(|row| format!("local function item_{row}(x) return x + {row} end\n"))
        .collect::<String>();
    let directory = temporary("diff-frame-latency");
    fs::create_dir_all(&directory).unwrap();
    for (name, comparison) in [("gutter", false), ("pair", true)] {
        let path = directory.join(format!("{name}.txt"));
        fs::write(&path, &base).unwrap();
        let mut app =
            App::new_in_project(Config::default(), Some(path.clone()), &directory).unwrap();
        if comparison {
            let other = directory.join("other.txt");
            fs::write(&other, format!("  {base}")).unwrap();
            frame(&mut app);
            app.execute_command("diff-this").unwrap();
            app.open_file(other).unwrap();
            app.execute_command("diff-this").unwrap();
            let left = app.diffs[0].side(Side::Left).buffer;
            assert!(app.apply_to_buffer(left, &Transaction::insert(0, " ")));
        } else {
            app.git
                .apply_staged_content(path, BaseContent::Text(base.clone()));
            let buffer = app.active().buffer;
            assert!(app.apply_to_buffer(buffer, &Transaction::insert(0, " ")));
        }
        frame(&mut app);
        std::thread::sleep(Duration::from_millis(200));
        frame(&mut app);
        let buffer = if comparison {
            app.diffs[0].side(Side::Left).buffer
        } else {
            app.active().buffer
        };
        let at = app.buffers[buffer]
            .text()
            .offset_of(Position::new(25_000, 0));
        for (label, inserted) in [("character", "x"), ("newline", "\n")] {
            let mut samples = Vec::new();
            let mut edits = Vec::new();
            let mut frames = Vec::new();
            for i in 0..40 {
                let transaction = if i % 2 == 0 {
                    Transaction::insert(at, inserted)
                } else {
                    Transaction::delete(at, at + 1)
                };
                let start = Instant::now();
                assert!(app.apply_to_buffer(buffer, &transaction));
                let after_edit = Instant::now();
                frame(&mut app);
                samples.push(start.elapsed());
                edits.push(after_edit.duration_since(start));
                frames.push(after_edit.elapsed());
            }
            eprintln!(
                "{name} {label}: total {:?}, edit {:?}, frame {:?}",
                median(samples),
                median(edits),
                median(frames)
            );
        }
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "manual timing, run with --nocapture"]
fn large_diff_history_to_frame_latency() {
    let base = (0..50_000)
        .map(|row| format!("local function item_{row}(x) return x + {row} end\n"))
        .collect::<String>();
    let directory = temporary("diff-history-latency");
    fs::create_dir_all(&directory).unwrap();
    for (state, prior_dirty) in [("clean", false), ("already-dirty", true)] {
        for (label, inserted) in [("character", "x"), ("newline", "\n")] {
            let left = directory.join(format!("{state}-{label}-left.txt"));
            let right = directory.join(format!("{state}-{label}-right.txt"));
            fs::write(&left, &base).unwrap();
            fs::write(&right, format!("  {base}")).unwrap();
            let mut app = App::new_in_project(Config::default(), Some(left), &directory).unwrap();
            frame(&mut app);
            app.execute_command("diff-this").unwrap();
            app.open_file(right).unwrap();
            app.execute_command("diff-this").unwrap();
            let side = app.diffs[0].side(Side::Left);
            app.active_pane = side.pane;
            if prior_dirty {
                // Keep the document dirty across each undo to isolate comparison
                // geometry from checking equality with the saved text.
                assert!(app.apply_to_buffer(side.buffer, &Transaction::insert(0, "z")));
            }
            let at = app.buffers[side.buffer]
                .text()
                .offset_of(Position::new(25_000, 0));
            assert!(app.apply_to_buffer(side.buffer, &Transaction::insert(at, inserted)));
            frame(&mut app);
            std::thread::sleep(Duration::from_millis(200));
            frame(&mut app);
            for _ in 0..5 {
                app.undo();
                frame(&mut app);
                app.redo();
                frame(&mut app);
            }
            let mut undo_total = Vec::new();
            let mut undo_frame = Vec::new();
            let mut redo_total = Vec::new();
            let mut redo_frame = Vec::new();
            for _ in 0..40 {
                let start = Instant::now();
                app.undo();
                let after_history = Instant::now();
                frame(&mut app);
                undo_total.push(start.elapsed());
                undo_frame.push(after_history.elapsed());

                let start = Instant::now();
                app.redo();
                let after_history = Instant::now();
                frame(&mut app);
                redo_total.push(start.elapsed());
                redo_frame.push(after_history.elapsed());
            }
            eprintln!(
                "pair {state} {label} undo: total {:?}, frame {:?}; redo: total {:?}, frame {:?}",
                median(undo_total),
                median(undo_frame),
                median(redo_total),
                median(redo_frame)
            );
        }
    }
    fs::remove_dir_all(directory).unwrap();
}
