// SPDX-License-Identifier: MPL-2.0

use super::*;

fn server() -> Server {
    Server {
        bridge: Bridge::new(None, "agent".into(), Duration::from_millis(100)).unwrap(),
        initialized: false,
        ready: false,
    }
}
async fn start(server: &mut Server) {
    let reply = server.handle(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":PROTOCOL}})).await.unwrap();
    assert_eq!(reply["result"]["capabilities"], json!({"tools":{}}));
    assert_eq!(reply["result"]["instructions"], INSTRUCTIONS);
    assert!(
        server
            .handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await
            .is_none()
    );
}
#[tokio::test]
async fn catalog_is_stable_before_pairing_and_discovery_needs_no_storage() {
    let mut server = server();
    start(&mut server).await;
    let catalog = server
        .handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .await
        .unwrap();
    let tools = catalog["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 9);
    assert!(tools.iter().any(|t| t["name"] == "append_buffer"));
    assert!(
        tools
            .iter()
            .all(|t| !t["name"].as_str().unwrap().contains("snapshot"))
    );
    let found = server
        .bridge
        .call("find_resources", json!({}))
        .await
        .unwrap();
    assert_eq!(found["workspaces"], json!([]));
    assert_eq!(found["results"], json!([]));
    assert!(
        found["permission_help"]
            .as_str()
            .unwrap()
            .contains(":mcp agent")
    );
    assert_eq!(
        catalog,
        server
            .handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
            .await
            .unwrap()
    );
}
#[tokio::test]
async fn lifecycle_errors_arguments_and_bounded_frames() {
    let mut server = server();
    for message in [
        json!({}),
        json!({"jsonrpc":"2.0","id":true,"method":"ping"}),
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    ] {
        assert!(server.handle(message).await.unwrap().get("error").is_some());
    }
    start(&mut server).await;
    for (method, params) in [
        ("tools/list", json!({"cursor":"x"})),
        ("tools/call", json!({"name":"find_resources","submit":true})),
        ("unknown", json!({})),
        ("ping", json!([])),
    ] {
        assert!(
            server
                .handle(json!({"jsonrpc":"2.0","id":3,"method":method,"params":params}))
                .await
                .unwrap()
                .get("error")
                .is_some()
        );
    }
    for (name, args) in [
        ("unknown", json!({})),
        ("read_terminal", json!({})),
        ("find_resources", json!({"submit":true})),
        ("find_resources", json!({"limit":-1})),
        ("find_resources", json!({"kind":"pane"})),
        ("find_resources", json!({"include_hidden":0})),
        (
            "edit_buffer",
            json!({"buffer":"b","expected_revision":"r","changes":[]}),
        ),
    ] {
        assert_eq!(
            server.bridge.call(name, args).await.unwrap_err().code,
            "invalid_argument"
        );
    }
    assert_eq!(
        server
            .bridge
            .call("append_buffer", json!({"buffer":"invented","text":"x"}))
            .await
            .unwrap_err()
            .code,
        "stale"
    );
    assert!(parse(b"{no}").is_err());
    assert!(parse(&[255]).is_err());
    assert!(
        read_frame(&mut BufReader::new(&b"12345\n"[..]), 4)
            .await
            .is_err()
    );
    assert!(
        read_frame(&mut BufReader::new(&b"123"[..]), 4)
            .await
            .is_err()
    );
    assert!(
        write_frame(&mut tokio::io::sink(), &json!({"big":"text"}), 2)
            .await
            .is_err()
    );
    let mut output = Vec::new();
    server
        .run(
            BufReader::new(&b"bad\n{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"ping\"}\n"[..]),
            &mut output,
        )
        .await
        .unwrap();
    assert!(String::from_utf8(output).unwrap().contains("Invalid JSON"));
}

#[cfg(unix)]
#[path = "tests/host.rs"]
mod host;
