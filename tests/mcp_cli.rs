// SPDX-License-Identifier: MPL-2.0

#![cfg(unix)]

use runyte::test_support::TestRuntimeRoot;
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn mcp_stdio_starts_without_editor_or_permissions_and_rejects_duplicate_fields() {
    let root = TestRuntimeRoot::new("mcp-cli").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["mcp", "--identity", "fixture"])
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("RUNYTE_CONTEXT_HOME", root.path().join("context"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(input,"{}",json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).unwrap();
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})
    )
    .unwrap();
    writeln!(input,"{}",json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"find_resources","arguments":{}}})).unwrap();
    writeln!(
        input,
        "{{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"ping\",\"method\":\"tools/list\"}}"
    )
    .unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 4);
    assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 9);
    assert_eq!(
        replies[2]["result"]["structuredContent"]["workspaces"],
        json!([])
    );
    assert_eq!(replies[3]["error"]["code"], -32700);
    assert!(!root.path().join("context").exists());
    assert!(!root.path().join("config").exists());
}

#[test]
fn mcp_cli_validates_options_without_starting_the_editor() {
    let root = TestRuntimeRoot::new("mcp-args").unwrap();
    for args in [
        vec!["--unknown"],
        vec!["--identity", "bad identity"],
        vec!["--identity"],
        vec!["--timeout", "NaN"],
        vec!["--timeout"],
        vec!["--timeout", "0"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_runyte"))
            .arg("mcp")
            .args(args)
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["mcp", "--help"])
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().contains(":mcp"));
}
