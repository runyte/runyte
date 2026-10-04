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
        contained: false,
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
        contained: false,
    };

    let (matches, limited) = perform(request, || false).unwrap().unwrap();

    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].path, unopened);
    assert_eq!(matches[1].path, open);
    assert_eq!(matches[1].row, 1);
    assert_eq!(matches[1].preview, "needle live");
    assert!(!limited);
}

#[test]
fn a_contained_search_refuses_the_filesystem_root() {
    let filesystem_root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .ancestors()
        .last()
        .unwrap()
        .to_path_buf();
    let request = WorkspaceSearchRequest {
        id: 1,
        root: filesystem_root,
        matcher: Regex::new("needle").unwrap(),
        show_hidden: false,
        open_buffers: Vec::new(),
        contained: true,
    };

    let error = perform(request, || false).unwrap_err();
    assert!(
        error.to_string().contains("open a narrower directory"),
        "{error}"
    );
}

#[test]
fn a_contained_search_spends_its_budget_on_hidden_entries_too() {
    // Hidden entries are never searched, but reading them is still work: a
    // directory of nothing but dotfiles must exhaust the budget rather than
    // be enumerated in full.
    let root = crate::test_support::TestRuntimeRoot::new("search").unwrap();
    for index in 0..=crate::scan_boundary::CONTAINED_SCAN_ENTRY_LIMIT {
        std::fs::write(root.path().join(format!(".{index:05}")), "needle\n").unwrap();
    }
    let request = |contained| WorkspaceSearchRequest {
        id: 1,
        root: root.path().to_path_buf(),
        matcher: Regex::new("needle").unwrap(),
        show_hidden: false,
        open_buffers: Vec::new(),
        contained,
    };

    let (matches, limited) = perform(request(true), || false).unwrap().unwrap();
    assert!(limited);
    assert!(matches.is_empty());

    let (matches, limited) = perform(request(false), || false).unwrap().unwrap();
    assert!(!limited, "a project search has no entry budget");
    assert!(matches.is_empty());
}

#[cfg(unix)]
#[test]
fn a_contained_search_passes_over_directories_it_cannot_read() {
    use std::os::unix::fs::PermissionsExt;

    let root = crate::test_support::TestRuntimeRoot::new("search").unwrap();
    let private = root.path().join("private");
    std::fs::create_dir(&private).unwrap();
    std::fs::write(private.join("secret"), "needle\n").unwrap();
    std::fs::write(root.path().join("open.conf"), "needle\n").unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Root reads every directory, so there is nothing unreadable to pass over.
    if std::fs::read_dir(&private).is_ok() {
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let request = |contained| WorkspaceSearchRequest {
        id: 1,
        root: root.path().to_path_buf(),
        matcher: Regex::new("needle").unwrap(),
        show_hidden: false,
        open_buffers: Vec::new(),
        contained,
    };

    let contained = perform(request(true), || false);
    let project = perform(request(false), || false);
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (matches, _) = contained.unwrap().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].path, root.path().join("open.conf"));
    assert!(
        project.is_err(),
        "a project search still reports the failure"
    );
}
