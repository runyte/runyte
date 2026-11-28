// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeMap, path::Path};

/// Decision 54: consuming a binding's representative target must never flatten
/// a configured program. Keep every application-side read under review, including
/// presentation reads, so adding a dispatcher to an existing file trips this audit.
#[test]
fn representative_target_consumers_have_reviewed_dispatch_contracts() {
    fn scan(directory: &Path, inventory: &mut BTreeMap<(String, String), usize>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().unwrap() != "tests" {
                    scan(&path, inventory);
                }
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            let mut function = String::new();
            for line in source.lines() {
                if let Some((_, declaration)) = line.split_once("fn ") {
                    function = declaration.split('(').next().unwrap().trim().to_owned();
                }
                let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
                for _ in compact.match_indices("binding.target") {
                    *inventory
                        .entry((
                            path.file_name().unwrap().to_str().unwrap().into(),
                            function.clone(),
                        ))
                        .or_default() += 1;
                }
            }
        }
    }
    let mut actual = BTreeMap::new();
    scan(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app"),
        &mut actual,
    );
    let expected = [
        // Discovery and labels do not execute the representative target.
        ("action_discovery.rs", "capture_described_input", 1),
        ("directory_tree.rs", "tree_legend", 1),
        ("git_merge_review.rs", "review_key_hint", 1),
        ("presentation.rs", "list_action_keys", 1),
        ("presentation.rs", "list_legend", 4),
        ("presentation.rs", "open_help", 1),
        ("presentation.rs", "path_hint_rows", 1),
        ("search_history.rs", "binding_label", 1),
        // Native approval navigation explicitly refuses configured/non-plain bindings.
        ("context_access.rs", "review_motion", 1),
        // These two dispatch sites require the compiler's single-action gate:
        // all Mode::List owners (including session-manager), and the three Git
        // merge review/detail/input scopes. mode_flow.rs covers each owner and
        // shared ancestors, and checks built-in bindings remain plain actions.
        ("language_workflows.rs", "list_input_action", 1),
        ("git_merge_review.rs", "handle_merge_review_key", 1),
    ]
    .into_iter()
    .map(|(file, function, count)| ((file.to_owned(), function.to_owned()), count))
    .collect();
    assert_eq!(
        actual, expected,
        "audit new target consumers: programs must execute through the program executor or be rejected for that owner"
    );
}
