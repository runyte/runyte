// SPDX-License-Identifier: MPL-2.0

use super::*;

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
