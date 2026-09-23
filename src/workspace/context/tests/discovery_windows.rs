// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::workspace::{
    context::{
        storage::{HostMode, Registration, Storage, random_token},
        transport::{Event, Reply, Server},
    },
    windows_process_identity::ProcessIdentity,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

fn fixture(
    label: &str,
) -> (
    crate::test_support::TestRuntimeRoot,
    Arc<Storage>,
    Registration,
) {
    let root = crate::test_support::TestRuntimeRoot::new(label).unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let project = project.canonicalize().unwrap();
    let storage = Arc::new(Storage::new(root.path().join("store")).unwrap());
    let host_incarnation = random_token().unwrap();
    let process = ProcessIdentity::current().unwrap();
    let registration = Registration {
        workspace_id: crate::workspace::workspace_id(&project),
        root: project,
        endpoint: storage.socket_path(&host_incarnation).unwrap(),
        host_incarnation,
        mode: HostMode::Persistent,
        pid: process.pid,
        creation_time: process.creation_time,
        environment: random_token().unwrap(),
    };
    (root, storage, registration)
}

#[tokio::test]
async fn missing_native_store_is_empty_and_not_created() {
    let root = crate::test_support::TestRuntimeRoot::new("ctx-native-discovery").unwrap();
    let missing = root.path().join("missing");
    let result = discover(Some(missing.clone()), &"a".repeat(64), false)
        .await
        .unwrap();
    assert!(result.workspaces.is_empty());
    assert!(!missing.exists());
}

#[tokio::test]
async fn exact_native_probe_is_discovered_without_credentials() {
    let (_root, storage, registration) = fixture("ctx-native-discovery-live");
    let (send, mut events) = mpsc::channel(2);
    let mut server = Server::bind(storage.clone(), registration.clone(), send).unwrap();
    let discovery = discover(
        Some(storage.root().to_owned()),
        &registration.environment,
        false,
    );
    tokio::pin!(discovery);
    let found = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            tokio::select! {
                result = &mut discovery => break result.unwrap(),
                event = events.recv() => {
                    let Event::Frame { bytes, reply, .. } = event.unwrap() else { continue };
                    assert_eq!(bytes, b"{\"type\":\"probe\"}\n");
                    assert!(reply.send(Reply {
                        value: json!({"type":"context_endpoint", "registration":registration}),
                        lease: None,
                        close: true,
                    }).is_ok());
                }
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(found.workspaces.len(), 1);
    assert_eq!(found.workspaces[0].registration, registration);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn registered_process_identity_mismatch_is_not_probed_or_removed() {
    let (_root, storage, registration) = fixture("ctx-native-discovery-mismatch");
    let (send, mut events) = mpsc::channel(2);
    let mut server = Server::bind(storage.clone(), registration.clone(), send).unwrap();
    let path = storage
        .root()
        .join(format!("host-{}.json", registration.host_incarnation));
    let mut mismatched = registration.clone();
    mismatched.creation_time = mismatched.creation_time.checked_add(1).unwrap();
    std::fs::write(&path, serde_json::to_vec(&mismatched).unwrap()).unwrap();
    let found = discover(
        Some(storage.root().to_owned()),
        &registration.environment,
        false,
    )
    .await
    .unwrap();
    assert!(found.workspaces.is_empty());
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(100), events.recv()).await
    {
        assert!(matches!(event, Event::Closed(_)));
    }
    assert!(path.exists());
    server.shutdown().await.unwrap();
}
