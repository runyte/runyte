// SPDX-License-Identifier: MPL-2.0

//! Host ownership of bounded helper processes, including retired generations.
use super::WorkspaceHost;
use crate::plugin::{application as api, process};
use api::{Error, ErrorCode as Code, Request, ResultValue};
use process::runtime;

pub(super) struct Managed {
    owner: usize,
    generation: String,
    identity: String,
    orphaned: bool,
    failed_start: bool,
    info: process::Info,
    stdout: process::Ring,
    stderr: Option<process::Ring>,
    control: Option<runtime::Handle>,
    start: Option<String>,
    write: Option<String>,
    close: Vec<String>,
    closing_snapshot: Option<process::Info>,
}
impl Managed {
    fn pending(&self) -> usize {
        usize::from(self.start.is_some()) + usize::from(self.write.is_some()) + self.close.len()
    }
    fn exited(&self) -> bool {
        self.info.state == process::State::Exited
    }
}

pub(super) fn is_process_request(request: &Request) -> bool {
    matches!(
        request,
        Request::ProcessStart(_)
            | Request::ProcessGet { .. }
            | Request::ProcessRead { .. }
            | Request::ProcessWrite { .. }
            | Request::ProcessClose { .. }
    )
}

impl WorkspaceHost {
    pub(super) fn pending_process_requests(&self, owner: Option<usize>) -> usize {
        self.plugin_processes
            .values()
            .filter(|entry| !entry.orphaned && owner.is_none_or(|owner| entry.owner == owner))
            .map(Managed::pending)
            .sum()
    }
    fn process_request_slot(&self, owner: usize) -> Result<(), Error> {
        if self.pending_process_requests(Some(owner)) >= api::MAX_REQUESTS {
            return Err(Error::new(Code::Busy, "Process request limit reached"));
        }
        Ok(())
    }
    fn owned_process(&self, owner: usize, handle: &str) -> Result<&Managed, Error> {
        let state = &self.app.plugins.instances[&owner].application;
        if !state.processes.contains(handle) {
            return Err(Error::new(Code::NotFound, "Unknown process"));
        }
        self.plugin_processes
            .get(handle)
            .filter(|entry| {
                !entry.orphaned && entry.owner == owner && entry.generation == state.generation
            })
            .ok_or_else(|| Error::new(Code::NotFound, "Unknown process"))
    }
    pub(super) fn process_info(&self, owner: usize, handle: &str) -> Option<&process::Info> {
        self.owned_process(owner, handle)
            .ok()
            .map(|entry| &entry.info)
    }
    pub(super) fn process_observation_info(
        &self,
        owner: usize,
        handle: &str,
    ) -> Option<&process::Info> {
        self.owned_process(owner, handle)
            .ok()
            .map(|entry| entry.closing_snapshot.as_ref().unwrap_or(&entry.info))
    }
    pub(super) fn application_process_request(
        &mut self,
        owner: usize,
        request: &str,
        operation: Request,
    ) -> Result<Option<ResultValue>, Error> {
        if !self.app.plugins.instances[&owner]
            .application
            .capabilities
            .contains("processes")
        {
            return Err(Error::new(
                Code::CapabilityDenied,
                "Processes capability was not granted",
            ));
        }
        match operation {
            Request::ProcessStart(start) => {
                start.validate()?;
                self.process_request_slot(owner)?;
                let instance = &self.app.plugins.instances[&owner];
                if self.plugin_processes.len() >= 32
                    || self
                        .plugin_processes
                        .values()
                        .filter(|entry| entry.identity == instance.config.id)
                        .count()
                        >= process::MAX_HANDLES
                {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "Process handle limit reached; close an existing helper",
                    ));
                }
                let sender = self
                    .plugin_events_sender
                    .as_ref()
                    .ok_or_else(|| {
                        Error::new(Code::Unavailable, "Process event channel unavailable")
                    })?
                    .clone();
                self.reserve_application_payload(owner, process::PROCESS_CHARGE)?;
                let instance = self.app.plugins.instances.get_mut(&owner).unwrap();
                let generation = instance.application.generation.clone();
                let identity = instance.config.id.clone();
                instance.application.next_handle += 1;
                let handle = format!("p:{}:{}", generation, instance.application.next_handle);
                instance.application.retained_payload += process::PROCESS_CHARGE;
                let capture_stderr = start.capture_stderr;
                let label = start.label.clone();
                let capacity = if capture_stderr {
                    process::MAX_OUTPUT_BYTES / 2
                } else {
                    process::MAX_OUTPUT_BYTES
                };
                let stdout = process::Ring::new(capacity);
                let stderr = capture_stderr.then(|| process::Ring::new(capacity));
                let control = match runtime::spawn(
                    owner,
                    generation.clone(),
                    handle.clone(),
                    self.app.project_root.clone(),
                    start,
                    sender,
                ) {
                    Ok(control) => control,
                    Err(error) => {
                        self.app
                            .plugins
                            .instances
                            .get_mut(&owner)
                            .unwrap()
                            .application
                            .retained_payload -= process::PROCESS_CHARGE;
                        return Err(error);
                    }
                };
                self.plugin_processes.insert(
                    handle.clone(),
                    Managed {
                        owner,
                        generation,
                        identity,
                        orphaned: false,
                        failed_start: false,
                        info: process::Info {
                            process: handle,
                            label,
                            state: process::State::Running,
                            stdout: Default::default(),
                            stderr: Default::default(),
                            capture_stderr,
                            stdin_closed: false,
                            output_truncated: false,
                            exit_code: None,
                            signal: None,
                        },
                        stdout,
                        stderr,
                        control: Some(control),
                        start: Some(request.into()),
                        write: None,
                        close: Vec::new(),
                        closing_snapshot: None,
                    },
                );
                Ok(None)
            }
            Request::ProcessGet { process: handle } => Ok(Some(ResultValue::Process(
                self.owned_process(owner, &handle)?.info.clone(),
            ))),
            Request::ProcessRead {
                process: handle,
                stream,
                offset,
                limit,
            } => {
                let entry = self.owned_process(owner, &handle)?;
                let ring = match stream {
                    process::Stream::Stdout => &entry.stdout,
                    process::Stream::Stderr => entry.stderr.as_ref().ok_or_else(|| {
                        Error::new(Code::Unsupported, "Process stderr capture is disabled")
                    })?,
                };
                let chunk = ring.read(offset, limit, entry.exited())?;
                Ok(Some(ResultValue::ProcessRead(process::Read {
                    process: handle,
                    stream,
                    offset: chunk.offset,
                    data: process::encode(&chunk.bytes),
                    next: chunk.next,
                    eof: chunk.eof,
                })))
            }
            Request::ProcessWrite {
                process: handle,
                data,
                eof,
            } => {
                let entry = self.owned_process(owner, &handle)?;
                if entry.info.state != process::State::Running || entry.info.stdin_closed {
                    return Err(Error::new(Code::Closed, "Process stdin is closed"));
                }
                if entry.write.is_some() {
                    return Err(Error::new(Code::Busy, "A process write is already pending"));
                }
                self.process_request_slot(owner)?;
                let bytes = process::decode(&data)?;
                let entry = self.plugin_processes.get_mut(&handle).unwrap();
                entry
                    .control
                    .as_ref()
                    .unwrap()
                    .write(request.into(), bytes, eof)?;
                entry.write = Some(request.into());
                Ok(None)
            }
            Request::ProcessClose { process: handle } => {
                let Ok(entry) = self.owned_process(owner, &handle) else {
                    return Ok(Some(ResultValue::Empty(api::Empty {})));
                };
                if entry.exited() {
                    self.remove_process(&handle);
                    return Ok(Some(ResultValue::Empty(api::Empty {})));
                }
                self.process_request_slot(owner)?;
                let entry = self.plugin_processes.get_mut(&handle).unwrap();
                if entry.close.is_empty() {
                    entry.closing_snapshot = Some(entry.info.clone());
                }
                entry.close.push(request.into());
                entry.info.state = process::State::Closing;
                entry.control.as_ref().unwrap().close();
                Ok(None)
            }
            _ => unreachable!(),
        }
    }

    fn remove_process(&mut self, handle: &str) {
        let Some(entry) = self.plugin_processes.remove(handle) else {
            return;
        };
        if entry.orphaned {
            self.app.plugins.orphaned_payload -= process::PROCESS_CHARGE;
        } else if let Some(instance) = self.app.plugins.instances.get_mut(&entry.owner)
            && instance.application.generation == entry.generation
        {
            instance.application.processes.remove(handle);
            instance.application.retained_payload -= process::PROCESS_CHARGE;
        }
    }

    pub(super) fn stop_plugin_processes(&mut self, owner: usize) {
        let Some(instance) = self.app.plugins.instances.get(&owner) else {
            return;
        };
        let generation = instance.application.generation.clone();
        let handles: Vec<_> = self
            .plugin_processes
            .iter()
            .filter(|(_, entry)| {
                entry.owner == owner && entry.generation == generation && !entry.orphaned
            })
            .map(|(handle, _)| handle.clone())
            .collect();
        for handle in handles {
            if self.plugin_processes[&handle].exited() {
                self.remove_process(&handle);
            } else {
                let entry = self.plugin_processes.get_mut(&handle).unwrap();
                entry.orphaned = true;
                entry.start = None;
                entry.write = None;
                entry.close.clear();
                entry.control.as_ref().unwrap().close();
                self.app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application
                    .retained_payload -= process::PROCESS_CHARGE;
                self.app.plugins.orphaned_payload += process::PROCESS_CHARGE;
            }
        }
    }

    pub(super) fn application_process_event(
        &mut self,
        owner: usize,
        event: runtime::Event,
    ) -> anyhow::Result<()> {
        let handle = event.process;
        let Some(entry) = self.plugin_processes.get_mut(&handle) else {
            return Ok(());
        };
        if entry.owner != owner || entry.generation != event.generation {
            return Ok(());
        }
        if !entry.orphaned
            && !self
                .app
                .plugins
                .instances
                .get(&owner)
                .is_some_and(|instance| instance.application.generation == entry.generation)
        {
            // A later generation must never inherit a queued earlier result.
            entry.orphaned = true;
            entry.start = None;
            entry.write = None;
            entry.close.clear();
            if let Some(control) = &entry.control {
                control.close();
            }
            self.app.plugins.orphaned_payload += process::PROCESS_CHARGE;
        }
        let mut replies = Vec::new();
        let mut remove = false;
        match event.kind {
            runtime::Kind::Started => {
                if entry.orphaned || entry.failed_start {
                    entry.control.as_ref().unwrap().close();
                } else if let Some(request) = entry.start.take() {
                    self.app
                        .plugins
                        .instances
                        .get_mut(&owner)
                        .unwrap()
                        .application
                        .processes
                        .insert(handle.clone());
                    replies.push((request, Ok(ResultValue::Process(entry.info.clone()))));
                }
            }
            runtime::Kind::StartFailed { error } => {
                entry.failed_start = true;
                entry.info.state = process::State::Closing;
                if let Some(control) = &entry.control {
                    control.close();
                }
                if !entry.orphaned
                    && let Some(request) = entry.start.take()
                {
                    replies.push((request, Err(error)));
                }
            }
            runtime::Kind::Output { stream, bytes } => {
                if !entry.orphaned {
                    match stream {
                        process::Stream::Stdout => {
                            entry
                                .stdout
                                .append(&bytes)
                                .map_err(|error| anyhow::anyhow!(error.message))?;
                            entry.info.stdout = entry.stdout.bounds();
                        }
                        process::Stream::Stderr => {
                            if let Some(stderr) = &mut entry.stderr {
                                stderr
                                    .append(&bytes)
                                    .map_err(|error| anyhow::anyhow!(error.message))?;
                                entry.info.stderr = stderr.bounds();
                            }
                        }
                    }
                }
            }
            runtime::Kind::Written { request, result } => {
                if !entry.orphaned && entry.write.as_ref() == Some(&request) {
                    entry.write = None;
                    let result = result.map(|written| {
                        entry.info.stdin_closed |= written.eof;
                        ResultValue::ProcessWrite(process::Write {
                            process: handle.clone(),
                            written: written.written,
                            stdin_closed: entry.info.stdin_closed,
                        })
                    });
                    if result
                        .as_ref()
                        .is_err_and(|error| error.code == Code::OutcomeUnknown)
                    {
                        entry.info.state = process::State::Closing;
                        entry.info.stdin_closed = true;
                        entry.control.as_ref().unwrap().close();
                    }
                    replies.push((request, result));
                }
            }
            runtime::Kind::Exited {
                code,
                signal,
                error,
                stdout,
                stderr,
                mut output_truncated,
                write,
            } => {
                if !entry.orphaned {
                    output_truncated |= entry.stdout.append(&stdout).is_err();
                    entry.info.stdout = entry.stdout.bounds();
                    if let Some(ring) = &mut entry.stderr {
                        output_truncated |= ring.append(&stderr).is_err();
                        entry.info.stderr = ring.bounds();
                    }
                }
                entry.info.output_truncated = output_truncated;
                entry.info.state = process::State::Exited;
                entry.info.stdin_closed = true;
                entry.info.exit_code = code;
                entry.info.signal = signal;
                entry.control = None;
                if entry.orphaned || entry.failed_start {
                    remove = true;
                } else {
                    if let Some((request, result)) = write
                        && entry.write.as_ref() == Some(&request)
                    {
                        entry.write = None;
                        replies.push((
                            request,
                            result.map(|written| {
                                ResultValue::ProcessWrite(process::Write {
                                    process: handle.clone(),
                                    written: written.written,
                                    stdin_closed: true,
                                })
                            }),
                        ));
                    }
                    if let Some(request) = entry.start.take() {
                        replies.push((
                            request,
                            Err(error.unwrap_or_else(|| {
                                Error::new(Code::Unavailable, "Process did not start")
                            })),
                        ));
                        remove = true;
                    }
                    if let Some(request) = entry.write.take() {
                        replies.push((
                            request,
                            Err(Error::new(
                                Code::OutcomeUnknown,
                                "Process exited before the write settled",
                            )),
                        ));
                    }
                    for request in entry.close.drain(..) {
                        replies.push((request, Ok(ResultValue::Empty(api::Empty {}))));
                        remove = true;
                    }
                }
            }
        }
        if remove {
            self.remove_process(&handle);
        }
        for (request, result) in replies {
            self.application_local_reply(owner, request, result)?;
        }
        Ok(())
    }
}
