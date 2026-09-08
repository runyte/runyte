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
