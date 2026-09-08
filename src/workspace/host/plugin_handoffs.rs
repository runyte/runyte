// SPDX-License-Identifier: MPL-2.0

//! Foreground admission, asynchronous preparation, and native handoff ownership.
use super::WorkspaceHost;
use crate::{
    external_open::system,
    plugin::{self, application as api, handoff},
    terminal,
};
use api::{Error, ErrorCode as Code, Request, ResultValue};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Pending {
    invocation: String,
    pub(super) deadline: std::time::Instant,
    charge: usize,
    cancelled: Arc<AtomicBool>,
    terminal: Option<terminal::TerminalCancellation>,
    orphaned: bool,
    launched: bool,
    replied: bool,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Pending {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(terminal) = &self.terminal {
            terminal.cancel();
        }
    }
}
pub(super) fn is_handoff_request(request: &Request) -> bool {
    matches!(
        request,
        Request::TerminalOpen(_) | Request::ExternalOpen { .. }
    )
}
fn external_error(error: system::Error) -> Error {
    Error::new(
        match error {
            system::Error::InvalidTarget => Code::InvalidArgument,
            system::Error::Busy => Code::Busy,
            system::Error::Unavailable => Code::Unavailable,
            system::Error::OutcomeUnknown => Code::OutcomeUnknown,
        },
        &error.to_string(),
    )
}
fn stale() -> Error {
    Error::new(Code::ContextChanged, "Handoff foreground context changed")
}

enum Work {
    Terminal(Box<terminal::TerminalPreparation>),
    External(system::Target),
}
impl WorkspaceHost {
    fn handoff_context(
        &self,
        owner: usize,
        invocation: &str,
    ) -> Result<api::CapturedContext, Error> {
        let context = self
            .app
            .plugins
            .instances
            .get(&owner)
            .and_then(|instance| instance.application.requests.get(invocation))
            .ok_or_else(stale)?;
        self.app.plugin_foreground(context)?;
        Ok(context.clone())
    }
    pub(super) fn application_handoff_request(
        &mut self,
        owner: usize,
        request: &str,
        operation: Request,
    ) -> Result<(), Error> {
        let (capability, invocation, charge) = match &operation {
            Request::TerminalOpen(start) => (
                "terminals",
                start.invocation.as_str(),
                terminal::PENDING_TERMINAL_CHARGE,
            ),
            Request::ExternalOpen { invocation, .. } => {
                ("external", invocation.as_str(), 128 * 1024)
            }
            _ => return Err(Error::new(Code::Unsupported, "Unknown handoff request")),
        };
        let state = &self.app.plugins.instances[&owner].application;
        if !state.capabilities.contains(capability) {
            return Err(Error::new(
                Code::CapabilityDenied,
                "Handoff capability was not granted",
            ));
        }
        let context = self.handoff_context(owner, invocation)?;
        let invocation = invocation.to_owned();
        if self
            .plugin_handoffs
            .iter()
            .any(|((id, _, _), pending)| *id == owner && !pending.orphaned)
        {
            return Err(Error::new(
                Code::Busy,
                "A native handoff is already pending",
            ));
        }
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| Error::new(Code::Unavailable, "Handoff runtime unavailable"))?;
        let sender = self
            .plugin_events_sender
            .clone()
            .ok_or_else(|| Error::new(Code::Unavailable, "Handoff event channel unavailable"))?;
        self.reserve_application_payload(owner, charge)?;
        let permit = self
            .plugin_local_slots
            .get_or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(16)))
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::new(Code::Busy, "Local operation service is busy"))?;
        let work = match operation {
            Request::TerminalOpen(start) => {
                start.validate()?;
                let request = terminal::TerminalRequest {
                    program: start.executable.into(),
                    arguments: start.args,
                    directory: start.cwd.map_or_else(
                        || self.app.project_root.clone(),
                        |cwd| self.app.project_root.join(cwd),
                    ),
                    label: start.label,
                };
                Work::Terminal(Box::new(
                    self.app.reserve_plugin_terminal(request, &context)?,
                ))
            }
            Request::ExternalOpen { target, .. } => {
                let text = match &target {
                    handoff::Target::Url { url } => url,
                    handoff::Target::File { path } => path,
                };
                if text.is_empty()
                    || text.len() > system::MAX_TARGET_BYTES
                    || text.chars().any(char::is_control)
                {
                    return Err(Error::new(Code::InvalidArgument, "Invalid external target"));
                }
                Work::External(match target {
                    handoff::Target::Url { url } => system::Target::Url(url),
                    handoff::Target::File { path } => system::Target::File(path),
                })
            }
            _ => unreachable!(),
        };
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        let generation = state.generation.clone();
        state.retained_payload += charge;
        let cancelled = Arc::new(AtomicBool::new(false));
        let terminal = match &work {
            Work::Terminal(work) => Some(work.cancellation()),
            _ => None,
        };
        self.plugin_handoffs.insert(
            (owner, generation.clone(), request.into()),
            Pending {
                invocation,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(8),
                charge,
                cancelled: cancelled.clone(),
                terminal: terminal.clone(),
                orphaned: false,
                launched: false,
                replied: false,
            },
        );
        let lease = Arc::new(handoff::Lease {
            owner,
            generation,
            request: request.into(),
            sender,
            permit: Some(permit),
        });
        let worker_lease = lease.clone();
        let root = self.app.project_root.clone();
        let mut work = runtime.spawn_blocking(move || {
            if cancelled.load(Ordering::Acquire) {
                return Err(stale());
            }
            match work {
                Work::Terminal(mut preparation) => {
                    // The root and cwd can require filesystem IO; both stay off the editor loop.
                    let root = root.canonicalize().map_err(|_| {
                        Error::new(Code::InvalidArgument, "Invalid terminal working directory")
                    })?;
                    let directory = preparation.request.directory.canonicalize().map_err(|_| {
                        Error::new(Code::InvalidArgument, "Invalid terminal working directory")
                    })?;
                    if !directory.starts_with(root) || !directory.is_dir() {
                        return Err(Error::new(
                            Code::InvalidArgument,
                            "Terminal working directory is outside the workspace",
                        ));
                    }
                    preparation.request.directory = directory;
                    preparation.retain_until_settled(Box::new(worker_lease));
                    preparation
                        .spawn()
                        .map(|pending| handoff::Prepared::Terminal(Box::new(pending)))
                        .map_err(|_| Error::new(Code::Unavailable, "Terminal could not be started"))
                }
                Work::External(target) => system::prepare(&root, target)
                    .map(handoff::Prepared::External)
                    .map_err(external_error),
            }
        });
        runtime.spawn(async move {
            let result = match tokio::time::timeout(std::time::Duration::from_secs(5), &mut work)
                .await
            {
                Ok(result) => result.unwrap_or_else(|_| {
                    Err(Error::new(Code::Internal, "Handoff preparation failed"))
                }),
                Err(_) => {
                    if let Some(terminal) = terminal {
                        terminal.cancel();
                    }
                    let result = Err(Error::new(Code::Timeout, "Handoff preparation timed out"));
                    send_result(lease.clone(), result).await;
                    // Actual worker completion, including late PTY cleanup, retains its slot.
                    drop(work.await);
                    return;
                }
            };
            send_result(lease, result).await;
        });
        Ok(())
    }
    pub(super) fn application_handoff_event(
        &mut self,
        owner: usize,
        event: handoff::Event,
    ) -> anyhow::Result<()> {
        let key = (owner, event.generation.clone(), event.request.clone());
        match event.kind {
            handoff::Kind::Settled { .. } => {
                if let Some(pending) = self.plugin_handoffs.remove(&key) {
                    if pending.orphaned {
                        self.app.plugins.orphaned_payload -= pending.charge;
                    } else if let Some(instance) = self.app.plugins.instances.get_mut(&owner)
                        && instance.application.generation == event.generation
                    {
                        instance.application.retained_payload -= pending.charge;
                    }
                }
            }
            handoff::Kind::Prepared { result, lease } => {
                let Some(pending) = self.plugin_handoffs.get(&key) else {
                    return Ok(());
                };
                if pending.orphaned || pending.replied {
                    return Ok(());
                }
                let invocation = pending.invocation.clone();
                let launched = pending.launched;
                let deadline = pending.deadline;
                let context = if launched {
                    None
                } else {
                    self.handoff_context(owner, &invocation).ok()
                };
                let result = if !launched && context.is_none() {
                    Err(stale())
                } else {
                    result
                };
                match result {
                    Ok(handoff::Prepared::External(prepared)) => {
                        if std::time::Instant::now() >= deadline {
                            self.plugin_handoffs.get_mut(&key).unwrap().replied = true;
                            return self.application_local_reply(
                                owner,
                                event.request,
                                Err(Error::new(
                                    Code::Timeout,
                                    "Handoff preparation budget expired",
                                )),
                            );
                        }
                        self.app
                            .plugins
                            .instances
                            .get_mut(&owner)
                            .unwrap()
                            .application
                            .requests
                            .get_mut(&invocation)
                            .unwrap()
                            .foreground_allowed = false;
                        self.plugin_handoffs.get_mut(&key).unwrap().launched = true;
                        #[cfg(test)]
                        let launcher = self.plugin_external_launcher.clone();
                        tokio::spawn(async move {
                            let mut work = tokio::task::spawn_blocking(move || {
                                #[cfg(test)]
                                if let Some(launcher) = launcher {
                                    return prepared
                                        .launch_for_test(launcher.as_os_str())
                                        .map(|()| handoff::Prepared::Launched)
                                        .map_err(external_error);
                                }
                                prepared
                                    .launch()
                                    .map(|()| handoff::Prepared::Launched)
                                    .map_err(external_error)
                            });
                            let result = match tokio::time::timeout_at(
                                tokio::time::Instant::from_std(deadline),
                                &mut work,
                            )
                            .await
                            {
                                Ok(result) => result.unwrap_or_else(|_| {
                                    Err(Error::new(
                                        Code::OutcomeUnknown,
                                        "System opener outcome is unknown",
                                    ))
                                }),
                                Err(_) => {
                                    send_result(
                                        lease.clone(),
                                        Err(Error::new(
                                            Code::OutcomeUnknown,
                                            "System opener outcome is unknown",
                                        )),
                                    )
                                    .await;
                                    // Admission is irreversible; the core owns late spawn/reap.
                                    // Retain the local charge until this worker really returns.
                                    drop(work.await);
                                    return;
                                }
                            };
                            send_result(lease, result).await;
                        });
                        return Ok(());
                    }
                    Ok(handoff::Prepared::Terminal(terminal)) => {
                        let result = self
                            .app
                            .install_plugin_terminal(*terminal, context.as_ref().unwrap())
                            .map(|terminal| ResultValue::TerminalOpened {
                                terminal: format!("t:{terminal}"),
                            });
                        if result.is_ok()
                            && let Some(context) = self
                                .app
                                .plugins
                                .instances
                                .get_mut(&owner)
                                .unwrap()
                                .application
                                .requests
                                .get_mut(&invocation)
                        {
                            context.foreground_allowed = false;
                        }
                        self.plugin_handoffs.get_mut(&key).unwrap().replied = true;
                        self.application_local_reply(owner, event.request, result)?;
                    }
                    Ok(handoff::Prepared::Launched) => {
                        self.plugin_handoffs.get_mut(&key).unwrap().replied = true;
                        self.application_local_reply(
                            owner,
                            event.request,
                            Ok(ResultValue::Empty(api::Empty {})),
                        )?;
                    }
                    Err(error) => {
                        self.plugin_handoffs.get_mut(&key).unwrap().replied = true;
                        self.application_local_reply(owner, event.request, Err(error))?;
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn stop_plugin_handoffs(&mut self, owner: usize) {
        for ((id, _, _), pending) in &mut self.plugin_handoffs {
            if *id == owner && !pending.orphaned {
                pending.cancel();
                pending.orphaned = true;
                self.app.plugins.orphaned_payload += pending.charge;
            }
        }
    }
    pub(super) fn sync_plugin_handoffs(&self) {
        for ((owner, _, _), pending) in &self.plugin_handoffs {
            if !pending.launched
                && !pending.replied
                && self.handoff_context(*owner, &pending.invocation).is_err()
            {
                pending.cancel();
            }
        }
    }
}
async fn send_result(lease: Arc<handoff::Lease>, result: Result<handoff::Prepared, Error>) {
    let sender = lease.sender.clone();
    let event = plugin::Event {
        plugin: lease.owner,
        result: Ok(plugin::ClientMessage::Handoff(handoff::Event {
            generation: lease.generation.clone(),
            request: lease.request.clone(),
            kind: handoff::Kind::Prepared { result, lease },
        })),
    };
    let _ = sender.send(event).await;
}
