// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::plugin::manager::{Action, Intent, Phase};
use std::time::Duration;

async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("managed worker must make progress without editor input")
        .expect("host retains event sender")
}

#[tokio::test]
#[cfg(unix)]
async fn plugin_manager_eight_saturated_owners_keep_slots_until_final_fifo_consumption() {
    let (root, mut host) = host();
    for index in 0..plugin::MAX_PLUGINS {
        let id = format!("owner-{index}");
        let program = root.join(&id);
        std::os::unix::fs::symlink(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
            &program,
        )
        .unwrap();
        // Registration is acknowledged before the sixteen producer permits are
        // saturated. The process then blocks on stdin until its owner stops it.
        std::fs::write(
            root.join(format!("{id}.behavior")),
            "read -r hello\nprintf '%s\\n' '{\"type\":\"register\",\"version\":\"runyte-experimental-1\",\"commands\":[{\"name\":\"upper\",\"description\":\"Fixture command\"}]}'\nread -r registered\nn=0\nwhile [ \"$n\" -lt 16 ]; do printf '{\"type\":\"subscribe\",\"request\":\"%s\",\"buffer\":\"0\"}\\n' \"$n\"; n=$((n + 1)); done\nwhile read -r line; do :; done\n",
        )
        .unwrap();
        let mut configured = config(&id);
        configured.executable = program;
        host.app.config.plugins.push(configured);
    }
    let mut events = host.start_plugins().unwrap();
    assert_eq!(host.plugin_workers.len(), plugin::MAX_PLUGINS);

    // Two replacements exercise fresh IDs and admission across more configured
    // generations than fit the channel's eight-worker producer allowance.
    for round in 0..3 {
        let owners: BTreeSet<_> = host.plugin_workers.keys().copied().collect();
        let mut counts = std::collections::BTreeMap::<usize, usize>::new();
        let mut held = Vec::new();
        while held.len() < plugin::MAX_PLUGINS * 16 {
            let event = next(&mut events).await;
            assert!(owners.contains(&event.plugin));
            let Ok(ClientMessage::Queued { message, .. }) = &event.result else {
                panic!("unexpected producer event: {:?}", event.result);
            };
            match message.as_ref() {
                ClientMessage::Register { .. } => {
                    host.handle_plugin_event(event);
                }
                ClientMessage::Subscribe { .. } => {
                    *counts.entry(event.plugin).or_default() += 1;
                    held.push(event);
                }
                other => panic!("unexpected producer message: {other:?}"),
            }
        }
        assert_eq!(counts.len(), plugin::MAX_PLUGINS);
        assert!(counts.values().all(|count| *count == 16));
        assert!(
            host.app
                .plugins
                .manager_entries
                .iter()
                .all(|entry| entry.phase == Phase::Running)
        );

        for index in 0..plugin::MAX_PLUGINS {
            let owner = host.app.plugins.manager_entries[index].owner;
            host.app.plugins.manager_intents.push_back(Intent {
                config_index: index,
                expected_owner: owner,
                action: if round < 2 {
                    Action::Restart
                } else {
                    Action::Stop
                },
            });
        }
        host.sync_plugin_observers();
        assert!(host.app.plugins.instances.is_empty());
        assert_eq!(host.plugin_workers.len(), plugin::MAX_PLUGINS);
        let next_owner = host.next_plugin_owner;
        let mut finals = Vec::new();
        while finals.len() < plugin::MAX_PLUGINS {
            let event = next(&mut events).await;
            assert!(owners.contains(&event.plugin));
            assert!(matches!(
                event.result,
                Ok(ClientMessage::WorkerStopped {
                    failure: None,
                    reaped: true
                })
            ));
            finals.push(event);
        }
        // Reaping alone is insufficient: all old producer events must precede
        // the final host consumption before the configured slot can be reused.
        assert_eq!(host.next_plugin_owner, next_owner);
        for event in held {
            host.handle_plugin_event(event);
            assert!(host.app.plugins.instances.is_empty());
            assert_eq!(host.next_plugin_owner, next_owner);
        }
        for (index, event) in finals.into_iter().enumerate() {
            let retired = event.plugin;
            host.handle_plugin_event(event);
            assert!(!host.plugin_workers.contains_key(&retired));
            if round < 2 {
                assert_eq!(host.plugin_workers.len(), plugin::MAX_PLUGINS);
                assert_eq!(host.app.plugins.instances.len(), index + 1);
                assert_eq!(host.next_plugin_owner, next_owner + index + 1);
                assert!(
                    host.app
                        .plugins
                        .instances
                        .keys()
                        .all(|id| !owners.contains(id))
                );
            }
        }
    }
    assert!(host.plugin_workers.is_empty());
    assert!(host.app.plugins.instances.is_empty());
    assert_eq!(host.next_plugin_owner, plugin::MAX_PLUGINS * 3);
    assert_eq!(host.app.plugin_active_job_count(), 0);
    host.take_plugin_presentation_change();
    host.sync_plugin_observers();
    assert!(!host.plugin_presentation_pending());
    assert!(events.try_recv().is_err());
}
