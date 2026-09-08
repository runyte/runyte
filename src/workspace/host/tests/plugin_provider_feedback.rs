// SPDX-License-Identifier: MPL-2.0

use super::provider_writes::{document, finished, upload};
use super::providers::{reply, resource_request};
use super::*;
use crate::{input::KeyStroke, plugin::provider as wire};

fn native_upload(
    host: &mut WorkspaceHost,
) -> (
    mpsc::Receiver<HostMessage>,
    mpsc::Receiver<HostMessage>,
    usize,
    String,
) {
    let (requester, mut provider, _, index) = document(host, "baseline\n");
    host.app.show_provider_document(index);
    host.app.buffers[index].apply(&Transaction::insert(0, "local "));
    type_command(host, "write");
    assert!(
        host.app
            .displayed_status_message()
            .contains("Provider save pending")
    );
    host.sync_plugin_observers();
    let begin = resource_request(&mut provider);
    assert!(matches!(
        next(&mut provider),
        api::HostMessage::Event {
            event: "job.changed",
            ..
        }
    ));
    let (commit, captured) = upload(host, &mut provider, begin);
    assert_eq!(captured, "local baseline\n");
    assert!(host.app.buffers[index].dirty);
    (requester, provider, index, commit)
}

#[test]
fn native_provider_success_replaces_pending_action_echo_without_later_input() {
    let (_root, mut host) = host();
    let (_requester, mut provider, index, commit) = native_upload(&mut host);
    host.take_plugin_presentation_change();
    reply(
        &mut host,
        commit,
        wire::Response::WriteCommitted {
            version: "version-2".into(),
        },
    );
    finished(&mut provider, api::JobState::Succeeded);
    assert!(!host.app.buffers[index].dirty);
    assert_eq!(
        host.app.displayed_status_message(),
        ":write (Remote document saved)"
    );
    assert!(!host.app.displayed_status_message_is_error());
    assert!(host.plugin_presentation_pending());
    assert!(host.provider_writes.is_empty());
}

#[test]
fn native_provider_failure_and_unknown_settle_pending_echo_with_failure_styling() {
    for code in [api::ErrorCode::Conflict, api::ErrorCode::OutcomeUnknown] {
        let (_root, mut host) = host();
        let (_requester, mut provider, index, commit) = native_upload(&mut host);
        reply(
            &mut host,
            commit,
            wire::Response::WriteRejected {
                error: api::Error::new(code.clone(), "Remote version changed"),
            },
        );
        let unknown = code == api::ErrorCode::OutcomeUnknown;
        finished(
            &mut provider,
            if unknown {
                api::JobState::OutcomeUnknown
            } else {
                api::JobState::Failed
            },
        );
        let echo = host.app.displayed_status_message();
        assert!(echo.starts_with(":write ("), "{echo}");
        assert!(!echo.contains("Provider save pending"), "{echo}");
        assert!(
            echo.contains(if unknown {
                "may have committed"
            } else {
                "Remote version changed"
            }),
            "{echo}"
        );
        assert!(host.app.displayed_status_message_is_error());
        assert!(host.app.buffers[index].dirty);
        assert_eq!(host.app.buffers[index].to_string(), "local baseline\n");
        assert_eq!(
            host.app.buffers[index]
                .provider()
                .unwrap()
                .uncertain
                .is_some(),
            unknown
        );
    }
}

#[test]
fn native_provider_completion_preserves_a_later_action_echo() {
    for success in [true, false] {
        let (_root, mut host) = host();
        let (_requester, mut provider, _, commit) = native_upload(&mut host);
        host.app.handle_key(KeyStroke::char('l')).unwrap();
        let later = host.app.displayed_status_message().to_owned();
        let later_error = host.app.displayed_status_message_is_error();
        assert!(!later.contains("Provider save pending"));
        reply(
            &mut host,
            commit,
            if success {
                wire::Response::WriteCommitted {
                    version: "version-2".into(),
                }
            } else {
                wire::Response::WriteRejected {
                    error: api::Error::new(api::ErrorCode::Conflict, "Remote version changed"),
                }
            },
        );
        finished(
            &mut provider,
            if success {
                api::JobState::Succeeded
            } else {
                api::JobState::Failed
            },
        );
        assert_eq!(host.app.displayed_status_message(), later);
        assert_eq!(host.app.displayed_status_message_is_error(), later_error);
    }
}

#[test]
fn native_provider_stale_admission_settles_its_echo_without_dispatching_upload() {
    let (_root, mut host) = host();
    let (_requester, mut provider, _, index) = document(&mut host, "baseline  \n");
    host.app.show_provider_document(index);
    type_command(&mut host, "write");
    host.app.buffers[index].apply(&Transaction::insert(0, "external edit "));
    host.sync_plugin_observers();
    let echo = host.app.displayed_status_message();
    assert!(
        echo.contains("Document changed before save admission"),
        "{echo}"
    );
    assert!(!echo.contains("Provider save pending"), "{echo}");
    assert!(host.app.displayed_status_message_is_error());
    assert!(host.provider_writes.is_empty());
    assert!(!host.app.document_mutation_pending(index));
    assert_eq!(
        host.app.buffers[index].to_string(),
        "external edit baseline  \n"
    );
    assert!(provider.try_recv().is_err());
}
