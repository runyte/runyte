// SPDX-License-Identifier: MPL-2.0

//! Configured-identity IO ownership persists through timeout and owner retirement.
use super::WorkspaceHost;
use crate::plugin::{self, application as api, state};
use api::{Error, ErrorCode as Code, Request, ResultValue};
use std::{sync::Arc, time::Instant};

pub(super) struct Pending {
    pub(super) owner: usize,
    generation: String,
    request: String,
    pub(super) token: String,
    pub(super) deadline: Instant,
    pub(super) control: Arc<state::Control>,
    orphaned: bool,
    replied: bool,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.control.cancel();
    }
}
pub(super) fn is_state_request(request: &Request) -> bool {
    matches!(
        request,
        Request::StateGet(_) | Request::StateSet { .. } | Request::StateDelete { .. }
    )
}
impl WorkspaceHost {
    pub(super) fn application_state_request(
        &mut self,
        owner: usize,
        request: &str,
        operation: Request,
    ) -> Result<(), Error> {
        let instance = &self.app.plugins.instances[&owner];
        if !instance.application.capabilities.contains("state") {
            return Err(Error::new(
                Code::CapabilityDenied,
                "State capability was not granted",
            ));
        }
        let identity = instance.config.id.clone();
        if self.plugin_state_requests.contains_key(&identity) {
            return Err(Error::new(
                Code::Busy,
                "Plugin state operation is still pending",
            ));
        }
        let task = match operation {
            Request::StateGet(_) => state::Task::Get,
            Request::StateSet {
                expected_revision,
                document,
            } => state::Task::Set {
                expected_revision,
                document,
            },
            Request::StateDelete { expected_revision } => state::Task::Delete { expected_revision },
            _ => return Err(Error::new(Code::Unsupported, "Unknown state operation")),
        };
        self.reserve_application_payload(owner, state::PREPARE_CHARGE)?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| Error::new(Code::Unavailable, "State runtime unavailable"))?;
        let sender = self
            .plugin_events_sender
            .clone()
            .ok_or_else(|| Error::new(Code::Unavailable, "State event channel unavailable"))?;
        let permit = self
            .plugin_local_slots
            .get_or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(16)))
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::new(Code::Busy, "Local operation service is busy"))?;
        let generation = self.app.plugins.instances[&owner]
            .application
            .generation
            .clone();
        let token = format!("state:{generation}:{request}");
        if self
            .plugin_send(
                owner,
                plugin::HostMessage::Deadline {
                    token: token.clone(),
                    after_ms: Some(10000),
                },
            )
            .is_err()
        {
            self.app
                .plugins
                .instances
                .get_mut(&owner)
                .unwrap()
                .application
                .deadlines
                .remove(&token);
            return Err(Error::new(
                Code::Unavailable,
                "State deadline service unavailable",
            ));
        }
        let control = Arc::new(state::Control::new());
        #[cfg(test)]
        if let Some(hook) = self.plugin_state_hook.clone() {
            control.set_hook(hook);
        }
        let instance = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        instance.retained_payload += state::PREPARE_CHARGE;
        instance.state_pending = true;
        self.plugin_state_requests.insert(
            identity.clone(),
            Pending {
                owner,
                generation: generation.clone(),
                request: request.into(),
                token: token.clone(),
                deadline: instance.deadlines[&token],
                control: control.clone(),
                orphaned: false,
                replied: false,
            },
        );
        let root = self.app.state_root.clone();
        let worker_identity = identity.clone();
        let permit = Arc::new(permit);
        let worker_permit = permit.clone();
        let task = runtime.spawn_blocking(move || {
            // Runtime teardown may drop the waiter while OS IO continues.
            // Both sides retain the same one shared service slot.
            let _permit = worker_permit;
            state::run(&root, &worker_identity, task, &control)
        });
        let request = request.to_owned();
        runtime.spawn(async move {
            let result = task.await.unwrap_or_else(|_| {
                Err(Error::new(
                    Code::OutcomeUnknown,
                    "State worker outcome is unknown; read state before retrying",
                ))
            });
            let _ = sender
                .send(plugin::Event {
                    plugin: owner,
                    result: Ok(plugin::ClientMessage::State(state::Event {
                        identity,
                        generation,
                        request,
                        result,
                        _permit: permit,
                    })),
                })
                .await;
        });
        Ok(())
    }
    pub(super) fn application_state_event(
        &mut self,
        owner: usize,
        event: state::Event,
    ) -> anyhow::Result<()> {
        let Some(pending) = self.plugin_state_requests.get(&event.identity) else {
            return Ok(());
        };
        if pending.owner != owner
            || pending.generation != event.generation
            || pending.request != event.request
        {
            return Ok(());
        }
        let pending = self.plugin_state_requests.remove(&event.identity).unwrap();
        if pending.orphaned {
            self.app.plugins.orphaned_payload -= state::PREPARE_CHARGE;
            self.app.plugins.state_orphans -= 1;
            return Ok(());
        }
        let Some(instance) = self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .filter(|instance| instance.application.generation == event.generation)
        else {
            return Ok(());
        };
        instance.application.retained_payload -= state::PREPARE_CHARGE;
        instance.application.state_pending = false;
        self.plugin_send(
            owner,
            plugin::HostMessage::Deadline {
                token: pending.token.clone(),
                after_ms: None,
            },
        )?;
        if !pending.replied {
            let result = if Instant::now() >= pending.deadline {
                Err(timeout(&pending.control))
            } else {
                event.result
            };
            self.application_local_reply(owner, event.request, result.map(ResultValue::State))?;
        }
        Ok(())
    }
    pub(super) fn state_deadline(&mut self, owner: usize, token: &str) -> anyhow::Result<bool> {
        if !token.starts_with("state:") {
            return Ok(false);
        }
        let pending = self
            .plugin_state_requests
            .values_mut()
            .find(|pending| !pending.orphaned && pending.owner == owner && pending.token == token);
        let Some(pending) = pending else {
            return Ok(true);
        };
        if pending.deadline > Instant::now() || pending.replied {
            return Ok(true);
        }
        pending.control.cancel();
        let error = timeout(&pending.control);
        pending.replied = true;
        let request = pending.request.clone();
        self.application_local_reply(owner, request, Err(error))?;
        Ok(true)
    }
    pub(super) fn stop_plugin_state(&mut self, owner: usize) {
        for pending in self
            .plugin_state_requests
            .values_mut()
            .filter(|pending| pending.owner == owner && !pending.orphaned)
        {
            pending.control.cancel();
            pending.orphaned = true;
            self.app.plugins.orphaned_payload += state::PREPARE_CHARGE;
            self.app.plugins.state_orphans += 1;
        }
    }
}
fn timeout(control: &state::Control) -> Error {
    if control.attempted() {
        Error::new(
            Code::OutcomeUnknown,
            "State mutation outcome is unknown; read state before retrying",
        )
    } else {
        Error::new(Code::Timeout, "State operation timed out before mutation")
    }
}
