// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::buffer::{ProviderDocument, ProviderIdentity};

fn provider(text: &str) -> (App, usize) {
    let mut app = App::new(Config::default(), None).unwrap();
    app.config.editor.trim_trailing_whitespace = true;
    let buffer = app.install_provider_document(
        Buffer::provider_document(
            ProviderDocument {
                identity: ProviderIdentity {
                    configured_plugin: "preview-app".into(),
                    provider: "notes".into(),
                    key: "opaque-note".into(),
                },
                label: "Remote note".into(),
                syntax_hint: None,
                version: "v1".into(),
                generation: "g1".into(),
                available: true,
                baseline_epoch: 0,
                uncertain: None,
            },
            text.into(),
        ),
        true,
    );
    (app, buffer)
}

#[test]
fn provider_save_preview_and_cancellation_preserve_live_text_and_open_undo_group() {
    let (mut app, buffer) = provider("base  \n");
    app.buffers[buffer].begin_undo_group();
    app.apply_to_buffer(buffer, &Transaction::insert(0, "first  \n"));
    let before = app.buffers[buffer].to_string();
    let revision = app.buffers[buffer].revision();
    let history = app.buffers[buffer].history_len();
    let selection = app.active().selection.clone();
    let baseline = app.buffers[buffer].provider().unwrap().clone();
    let preview = app.preview_provider_save(buffer).unwrap();
    assert_eq!(preview.snapshot.text.to_string(), "first\nbase\n");
    assert_eq!(preview.source_revision, revision);
    assert_eq!(app.buffers[buffer].to_string(), before);
    assert_eq!(app.buffers[buffer].revision(), revision);
    assert_eq!(app.buffers[buffer].history_len(), history);
    assert_eq!(app.active().selection, selection);
    assert_eq!(app.buffers[buffer].provider(), Some(&baseline));
    assert!(app.buffers[buffer].dirty);
    drop(preview); // Native cancellation only releases the preview.
    app.apply_to_buffer(buffer, &Transaction::insert(0, "second "));
    app.buffers[buffer].commit_undo_group();
    assert!(app.buffers[buffer].undo());
    assert_eq!(app.buffers[buffer].to_string(), "base  \n");
    assert!(!app.buffers[buffer].dirty);
    assert!(!app.buffers[buffer].undo());
}

#[test]
fn provider_save_preview_preserves_clean_state_and_redo_on_cancellation() {
    let (mut app, buffer) = provider("base  \n");
    app.apply_to_buffer(buffer, &Transaction::insert(0, "edited "));
    assert!(app.buffers[buffer].undo());
    let revision = app.buffers[buffer].revision();
    let preview = app.preview_provider_save(buffer).unwrap();
    drop(preview);
    assert!(!app.buffers[buffer].dirty);
    assert_eq!(app.buffers[buffer].revision(), revision);
    assert!(app.buffers[buffer].redo());
    assert_eq!(app.buffers[buffer].to_string(), "edited base  \n");
}

#[test]
fn provider_save_preview_applies_approved_unicode_crlf_bytes_as_one_undo_step() {
    let (mut app, buffer) = provider("α  \r\n😀\t \r\n尾  ");
    app.buffers[buffer].begin_undo_group();
    app.apply_to_buffer(buffer, &Transaction::insert(0, "é  \r\n"));
    let original = app.buffers[buffer].to_string();
    let preview = app.preview_provider_save(buffer).unwrap();
    let approved = preview.snapshot.text.to_string();
    let preview_revision = preview.snapshot.text.revision();
    assert_eq!(approved, "é\r\nα\r\n😀\r\n尾");
    app.config.editor.trim_trailing_whitespace = false;
    let snapshot = app.apply_provider_save_preview(buffer, preview).unwrap();
    assert_eq!(snapshot.text.to_string(), approved);
    assert_eq!(app.buffers[buffer].to_string(), approved);
    assert_eq!(snapshot.text.revision(), app.buffers[buffer].revision());
    assert_ne!(snapshot.text.revision(), preview_revision);
    assert!(app.buffers[buffer].accept_provider_save(snapshot, "v2".into()));
    assert!(!app.buffers[buffer].dirty);
    assert!(app.buffers[buffer].undo());
    assert_eq!(app.buffers[buffer].to_string(), original);
    assert!(app.buffers[buffer].dirty);
    assert!(app.buffers[buffer].undo());
    assert_eq!(app.buffers[buffer].to_string(), "α  \r\n😀\t \r\n尾  ");
    assert!(!app.buffers[buffer].undo());
}

#[test]
fn provider_save_preview_does_not_add_hooks_enabled_after_review() {
    let (mut app, buffer) = provider("unchanged  \n");
    app.config.editor.trim_trailing_whitespace = false;
    let revision = app.buffers[buffer].revision();
    let preview = app.preview_provider_save(buffer).unwrap();
    app.config.editor.trim_trailing_whitespace = true;
    let snapshot = app.apply_provider_save_preview(buffer, preview).unwrap();
    assert_eq!(snapshot.text.to_string(), "unchanged  \n");
    assert_eq!(snapshot.text.revision(), revision);
    assert!(!app.buffers[buffer].dirty);
    assert!(!app.buffers[buffer].undo());
}

#[test]
fn provider_save_preview_rejects_edits_and_edit_undo_without_applying_hooks() {
    for undo in [false, true] {
        let (mut app, buffer) = provider("base  \n");
        let preview = app.preview_provider_save(buffer).unwrap();
        app.apply_to_buffer(buffer, &Transaction::insert(0, "newer "));
        if undo {
            assert!(app.buffers[buffer].undo());
        }
        let before = app.buffers[buffer].to_string();
        let revision = app.buffers[buffer].revision();
        let history = app.buffers[buffer].history_len();
        assert!(app.apply_provider_save_preview(buffer, preview).is_err());
        assert_eq!(app.buffers[buffer].to_string(), before);
        assert_eq!(app.buffers[buffer].revision(), revision);
        assert_eq!(app.buffers[buffer].history_len(), history);
        if undo {
            assert!(app.buffers[buffer].redo());
            assert_eq!(app.buffers[buffer].to_string(), "newer base  \n");
        }
    }
}

#[test]
fn provider_save_preview_rejects_rebinding_and_each_changed_baseline_component() {
    for change in 0..8 {
        let (mut app, buffer) = provider("base  \n");
        let preview = app.preview_provider_save(buffer).unwrap();
        let revision = app.buffers[buffer].revision();
        match change {
            0 => {
                app.buffers[buffer]
                    .provider_mut()
                    .unwrap()
                    .identity
                    .configured_plugin = "other".into()
            }
            1 => {
                app.buffers[buffer]
                    .provider_mut()
                    .unwrap()
                    .identity
                    .provider = "other".into()
            }
            2 => app.buffers[buffer].provider_mut().unwrap().identity.key = "other".into(),
            3 => app.buffers[buffer].provider_mut().unwrap().generation = "g2".into(),
            4 => app.buffers[buffer].provider_mut().unwrap().baseline_epoch += 1,
            5 => app.buffers[buffer].provider_mut().unwrap().version = "v2".into(),
            6 => app.buffers[buffer].provider_mut().unwrap().available = false,
            7 => {
                let identity = app.buffers[buffer].provider().unwrap().identity.clone();
                app.buffers[buffer]
                    .reconcile_provider(&identity, "g2".into(), "v2".into(), "base  \n")
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            app.apply_provider_save_preview(buffer, preview).is_err(),
            "case {change}"
        );
        assert_eq!(app.buffers[buffer].revision(), revision);
        assert_eq!(app.buffers[buffer].to_string(), "base  \n");
        assert!(!app.buffers[buffer].undo());
    }
}

#[test]
fn provider_save_preview_rejects_uncertainty_and_closed_or_wrong_targets() {
    let (mut app, buffer) = provider("base  \n");
    let preview = app.preview_provider_save(buffer).unwrap();
    let uncertain = app.buffers[buffer].prepare_provider_save().unwrap();
    assert!(app.buffers[buffer].mark_provider_uncertain(uncertain));
    assert!(app.apply_provider_save_preview(buffer, preview).is_err());
    assert!(app.preview_provider_save(buffer).is_err());
    assert_eq!(app.buffers[buffer].to_string(), "base  \n");
    assert!(!app.buffers[buffer].undo());
    let (mut app, buffer) = provider("base  \n");
    let preview = app.preview_provider_save(buffer).unwrap();
    app.host_close_buffer(buffer, true).unwrap();
    assert!(app.apply_provider_save_preview(buffer, preview).is_err());
    assert!(app.preview_provider_save(buffer).is_err());
    assert!(app.preview_provider_save(usize::MAX).is_err());
    let (mut app, buffer) = provider("base  \n");
    let preview = app.preview_provider_save(buffer).unwrap();
    app.buffers[buffer].kind = BufferKind::Scratch;
    assert!(app.apply_provider_save_preview(buffer, preview).is_err());
    assert_eq!(app.buffers[buffer].to_string(), "base  \n");
    assert!(!app.buffers[buffer].undo());
}

#[test]
fn provider_save_preview_bounds_dense_hooks_before_any_live_change() {
    let limit = super::super::file_workflows::PROVIDER_SAVE_HOOK_LIMIT;
    // Transaction::new temporarily retains the collected and ordered vectors.
    assert!(
        std::mem::size_of::<crate::text::Change>() * limit * 2
            + std::mem::size_of::<Vec<crate::text::Change>>() * 2
            <= 512 * 1024
    );
    let original = "x \n".repeat(limit);
    let (mut app, buffer) = provider(&original);
    let preview = app.preview_provider_save(buffer).unwrap();
    assert_eq!(preview.snapshot.text.to_string(), "x\n".repeat(limit));
    app.apply_provider_save_preview(buffer, preview).unwrap();
    assert!(app.buffers[buffer].undo());
    assert_eq!(app.buffers[buffer].to_string(), original);
    assert!(!app.buffers[buffer].undo());

    let original = "x \n".repeat(limit + 1);
    let (mut app, buffer) = provider(&original);
    app.buffers[buffer].begin_undo_group();
    app.apply_to_buffer(buffer, &Transaction::insert(0, "first "));
    let before = app.buffers[buffer].to_string();
    let revision = app.buffers[buffer].revision();
    let history = app.buffers[buffer].history_len();
    let error = app.preview_provider_save(buffer).err().unwrap();
    assert!(error.is::<ProviderSavePreviewLimit>());
    assert!(error.to_string().contains("4096 whitespace changes"));
    assert_eq!(app.buffers[buffer].to_string(), before);
    assert_eq!(app.buffers[buffer].revision(), revision);
    assert_eq!(app.buffers[buffer].history_len(), history);
    app.apply_to_buffer(buffer, &Transaction::insert(0, "second "));
    app.buffers[buffer].commit_undo_group();
    assert!(app.buffers[buffer].undo());
    assert_eq!(app.buffers[buffer].to_string(), original);
    assert!(!app.buffers[buffer].undo());
}
