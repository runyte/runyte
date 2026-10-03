// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{config::Config, test_support::TestRuntimeRoot};

#[test]
fn directory_host_open_refuses_replacement_before_publishing_any_buffer() {
    for existing_explorer in [false, true] {
        let root = TestRuntimeRoot::new("host-directory-publication").unwrap();
        let folder = root.join("folder");
        fs::create_dir_all(&folder).unwrap();
        let second = root.join("second.txt");
        fs::write(&second, "second text\n").unwrap();
        let mut app = App::new(
            Config::default(),
            existing_explorer.then(|| root.path().to_owned()),
        )
        .unwrap();
        let buffer = app.active().buffer;
        let count = app.buffers.len();
        let text = app.buffers[buffer].text().to_string();
        let selection = app.active().selection.clone();
        let result = app.host_open_files_prepared(vec![folder.clone(), second], true, None, || {
            fs::rename(&folder, root.join("moved")).unwrap();
            fs::write(&folder, "directory replaced by text file").unwrap();
        });

        assert!(result.is_err(), "a directory request became a file open");
        assert_eq!(
            app.buffers.len(),
            count,
            "failed request published a buffer"
        );
        assert_eq!(app.active().buffer, buffer);
        assert_eq!(app.buffers[buffer].text().to_string(), text);
        assert_eq!(app.active().selection, selection);
    }
}

#[test]
fn host_open_activates_prepared_buffers_without_reopening_changed_paths() {
    for refresh in [false, true] {
        let root = TestRuntimeRoot::new("host-open-publication").unwrap();
        let folder = root.join("folder");
        fs::create_dir_all(&folder).unwrap();
        let first = folder.join("note.txt");
        let second = root.join("second.txt");
        fs::write(&first, "original text\n").unwrap();
        fs::write(&second, "second text\n").unwrap();
        let mut app = App::new(Config::default(), refresh.then(|| first.clone())).unwrap();
        fs::write(&first, "prepared text\n").unwrap();
        let pending = HashSet::new();
        let opened = app
            .host_open_files_prepared(
                vec![first.clone(), second],
                true,
                refresh.then_some(&pending),
                || {
                    fs::remove_file(&first).unwrap();
                    std::os::unix::fs::symlink("note.txt", &first).unwrap();
                },
            )
            .expect("published buffers must activate by their prepared identity");

        assert_eq!(opened.len(), 2);
        assert_eq!(app.active().buffer, opened[0]);
        assert_eq!(app.buffers[opened[0]].text().to_string(), "prepared text\n");
        assert_eq!(app.buffers[opened[1]].text().to_string(), "second text\n");
        assert_eq!(app.buffers[opened[0]].path.as_ref(), Some(&first));
        assert_eq!(app.active().selection, Selection::point(0));
    }
}
