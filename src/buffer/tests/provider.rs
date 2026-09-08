// SPDX-License-Identifier: MPL-2.0

use super::*;

fn document() -> ProviderDocument {
    ProviderDocument {
        identity: ProviderIdentity {
            configured_plugin: "test".into(),
            provider: "memory".into(),
            key: "opaque-private-resource".into(),
        },
        label: "Notes".into(),
        syntax_hint: Some("rust".into()),
        version: "v1".into(),
        generation: "g1".into(),
        available: true,
        baseline_epoch: 0,
        uncertain: None,
    }
}

#[test]
fn provider_text_has_an_editable_baseline_without_local_identity() {
    let mut buffer = Buffer::provider_document(document(), "é猫\r\n".into());
    assert!(!buffer.dirty);
    assert!(!buffer.is_read_only());
    assert!(!buffer.is_special());
    assert!(buffer.path.is_none());
    assert!(buffer.file_observation_request(0).is_none());
    assert_eq!(buffer.display_name(), "[remote] Notes");
    assert!(!buffer.undo());
    buffer.apply(&Transaction::insert(0, "edited "));
    buffer.commit_undo_group();
    assert!(buffer.holds_unsaved_work());
    assert!(buffer.undo());
    assert!(!buffer.dirty);
    assert!(buffer.redo());
    buffer.provider_mut().unwrap().available = false;
    assert!(buffer.display_name().contains("[unavailable]"));
    assert!(!buffer.is_read_only());
    buffer.discard_provider_changes().unwrap();
    assert_eq!(buffer.to_string(), "é猫\r\n");
    assert!(!buffer.dirty);
    assert!(!buffer.undo());
    assert!(!buffer.redo());
    assert_eq!(buffer.provider().unwrap().version, "v1");
}

#[test]
fn provider_local_save_and_reload_refuse_without_changing_text_or_identity() {
    let mut buffer = Buffer::provider_document(document(), "baseline".into());
    buffer.apply(&Transaction::insert(0, "local "));
    buffer.commit_undo_group();
    let before = buffer.to_string();
    for force in [false, true] {
        assert!(buffer.save(force).is_err());
        assert!(
            buffer
                .save_as_with(PathBuf::from("unused-local-path"), force, |_, _| panic!(
                    "provider reached local write"
                ))
                .is_err()
        );
    }
    assert!(buffer.reload().is_err());
    assert_eq!(buffer.to_string(), before);
    assert!(buffer.provider().is_some());
    assert!(buffer.path.is_none());
    assert!(buffer.dirty);
    assert!(buffer.undo());
}

fn append(buffer: &mut Buffer, text: &str) {
    buffer.apply(&Transaction::insert(buffer.len_chars(), text));
    buffer.commit_undo_group();
}

#[test]
fn provider_save_accepts_only_captured_text_while_later_edits_and_undo_remain_live() {
    let mut buffer = Buffer::provider_document(document(), "a".into());
    append(&mut buffer, "é");
    let save = buffer.prepare_provider_save().unwrap();
    append(&mut buffer, "猫");
    assert!(buffer.accept_provider_save(save, "v2".into()));
    assert_eq!(buffer.to_string(), "aé猫");
    assert!(buffer.dirty);
    assert_eq!(buffer.provider().unwrap().baseline_epoch, 1);
    assert_eq!(buffer.prepare_provider_save().unwrap().version, "v2");
    assert!(buffer.undo());
    assert_eq!(buffer.to_string(), "aé");
    assert!(!buffer.dirty);
    assert!(buffer.undo());
    assert_eq!(buffer.to_string(), "a");
    assert!(buffer.dirty);
    assert!(buffer.redo());
    assert!(!buffer.dirty);
    assert!(buffer.path.is_none());
    assert!(buffer.disk_state.is_none());
}

#[test]
fn undo_during_upload_does_not_change_the_accepted_provider_snapshot() {
    let mut buffer = Buffer::provider_document(document(), "a".into());
    append(&mut buffer, "b");
    let save = buffer.prepare_provider_save().unwrap();
    assert!(buffer.undo());
    assert!(!buffer.dirty);
    assert!(buffer.accept_provider_save(save, "v2".into()));
    assert!(buffer.dirty);
    assert_eq!(buffer.to_string(), "a");
    assert!(buffer.redo());
    assert_eq!(buffer.to_string(), "ab");
    assert!(!buffer.dirty);
}

#[test]
fn provider_save_completion_rejects_stale_identity_generation_epoch_and_version() {
    for field in ["identity", "generation", "epoch", "version", "unavailable"] {
        let mut buffer = Buffer::provider_document(document(), "a".into());
        append(&mut buffer, "b");
        let save = buffer.prepare_provider_save().unwrap();
        let metadata = buffer.provider_mut().unwrap();
        match field {
            "identity" => metadata.identity.key.push_str("-other"),
            "generation" => metadata.generation.push_str("-new"),
            "epoch" => metadata.baseline_epoch += 1,
            "version" => metadata.version.push_str("-new"),
            "unavailable" => metadata.available = false,
            _ => unreachable!(),
        }
        let before = buffer.provider().unwrap().clone();
        let revision = buffer.revision();
        assert!(
            !buffer.accept_provider_save(save, "committed".into()),
            "{field}"
        );
        assert_eq!(buffer.provider(), Some(&before));
        assert_eq!(buffer.revision(), revision);
        assert_eq!(buffer.saved_text.as_ref().unwrap().to_string(), "a");
        assert!(buffer.dirty);
    }
}

#[test]
fn provider_save_snapshot_requires_available_reconciled_bounded_text() {
    assert!(Buffer::scratch().prepare_provider_save().is_err());
    let mut buffer = Buffer::provider_document(document(), "a".into());
    buffer.provider_mut().unwrap().available = false;
    assert!(buffer.prepare_provider_save().is_err());
    buffer.provider_mut().unwrap().available = true;
    let save = buffer.prepare_provider_save().unwrap();
    assert!(buffer.mark_provider_uncertain(save));
    assert!(buffer.prepare_provider_save().is_err());
    let buffer = Buffer::provider_document(document(), "x".repeat(8 * 1024 * 1024 + 1));
    assert!(buffer.prepare_provider_save().is_err());
    assert!(!buffer.dirty);
}

#[test]
fn provider_save_rejects_edited_binary_nul_without_changing_text_or_baseline() {
    let mut buffer = Buffer::provider_document(document(), "base".into());
    append(&mut buffer, "\0");
    let revision = buffer.revision();
    assert!(
        buffer
            .prepare_provider_save()
            .unwrap_err()
            .to_string()
            .contains("NUL")
    );
    assert_eq!(buffer.to_string(), "base\0");
    assert_eq!(buffer.revision(), revision);
    assert_eq!(buffer.saved_text.as_ref().unwrap().to_string(), "base");
    assert_eq!(buffer.provider().unwrap().baseline_epoch, 0);
    assert!(buffer.dirty);
    assert!(buffer.undo());
    assert!(buffer.prepare_provider_save().is_ok());
}

#[test]
fn uncertain_provider_write_survives_discard_and_rejects_late_acceptance() {
    let mut buffer = Buffer::provider_document(document(), "a".into());
    append(&mut buffer, "b");
    let save = buffer.prepare_provider_save().unwrap();
    append(&mut buffer, "c");
    buffer.provider_mut().unwrap().available = false;
    assert!(buffer.mark_provider_uncertain(save.clone()));
    assert!(!buffer.mark_provider_uncertain(save.clone()));
    assert!(
        buffer
            .discard_changes_to("unvalidated replacement")
            .is_err()
    );
    assert_eq!(buffer.to_string(), "abc");
    buffer.discard_provider_changes().unwrap();
    assert_eq!(buffer.to_string(), "a");
    assert!(buffer.dirty);
    assert!(buffer.holds_unsaved_work());
    assert!(buffer.prepare_provider_save().is_err());
    assert!(!buffer.undo());
    assert!(!buffer.redo());
    let unknown = buffer.provider().unwrap().uncertain.as_ref().unwrap();
    assert_eq!(unknown.text.to_string(), "ab");
    assert_eq!(unknown.version, "v1");
    buffer.provider_mut().unwrap().available = true;
    assert!(!buffer.accept_provider_save(save, "late".into()));
    assert_eq!(buffer.provider().unwrap().version, "v1");
    assert!(buffer.dirty);
}

#[test]
fn provider_reconciliation_accepts_only_known_baselines_and_preserves_live_history() {
    for committed in [false, true] {
        let mut buffer = Buffer::provider_document(document(), "a".into());
        append(&mut buffer, "b");
        let save = buffer.prepare_provider_save().unwrap();
        assert!(buffer.mark_provider_uncertain(save.clone()));
        append(&mut buffer, "c");
        buffer.provider_mut().unwrap().available = false;
        let identity = buffer.provider().unwrap().identity.clone();
        let revision = buffer.revision();
        let history = buffer.history_len();
        buffer
            .reconcile_provider(
                &identity,
                "g2".into(),
                "v2".into(),
                if committed { "ab" } else { "a" },
            )
            .unwrap();
        assert_eq!(buffer.to_string(), "abc");
        assert_eq!(buffer.revision(), revision);
        assert_eq!(buffer.history_len(), history);
        assert!(buffer.dirty);
        let metadata = buffer.provider().unwrap();
        assert_eq!(metadata.generation, "g2");
        assert_eq!(metadata.version, "v2");
        assert_eq!(metadata.baseline_epoch, 1);
        assert!(metadata.available);
        assert!(metadata.uncertain.is_none());
        assert!(buffer.prepare_provider_save().is_ok());
        assert!(!buffer.accept_provider_save(save, "stale-completion".into()));
        assert!(buffer.undo());
        assert_eq!(buffer.to_string(), "ab");
        assert_eq!(buffer.dirty, !committed);
    }
}

#[test]
fn divergent_or_foreign_provider_reconciliation_changes_nothing() {
    let mut buffer = Buffer::provider_document(document(), "a".into());
    append(&mut buffer, "b");
    let save = buffer.prepare_provider_save().unwrap();
    assert!(buffer.mark_provider_uncertain(save));
    append(&mut buffer, "c");
    buffer.provider_mut().unwrap().available = false;
    let before = buffer.provider().unwrap().clone();
    let revision = buffer.revision();
    let history = buffer.history_len();
    let mut foreign = before.identity.clone();
    foreign.key = "other".into();
    for (identity, remote) in [(&before.identity, "remote divergence"), (&foreign, "ab")] {
        let error = buffer
            .reconcile_provider(identity, "g2".into(), "v2".into(), remote)
            .unwrap_err();
        assert!(error.is::<ProviderConflict>());
        assert_eq!(buffer.provider(), Some(&before));
        assert_eq!(buffer.to_string(), "abc");
        assert_eq!(buffer.revision(), revision);
        assert_eq!(buffer.history_len(), history);
        assert!(buffer.dirty);
    }
}

#[test]
fn explicit_provider_rebind_to_old_baseline_preserves_edits_without_an_unknown_write() {
    let mut buffer = Buffer::provider_document(document(), "a".into());
    append(&mut buffer, "local");
    let old = buffer.prepare_provider_save().unwrap();
    let identity = buffer.provider().unwrap().identity.clone();
    buffer.provider_mut().unwrap().available = false;
    buffer
        .reconcile_provider(&identity, "g2".into(), "v2".into(), "a")
        .unwrap();
    assert_eq!(buffer.to_string(), "alocal");
    assert!(buffer.dirty);
    assert!(buffer.prepare_provider_save().is_ok());
    assert!(!buffer.mark_provider_uncertain(old));
    assert!(buffer.undo());
    assert!(!buffer.dirty);
}
