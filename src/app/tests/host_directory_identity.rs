// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{config::Config, test_support::TestRuntimeRoot};

#[test]
fn directory_host_open_preserves_other_requested_explorer_identity() {
    let root = TestRuntimeRoot::new("host-directory-request-identity").unwrap();
    let first = root.join("first");
    let second = root.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let mut app = App::new(Config::default(), Some(second.clone())).unwrap();
    let original = app.active().buffer;

    let opened = app
        .host_open_files(vec![first.clone(), second.clone(), second.clone()], true)
        .unwrap();

    assert_eq!(app.buffers[opened[0]].path.as_ref(), Some(&first));
    assert_eq!(app.buffers[opened[1]].path.as_ref(), Some(&second));
    assert_eq!(opened[1], original);
    assert_eq!(opened[1], opened[2]);
    assert_ne!(opened[0], opened[1]);
    assert_eq!(app.active().buffer, opened[0]);
    assert_eq!(app.active().directory_buffer, Some(opened[0]));
}

#[test]
fn directory_host_open_returns_one_activated_id_for_existing_aliases() {
    let root = TestRuntimeRoot::new("host-directory-alias-identity").unwrap();
    let first = root.join("first");
    let second = root.join("second");
    let alias = root.join("alias");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    std::os::unix::fs::symlink(&first, &alias).unwrap();
    let mut app = App::new(Config::default(), Some(second)).unwrap();
    app.host_open_files(vec![first.clone()], false).unwrap();

    let opened = app
        .host_open_files(vec![first.clone(), alias, first.clone()], true)
        .unwrap();

    assert!(opened.iter().all(|buffer| *buffer == app.active().buffer));
    assert_eq!(app.buffers[opened[0]].path.as_ref(), Some(&first));
}

#[test]
fn directory_host_open_adopts_the_existing_unclaimed_target() {
    let root = TestRuntimeRoot::new("host-directory-target-adoption").unwrap();
    let first = root.join("first");
    let second = root.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let mut app = App::new(Config::default(), Some(second.clone())).unwrap();
    let second_id = app.active().buffer;
    let first_id = app.host_open_file(first.clone(), false).unwrap();
    let count = app.buffers.len();

    for (paths, expected) in [
        (
            vec![first.clone(), second.clone()],
            vec![first_id, second_id],
        ),
        (
            vec![second.clone(), first.clone()],
            vec![second_id, first_id],
        ),
        (vec![first, second], vec![first_id, second_id]),
    ] {
        let opened = app.host_open_files(paths, true).unwrap();
        assert_eq!(opened, expected);
        assert_eq!(app.buffers.len(), count);
        assert_eq!(app.active().directory_buffer, Some(opened[0]));
    }
}

#[test]
fn directory_host_open_keeps_another_panes_explorer_claim() {
    let root = TestRuntimeRoot::new("host-directory-other-pane").unwrap();
    let first = root.join("first");
    let second = root.join("second");
    let third = root.join("third");
    for path in [&first, &second, &third] {
        fs::create_dir_all(path).unwrap();
    }
    let mut app = App::new(Config::default(), Some(first.clone())).unwrap();
    let other_pane = app.active_pane;
    let other_buffer = app.active().buffer;
    app.split(Axis::Horizontal, Some(second.clone())).unwrap();
    let second_id = app.active().buffer;

    let opened = app
        .host_open_files(vec![first.clone(), second.clone()], true)
        .unwrap();

    assert_ne!(opened[0], other_buffer);
    assert_eq!(opened[1], second_id);
    assert_eq!(app.active().directory_buffer, Some(opened[0]));
    app.open_file(third).unwrap();
    assert_eq!(app.panes[&other_pane].buffer, other_buffer);
    assert_eq!(app.buffers[other_buffer].path.as_ref(), Some(&first));
    assert_eq!(app.buffers[second_id].path.as_ref(), Some(&second));
}

#[test]
fn reserved_explorer_is_unchanged_when_requested_directory_read_fails() {
    let root = TestRuntimeRoot::new("host-reserved-directory-failure").unwrap();
    let first = root.join("first");
    let second = root.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let mut app = App::new(Config::default(), Some(second.clone())).unwrap();
    let buffer = app.active().buffer;
    let revision = app.buffers[buffer].revision();
    let count = app.buffers.len();

    let result =
        app.host_open_files_prepared(vec![first.clone(), second.clone()], true, None, || {
            fs::remove_dir(&first).unwrap();
            fs::write(&first, "directory replaced by text file").unwrap();
        });

    assert!(result.is_err());
    assert_eq!(app.buffers.len(), count);
    assert_eq!(app.active().buffer, buffer);
    assert_eq!(app.buffers[buffer].path.as_ref(), Some(&second));
    assert_eq!(app.buffers[buffer].revision(), revision);
}
