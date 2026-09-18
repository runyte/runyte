// SPDX-License-Identifier: MPL-2.0

use super::*;
use serde_json::json;

fn request(method: &str, params: serde_json::Value) -> String {
    json!({"type":"request","id":"r:1","method":method,"params":params}).to_string()
}

#[test]
fn authentication_is_strict_and_does_not_accept_other_messages() {
    let valid = json!({"type":"authenticate","credential":"a".repeat(64)}).to_string();
    assert!(parse_authentication(&valid).is_ok());
    for invalid in [
        valid.replace("authenticate", "register"),
        valid.replace(&"a".repeat(64), &"A".repeat(64)),
        valid.replace(&"a".repeat(64), &"a".repeat(63)),
        valid.replace("\"type\":", "\"capability\":\"terminal_read\",\"type\":"),
        valid.replace("\"type\":", "\"type\":\"authenticate\",\"type\":"),
    ] {
        assert!(parse_authentication(&invalid).is_err());
    }
}

fn registration() -> serde_json::Value {
    json!({"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.4.0","name":"Local bridge",
        "required_features":[FEATURE],"optional_features":[],"commands":[],
        "required_capabilities":["terminal_read"],"optional_capabilities":["editor_context_read"]})
}

#[test]
fn registration_requires_profile_and_never_grants_generic_plugin_authority() {
    let valid = registration();
    assert!(parse_registration(&valid.to_string()).is_ok());
    for (key, value) in [
        ("version", json!("runyte-2")),
        ("required_features", json!([])),
        ("optional_features", json!([FEATURE])),
        ("required_features", json!([FEATURE, "unknown"])),
        ("required_features", json!([FEATURE, FEATURE])),
        ("required_capabilities", json!(["text"])),
        ("optional_capabilities", json!(["terminal_read"])),
        ("name", json!("name\nspoof")),
        ("runyte", json!("*")),
        ("settings_schema", json!({})),
        (
            "commands",
            json!([{"name":"write","description":"x","context":"workspace"}]),
        ),
    ] {
        let mut invalid = valid.clone();
        invalid[key] = value;
        assert!(
            parse_registration(&invalid.to_string()).is_err(),
            "{invalid}"
        );
    }
    let duplicate = valid
        .to_string()
        .replace("\"name\":", "\"name\":\"other\",\"name\":");
    assert!(parse_registration(&duplicate).is_err());
}

#[test]
fn scopes_require_reads_and_unknown_scopes_are_rejected() {
    for scope in [
        Scope::TerminalRead,
        Scope::EditorContextRead,
        Scope::BufferEdit,
        Scope::TerminalPropose,
    ] {
        assert_eq!(Scope::from_capability(scope.capability()), Some(scope));
        assert_eq!(
            serde_json::to_value(scope).unwrap(),
            json!(scope.capability())
        );
    }
    assert_eq!(Scope::from_capability("terminal_input"), None);
    assert!(serde_json::from_str::<Scope>("\"terminal_submit\"").is_err());
    assert!(validate_scopes(&BTreeSet::new()).is_ok());
    assert!(validate_scopes(&[Scope::TerminalPropose].into()).is_err());
    assert!(validate_scopes(&[Scope::BufferEdit].into()).is_err());
    assert!(
        validate_scopes(
            &[
                Scope::TerminalRead,
                Scope::TerminalPropose,
                Scope::EditorContextRead,
                Scope::BufferEdit
            ]
            .into()
        )
        .is_ok()
    );
}

#[test]
fn every_allowlisted_method_has_an_explicit_scope_and_round_trips() {
    for (method, params, scope) in [
        (
            "buffer.list",
            json!({"offset":0,"limit":10}),
            Scope::EditorContextRead,
        ),
        (
            "buffer.read",
            json!({"buffer":"b:1","expected_revision":"r:1","from":0,"to":3}),
            Scope::EditorContextRead,
        ),
        (
            "buffer.edit",
            json!({"buffer":"b:1","expected_revision":"r:1","changes":[{"from":0,"to":0,"text":"line\nnext\n"}]}),
            Scope::BufferEdit,
        ),
        (
            "buffer.append",
            json!({"buffer":"b:1","text":"line\nnext\n","expected_tail":"end"}),
            Scope::BufferEdit,
        ),
        (
            "buffer.snapshot.open",
            json!({"buffer":"b:1","expected_revision":"r:1"}),
            Scope::EditorContextRead,
        ),
        (
            "buffer.snapshot.read",
            json!({"snapshot":"s:1","from":0,"to":3}),
            Scope::EditorContextRead,
        ),
        (
            "buffer.snapshot.close",
            json!({"snapshot":"s:1"}),
            Scope::EditorContextRead,
        ),
        (
            "selection.get",
            json!({"pane":"p:1"}),
            Scope::EditorContextRead,
        ),
        ("pane.context.list", json!({}), Scope::EditorContextRead),
        (
            "pane.viewport.read",
            json!({"pane":"p:1","expected_revision":null,"max_rows":10,"max_bytes":1024,"max_cells":1024}),
            Scope::EditorContextRead,
        ),
        (
            "terminal.list",
            json!({"offset":0,"limit":10}),
            Scope::TerminalRead,
        ),
        (
            "terminal.read",
            json!({"terminal":"t:1","region":"tail","expected_revision":null,"max_rows":10,"max_bytes":1024,"max_cells":1024}),
            Scope::TerminalRead,
        ),
        (
            "terminal.snapshot.open",
            json!({"terminal":"t:1","region":"screen","expected_revision":"r:1","max_rows":10,"max_bytes":1024,"max_cells":1024}),
            Scope::TerminalRead,
        ),
        (
            "terminal.snapshot.read",
            json!({"snapshot":"s:1","offset":0,"limit":10}),
            Scope::TerminalRead,
        ),
        (
            "terminal.snapshot.close",
            json!({"snapshot":"s:1"}),
            Scope::TerminalRead,
        ),
        (
            "terminal.input.propose",
            json!({"terminal":"t:1","text":"echo hello"}),
            Scope::TerminalPropose,
        ),
        (
            "terminal.input.status",
            json!({"proposal":"i:1"}),
            Scope::TerminalPropose,
        ),
        (
            "terminal.input.cancel",
            json!({"proposal":"i:1"}),
            Scope::TerminalPropose,
        ),
    ] {
        let raw = request(method, params.clone());
        let parsed = parse_request(&raw).unwrap();
        assert_eq!(parsed.request.required_scope(), scope, "{method}");
        assert_eq!(
            serde_json::to_value(parsed).unwrap(),
            serde_json::from_str::<serde_json::Value>(&raw).unwrap()
        );
        let mut invalid = params;
        invalid["submit"] = json!(true);
        assert!(
            parse_request(&request(method, invalid)).is_err(),
            "{method}"
        );
    }
}

#[test]
fn profile_has_no_submission_approval_or_control_escape_hatch() {
    for method in [
        "terminal.input",
        "terminal.submit",
        "terminal.open",
        "terminal.input.approve",
        "buffer.open",
        "buffer.save",
        "selection.set",
        "command.execute",
        "context.grant",
        "keys",
        "workspace.attach",
    ] {
        assert!(
            parse_request(&request(method, json!({}))).is_err(),
            "{method}"
        );
    }
    for text in [
        "echo x\n",
        "echo x\r",
        "\u{1b}[201~",
        "x\t",
        "\u{7f}",
        "\u{85}",
        "\u{2028}",
        "\u{2029}",
        "",
    ] {
        assert!(
            parse_request(&request(
                "terminal.input.propose",
                json!({"terminal":"t:1","text":text})
            ))
            .is_err()
        );
    }
    assert!(
        parse_request(&request(
            "terminal.input.propose",
            json!({"terminal":"t:1","text":"literal \\n"})
        ))
        .is_ok()
    );
}

#[test]
fn duplicate_envelope_and_nested_fields_remain_visible_to_validation() {
    for raw in [
        r#"{"type":"request","id":"a","id":"b","method":"buffer.list","params":{"offset":0,"limit":1}}"#,
        r#"{"type":"request","id":"a","method":"buffer.list","params":{"offset":0,"limit":1,"limit":2}}"#,
        r#"{"type":"request","id":"a","method":"buffer.edit","params":{"buffer":"b:1","expected_revision":"r:1","changes":[{"from":0,"to":0,"text":"a","text":"b"}]}}"#,
        r#"{"type":"request","id":"a","method":"buffer.list","params":{"offset":0,"limit":1},"granted":true}"#,
    ] {
        assert!(parse_request(raw).is_err(), "{raw}");
    }
}

#[test]
fn bounds_reject_oversized_untrusted_work_before_host_dispatch() {
    for params in [
        json!({"offset":0,"limit":0}),
        json!({"offset":0,"limit":257}),
        json!({"offset":usize::MAX,"limit":1}),
    ] {
        assert!(parse_request(&request("terminal.list", params)).is_err());
    }
    for (rows, bytes, cells) in [
        (0, 1, 1),
        (1001, 1, 1),
        (1, 0, 1),
        (1, 262145, 1),
        (1, 1, 0),
        (1, 1, 262145),
    ] {
        assert!(parse_request(&request("terminal.read",json!({"terminal":"t:1","region":"tail","max_rows":rows,"max_bytes":bytes,"max_cells":cells}))).is_err());
    }
    for (from, to) in [(4, 3), (0, 262145)] {
        assert!(
            parse_request(&request(
                "buffer.read",
                json!({"buffer":"b:1","expected_revision":"r:1","from":from,"to":to})
            ))
            .is_err()
        );
    }
    for identifier in [
        "".to_owned(),
        "x".repeat(257),
        "x\n".to_owned(),
        "with space".to_owned(),
    ] {
        assert!(parse_request(&request("selection.get", json!({"pane":identifier}))).is_err());
    }
    assert!(parse_request(&" ".repeat(MAX_FRAME_BYTES + 1)).is_err());
    assert!(
        parse_request(&request(
            "terminal.input.propose",
            json!({"terminal":"t:1","text":"x".repeat(4097)})
        ))
        .is_err()
    );
    assert!(
        parse_request(&request(
            "buffer.edit",
            json!({"buffer":"b:1","expected_revision":"r:1","changes":[]})
        ))
        .is_err()
    );
    assert!(parse_request(&request("buffer.edit",json!({"buffer":"b:1","expected_revision":"r:1","changes":[{"from":2,"to":1,"text":""}]}))).is_err());
    assert!(parse_request(&request("buffer.edit",json!({"buffer":"b:1","expected_revision":"r:1","changes":[{"from":0,"to":0,"text":"x".repeat(editor::MAX_REPLACEMENT_BYTES+1)}]}))).is_err());
}

#[test]
fn advertised_context_limits_exclude_unavailable_plugin_resources() {
    let value = limits();
    assert_eq!(value["line_bytes"], MAX_FRAME_BYTES);
    assert_eq!(value["requests"], MAX_REQUESTS);
    assert_eq!(value["commands"], 0);
    assert_eq!(value["resources"]["views"], 0);
    assert_eq!(value["context"]["read_rows"], 1000);
    assert_eq!(value["context"]["proposal_text_bytes"], 4096);
    assert_eq!(
        value["context"]["edit_replacement_bytes"],
        editor::MAX_REPLACEMENT_BYTES
    );
}

#[test]
fn proposal_reason_is_optional_bounded_and_never_an_authorization_flag() {
    let valid = request(
        "terminal.input.propose",
        json!({"terminal":"t:1","text":"cargo test","reason":"Run the focused test suite"}),
    );
    assert!(parse_request(&valid).is_ok());
    for reason in ["x".repeat(257), "a\nb".into(), "\u{2028}".into()] {
        assert!(
            parse_request(&request(
                "terminal.input.propose",
                json!({"terminal":"t:1","text":"cargo test","reason":reason})
            ))
            .is_err()
        );
    }
    assert!(
        parse_request(&request(
            "terminal.input.propose",
            json!({"terminal":"t:1","text":"cargo test","reason":"approved","approved":true})
        ))
        .is_err()
    );
}

#[test]
fn buffer_append_is_a_strict_buffer_edit_request_with_advertised_bounds() {
    let parsed = parse_request(&request(
        "buffer.append",
        json!({"buffer":"b:1","text":"line\n","expected_tail":"end"}),
    ))
    .unwrap();
    assert_eq!(parsed.request.required_scope(), Scope::BufferEdit);
    assert!(
        parse_request(&request(
            "buffer.append",
            json!({"buffer":"b:1","text":"x"})
        ))
        .is_ok()
    );
    assert!(
        parse_request(&request(
            "buffer.append",
            json!({"buffer":"b:1","text":"x","expected_tail":null})
        ))
        .is_ok()
    );
    for params in [
        json!({"buffer":"b:1","text":"x","expected_revision":"r:1"}),
        json!({"buffer":"b:1"}),
        json!({"buffer":"b:1","text":""}),
        json!({"buffer":"b:1","text":"x","expected_tail":""}),
        json!({"buffer":"b 1","text":"x"}),
        json!({"buffer":"b:1","text":"x","expected_tail":"x".repeat(MAX_EXPECTED_TAIL_BYTES + 1)}),
    ] {
        assert!(parse_request(&request("buffer.append", params)).is_err());
    }
    let context = &limits()["context"];
    assert_eq!(context["append_tail_bytes"], MAX_EXPECTED_TAIL_BYTES);
    assert_eq!(context["append_preview_chars"], APPEND_PREVIEW_CHARS);
}
