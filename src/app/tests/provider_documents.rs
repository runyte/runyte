// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::buffer::{ProviderDocument, ProviderIdentity};

fn document(key: &str, text: &str) -> Buffer {
    Buffer::provider_document(
        ProviderDocument {
            identity: ProviderIdentity {
                configured_plugin: "memory-app".into(),
                provider: "memory".into(),
                key: key.into(),
            },
            label: "Remote notes".into(),
            syntax_hint: Some("rust".into()),
            version: "v1".into(),
            generation: "g1".into(),
            available: true,
            baseline_epoch: 0,
            uncertain: None,
        },
        text.into(),
    )
}

#[test]
fn provider_documents_reuse_identity_preserve_edits_and_have_ordinary_retention() {
    let mut app = App::new(Config::default(), None).unwrap();
    let id = app.install_provider_document(document("opaque-key", "fn main() {}\n"), true);
    assert_eq!(app.active().buffer, id);
    assert!(app.syntax[id].is_some());
    assert!(app.buffers[id].path.is_none());
    assert!(!app.lsp_touch(id));
    assert!(!app.lsp_documents.contains_key(&id));
    app.apply_to_buffer(id, &Transaction::insert(0, "// edited\n"));
    let before = app.buffers[id].to_string();
    assert_eq!(
        app.install_provider_document(document("opaque-key", "replacement"), false),
        id
    );
    assert_eq!(app.buffers[id].to_string(), before);
    app.host_close_buffer(id, false).unwrap_err();
    assert_eq!(
        app.available_buffer_actions(id),
        vec![BufferAction::Discard]
    );
    app.discard_buffer_changes(id).unwrap();
    assert_eq!(app.buffers[id].to_string(), "fn main() {}\n");
    assert!(!app.buffers[id].undo());
    open_filler_special_buffers(&mut app, SPECIAL_BUFFER_RETENTION_LIMIT + 2);
    app.retire_detached_ephemeral_buffers();
    assert!(!app.host_buffer_is_closed(id));
    app.host_close_buffer(id, false).unwrap();
    let reopened = app.install_provider_document(document("opaque-key", "fresh"), false);
    assert_ne!(reopened, id);
}

#[test]
fn provider_commands_refuse_local_save_reload_and_dirty_close_before_edit_hooks() {
    let mut app = App::new(Config::default(), None).unwrap();
    let root = temporary("provider-write-refusal");
    fs::create_dir_all(&root).unwrap();
    app.working_directory = root.clone();
    app.config.editor.trim_trailing_whitespace = true;
    let id =
        app.install_provider_document(document("/opaque/local-looking/path", "baseline  \n"), true);
    app.apply_to_buffer(id, &Transaction::insert(0, "edited  \n"));
    let before = app.buffers[id].to_string();
    let revision = app.buffers[id].revision();
    for command in [
        "write!",
        "write unused-local-file",
        "write! unused-local-file",
        "reload",
        "close",
    ] {
        app.execute_command(command).unwrap();
        assert!(app.status_error, "{command}");
        assert_eq!(app.buffers[id].to_string(), before, "{command}");
        assert_eq!(app.buffers[id].revision(), revision, "{command}");
        assert!(app.buffers[id].path.is_none(), "{command}");
        assert!(!app.host_buffer_is_closed(id), "{command}");
        assert!(!app.should_quit, "{command}");
    }
    app.buffers[id].provider_mut().unwrap().available = false;
    assert!(app.apply_to_buffer(id, &Transaction::insert(0, "still editable ")));
    app.host_close_buffer(id, true).unwrap();
    assert!(app.host_buffer_is_closed(id));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
}
