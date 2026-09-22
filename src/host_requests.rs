// SPDX-License-Identifier: MPL-2.0

//! Transport-independent semantic requests handled by the persistent host.
//! Connection roles, physical input, lifecycle and frame delivery stay with
//! the platform host loop. Merely compiling this module enables no CLI mode.

use runyte::{
    protocol::{ClientRequest, HostResponse, TransportChange, decode_path},
    workspace::WorkspaceHost,
};
use std::io;

pub(super) fn is_workspace_request(request: &ClientRequest) -> bool {
    matches!(
        request,
        ClientRequest::Invoke { .. }
            | ClientRequest::Health
            | ClientRequest::SessionPreview
            | ClientRequest::DestinationInventory
            | ClientRequest::VisitDestination { .. }
            | ClientRequest::ListBuffers
            | ClientRequest::ReadBuffer { .. }
            | ClientRequest::OpenBuffers { .. }
            | ClientRequest::ApplyTransaction { .. }
            | ClientRequest::SaveBuffer { .. }
            | ClientRequest::CloseBuffer { .. }
            | ClientRequest::CreateWait { .. }
            | ClientRequest::WaitStatus { .. }
            | ClientRequest::CompleteWaitBuffer { .. }
            | ClientRequest::CancelWait { .. }
    )
}

pub(super) struct WorkspaceReply {
    pub(super) response: HostResponse,
    pub(super) publish_frame: bool,
}

fn workspace_response_publishes_frame(response: &HostResponse) -> bool {
    matches!(
        response,
        HostResponse::CommandResult { .. }
            | HostResponse::Opened { .. }
            | HostResponse::TransactionApplied { .. }
            | HostResponse::Saved { .. }
            | HostResponse::Closed { .. }
            | HostResponse::WaitCreated { .. }
            | HostResponse::DestinationVisitResult { .. }
    )
}

pub(super) fn bounded_destination_label(value: &str) -> String {
    let mut end = value
        .len()
        .min(runyte::protocol::MAX_DESTINATION_LABEL_BYTES);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

pub(super) fn handle_workspace_request(
    host: &mut WorkspaceHost,
    request: ClientRequest,
    interactive_attached: bool,
    allow_invoke: bool,
) -> Option<WorkspaceReply> {
    use runyte::{
        command::parse_named_command,
        text::{Change, Transaction},
        workspace::BufferRequestError,
    };

    let result = match request {
        ClientRequest::Health => Ok(HostResponse::Health {
            plugin_jobs: host.protected_state().plugin_jobs,
            activity_leases: host.protected_state().activity_leases,
            activities: host
                .app()
                .plugin_activity_health()
                .into_iter()
                .map(Into::into)
                .collect(),
            protocol: runyte::protocol::VERSION,
            pid: std::process::id(),
            interactive_attached,
            unsaved_buffers: host.protected_state().unsaved_buffers,
            open_buffers: host.open_buffer_count(),
            pending_wait_requests: host.protected_state().pending_wait_requests,
            live_terminals: host.protected_state().live_terminals,
            terminal_sessions: host.app().terminals.len(),
            terminal_line_activity_unix_seconds: host.terminal_line_activity_unix_seconds(),
            unread_terminals: host
                .app()
                .terminals
                .iter()
                .filter(|terminal| terminal.unread_activity())
                .count(),
            terminal_bell: host.app().terminals.iter().any(|terminal| terminal.bell()),
        }),
        ClientRequest::DestinationInventory => {
            let entries = host.app().open_destination_inventory();
            let truncated = entries.len() > runyte::protocol::MAX_DESTINATIONS;
            Ok(HostResponse::DestinationInventory {
                incarnation: host.incarnation().to_owned(),
                truncated,
                entries: entries
                    .into_iter()
                    .take(runyte::protocol::MAX_DESTINATIONS)
                    .map(|entry| {
                        let destination = match entry.destination {
                            runyte::app::OpenDestination::Buffer(index) => {
                                runyte::protocol::OpenDestination::Buffer(index as u64 + 1)
                            }
                            runyte::app::OpenDestination::Terminal(id) => {
                                runyte::protocol::OpenDestination::Terminal(id.get())
                            }
                        };
                        runyte::protocol::OpenDestinationEntry {
                            destination,
                            label: bounded_destination_label(&entry.label),
                            detail: bounded_destination_label(&entry.detail),
                        }
                    })
                    .collect(),
            })
        }
        ClientRequest::VisitDestination {
            incarnation,
            destination,
        } => {
            if !allow_invoke {
                Err(anyhow::anyhow!(
                    "visiting a destination requires the interactive attachment"
                ))
            } else {
                let destination = match destination {
                    runyte::protocol::OpenDestination::Buffer(id) => usize::try_from(id)
                        .ok()
                        .and_then(|id| id.checked_sub(1))
                        .map(runyte::app::OpenDestination::Buffer),
                    runyte::protocol::OpenDestination::Terminal(id) => {
                        Some(runyte::app::OpenDestination::Terminal(
                            runyte::terminal::TerminalId::from_raw(id),
                        ))
                    }
                };
                let error = if incarnation != host.incarnation() {
                    Some(
                        "the persistent host was replaced; its restored layout is unchanged"
                            .to_owned(),
                    )
                } else if !destination
                    .is_some_and(|destination| host.app_mut().visit_open_destination(destination))
                {
                    Some(
                        "the selected resource is no longer open; the restored layout is unchanged"
                            .to_owned(),
                    )
                } else {
                    None
                };
                if let Some(message) = &error {
                    host.report_host_error(message.clone());
                }
                Ok(HostResponse::DestinationVisitResult { error })
            }
        }
        ClientRequest::SessionPreview => Ok(HostResponse::SessionPreview {
            preview: host.session_preview().into(),
        }),
        ClientRequest::Invoke { command } => {
            if !allow_invoke {
                Err(anyhow::anyhow!(
                    "semantic commands require the attached interactive client"
                ))
            } else {
                parse_named_command(&command.name, command.argument.as_deref())
                    .map_err(anyhow::Error::from)
                    .and_then(|invocation| {
                        host.execute_expected_command(
                            command.frame.into(),
                            command.buffer.into(),
                            command.revision.into(),
                            invocation,
                        )
                        .map_err(anyhow::Error::from)
                    })
                    .map(|outcome| HostResponse::CommandResult {
                        outcome: outcome.into(),
                    })
            }
        }
        ClientRequest::ListBuffers => Ok(HostResponse::Buffers {
            buffers: host.buffer_metadata().into_iter().map(Into::into).collect(),
        }),
        ClientRequest::ReadBuffer { buffer } => host
            .read_buffer(buffer.into())
            .map(|buffer| HostResponse::Buffer {
                buffer: buffer.into(),
            })
            .map_err(anyhow::Error::from),
        ClientRequest::OpenBuffers { paths, activate } => {
            if paths.is_empty() || paths.len() > 32 {
                Err(anyhow::anyhow!("open request requires 1 to 32 paths"))
            } else {
                paths
                    .into_iter()
                    .map(decode_path)
                    .collect::<io::Result<Vec<_>>>()
                    .map_err(anyhow::Error::from)
                    .and_then(|paths| host.open_buffers(paths, activate))
                    .map(|buffers| HostResponse::Opened {
                        buffers: buffers.into_iter().map(Into::into).collect(),
                    })
            }
        }
        ClientRequest::ApplyTransaction {
            buffer,
            expected,
            changes,
        } => {
            if changes.is_empty() || changes.len() > 4096 {
                Err(anyhow::anyhow!("transaction requires 1 to 4096 changes"))
            } else if changes.iter().any(|change| change.from > change.to) {
                Err(anyhow::anyhow!(
                    "transaction ranges must be forward and half-open"
                ))
            } else {
                let transaction = Transaction::new(
                    changes
                        .into_iter()
                        .map(|TransportChange { from, to, text }| Change::new(from, to, text))
                        .collect(),
                );
                match host.apply_expected_transaction(buffer.into(), expected.into(), transaction) {
                    Ok(revision) => Ok(HostResponse::TransactionApplied {
                        buffer,
                        revision: revision.into(),
                    }),
                    Err(BufferRequestError::Stale { expected, actual }) => {
                        Ok(HostResponse::StaleRevision {
                            buffer,
                            expected: expected.into(),
                            actual: actual.into(),
                        })
                    }
                    Err(error) => Err(anyhow::Error::from(error)),
                }
            }
        }
        ClientRequest::SaveBuffer { buffer } => {
            host.save_buffer(buffer.into())
                .map(|revision| HostResponse::Saved {
                    buffer,
                    revision: revision.into(),
                })
        }
        ClientRequest::CloseBuffer { buffer, discard } => host
            .close_buffer(buffer.into(), discard)
            .map(|()| HostResponse::Closed { buffer }),
        ClientRequest::CreateWait { paths } => {
            if paths.is_empty() || paths.len() > 32 {
                Err(anyhow::anyhow!("wait request requires 1 to 32 paths"))
            } else {
                paths
                    .into_iter()
                    .map(decode_path)
                    .collect::<io::Result<Vec<_>>>()
                    .map_err(anyhow::Error::from)
                    .and_then(|paths| host.create_wait_request(paths, true))
                    .map(|(token, buffers)| HostResponse::WaitCreated {
                        token: token.into(),
                        buffers: buffers.into_iter().map(Into::into).collect(),
                        interactive_attached,
                    })
            }
        }
        ClientRequest::WaitStatus { token } => host
            .wait_status(token.into())
            .map(|status| HostResponse::WaitState {
                token,
                status: status.into(),
                interactive_attached,
            })
            .ok_or_else(|| anyhow::anyhow!("unknown wait token {token}")),
        ClientRequest::CompleteWaitBuffer { token, buffer } => host
            .complete_wait_buffer(token.into(), buffer.into())
            .and_then(|()| {
                host.wait_status(token.into())
                    .ok_or_else(|| anyhow::anyhow!("unknown wait token {token}"))
            })
            .map(|status| HostResponse::WaitState {
                token,
                status: status.into(),
                interactive_attached,
            }),
        ClientRequest::CancelWait { token } => host
            .cancel_wait(token.into(), "wait client cancelled the request")
            .and_then(|()| {
                host.wait_status(token.into())
                    .ok_or_else(|| anyhow::anyhow!("unknown wait token {token}"))
            })
            .map(|status| HostResponse::WaitState {
                token,
                status: status.into(),
                interactive_attached,
            }),
        _ => return None,
    };
    let response = result.unwrap_or_else(|error| HostResponse::Error {
        message: error.to_string(),
    });
    let publish_frame = workspace_response_publishes_frame(&response);
    Some(WorkspaceReply {
        response,
        publish_frame,
    })
}

#[cfg(test)]
mod tests;
