// SPDX-License-Identifier: MPL-2.0
use super::*;
use serde_json::json;

fn finish(handle: &str, message: &str) -> api::Request {
    serde_json::from_value(
        json!({"method":"job.finish","params":{"job":handle,"state":"failed","message":message}}),
    )
    .unwrap()
}

#[test]
fn job_feedback_is_negotiated_bounded_owned_and_keeps_other_foreground_work() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["jobs"]);
    next(&mut output);
    request(
        &mut host,
        0,
        1,
        api::Request::JobCreate {
            title: "Show full value".into(),
            deadline_seconds: 60,
        },
    );
    let created = job(&mut output);
    next(&mut output);
    let before = host.app.plugins.instances[&0].application.retained_payload;
    assert_eq!(
        host.application_request(0, finish(&created.job, "Quota unavailable"))
            .unwrap_err()
            .code,
        api::ErrorCode::Unsupported
    );
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .features
        .insert(api::JOB_FEEDBACK.into());
    for message in ["".to_owned(), "bad\nmessage".into(), "x".repeat(1025)] {
        assert_eq!(
            host.application_request(0, finish(&created.job, &message))
                .unwrap_err()
                .code,
            api::ErrorCode::InvalidArgument
        );
    }
    assert_eq!(
        host.application_request(0, finish("unowned", "No capacity"))
            .unwrap_err()
            .code,
        api::ErrorCode::NotFound
    );
    let state = &host.app.plugins.instances[&0].application;
    assert_eq!(state.jobs[&created.job].state, api::JobState::Running);
    assert_eq!(state.retained_payload, before);
    assert!(host.app.notifications().entries().is_empty());
    host.app.status = "Later editor action".into();
    host.app.note_plugin_frontend(false);
    request(
        &mut host,
        0,
        2,
        finish(&created.job, "Not enough memory to show the full value"),
    );
    let finished = job(&mut output);
    next(&mut output);
    assert_eq!(
        finished.message.as_deref(),
        Some("Not enough memory to show the full value")
    );
    assert_eq!(finished.state, api::JobState::Failed);
    assert_eq!(host.app.status, "Later editor action");
    assert_eq!(
        host.app.notifications().entries()[0].body,
        "Not enough memory to show the full value"
    );
    assert!(host.app.plugins.instances[&0].application.retained_payload > before);
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .remove_job(&created.job);
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        before
    );
    assert!(
        serde_json::from_value::<api::Request>(
            json!({"method":"job.finish","params":{"job":"j:1","state":"failed","message":null}})
        )
        .is_err()
    );
}

#[test]
fn finishing_before_command_acceptance_preserves_feedback_without_duplicate_notification() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["jobs"]);
    next(&mut output);
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .features
        .insert(api::JOB_FEEDBACK.into());
    type_command(&mut host, "plugin.app-0.open");
    let api::HostMessage::Request { id, .. } = next(&mut output) else {
        panic!()
    };
    request(
        &mut host,
        0,
        1,
        api::Request::JobCreate {
            title: "Show full value".into(),
            deadline_seconds: 60,
        },
    );
    let created = job(&mut output);
    next(&mut output);
    request(
        &mut host,
        0,
        2,
        finish(&created.job, "Complete value unavailable"),
    );
    job(&mut output);
    next(&mut output);
    host.application_message(
        0,
        api::ClientMessage::Response {
            id,
            outcome: api::CommandResponse::Success {
                result: api::CommandResult {
                    job: Some(created.job),
                },
            },
        },
    )
    .unwrap();
    assert_eq!(host.app.notifications().entries().len(), 1);
    assert!(
        host.app
            .displayed_status_message()
            .contains("Complete value unavailable")
    );
    assert!(host.app.displayed_status_message_is_error());
}

#[test]
fn reserved_job_feedback_finishes_when_shared_payload_is_full() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["jobs"]);
    next(&mut output);
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .features
        .insert(api::JOB_FEEDBACK.into());
    request(
        &mut host,
        0,
        1,
        api::Request::JobCreate {
            title: "Full value".into(),
            deadline_seconds: 60,
        },
    );
    let created = job(&mut output);
    next(&mut output);
    let state = &mut host.app.plugins.instances.get_mut(&0).unwrap().application;
    assert_eq!(state.retained_payload, 1024);
    assert!(state.job_feedback_reservations.contains(&created.job));
    state.retained_payload = api::MAX_HOST_RETAINED_BYTES;
    request(
        &mut host,
        0,
        2,
        finish(&created.job, "Full value unavailable: memory limit"),
    );
    let finished = job(&mut output);
    next(&mut output);
    assert_eq!(finished.state, api::JobState::Failed);
    let state = &host.app.plugins.instances[&0].application;
    assert!(state.job_feedback_reservations.is_empty());
    assert_eq!(
        state.retained_payload,
        api::MAX_HOST_RETAINED_BYTES - 1024 + finished.message.unwrap().len()
    );
}
