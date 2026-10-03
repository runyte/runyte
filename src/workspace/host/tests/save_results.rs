// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{buffer::Buffer, test_support::TestRuntimeRoot};

#[test]
fn host_save_returns_errors_for_failed_and_stale_writes() {
    let root = TestRuntimeRoot::new("save-result").unwrap();
    let mut host = host();
    let scratch = host.buffer_metadata()[0].id;
    host.app.buffers[scratch.index().unwrap()].apply(&Transaction::insert(0, "unsaved"));
    assert!(host.save_buffer(scratch).is_err());
    assert!(host.app.buffers[scratch.index().unwrap()].dirty);

    let missing = root.path().join("missing-parent").join("note.txt");
    let new_file = host.open_buffer(missing.clone(), false).unwrap();
    host.app.buffers[new_file.index().unwrap()].apply(&Transaction::insert(0, "unsaved"));
    assert!(host.save_buffer(new_file).is_err());
    assert!(!missing.exists());
    assert!(host.app.buffers[new_file.index().unwrap()].dirty);

    let path = root.path().join("note.txt");
    std::fs::write(&path, "original").unwrap();
    let file = host.open_buffer(path.clone(), true).unwrap();
    let revision = host.read_buffer(file).unwrap().metadata.revision;
    host.apply_expected_transaction(file, revision, Transaction::insert(0, "edited "))
        .unwrap();
    std::fs::write(&path, "external replacement").unwrap();
    // The first attempt discovers the conflict during the write; the next is
    // refused using the recorded stale-file observation.
    for _ in 0..2 {
        assert!(host.save_buffer(file).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "external replacement"
        );
        assert!(host.app.buffers[file.index().unwrap()].dirty);
    }
}

#[test]
fn host_save_refuses_read_only_commit_and_directory_buffers_without_side_effects() {
    let root = TestRuntimeRoot::new("save-kinds").unwrap();
    let mut host = host();
    let id = host.buffer_metadata()[0].id;
    host.app.buffers[id.index().unwrap()] = Buffer::virtual_text("report", "read only");
    assert!(!host.app.buffers[id.index().unwrap()].dirty);
    assert!(host.save_buffer(id).is_err());

    host.app.buffers[id.index().unwrap()] = Buffer::commit_message("commit message");
    let feedback = host.app.status.clone();
    assert!(host.save_buffer(id).is_err());
    assert_eq!(host.app.status, feedback, "no native commit command ran");
    assert_eq!(
        host.app.buffers[id.index().unwrap()].to_string(),
        "commit message"
    );

    let directory = host.open_buffer(root.path().to_path_buf(), true).unwrap();
    host.app.buffers[directory.index().unwrap()].apply(&Transaction::insert(0, "new.txt\n"));
    assert!(host.save_buffer(directory).is_err());
    assert!(host.app.fs_confirmation.is_none());
    assert!(!root.path().join("new.txt").exists());
    assert!(host.app.buffers[directory.index().unwrap()].dirty);
}

#[test]
fn host_save_returns_the_revision_after_writing_the_requested_buffer() {
    let root = TestRuntimeRoot::new("save-success").unwrap();
    let path = root.path().join("note.txt");
    std::fs::write(&path, "original").unwrap();
    let mut host = host();
    let id = host.open_buffer(path.clone(), false).unwrap();
    host.app.plugins.filesystem_applying = true;
    assert!(host.save_buffer(id).is_err());
    assert!(!host.app.buffers[id.index().unwrap()].dirty);
    host.app.plugins.filesystem_applying = false;
    let before = host.read_buffer(id).unwrap().metadata.revision;
    let revision = host
        .apply_expected_transaction(id, before, Transaction::insert(0, "edited "))
        .unwrap();
    assert_eq!(host.save_buffer(id).unwrap(), revision);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited original");
    assert!(!host.app.buffers[id.index().unwrap()].dirty);
}
