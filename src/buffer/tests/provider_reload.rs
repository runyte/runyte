// SPDX-License-Identifier: MPL-2.0
use super::*;
use std::sync::atomic::AtomicBool;
fn buffer() -> Buffer {
    Buffer::provider_document(
        ProviderDocument {
            identity: ProviderIdentity {
                configured_plugin: "remote".into(),
                provider: "files".into(),
                key: "notes".into(),
            },
            label: "Notes".into(),
            syntax_hint: None,
            version: "v1".into(),
            generation: "old".into(),
            available: false,
            baseline_epoch: 0,
            uncertain: None,
        },
        "α\r\n猫\n".into(),
    )
}
fn prepared(buffer: &Buffer, text: &str) -> PreparedProviderReload {
    buffer
        .provider_reload_source()
        .unwrap()
        .prepare(
            text.into(),
            "new".into(),
            "v2".into(),
            false,
            &AtomicBool::new(false),
        )
        .unwrap()
}
#[test]
fn reload_prepares_without_mutation_and_replaces_as_one_undo_against_new_baseline() {
    let mut buffer = buffer();
    buffer.begin_undo_group();
    buffer.apply(&Transaction::insert(0, "local "));
    let before = buffer.to_string();
    let revision = buffer.revision();
    let history = buffer.history_footprint();
    let candidate = prepared(&buffer, "remote 🦀\r\n");
    assert_eq!(buffer.revision(), revision);
    assert_eq!(buffer.history_footprint(), history);
    buffer
        .accept_provider_reload(candidate, ProviderReloadChoice::ReloadRemote)
        .unwrap();
    assert_eq!(buffer.to_string(), "remote 🦀\r\n");
    assert!(!buffer.dirty);
    assert_eq!(buffer.history_len(), 2);
    assert_eq!(buffer.provider().unwrap().version, "v2");
    assert_eq!(buffer.provider().unwrap().generation, "new");
    assert!(buffer.provider().unwrap().available);
    assert!(buffer.undo());
    assert_eq!(buffer.to_string(), before);
    assert!(buffer.dirty);
    assert_eq!(buffer.provider().unwrap().version, "v2");
    assert!(buffer.redo());
    assert!(!buffer.dirty);
}
#[test]
fn keep_local_adopts_remote_baseline_without_touching_text_undo_group_or_redo() {
    let mut buffer = buffer();
    buffer.apply(&Transaction::insert(0, "future "));
    buffer.undo();
    buffer.begin_undo_group();
    let revision = buffer.revision();
    let history = (
        buffer.undo.clone(),
        buffer.redo.clone(),
        buffer.undo_group.clone(),
    );
    let candidate = prepared(&buffer, "different remote");
    let applied = buffer
        .accept_provider_reload(candidate, ProviderReloadChoice::KeepLocal)
        .unwrap();
    assert!(applied.transaction.is_none());
    assert_eq!(buffer.revision(), revision);
    assert_eq!(
        (
            buffer.undo.clone(),
            buffer.redo.clone(),
            buffer.undo_group.clone()
        ),
        history
    );
    assert!(buffer.dirty);
    assert!(buffer.redo());
    assert_eq!(buffer.to_string(), "future α\r\n猫\n");
    assert_eq!(buffer.provider().unwrap().version, "v2");
}
#[test]
fn equal_remote_content_adopts_without_creating_an_undo_checkpoint() {
    let mut buffer = buffer();
    let revision = buffer.revision();
    let candidate = prepared(&buffer, &buffer.to_string());
    let applied = buffer
        .accept_provider_reload(candidate, ProviderReloadChoice::ReloadRemote)
        .unwrap();
    assert!(applied.transaction.is_none());
    assert_eq!(buffer.revision(), revision);
    assert_eq!(buffer.history_len(), 0);
    assert!(!buffer.dirty);
}
#[test]
fn reload_guards_reject_text_aba_and_metadata_changes_without_mutating_history() {
    for change in 0..5 {
        let mut buffer = buffer();
        let candidate = prepared(&buffer, "new remote");
        match change {
            0 => {
                buffer.apply(&Transaction::insert(0, "edit"));
                buffer.undo();
            }
            1 => buffer.provider_mut().unwrap().generation = "other".into(),
            2 => buffer.provider_mut().unwrap().baseline_epoch += 1,
            3 => buffer.provider_mut().unwrap().available = true,
            _ => buffer.mark_write_uncertain(),
        }
        let before = (
            buffer.to_string(),
            buffer.revision(),
            buffer.undo.clone(),
            buffer.redo.clone(),
            buffer.provider().cloned(),
        );
        assert!(
            buffer
                .accept_provider_reload(candidate, ProviderReloadChoice::ReloadRemote)
                .is_err()
        );
        assert_eq!(
            (
                buffer.to_string(),
                buffer.revision(),
                buffer.undo.clone(),
                buffer.redo.clone(),
                buffer.provider().cloned()
            ),
            before
        );
    }
}
#[test]
fn uncertain_reload_requires_settlement_even_for_matching_text_and_clears_only_on_accept() {
    let mut buffer = buffer();
    buffer.provider_mut().unwrap().uncertain = Some(ProviderUncertain {
        text: Text::from_str("uploaded"),
        version: "v1".into(),
    });
    buffer.mark_write_uncertain();
    let source = buffer.provider_reload_source().unwrap();
    assert!(source.requires_choice && source.requires_settlement());
    assert!(
        source
            .clone()
            .prepare(
                "uploaded".into(),
                "new".into(),
                "v2".into(),
                false,
                &AtomicBool::new(false)
            )
            .is_err()
    );
    let candidate = source
        .prepare(
            "divergent".into(),
            "new".into(),
            "v2".into(),
            true,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(buffer.provider().unwrap().uncertain.is_some());
    buffer
        .accept_provider_reload(candidate, ProviderReloadChoice::KeepLocal)
        .unwrap();
    assert!(buffer.provider().unwrap().uncertain.is_none());
    assert!(buffer.dirty);
}
#[test]
fn reload_limits_and_cancellation_refuse_before_any_live_mutation() {
    let mut buffer = buffer();
    let history = buffer.history_len();
    assert!(
        buffer
            .provider_reload_source()
            .unwrap()
            .prepare(
                "new".into(),
                "g".into(),
                "v".into(),
                false,
                &AtomicBool::new(true)
            )
            .is_err()
    );
    for remote in ["binary\0".into(), "x".repeat(8 * 1024 * 1024 + 1)] {
        assert!(
            buffer
                .provider_reload_source()
                .unwrap()
                .prepare(
                    remote,
                    "g".into(),
                    "v".into(),
                    false,
                    &AtomicBool::new(false)
                )
                .is_err()
        );
    }
    assert_eq!(buffer.history_len(), history);
    buffer.apply(&Transaction::insert(0, "x".repeat(8 * 1024 * 1024)));
    assert!(buffer.provider_reload_source().is_err());
}
