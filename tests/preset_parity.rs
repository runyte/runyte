// SPDX-License-Identifier: MPL-2.0

use runyte::keymap::keymap_for;

// Captured from the approved Rust preset at #151, before the YAML migration.
// Compare every binding field independently of cross-mode table ordering.
// Namespace and context-action order remains exact; preferred spellings are
// checked separately against their pre-migration inventory.
#[test]
fn runyte_preserves_the_complete_pre_yaml_registry() {
    for (fast, fixture) in [
        (false, include_str!("fixtures/keymap/runyte.txt")),
        (true, include_str!("fixtures/keymap/runyte-fast.txt")),
    ] {
        let map = keymap_for(fast);
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
                    bindings.push(line);
                } else {
                    result.push(line);
                }
            }
            bindings.sort();
            result.extend(bindings);
            result
        }
        assert_eq!(
            canonical(rows.into_iter()),
            canonical(fixture.lines().map(str::to_owned)),
            "fast-pane-keys={fast}"
        );
    }
}

#[test]
fn preferred_global_spellings_preserve_pre_yaml_ordering() {
    use runyte::command::Mode;
    let mut rows = Vec::new();
    for fast in [false, true] {
        let map = keymap_for(fast);
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
                        runyte::keymap::actions::mode_name(mode),
                        binding.target.name()
                    ));
                }
            }
        }
    }
    rows.sort();
    assert_eq!(
        rows.join("\n"),
        include_str!("fixtures/keymap/preferred.txt")
            .lines()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
