// SPDX-License-Identifier: MPL-2.0

//! Owner-scoped semantic context reads. No operation prepares native geometry,
//! changes focus, or borrows an application's mutation capabilities.

use std::{
    collections::BTreeMap,
    collections::BTreeSet,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::{BufferId, BufferRevision, WorkspaceHost};
use crate::{
    plugin::{
        application::{Error, ErrorCode as Code},
        editor,
    },
    snapshot::{SnapshotRow, TextRunKind},
    terminal::{TerminalId, read},
    text::{Change, Transaction},
    workspace::context::wire::{self, Request, Scope},
};

const MAX_HANDLES: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq)]
struct PaneTarget {
    id: usize,
    generation: u64,
    buffer: usize,
    terminal: Option<TerminalId>,
}

enum Capture {
    Buffer {
        index: usize,
        handle: String,
        revision: String,
        text: String,
        checkpoints: Vec<usize>,
        chars: usize,
    },
    Terminal {
        handle: String,
        capture: read::Snapshot,
    },
}
struct Snapshot {
    capture: Capture,
    last_read: Instant,
    bytes: usize,
}

pub(super) struct ReadState {
    prefix: String,
    next: u64,
    buffers: BTreeMap<String, usize>,
    panes: BTreeMap<String, PaneTarget>,
    terminals: BTreeMap<String, TerminalId>,
    snapshots: BTreeMap<String, Snapshot>,
}

impl ReadState {
    pub(super) fn new(prefix: String) -> Self {
        Self {
            prefix,
            next: 0,
            buffers: BTreeMap::new(),
            panes: BTreeMap::new(),
            terminals: BTreeMap::new(),
            snapshots: BTreeMap::new(),
        }
    }
    pub(super) fn retained_bytes(&self) -> usize {
        self.snapshots.values().map(|s| s.bytes).sum()
    }
    pub(super) fn expire(&mut self) {
        self.snapshots.retain(|_, snapshot| {
            snapshot.last_read.elapsed() < Duration::from_secs(wire::SNAPSHOT_IDLE_SECONDS)
        });
    }
    fn handle(&mut self, kind: &str) -> Result<String, Error> {
        if self.buffers.len() + self.panes.len() + self.terminals.len() + self.snapshots.len()
            >= MAX_HANDLES
        {
            return Err(Error::new(
                Code::LimitExceeded,
                "Reader handle limit reached; reconnect to renew inventory",
            ));
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| Error::new(Code::LimitExceeded, "Reader handle sequence exhausted"))?;
        Ok(format!("{kind}:{}:{}", self.prefix, self.next))
    }
    fn buffer(&mut self, index: usize) -> Result<String, Error> {
        if let Some((handle, _)) = self.buffers.iter().find(|(_, value)| **value == index) {
            return Ok(handle.clone());
        }
        let handle = self.handle("b")?;
        self.buffers.insert(handle.clone(), index);
        Ok(handle)
    }
    fn pane(&mut self, target: PaneTarget) -> Result<String, Error> {
        if let Some((handle, _)) = self.panes.iter().find(|(_, value)| **value == target) {
            return Ok(handle.clone());
        }
        let handle = self.handle("p")?;
        self.panes.insert(handle.clone(), target);
        Ok(handle)
    }
    fn terminal_handle(&mut self, id: TerminalId) -> Result<String, Error> {
        if let Some((handle, _)) = self.terminals.iter().find(|(_, value)| **value == id) {
            return Ok(handle.clone());
        }
        let handle = self.handle("t")?;
        self.terminals.insert(handle.clone(), id);
        Ok(handle)
    }
    pub(super) fn terminal(&self, handle: &str) -> Result<TerminalId, Error> {
        self.terminals
            .get(handle)
            .copied()
            .ok_or_else(|| Error::new(Code::NotFound, "Unknown terminal handle"))
    }
    fn store(&mut self, capture: Capture, bytes: usize) -> Result<String, Error> {
        // Include the owned key as well as every retained content allocation.
        let bytes = bytes.saturating_add(self.prefix.len() + 24);
        if self.snapshots.len() >= wire::MAX_SNAPSHOTS
            || self.retained_bytes().saturating_add(bytes) > wire::MAX_RETAINED_BYTES
        {
            return Err(Error::new(
                Code::LimitExceeded,
                "Reader snapshot budget exhausted",
            ));
        }
        let handle = self.handle("s")?;
        self.snapshots.insert(
            handle.clone(),
            Snapshot {
                capture,
                last_read: Instant::now(),
                bytes,
            },
        );
        Ok(handle)
    }
}

impl WorkspaceHost {
    fn reserve_context_capture(&self, state: &ReadState, bytes: usize) -> Result<(), Error> {
        let used = self
            .context
            .retained_reader_bytes()
            .saturating_add(state.retained_bytes())
            .saturating_add(self.app.terminals.retained_payload_bytes());
        let charge = bytes.saturating_add(state.prefix.len() + 24);
        if used.saturating_add(charge) > self.app.terminals.retained_payload_limit() {
            return Err(Error::new(
                Code::LimitExceeded,
                "Workspace retained context budget exhausted",
            ));
        }
        Ok(())
    }

    fn context_buffer(&self, state: &ReadState, handle: &str) -> Result<usize, Error> {
        let index = state
            .buffers
            .get(handle)
            .copied()
            .ok_or_else(|| Error::new(Code::NotFound, "Unknown buffer handle"))?;
        if self.app.host_buffer_is_closed(index) {
            return Err(Error::new(Code::Closed, "Buffer closed"));
        }
        Ok(index)
    }
    fn context_pane(&self, state: &ReadState, handle: &str) -> Result<PaneTarget, Error> {
        let target = state
            .panes
            .get(handle)
            .copied()
            .ok_or_else(|| Error::new(Code::NotFound, "Unknown pane handle"))?;
        let pane = self
            .app
            .panes
            .get(&target.id)
            .ok_or_else(|| Error::new(Code::Closed, "Pane closed"))?;
        if pane.buffer != target.buffer
            || pane.terminal != target.terminal
            || pane.binding_generation != target.generation
        {
            return Err(Error::new(Code::Stale, "Pane target changed"));
        }
        Ok(target)
    }

    pub(super) fn context_read_request(
        &mut self,
        state: &mut ReadState,
        scopes: &BTreeSet<Scope>,
        request: Request,
    ) -> Result<Value, Error> {
        wire::validate_scopes(scopes)?;
        if !scopes.contains(&request.required_scope()) {
            return Err(Error::new(
                Code::CapabilityDenied,
                "Context scope was not granted",
            ));
        }
        request.validate()?;
        state.expire();
        // Closed resources invalidate their captures, even if their old bytes
        // remain immutable. They cannot be resurrected through snapshot handles.
        state
            .snapshots
            .retain(|_, snapshot| match &snapshot.capture {
                Capture::Buffer { index, .. } => !self.app.host_buffer_is_closed(*index),
                Capture::Terminal { capture, .. } => {
                    self.app.terminals.get(capture.terminal).is_some()
                }
            });
        let capture_terminal = matches!(&request, Request::TerminalSnapshotOpen { .. });
        match request {
            Request::BufferList { offset, limit } => {
                let indices = (0..self.app.buffers.len())
                    .filter(|index| !self.app.host_buffer_is_closed(*index))
                    .skip(offset)
                    .take(limit + 1)
                    .collect::<Vec<_>>();
                let next = (indices.len() > limit).then_some(offset + limit);
                let mut buffers = Vec::new();
                for index in indices.into_iter().take(limit) {
                    let buffer = &self.app.buffers[index];
                    buffers.push(json!({"buffer":state.buffer(index)?,"revision":revision(buffer.revision()),"name":buffer.display_name(),"chars":buffer.len_chars(),"read_only":buffer.is_read_only(),"dirty":buffer.dirty}));
                }
                Ok(json!({"buffers":buffers,"next":next}))
            }
            Request::BufferRead {
                buffer,
                expected_revision,
                from,
                to,
            } => {
                let index = self.context_buffer(state, &buffer)?;
                let value = &self.app.buffers[index];
                check_revision(&expected_revision, value.revision())?;
                let mut result = serde_json::to_value(super::plugin_editor::read(
                    value.text(),
                    expected_revision,
                    from,
                    to,
                )?)
                .map_err(|_| Error::new(Code::Internal, "Read serialization failed"))?;
                result["buffer"] = json!(buffer);
                Ok(result)
            }
            Request::BufferEdit {
                buffer,
                expected_revision,
                changes,
            } => {
                let index = self.context_buffer(state, &buffer)?;
                let value = &self.app.buffers[index];
                check_revision(&expected_revision, value.revision())?;
                if value.is_read_only() {
                    return Err(Error::new(Code::ReadOnly, "Buffer is read-only"));
                }
                if changes.iter().any(|change| change.to > value.len_chars())
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
                let expected = value.revision();
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
                        BufferRevision::from_raw(expected),
                        transaction,
                    )
                    .map_err(|_| Error::new(Code::Conflict, "Edit could not be applied"))?;
                    self.app.buffers[index].commit_undo_group();
                    self.app.plugins.presentation_dirty = true;
                }
                Ok(
                    json!({"buffer":buffer,"revision":revision(self.app.buffers[index].revision()),"changes":count}),
                )
            }
            Request::BufferAppend {
                buffer,
                text,
                expected_tail,
            } => {
                let index = self.context_buffer(state, &buffer)?;
                let value = &self.app.buffers[index];
                if value.is_read_only() {
                    return Err(Error::new(Code::ReadOnly, "Buffer is read-only"));
                }
                // The end and the tail are read in the same host turn that
                // applies the insertion, so no other writer can intervene.
                let from = value.len_chars();
                if let Some(tail) = &expected_tail {
                    let count = tail.chars().count();
                    if count > from || value.text().slice_string(from - count, from) != *tail {
                        return Err(Error::new(
                            Code::Stale,
                            "Buffer tail does not match expected_tail; nothing was appended",
                        ));
                    }
                }
                let expected = value.revision();
                self.app.buffers[index].commit_undo_group();
                self.apply_expected_transaction(
                    BufferId::from_index(index),
                    BufferRevision::from_raw(expected),
                    Transaction::new(vec![Change::new(from, from, text)]),
                )
                .map_err(|_| Error::new(Code::Conflict, "Append could not be applied"))?;
                self.app.buffers[index].commit_undo_group();
                self.app.plugins.presentation_dirty = true;
                // Report what the buffer now holds rather than echoing the
                // request, so a caller sees exactly what landed.
                let value = &self.app.buffers[index];
                let to = value.len_chars();
                let preview_to = to.min(from + wire::APPEND_PREVIEW_CHARS);
                let line_breaks =
                    value.text().position_of(to).row - value.text().position_of(from).row;
                Ok(json!({
                    "buffer":buffer,"revision":revision(value.revision()),"from":from,"to":to,
                    "line_breaks":line_breaks,"preview":value.text().slice_string(from, preview_to),
                    "preview_truncated":preview_to < to
                }))
            }
            Request::SnapshotOpen {
                buffer,
                expected_revision,
            } => {
                let index = self.context_buffer(state, &buffer)?;
                let value = &self.app.buffers[index];
                check_revision(&expected_revision, value.revision())?;
                if value
                    .len_bytes()
                    .saturating_add(std::mem::size_of::<Snapshot>())
                    > wire::MAX_RETAINED_BYTES.saturating_sub(state.retained_bytes())
                {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "Buffer snapshot exceeds reader budget",
                    ));
                }
                let text = value.text().to_string();
                let chars = value.len_chars();
                let checkpoints = text
                    .char_indices()
                    .enumerate()
                    .filter_map(|(index, (byte, _))| index.is_multiple_of(256).then_some(byte))
                    .collect::<Vec<_>>();
                let bytes = text.capacity()
                    + checkpoints.capacity() * std::mem::size_of::<usize>()
                    + buffer.capacity()
                    + expected_revision.capacity()
                    + std::mem::size_of::<Snapshot>();
                self.reserve_context_capture(state, bytes)?;
                let snapshot = state.store(
                    Capture::Buffer {
                        index,
                        handle: buffer,
                        revision: expected_revision.clone(),
                        text,
                        checkpoints,
                        chars,
                    },
                    bytes,
                )?;
                Ok(json!({"snapshot":snapshot,"revision":expected_revision,"chars":chars}))
            }
            Request::SnapshotRead { snapshot, from, to } => {
                let value = state
                    .snapshots
                    .get_mut(&snapshot)
                    .ok_or_else(|| Error::new(Code::Closed, "Snapshot closed or expired"))?;
                let Capture::Buffer {
                    handle,
                    revision,
                    text,
                    checkpoints,
                    chars,
                    ..
                } = &value.capture
                else {
                    return Err(Error::new(
                        Code::InvalidArgument,
                        "Snapshot is not a buffer",
                    ));
                };
                if to > *chars {
                    return Err(Error::new(Code::InvalidArgument, "Invalid snapshot range"));
                }
                let start = snapshot_byte(text, checkpoints, *chars, from);
                let end = snapshot_byte(text, checkpoints, *chars, to);
                if end - start > editor::MAX_CHUNK_BYTES {
                    return Err(Error::new(Code::LimitExceeded, "Text chunk exceeds limit"));
                }
                let result = &text[start..end];
                value.last_read = Instant::now();
                Ok(
                    json!({"snapshot":snapshot,"buffer":handle,"revision":revision,"from":from,"to":to,"text":result}),
                )
            }
            Request::SnapshotClose { snapshot } | Request::TerminalSnapshotClose { snapshot } => {
                if state.snapshots.remove(&snapshot).is_none() {
                    return Err(Error::new(Code::Closed, "Snapshot closed or expired"));
                }
                Ok(json!({}))
            }
            Request::SelectionGet { pane } => {
                let target = self.context_pane(state, &pane)?;
                if target.terminal.is_some() {
                    return Err(Error::new(Code::Conflict, "Pane displays a terminal"));
                }
                let value = &self.app.panes[&target.id];
                if value.selection.ranges().len() > 1024 {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "Selection exceeds context limit",
                    ));
                }
                Ok(
                    json!({"pane":pane,"buffer":state.buffer(target.buffer)?,"revision":format!("q:{}",self.app.plugin_selection_revision(target.id)),"buffer_revision":revision(self.app.buffers[target.buffer].revision()),"ranges":value.selection.ranges().iter().map(|r| editor::Range {anchor:r.anchor,head:r.head}).collect::<Vec<_>>(),"spans":self.app.plugin_selection_spans(target.id),"primary":value.selection.primary_index()}),
                )
            }
            Request::PaneContextList(_) => {
                let mut panes = Vec::new();
                let mut ids = self.app.panes.keys().copied().collect::<Vec<_>>();
                ids.sort_unstable();
                if ids.len() > wire::MAX_LIST_ITEMS {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "Pane inventory exceeds limit",
                    ));
                }
                for id in ids {
                    let pane = &self.app.panes[&id];
                    let handle = state.pane(PaneTarget {
                        id,
                        generation: pane.binding_generation,
                        buffer: pane.buffer,
                        terminal: pane.terminal,
                    })?;
                    let buffer = if pane.terminal.is_none() {
                        Some(state.buffer(pane.buffer)?)
                    } else {
                        None
                    };
                    let terminal = if scopes.contains(&Scope::TerminalRead) {
                        pane.terminal
                            .map(|id| state.terminal_handle(id))
                            .transpose()?
                    } else {
                        None
                    };
                    panes.push(json!({"pane":handle,"buffer":buffer,"terminal":terminal,"focused":id == self.app.active_pane,"selection_revision":if pane.terminal.is_none() {Some(format!("q:{}",self.app.plugin_selection_revision(id)))} else {None}}));
                }
                Ok(
                    json!({"inventory_revision":crate::hash::sha256_hex(&serde_json::to_vec(&panes).expect("pane inventory serializes")),"panes":panes,"revision":format!("v:{}",self.context.frame_id),"attachment_generation":self.app.plugins.attachment_generation.to_string(),"attached":self.context.frame.is_some()}),
                )
            }
            Request::TerminalList { offset, limit } => {
                let ids = self.app.terminals.ids();
                let next = (offset.saturating_add(limit) < ids.len()).then_some(offset + limit);
                let mut terminals = Vec::new();
                for id in ids.into_iter().skip(offset).take(limit) {
                    let value = self.app.terminals.get(id).expect("listed terminal");
                    let pane = self
                        .app
                        .panes
                        .iter()
                        .find(|(_, pane)| pane.terminal == Some(id))
                        .map(|(&id, pane)| {
                            state.pane(PaneTarget {
                                id,
                                generation: pane.binding_generation,
                                buffer: pane.buffer,
                                terminal: pane.terminal,
                            })
                        })
                        .transpose()?;
                    terminals.push(json!({"terminal":state.terminal_handle(id)?,"name":value.name(),"live":value.live(),"exit_code":value.exit_code(),"pane":pane,"columns":value.columns(),"screen_rows":value.plain_line_count()-value.scrollback_rows(),"alternate_screen":value.alternate_screen(),"revision":revision(value.read_revision())}));
                }
                Ok(json!({"terminals":terminals,"next":next}))
            }
            Request::TerminalRead {
                terminal,
                region,
                expected_revision,
                max_rows,
                max_bytes,
                max_cells,
            }
            | Request::TerminalSnapshotOpen {
                terminal,
                region,
                expected_revision,
                max_rows,
                max_bytes,
                max_cells,
            } => {
                let id = state.terminal(&terminal)?;
                let value = self
                    .app
                    .terminals
                    .get(id)
                    .ok_or_else(|| Error::new(Code::Closed, "Terminal closed"))?;
                if let Some(expected) = &expected_revision {
                    check_revision(expected, value.read_revision())?;
                }
                let captured = value
                    .read_output(
                        match region {
                            wire::Region::Screen => read::Region::Screen,
                            wire::Region::Tail => read::Region::Tail,
                        },
                        read::Limits {
                            rows: max_rows,
                            bytes: max_bytes,
                            cells: max_cells,
                        },
                        None,
                    )
                    .map_err(|_| Error::new(Code::InvalidArgument, "Invalid terminal read"))?;
                if capture_terminal {
                    let rows = captured.rows.len();
                    let rev = revision(captured.revision);
                    let bytes = captured.rows.capacity() * std::mem::size_of::<read::Row>()
                        + captured
                            .rows
                            .iter()
                            .map(|row| row.text.capacity())
                            .sum::<usize>()
                        + terminal.capacity()
                        + std::mem::size_of::<Snapshot>();
                    if bytes > wire::MAX_TERMINAL_SNAPSHOT_BYTES {
                        return Err(Error::new(
                            Code::LimitExceeded,
                            "Terminal snapshot exceeds limit",
                        ));
                    }
                    self.reserve_context_capture(state, bytes)?;
                    let snapshot = state.store(
                        Capture::Terminal {
                            handle: terminal,
                            capture: captured,
                        },
                        bytes,
                    )?;
                    Ok(json!({"snapshot":snapshot,"revision":rev,"rows":rows}))
                } else {
                    Ok(terminal_value(&terminal, &captured, 0, captured.rows.len()))
                }
            }
            Request::TerminalSnapshotRead {
                snapshot,
                offset,
                limit,
            } => {
                let value = state
                    .snapshots
                    .get_mut(&snapshot)
                    .ok_or_else(|| Error::new(Code::Closed, "Snapshot closed or expired"))?;
                let Capture::Terminal { handle, capture } = &value.capture else {
                    return Err(Error::new(
                        Code::InvalidArgument,
                        "Snapshot is not a terminal",
                    ));
                };
                if offset > capture.rows.len() {
                    return Err(Error::new(
                        Code::InvalidArgument,
                        "Snapshot offset is out of bounds",
                    ));
                }
                let mut result = terminal_value(handle, capture, offset, limit);
                result["snapshot"] = json!(snapshot);
                value.last_read = Instant::now();
                Ok(result)
            }
            Request::PaneViewportRead {
                pane,
                expected_revision,
                max_rows,
                max_bytes,
                max_cells,
            } => self.context_viewport(
                state,
                scopes,
                &pane,
                expected_revision.as_deref(),
                read::Limits {
                    rows: max_rows,
                    bytes: max_bytes,
                    cells: max_cells,
                },
            ),
            Request::TerminalInputPropose { .. }
            | Request::TerminalInputStatus { .. }
            | Request::TerminalInputCancel { .. } => Err(Error::new(
                Code::Unsupported,
                "Terminal proposals require the native review coordinator",
            )),
        }
    }
}

fn revision(value: u64) -> String {
    format!("r:{value}")
}

fn snapshot_byte(text: &str, checkpoints: &[usize], chars: usize, offset: usize) -> usize {
    if offset == chars {
        return text.len();
    }
    let base = checkpoints[offset / 256];
    base + text[base..]
        .char_indices()
        .nth(offset % 256)
        .expect("bounded scalar offset")
        .0
}
fn check_revision(expected: &str, actual: u64) -> Result<(), Error> {
    if expected != revision(actual) {
        return Err(Error::new(Code::Stale, "Resource revision changed"));
    }
    Ok(())
}
fn terminal_value(handle: &str, value: &read::Snapshot, offset: usize, limit: usize) -> Value {
    let rows = value.rows.iter().skip(offset).take(limit).map(|row| json!({"index":row.index,"line_id":{"generation":row.id.generation().to_string(),"local":row.id.local().to_string()},"text":row.text,"columns":row.columns,"clipped":row.clipped})).collect::<Vec<_>>();
    json!({"terminal":handle,"revision":revision(value.revision),"region":match value.region {read::Region::Screen=>"screen",read::Region::Tail=>"tail"},"alternate_screen":value.alternate_screen,"live":value.live,"columns":value.columns,"screen_rows":value.screen_rows,"history_rows":value.history_rows,"lost_history_rows":value.lost_history_rows.to_string(),"available_rows":value.available_rows,"rows":rows,"offset":offset,"next":(offset.saturating_add(limit)<value.rows.len()).then_some(offset.saturating_add(limit)),"truncation":{"rows":value.truncation.rows,"bytes":value.truncation.bytes,"cells":value.truncation.cells},"visited_cells":value.visited_cells})
}

impl WorkspaceHost {
    fn context_viewport(
        &self,
        state: &mut ReadState,
        scopes: &BTreeSet<Scope>,
        handle: &str,
        expected: Option<&str>,
        limits: read::Limits,
    ) -> Result<Value, Error> {
        let target = self.context_pane(state, handle)?;
        if target.terminal.is_some() && !scopes.contains(&Scope::TerminalRead) {
            return Err(Error::new(
                Code::CapabilityDenied,
                "Terminal viewport requires terminal_read",
            ));
        }
        let frame = self
            .context
            .frame
            .as_ref()
            .ok_or_else(|| Error::new(Code::NoFrontend, "No native viewport is attached"))?;
        if self.context.frame_foreground != self.app.plugins.foreground_generation
            || self.context.frame_attachment != self.app.plugins.attachment_generation
        {
            return Err(Error::new(
                Code::ContextChanged,
                "Native viewport ownership changed",
            ));
        }
        let revision = format!("v:{}", self.context.frame_id);
        if expected.is_some_and(|expected| expected != revision) {
            return Err(Error::new(Code::Stale, "Native viewport changed"));
        }
        let source = self
            .context
            .frame_sources
            .get(&target.id)
            .ok_or_else(|| Error::new(Code::Stale, "Native pane source is unavailable"))?;
        if source.0 != target.buffer
            || source.1 != self.app.buffers[target.buffer].revision()
            || source.2.map(|pair| pair.0) != target.terminal
        {
            return Err(Error::new(Code::Stale, "Native pane source changed"));
        }
        let pane = frame
            .panes
            .iter()
            .find(|pane| pane.pane_id == target.id)
            .ok_or_else(|| Error::new(Code::Unavailable, "Pane has no visible native viewport"))?;
        if let Some((id, rev)) = source.2 {
            let value = self
                .app
                .terminals
                .get(id)
                .ok_or_else(|| Error::new(Code::Closed, "Terminal closed"))?;
            if value.read_revision() != rev {
                return Err(Error::new(
                    Code::Stale,
                    "Terminal changed since native capture",
                ));
            }
        }
        let mut bytes_left = limits.bytes;
        let mut cells_left = limits.cells;
        let mut rows = Vec::new();
        let mut clipped = false;
        let available;
        if let Some(terminal) = &pane.terminal {
            if !scopes.contains(&Scope::TerminalRead) {
                return Err(Error::new(
                    Code::CapabilityDenied,
                    "Terminal viewport requires terminal_read",
                ));
            }
            available = terminal.rows.len();
            for (index, row) in terminal.rows.iter().take(limits.rows).enumerate() {
                let mut text = String::new();
                let mut columns = 0;
                for cell in row {
                    if cells_left == 0 {
                        clipped = true;
                        break;
                    }
                    if cell.width == 2 && cells_left < 2 {
                        clipped = true;
                        break;
                    }
                    let needed = if cell.width == 0 {
                        0
                    } else {
                        cell.character.len_utf8()
                            + cell.combining[..usize::from(cell.combining_len)]
                                .iter()
                                .map(|c| c.len_utf8())
                                .sum::<usize>()
                    };
                    if needed > bytes_left {
                        clipped = true;
                        break;
                    }
                    cells_left -= 1;
                    columns += 1;
                    bytes_left -= needed;
                    if cell.width != 0 {
                        text.push(cell.character);
                        text.extend(&cell.combining[..usize::from(cell.combining_len)]);
                    }
                }
                rows.push(json!({"kind":"terminal","text":text.trim_end_matches(' '),"columns":columns,"line_id":terminal.line_ids.get(index).copied().flatten().map(|id|id.to_string()),"clipped":columns<row.len()}));
                if clipped {
                    break;
                }
            }
        } else {
            available = pane.rows.len();
            for row in pane.rows.iter().take(limits.rows) {
                match row {
                    SnapshotRow::Text(row) => {
                        let mut runs = Vec::new();
                        for run in &row.runs {
                            let mut text = String::new();
                            for character in run.text.chars() {
                                let columns = unicode_width::UnicodeWidthChar::width(character)
                                    .unwrap_or(0)
                                    .max(1);
                                if character.len_utf8() > bytes_left || columns > cells_left {
                                    clipped = true;
                                    break;
                                }
                                cells_left -= columns;
                                bytes_left -= character.len_utf8();
                                text.push(character);
                            }
                            runs.push(json!({"text":text,"kind":match run.kind {TextRunKind::Text{whitespace:true,..}=>"whitespace",TextRunKind::Text{..}=>"text",TextRunKind::JumpLabel(_)=>"jump_label",TextRunKind::InlineDiagnostic(_)=>"diagnostic",TextRunKind::FoldMarker=>"fold_marker",TextRunKind::Hint=>"hint"}}));
                            if clipped {
                                break;
                            }
                        }
                        rows.push(json!({"kind":"text","document_row":row.document_row,"continuation":row.continuation,"folded":row.folded,"runs":runs,"scalar_mapping":null,"clipped":clipped}));
                    }
                    SnapshotRow::Placeholder => rows.push(json!({"kind":"placeholder"})),
                    SnapshotRow::Padding => rows.push(json!({"kind":"padding"})),
                    SnapshotRow::Filler => rows.push(json!({"kind":"filler"})),
                }
                if clipped {
                    break;
                }
            }
        }
        let review_revision = self.context.frame_review_sources.get(&target.id).copied();
        let source_revision =
            review_revision.unwrap_or_else(|| source.2.map(|(_, r)| r).unwrap_or(source.1));
        let source_revision_kind = if review_revision.is_some() {
            "terminal_review_content"
        } else if target.terminal.is_some() {
            "terminal_read"
        } else {
            "buffer"
        };
        Ok(
            json!({"pane":handle,"revision":revision,"attachment_generation":self.context.frame_attachment.to_string(),"buffer":if target.terminal.is_none(){Some(state.buffer(target.buffer)?)}else{None},"terminal":target.terminal.map(|id|state.terminal_handle(id)).transpose()?,"source_revision":source_revision.to_string(),"source_revision_kind":source_revision_kind,"review":pane.terminal.as_ref().is_some_and(|t|t.review),"geometry":{"x":pane.body.x,"y":pane.body.y,"width":pane.body.width,"height":pane.body.height},"content_indent":pane.content_indent,"scroll_row":pane.scroll_row,"scroll_wrap":pane.scroll_wrap,"wrap_width":pane.wrap_width,"rows":rows,"available_rows":available,"truncated":clipped||available>limits.rows}),
        )
    }
}

#[cfg(test)]
#[path = "tests/context_reads.rs"]
mod tests;
