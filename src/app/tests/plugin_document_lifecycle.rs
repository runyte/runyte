// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::plugin::application::JobState;

struct DocumentFixture(PathBuf);
impl DocumentFixture {
    fn new(name: &str) -> (Self, App, usize) {
        let root = temporary(name);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("notes.txt");
        fs::write(&path, "baseline\n").unwrap();
        let app = App::new(Config::default(), Some(path)).unwrap();
        let buffer = app.active().buffer;
        (Self(root), app, buffer)
    }
}
impl Drop for DocumentFixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn pending_document_write_refuses_every_global_quit_spelling_even_when_clean() {
    for hidden in [false, true] {
        for command in [
            "quit",
            "q",
            "quit!",
            "q!",
            "quit-all",
            "qa",
            "quit-all!",
            "qa!",
            "quit-here",
            "qh",
            "quit-here!",
            "qh!",
            "write-quit",
            "wq",
        ] {
            let (_fixture, mut app, buffer) = DocumentFixture::new("pending-document-quit");
            app.quit_directory_handoff = true;
            if hidden {
                let scratch = app.buffers.len();
                app.buffers.push(Buffer::scratch());
                app.syntax.push(None);
                app.switch_buffer(scratch);
            }
            assert!(app.buffers.iter().all(|buffer| !buffer.dirty));
            app.plugins.document_saves.insert(buffer);
            app.execute_command(command).unwrap();
            assert!(!app.should_quit, "{command}, hidden={hidden}");
            assert!(app.persistent_exit_request.is_none(), "{command}");
            assert!(app.quit_directory.is_none(), "{command}");
            assert!(!app.host_buffer_is_closed(buffer), "{command}");
        }
    }
}

#[test]
fn pending_document_write_protects_reload_discard_close_and_save_baseline() {
    let (fixture, mut app, buffer) = DocumentFixture::new("pending-document-lifecycle");
    app.apply_to_buffer(buffer, &Transaction::insert(0, "local "));
    app.buffers[buffer].commit_undo_group();
    let original = app.buffers[buffer].to_string();
    let revision = app.buffers[buffer].revision();
    let history = app.buffers[buffer].history_len();
    let baseline = app.buffers[buffer]
        .file_observation_request(buffer)
        .unwrap();
    fs::write(fixture.0.join("notes.txt"), "external\n").unwrap();
    let observation = app.buffers[buffer].observe_now(buffer).unwrap();
    app.plugins.document_saves.insert(buffer);

    assert!(app.reload_file().is_err());
    assert!(
        app.install_file_reload(buffer, &observation.observation)
            .is_err()
    );
    assert!(app.discard_buffer_changes(buffer).is_err());
    for discard in [false, true] {
        assert!(app.host_close_buffer(buffer, discard).is_err());
        app.close_active_buffer(discard);
        assert!(!app.host_buffer_is_closed(buffer));
    }
    app.close_buffer_discarding(buffer);
    app.close_buffer(buffer);
    app.save_buffer(buffer, None, true).unwrap();
    app.apply_file_observation(observation);

    assert!(!app.host_buffer_is_closed(buffer));
    assert_eq!(app.buffers[buffer].to_string(), original);
    assert_eq!(app.buffers[buffer].revision(), revision);
    assert_eq!(app.buffers[buffer].history_len(), history);
    assert_eq!(
        app.buffers[buffer]
            .file_observation_request(buffer)
            .unwrap(),
        baseline
    );
    assert!(app.buffers[buffer].dirty);
    assert!(app.file_reload_confirmation.is_none());
    assert_eq!(
        fs::read_to_string(fixture.0.join("notes.txt")).unwrap(),
        "external\n"
    );
}

#[test]
fn pending_observation_cannot_advance_captured_save_baseline_and_later_edits_stay_dirty() {
    let (fixture, mut app, buffer) = DocumentFixture::new("pending-document-observation");
    app.apply_to_buffer(buffer, &Transaction::insert(0, "saved "));
    let work = app.prepare_plugin_document_save(buffer);
    app.plugins.document_saves.insert(buffer);
    let baseline = app.buffers[buffer]
        .file_observation_request(buffer)
        .unwrap();
    let saved = work.run(&fixture.0).unwrap();
    let observation = app.buffers[buffer].observe_now(buffer).unwrap();
    app.apply_file_observation(observation);
    assert!(app.buffers[buffer].dirty);
    assert_eq!(
        app.buffers[buffer]
            .file_observation_request(buffer)
            .unwrap(),
        baseline
    );

    app.apply_to_buffer(buffer, &Transaction::insert(0, "newer "));
    app.buffers[buffer].commit_undo_group();
    app.plugins.document_saves.remove(&buffer);
    assert_eq!(
        app.finish_plugin_document_save(buffer, false, Ok(Some(saved))),
        JobState::Succeeded
    );
    assert_eq!(app.buffers[buffer].to_string(), "newer saved baseline\n");
    assert!(app.buffers[buffer].dirty);
    assert_eq!(
        fs::read_to_string(fixture.0.join("notes.txt")).unwrap(),
        "saved baseline\n"
    );
    app.undo();
    assert_eq!(app.buffers[buffer].to_string(), "saved baseline\n");
    assert!(!app.buffers[buffer].dirty);
}

#[test]
fn uncertain_matching_observations_preserve_dirty_state_until_explicit_reload() {
    let (_fixture, mut app, buffer) = DocumentFixture::new("uncertain-document-observation");
    app.buffers[buffer].mark_write_uncertain();
    assert!(app.buffers[buffer].dirty);
    let baseline = app.buffers[buffer]
        .file_observation_request(buffer)
        .unwrap();
    for _ in 0..2 {
        let observation = app.buffers[buffer].observe_now(buffer).unwrap();
        app.apply_file_observation(observation);
        assert!(app.buffers[buffer].dirty);
        assert_eq!(
            app.buffers[buffer]
                .file_observation_request(buffer)
                .unwrap(),
            baseline
        );
    }
    app.apply_to_buffer(buffer, &Transaction::insert(0, "temporary "));
    app.buffers[buffer].commit_undo_group();
    app.undo();
    assert_eq!(app.buffers[buffer].to_string(), "baseline\n");
    assert!(app.buffers[buffer].dirty);
    app.reload_file().unwrap();
    assert!(app.buffers[buffer].dirty);
    assert!(app.file_reload_confirmation.is_some());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.file_reload_confirmation.is_none());
    assert!(!app.buffers[buffer].dirty);
    assert_eq!(app.buffers[buffer].to_string(), "baseline\n");
}

#[test]
fn cancelled_committed_write_marks_even_clean_documents_uncertain() {
    let (fixture, mut app, buffer) = DocumentFixture::new("cancelled-document-write");
    let saved = app.buffers[buffer]
        .prepare_document_save()
        .unwrap()
        .run(&fixture.0)
        .unwrap();
    assert!(!app.buffers[buffer].dirty);
    assert_eq!(
        app.finish_plugin_document_save(buffer, true, Ok(Some(saved))),
        JobState::OutcomeUnknown
    );
    let observation = app.buffers[buffer].observe_now(buffer).unwrap();
    app.apply_file_observation(observation);
    assert!(app.buffers[buffer].dirty);
    assert!(app.host_close_buffer(buffer, false).is_err());
    assert_eq!(
        fs::read_to_string(fixture.0.join("notes.txt")).unwrap(),
        "baseline\n"
    );
}
