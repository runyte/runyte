// SPDX-License-Identifier: MPL-2.0
// Shared by native-target integration tests and every-platform loader tests.
use super::{Keymap, Mode, actions};

pub fn assert_registry(map: &Keymap, fast: bool, platform: &str) {
    let base = if fast {
        include_str!("../fixtures/keymap/runyte-fast.txt")
    } else {
        include_str!("../fixtures/keymap/runyte.txt")
    };
    let platform_bindings = match platform {
        "macos" => include_str!("../fixtures/keymap/platforms/macos.txt"),
        "windows" => include_str!("../fixtures/keymap/platforms/windows.txt"),
        "linux" => include_str!("../fixtures/keymap/platforms/linux.txt"),
        _ => "",
    };
    let mut rows =
        vec!["# Frozen from #151 (0975b492); semantic preset migration baseline.".to_owned()];
    rows.push("[bindings]".into());
    rows.extend(map.bindings().iter().map(|item| format!("{item:?}")));
    rows.push("[namespaces]".into());
    rows.extend(map.namespaces().iter().map(|item| format!("{item:?}")));
    rows.push("[context_actions]".into());
    rows.extend(
        map.all_context_actions()
            .iter()
            .map(|item| format!("{item:?}")),
    );
    fn canonical(lines: impl Iterator<Item = String>) -> Vec<String> {
        let mut result = Vec::new();
        let mut bindings = Vec::new();
        for line in lines {
            if line.starts_with("Binding {") {
                let line = if line.starts_with("Binding { binding_mode:") {
                    let end = line.find(", modes:").expect("binding mode field");
                    format!("Binding {{{}", &line[end + 1..])
                } else {
                    line
                };
                // Named tables can share a binding through inheritance instead
                // of duplicating its row for each legacy editing role.
                let start = line.find("modes: [").unwrap() + "modes: [".len();
                let end = start + line[start..].find(']').unwrap();
                for mode in line[start..end].split(", ") {
                    bindings.push(format!("{}{}{}", &line[..start], mode, &line[end..]));
                }
            } else {
                result.push(line);
            }
        }
        bindings.sort();
        result.extend(bindings);
        result
    }
    let expected = base.lines().chain(
        platform_bindings
            .lines()
            .filter(|line| line.starts_with("Binding {")),
    );
    assert_eq!(
        canonical(rows.into_iter()),
        canonical(expected.map(str::to_owned)),
        "{platform}, fast-pane-keys={fast}"
    );
}

pub fn preferred_rows(map: &Keymap, fast: bool) -> Vec<String> {
    let mut rows = Vec::new();
    for mode in [
        Mode::Normal,
        Mode::Select,
        Mode::Insert,
        Mode::Replace,
        Mode::Command,
        Mode::List,
    ] {
        let mut seen = std::collections::HashSet::new();
        for binding in map.bindings() {
            if seen.insert(binding.target)
                && let Some(sequence) = map.global_sequence_for(mode, binding.target)
            {
                rows.push(format!(
                    "{fast}|{}|{}|{sequence}",
                    actions::mode_name(mode),
                    binding.target.name()
                ));
            }
        }
    }
    rows
}

pub fn assert_preferred(mut rows: Vec<String>) {
    rows.sort();
    assert_eq!(
        rows.join("\n"),
        include_str!("../fixtures/keymap/preferred.txt")
            .lines()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
