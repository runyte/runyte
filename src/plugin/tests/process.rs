// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::plugin::{
    application as api,
    observation::{Delivery, Registry, Snapshot, Source},
};

fn start() -> Start {
    Start {
        label: "Indexer 猫".into(),
        executable: "/usr/bin/worker".into(),
        args: vec![],
        cwd: None,
        capture_stderr: false,
    }
}

#[test]
fn process_start_validates_byte_limits_and_passes_arguments_without_shell_interpretation() {
    let mut value = start();
    value.args = vec![
        "".into(),
        "literal;$(command)".into(),
        "line\nargument".into(),
    ];
    value.validate().unwrap();
    value.args = vec!["x".repeat(1024); MAX_ARGUMENTS];
    value.validate().unwrap();
    value.args.push("".into());
    assert_eq!(value.validate().unwrap_err().code, ErrorCode::LimitExceeded);
    value.args = vec!["x".repeat(MAX_ARGUMENT_LENGTH + 1)];
    assert_eq!(value.validate().unwrap_err().code, ErrorCode::LimitExceeded);
    value.args = vec!["x".repeat(MAX_ARGUMENT_LENGTH); 17];
    assert_eq!(value.validate().unwrap_err().code, ErrorCode::LimitExceeded);
    value.args = vec!["null\0argument".into()];
    assert_eq!(
        value.validate().unwrap_err().code,
        ErrorCode::InvalidArgument
    );
}

#[test]
fn process_start_refuses_unsafe_labels_and_invalid_operating_system_strings() {
    for label in [
        "".into(),
        "x".repeat(161),
        "hidden\nlabel".into(),
        "猫".repeat(54),
    ] {
        let mut value = start();
        value.label = label;
        assert_eq!(
            value.validate().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
    for executable in ["".into(), "x".repeat(4097), "null\0program".into()] {
        let mut value = start();
        value.executable = executable;
        assert_eq!(
            value.validate().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
    for cwd in ["".into(), "x".repeat(4097), "null\0directory".into()] {
        let mut value = start();
        value.cwd = Some(cwd);
        assert_eq!(
            value.validate().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
}

#[test]
fn process_wire_defaults_and_strict_stream_names_preserve_public_shapes() {
    let request: api::Request = serde_json::from_value(serde_json::json!({
        "method":"process.start","params":{"label":"Worker","executable":"worker"}
    }))
    .unwrap();
    let api::Request::ProcessStart(value) = request else {
        panic!("wrong request")
    };
    assert!(value.args.is_empty());
    assert!(value.cwd.is_none());
    assert!(!value.capture_stderr);
    for stream in ["STDOUT", "output", ""] {
        assert!(serde_json::from_value::<api::Request>(serde_json::json!({
            "method":"process.read","params":{"process":"pr:1","stream":stream,"offset":0,"limit":1}
        })).is_err());
    }
    assert!(
        serde_json::from_value::<api::Request>(serde_json::json!({
            "method":"process.start","params":{"label":"Worker","executable":"worker","shell":true}
        }))
        .is_err()
    );
    let write: api::Request = serde_json::from_value(serde_json::json!({
        "method":"process.write","params":{"process":"pr:1","data":""}
    }))
    .unwrap();
    assert!(matches!(
        write,
        api::Request::ProcessWrite { eof: false, .. }
    ));
}

#[test]
fn process_base64_is_binary_exact_canonical_and_bounded_before_decode() {
    let bytes: Vec<_> = (0..MAX_IO_BYTES).map(|index| index as u8).collect();
    assert_eq!(decode(&encode(&bytes)).unwrap(), bytes);
    assert!(decode("").unwrap().is_empty());
    assert_eq!(
        decode(&encode(&vec![0; MAX_IO_BYTES + 1]))
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded
    );
    assert_eq!(
        decode(&"A".repeat(MAX_IO_BYTES * 2)).unwrap_err().code,
        ErrorCode::LimitExceeded
    );
    for malformed in ["Zg", "Zh==", "Zg==\n", "____", "💥"] {
        assert_eq!(
            decode(malformed).unwrap_err().code,
            ErrorCode::InvalidArgument,
            "{malformed}"
        );
    }
}

#[test]
fn process_ring_wraps_and_reports_truncation_with_monotonic_byte_offsets() {
    let mut ring = Ring::new(5);
    ring.append(b"abc").unwrap();
    ring.append(b"defg").unwrap();
    assert_eq!(ring.bounds(), Bounds { start: 2, end: 7 });
    assert_eq!(ring.read(2, 3, false).unwrap().bytes, b"cde");
    assert_eq!(ring.read(5, 5, false).unwrap().bytes, b"fg");
    assert_eq!(ring.read(1, 1, false).unwrap_err().code, ErrorCode::Stale);
    assert_eq!(
        ring.read(8, 1, false).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    ring.append(b"0123456789").unwrap();
    assert_eq!(ring.bounds(), Bounds { start: 12, end: 17 });
    assert_eq!(ring.read(12, 5, true).unwrap().bytes, b"56789");
    let before = ring.bounds();
    ring.append(b"").unwrap();
    assert_eq!(ring.bounds(), before);
}

#[test]
fn process_ring_eof_requires_exit_and_the_exact_end_not_just_empty_available_output() {
    let mut ring = Ring::new(8);
    assert!(!ring.read(0, 1, false).unwrap().eof);
    assert!(ring.read(0, 1, true).unwrap().eof);
    ring.append(&[0, 0xff, 0xc3, 0xa9]).unwrap();
    let prefix = ring.read(0, 2, true).unwrap();
    assert_eq!(prefix.bytes, [0, 0xff]);
    assert_eq!(prefix.next, 2);
    assert!(!prefix.eof);
    let end = ring.read(2, 8, true).unwrap();
    assert_eq!(end.next, 4);
    assert!(end.eof);
    assert!(!ring.read(4, 1, false).unwrap().eof);
    for limit in [0, MAX_IO_BYTES + 1, usize::MAX] {
        assert_eq!(
            ring.read(0, limit, false).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
}

#[test]
fn process_rings_split_the_fixed_output_budget_and_reads_never_exceed_chunk_limit() {
    for capacity in [MAX_OUTPUT_BYTES, MAX_OUTPUT_BYTES / 2] {
        let mut ring = Ring::new(capacity);
        ring.append(&vec![b'x'; MAX_OUTPUT_BYTES + 17]).unwrap();
        let bounds = ring.bounds();
        assert_eq!(bounds.end - bounds.start, capacity as u64);
        assert_eq!(
            ring.read(bounds.start, MAX_IO_BYTES, true)
                .unwrap()
                .bytes
                .len(),
            MAX_IO_BYTES
        );
    }
}

#[test]
fn process_output_coalesces_but_stdin_close_and_exit_are_reliable() {
    let source = Source::Process {
        process: "pr:g:1".into(),
    };
    let state = |phase, closed, end| Snapshot::Process {
        state: phase,
        stdout: Bounds { start: 0, end },
        stderr: Bounds::default(),
        stdin_closed: closed,
        output_truncated: false,
        exit_code: (phase == State::Exited).then_some(7),
        signal: None,
    };
    let mut registry = Registry::default();
    registry
        .subscribe(
            "s:1".into(),
            vec![source.clone()],
            vec![(source.clone(), state(State::Running, false, 0))],
        )
        .unwrap();
    registry
        .observe(&source, state(State::Running, false, 1))
        .unwrap();
    registry
        .observe(&source, state(State::Running, false, 2))
        .unwrap();
    assert!(matches!(registry.peek_ready(),Some(Delivery::Changed(change)) if change.coalesced==1));
    registry
        .observe(&source, state(State::Running, true, 2))
        .unwrap();
    assert!(matches!(
        registry.peek_ready(),
        Some(Delivery::ReliableChanged(_))
    ));
    registry.ack_ready();
    registry
        .observe(&source, state(State::Closing, true, 2))
        .unwrap();
    assert!(matches!(
        registry.peek_ready(),
        Some(Delivery::ReliableChanged(_))
    ));
    registry.ack_ready();
    registry
        .observe(&source, state(State::Exited, true, 3))
        .unwrap();
    match registry.peek_ready().unwrap() {
        Delivery::ReliableChanged(change) => assert!(matches!(
            change.sources[0].state,
            Snapshot::Process {
                state: State::Exited,
                exit_code: Some(7),
                ..
            }
        )),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn process_methods_are_recognized_by_the_public_envelope_decoder() {
    for (method, params) in [
        (
            "process.start",
            serde_json::json!({"label":"Worker","executable":"worker"}),
        ),
        ("process.get", serde_json::json!({"process":"pr:g:1"})),
        (
            "process.read",
            serde_json::json!({"process":"pr:g:1","stream":"stderr","offset":0,"limit":1}),
        ),
        (
            "process.write",
            serde_json::json!({"process":"pr:g:1","data":"AA==","eof":true}),
        ),
        ("process.close", serde_json::json!({"process":"pr:g:1"})),
    ] {
        let bytes = serde_json::to_vec(
            &serde_json::json!({"type":"request","id":"p:1","method":method,"params":params}),
        )
        .unwrap();
        let crate::plugin::ClientMessage::Application(api::ClientMessage::Request {
            request, ..
        }) = api::decode(&bytes).unwrap()
        else {
            panic!("{method}")
        };
        match (method, request) {
            ("process.start", api::Request::ProcessStart(start)) => {
                assert_eq!(start.label, "Worker");
                assert_eq!(start.executable, "worker");
                assert!(start.args.is_empty());
            }
            ("process.get", api::Request::ProcessGet { process })
            | ("process.close", api::Request::ProcessClose { process }) => {
                assert_eq!(process, "pr:g:1")
            }
            (
                "process.read",
                api::Request::ProcessRead {
                    process,
                    stream,
                    offset,
                    limit,
                },
            ) => {
                assert_eq!(
                    (process.as_str(), stream, offset, limit),
                    ("pr:g:1", Stream::Stderr, 0, 1)
                );
            }
            ("process.write", api::Request::ProcessWrite { process, data, eof }) => {
                assert_eq!(
                    (process.as_str(), data.as_str(), eof),
                    ("pr:g:1", "AA==", true)
                );
            }
            other => panic!("Unexpected process decoder result: {other:?}"),
        }
    }
}

#[test]
fn process_argument_decode_refuses_structural_expansion_before_retaining_the_array() {
    let mut wire =
        serde_json::json!({"label":"Helper","executable":"helper","args":vec![""; MAX_ARGUMENTS]});
    assert_eq!(
        serde_json::from_value::<Start>(wire.clone())
            .unwrap()
            .args
            .len(),
        MAX_ARGUMENTS
    );
    wire["args"] = serde_json::json!(vec![""; MAX_ARGUMENTS + 1]);
    assert!(serde_json::from_value::<Start>(wire.clone()).is_err());
    let frame =
        serde_json::json!({"type":"request","id":"p:1","method":"process.start","params":wire});
    assert!(api::decode(&serde_json::to_vec(&frame).unwrap()).is_err());
}
