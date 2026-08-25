// SPDX-License-Identifier: MPL-2.0

use std::{fs, path::Path};

fn rust_sources_below(directory: &Path, sources: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources_below(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

#[test]
fn crossterm_is_confined_to_terminal_acquisition_and_the_tui_adapter() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let main = source_root.join("main.rs");
    let adapter = source_root.join("tui/input.rs");
    let mut sources = Vec::new();
    rust_sources_below(&source_root, &mut sources);

    for source in sources {
        if source == main || source == adapter {
            continue;
        }
        let contents = fs::read_to_string(&source).unwrap();
        assert!(
            !contents.contains("crossterm"),
            "{} crosses the frontend input boundary",
            source.display()
        );
    }
}

#[test]
fn core_input_consumers_do_not_name_frontend_key_events() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = vec![
        source_root.join("app.rs"),
        source_root.join("keymap.rs"),
        source_root.join("key_hints.rs"),
    ];
    rust_sources_below(&source_root.join("app"), &mut sources);

    for source in sources {
        let contents = fs::read_to_string(&source).unwrap();
        assert!(
            !contents.contains("KeyEvent"),
            "{} names KeyEvent",
            source.display()
        );
        assert!(
            !contents.contains("KeyModifiers"),
            "{} names KeyModifiers",
            source.display()
        );
    }
}
