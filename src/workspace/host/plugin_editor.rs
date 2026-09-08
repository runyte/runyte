// SPDX-License-Identifier: MPL-2.0

use super::{BufferId, BufferRevision, WorkspaceHost};
use crate::{
    plugin::{HostMessage, application as api, editor as wire},
    text::{Change, Transaction},
};
use api::{Error, ErrorCode as Code, Request, ResultValue};

impl WorkspaceHost {
    fn application_buffer(&self, owner: usize, handle: &str) -> Result<usize, Error> {
        let index = *self.app.plugins.instances[&owner]
            .application
            .buffers
            .get(handle)
            .ok_or_else(|| Error::new(Code::NotFound, "Unknown buffer"))?;
        if self.app.host_buffer_is_closed(index) {
            return Err(Error::new(Code::Closed, "Buffer is closed"));
        }
        Ok(index)
    }
    fn issue_application_buffer(&mut self, owner: usize, buffer: usize) -> Result<String, Error> {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        state.buffer_handle(buffer)
    }
    pub(super) fn application_editor_request(
        &mut self,
        owner: usize,
        request: Request,
    ) -> Result<ResultValue, Error> {
        let capability = match request {
            Request::BufferList { .. } | Request::PaneList(_) => "workspace",
            Request::SelectionGet { .. } | Request::SelectionSet { .. } => "selections",
            _ => "text",
        };
        if !self.app.plugins.instances[&owner]
            .application
            .capabilities
            .contains(capability)
        {
            return Err(Error::new(
                Code::CapabilityDenied,
                "Capability was not granted",
            ));
        }
        match request {
            Request::BufferList { offset, limit } => {
                if !(1..=128).contains(&limit) {
                    return Err(Error::new(
                        Code::InvalidArgument,
                        "Page limit must be 1–128",
                    ));
                }
                let indices = (0..self.app.buffers.len())
                    .filter(|index| !self.app.host_buffer_is_closed(*index))
                    .skip(offset)
                    .take(limit + 1)
                    .collect::<Vec<_>>();
                let next = (indices.len() > limit).then_some(offset.saturating_add(limit));
                let mut buffers = Vec::new();
                for index in indices.into_iter().take(limit) {
                    let handle = self.issue_application_buffer(owner, index)?;
                    let buffer = &self.app.buffers[index];
                    buffers.push(wire::Buffer {
                        buffer: handle,
                        revision: format!("r:{}", buffer.revision()),
                        name: buffer.display_name(),
                        chars: buffer.len_chars(),
                        read_only: buffer.is_read_only(),
                        dirty: buffer.dirty,
                    });
                }
                Ok(ResultValue::Buffers { buffers, next })
            }
            Request::PaneList(_) => {
                let mut panes = Vec::new();
                let mut ids = self.app.panes.keys().copied().collect::<Vec<_>>();
                ids.sort();
                for pane in ids {
                    let buffer = if self.app.panes[&pane].terminal.is_none() {
                        Some(self.issue_application_buffer(owner, self.app.panes[&pane].buffer)?)
                    } else {
                        None
                    };
                    let state = &mut self
                        .app
                        .plugins
                        .instances
                        .get_mut(&owner)
                        .unwrap()
                        .application;
                    let handle = state.pane_handle(pane)?;
                    panes.push(wire::Pane {
                        pane: handle,
                        buffer,
                        selection_revision: format!(
                            "q:{}",
                            self.app.plugin_selection_revision(pane)
                        ),
                    });
                }
                Ok(ResultValue::Panes { panes })
            }
            Request::BufferRead {
                buffer,
                expected_revision,
                from,
                to,
            } => {
                let index = self.application_buffer(owner, &buffer)?;
                let value = &self.app.buffers[index];
                if expected_revision != format!("r:{}", value.revision()) {
                    return Err(Error::new(Code::Stale, "Buffer changed"));
                }
                read(value.text(), expected_revision, from, to)
            }
            Request::BufferEdit {
                buffer,
                expected_revision,
                changes,
            } => {
                let index = self.application_buffer(owner, &buffer)?;
                let value = &self.app.buffers[index];
                if expected_revision != format!("r:{}", value.revision()) {
                    return Err(Error::new(Code::Stale, "Buffer changed"));
                }
                if value.is_read_only() {
                    return Err(Error::new(Code::ReadOnly, "Buffer is read-only"));
                }
                if changes.len() > wire::MAX_CHANGES
                    || changes
                        .iter()
                        .map(|change| change.text.len())
                        .sum::<usize>()
                        > wire::MAX_REPLACEMENT_BYTES
                {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "Transaction limit exceeded",
                    ));
                }
                if changes
                    .iter()
                    .any(|change| change.from > change.to || change.to > value.len_chars())
                    || changes
                        .windows(2)
                        .any(|pair| pair[0].to > pair[1].from || pair[0].from == pair[1].from)
                {
                    return Err(Error::new(
                        Code::InvalidArgument,
                        "Invalid or overlapping edit ranges",
                    ));
                }
                let count = changes.len();
                let revision = value.revision();
                let transaction = Transaction::new(
                    changes
                        .into_iter()
                        .map(|change| Change::new(change.from, change.to, change.text))
                        .collect(),
                );
                if !transaction.is_empty() {
                    self.app.buffers[index].commit_undo_group();
                    self.apply_expected_transaction(
                        BufferId::from_index(index),
                        BufferRevision::from_raw(revision),
                        transaction,
                    )
                    .map_err(|_| Error::new(Code::Conflict, "Edit could not be applied"))?;
                }
                Ok(ResultValue::Edited {
                    buffer,
                    revision: format!("r:{}", self.app.buffers[index].revision()),
                    changes: count,
                })
            }
            Request::SnapshotOpen {
                buffer,
                expected_revision,
            } => {
                let index = self.application_buffer(owner, &buffer)?;
                let buffer = &self.app.buffers[index];
                if expected_revision != format!("r:{}", buffer.revision()) {
                    return Err(Error::new(Code::Stale, "Buffer changed"));
                }
                if buffer.len_bytes() > wire::MAX_SNAPSHOT_BYTES {
                    return Err(Error::new(Code::LimitExceeded, "Snapshot exceeds 16 MiB"));
                }
                self.reserve_application_payload(owner, buffer.len_bytes())?;
                let state = &mut self
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application;
                if state.snapshots.len() >= wire::MAX_SNAPSHOTS {
                    return Err(Error::new(Code::LimitExceeded, "Snapshot limit reached"));
                }
                state.next_handle += 1;
                let handle = format!("t:{}:{}", state.generation, state.next_handle);
                let chars = buffer.len_chars();
                state.retained_payload += buffer.len_bytes();
                state.snapshots.insert(
                    handle.clone(),
                    wire::Snapshot {
                        buffer: index,
                        text: buffer.text().clone(),
                        revision: expected_revision.clone(),
                    },
                );
                self.plugin_send(
                    owner,
                    HostMessage::Deadline {
                        token: handle.clone(),
                        after_ms: Some(wire::SNAPSHOT_IDLE_SECONDS * 1000),
                    },
                )
                .map_err(|_| Error::new(Code::Unavailable, "Application queue unavailable"))?;
                Ok(ResultValue::Snapshot {
                    snapshot: handle,
                    revision: expected_revision,
                    chars,
                })
            }
            Request::SnapshotRead { snapshot, from, to } => {
                let value = self.app.plugins.instances[&owner]
                    .application
                    .snapshots
                    .get(&snapshot)
                    .ok_or_else(|| Error::new(Code::Closed, "Snapshot closed or expired"))?;
                let result = read(&value.text, value.revision.clone(), from, to)?;
                self.plugin_send(
                    owner,
                    HostMessage::Deadline {
                        token: snapshot,
                        after_ms: Some(wire::SNAPSHOT_IDLE_SECONDS * 1000),
                    },
                )
                .map_err(|_| Error::new(Code::Unavailable, "Application queue unavailable"))?;
                Ok(result)
            }
            Request::SnapshotClose { snapshot } => {
                let state = &mut self
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application;
                if let Some(value) = state.snapshots.remove(&snapshot) {
                    state.retained_payload -= value.text.len_bytes();
                    self.plugin_send(
                        owner,
                        HostMessage::Deadline {
                            token: snapshot,
                            after_ms: None,
                        },
                    )
                    .map_err(|_| Error::new(Code::Unavailable, "Application queue unavailable"))?;
                }
                Ok(ResultValue::Empty(api::Empty {}))
            }
            Request::SelectionGet { ref pane } | Request::SelectionSet { ref pane, .. } => {
                let handle = pane.clone();
                let pane = *self.app.plugins.instances[&owner]
                    .application
                    .panes
                    .get(pane)
                    .ok_or_else(|| Error::new(Code::NotFound, "Unknown pane"))?;
                let target = self
                    .app
                    .panes
                    .get(&pane)
                    .ok_or_else(|| Error::new(Code::Closed, "Pane closed"))?;
                if target.terminal.is_some() {
                    return Err(Error::new(Code::Conflict, "Pane displays a terminal"));
                }
                let index = target.buffer;
                if let Request::SelectionSet {
                    buffer,
                    expected_revision,
                    ranges,
                    primary,
                    ..
                } = request
                {
                    if self.application_buffer(owner, &buffer)? != index
                        || expected_revision
                            != format!("q:{}", self.app.plugin_selection_revision(pane))
                    {
                        return Err(Error::new(Code::Stale, "Pane selection or target changed"));
                    }
                    if ranges.is_empty()
                        || ranges.len() > 1024
                        || primary >= ranges.len()
                        || ranges.iter().any(|range| {
                            range.anchor > self.app.buffers[index].len_chars()
                                || range.head > self.app.buffers[index].len_chars()
                        })
                    {
                        return Err(Error::new(
                            Code::InvalidArgument,
                            "Invalid selection ranges",
                        ));
                    }
                    self.app.plugin_set_selection(pane, ranges, primary);
                }
                let buffer = self.issue_application_buffer(owner, index)?;
                let target = &self.app.panes[&pane];
                Ok(ResultValue::Selection {
                    pane: handle,
                    buffer,
                    revision: format!("q:{}", self.app.plugin_selection_revision(pane)),
                    ranges: target
                        .selection
                        .ranges()
                        .iter()
                        .map(|range| wire::Range {
                            anchor: range.anchor,
                            head: range.head,
                        })
                        .collect(),
                    primary: target.selection.primary_index(),
                })
            }
            _ => unreachable!("editor request routing"),
        }
    }
}

fn read(
    text: &crate::text::Text,
    revision: String,
    from: usize,
    to: usize,
) -> Result<ResultValue, Error> {
    if from > to || to > text.len_chars() {
        return Err(Error::new(Code::InvalidArgument, "Invalid text range"));
    }
    let slice = text.rope().slice(from..to);
    if slice.len_bytes() > wire::MAX_CHUNK_BYTES {
        return Err(Error::new(
            Code::LimitExceeded,
            "Text chunk exceeds 256 KiB",
        ));
    }
    let text = slice.to_string();
    if serde_json::to_string(&text)
        .expect("string serialization")
        .len()
        > crate::plugin::MAX_BYTES - 1024
    {
        return Err(Error::new(
            Code::LimitExceeded,
            "Encoded text chunk exceeds line limit",
        ));
    }
    Ok(ResultValue::Text {
        revision,
        from,
        to,
        text,
    })
}
