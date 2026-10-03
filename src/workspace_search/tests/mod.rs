// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn dense_unicode_matches_keep_columns_and_share_the_line_preview() {
    let line = format!(
        "{}{}",
        "éλ".repeat(GLOBAL_SEARCH_RESULT_LIMIT),
        " ".repeat(1024 * 1024)
    );
    let mut matches = Vec::new();
    assert!(extend_matches(
        &mut matches,
        Path::new("dense.txt"),
        std::iter::once((0, line)),
        &Regex::new("λ").unwrap(),
        &|| false,
    ));
    assert_eq!(matches.len(), GLOBAL_SEARCH_RESULT_LIMIT);
    for (index, found) in matches.iter().enumerate() {
        assert_eq!(found.column, index * 2 + 1);
        assert_eq!(found.length, 1);
        assert_eq!(found.preview, "éλ".repeat(120));
    }
}

#[test]
fn zero_width_matches_keep_unicode_character_columns() {
    let mut matches = Vec::new();
    assert!(extend_matches(
        &mut matches,
        Path::new("unicode.txt"),
        std::iter::once((3, "é中x")),
        &Regex::new("").unwrap(),
        &|| false,
    ));
    assert_eq!(
        matches
            .iter()
            .map(|found| (found.row, found.column, found.length))
            .collect::<Vec<_>>(),
        [(3, 0, 0), (3, 1, 0), (3, 2, 0), (3, 3, 0)]
    );
}

#[test]
fn cancellation_is_observed_between_matches_on_one_line() {
    let checks = std::cell::Cell::new(0);
    let mut matches = Vec::new();
    assert!(!extend_matches(
        &mut matches,
        Path::new("dense.txt"),
        std::iter::once((0, "many matches")),
        &Regex::new(".").unwrap(),
        &|| {
            let observed = checks.get();
            checks.set(observed + 1);
            observed == 3
        },
    ));
    assert_eq!(matches.len(), 2);
}

#[test]
fn stale_disk_matches_in_open_buffers_do_not_exhaust_workspace_search() {
    let root = crate::test_support::TestRuntimeRoot::new("search").unwrap();
    let unopened = root.path().join("a.txt");
    let open = root.path().join("z.txt");
    std::fs::write(&unopened, "needle\n").unwrap();
    std::fs::write(&open, "needle\n".repeat(GLOBAL_SEARCH_RESULT_LIMIT)).unwrap();
    let request = WorkspaceSearchRequest {
        id: 1,
        root: root.path().to_path_buf(),
        matcher: Regex::new("needle").unwrap(),
        show_hidden: false,
        open_buffers: vec![WorkspaceSearchSnapshot {
            path: open,
            text: Text::from_str("unsaved replacement\n"),
        }],
    };

    let (matches, limited) = perform(request, || false).unwrap().unwrap();

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].path, unopened);
    assert_eq!(matches[0].row, 0);
    assert!(!limited);
}

#[test]
fn workspace_search_combines_live_and_unopened_matches_without_duplicates() {
    let root = crate::test_support::TestRuntimeRoot::new("search").unwrap();
    let unopened = root.path().join("a.txt");
    let open = root.path().join("z.txt");
    std::fs::write(&unopened, "needle\n").unwrap();
    std::fs::write(&open, "needle disk\n").unwrap();
    let request = WorkspaceSearchRequest {
        id: 1,
        root: root.path().to_path_buf(),
        matcher: Regex::new("needle").unwrap(),
        show_hidden: false,
        open_buffers: vec![WorkspaceSearchSnapshot {
            path: open.clone(),
            text: Text::from_str("changed\nneedle live\n"),
        }],
    };

    let (matches, limited) = perform(request, || false).unwrap().unwrap();

    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].path, unopened);
    assert_eq!(matches[1].path, open);
    assert_eq!(matches[1].row, 1);
    assert_eq!(matches[1].preview, "needle live");
    assert!(!limited);
}
