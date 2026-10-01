// SPDX-License-Identifier: MPL-2.0

//! Commit-network generations, cached pages and native buffer navigation.

use super::{App, Buffer, GitOperation, GitRequestId, Mode, Selection};
use crate::git::{NetworkPage, NetworkRequest, NetworkScope, network::MAX_NETWORK_PAGES};

#[derive(Default)]
pub(super) struct NetworkState {
    pub scope: NetworkScope,
    pages: Vec<NetworkPage>,
    page: usize,
    buffer: Option<usize>,
    pending: Option<PendingNetwork>,
    selected: Option<String>,
    generation: u64,
    roots_check: Option<(GitRequestId, u64)>,
    roots_dirty: bool,
    returns: std::collections::HashMap<usize, NetworkReturn>,
    detail_requests: std::collections::HashMap<GitRequestId, NetworkDetailRequest>,
    latest_detail: Option<GitRequestId>,
    spans: Vec<(std::ops::Range<usize>, crate::snapshot::TextRole)>,
    loading: Option<NetworkLoading>,
}

pub(super) struct NetworkReturnOrigin {
    pane: usize,
    generation: u64,
    page: usize,
    position: super::view_position::ViewPosition<NetworkSelection>,
}
type RowColumn = (usize, usize);

/// Document coordinates preserve ranges when the network header changes.
struct NetworkSelection {
    ranges: Vec<(RowColumn, RowColumn)>,
    primary: usize,
}
impl NetworkSelection {
    fn capture(buffer: &Buffer, selection: &Selection) -> Self {
        let locate = |offset| {
            let row = buffer.offset_to_row(offset);
            (row, offset.saturating_sub(buffer.line_to_offset(row)))
        };
        Self {
            ranges: selection
                .ranges()
                .iter()
                .map(|range| (locate(range.anchor), locate(range.head)))
                .collect(),
            primary: selection.primary_index(),
        }
    }

    fn resolve(&self, buffer: &Buffer) -> Selection {
        let locate = |(row, column): RowColumn| {
            let row = row.min(buffer.len_lines().saturating_sub(1));
            buffer.line_to_offset(row) + column.min(buffer.line_len(row))
        };
        Selection::new(
            self.ranges
                .iter()
                .map(|&(anchor, head)| crate::selection::Range::new(locate(anchor), locate(head)))
                .collect(),
            self.primary,
        )
    }
}

struct NetworkReturn {
    detail: usize,
    origin: NetworkReturnOrigin,
}
struct NetworkDetailRequest {
    pane: usize,
    buffer: usize,
    binding: u64,
    selection: u64,
}

struct NetworkLoading {
    scope: NetworkScope,
    pages: Vec<NetworkPage>,
    anchor: Option<String>,
}

struct PendingNetwork {
    id: GitRequestId,
    page: usize,
    pane: usize,
    binding: u64,
    selection: u64,
    buffer: usize,
    scope: NetworkScope,
    anchor: Option<String>,
}

impl App {
    pub(super) fn open_git_network(&mut self, scope: NetworkScope) {
        let identity = (scope == self.network.scope)
            .then(|| self.network_row_identity())
            .flatten();
        let anchor = identity.and_then(|index| {
            self.network
                .pages
                .get(self.network.page)?
                .rows
                .get(index)
                .map(|row| row.commit.oid.clone())
        });
        self.network.loading = None;
        self.request_network_page(
            NetworkRequest {
                scope,
                cursor: None,
            },
            0,
            anchor,
        );
    }

    fn request_network_page(
        &mut self,
        request: NetworkRequest,
        page: usize,
        anchor: Option<String>,
    ) {
        if anchor.is_none() && page > 0 {
            self.network.loading = None;
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.action_failed("this project is not in a Git repository");
            return;
        };
        if self.ports.git_service.is_some() {
            let scope = request.scope.clone();
            if let Some(id) = self.request_git(GitOperation::Network {
                repository,
                request,
            }) {
                self.network.pending = Some(PendingNetwork {
                    id,
                    page,
                    pane: self.active_pane,
                    binding: self.active().binding_generation,
                    selection: self.active().selection_revision,
                    buffer: self.active().buffer,
                    scope,
                    anchor,
                });
            }
        } else if let Some(provider) = self.ports.git.as_deref() {
            match provider.network_page(&repository, &request) {
                Ok(result) => self.open_network_result(result, page, request.scope, anchor),
                Err(error) => self.error_from("Git", "Git network failed", error.to_string()),
            }
        }
    }

    pub(super) fn apply_network_response(&mut self, id: Option<GitRequestId>, page: NetworkPage) {
        let Some(pending) = self.network.pending.as_ref() else {
            return;
        };
        if id != Some(pending.id) {
            return;
        }
        let pending = self.network.pending.take().unwrap();
        if self.active_pane != pending.pane
            || self.active().binding_generation != pending.binding
            || self.active().selection_revision != pending.selection
            || self.active().buffer != pending.buffer
        {
            self.network.loading = None;
            return;
        }
        self.open_network_result(page, pending.page, pending.scope, pending.anchor);
    }

    fn open_network_result(
        &mut self,
        page: NetworkPage,
        target: usize,
        scope: NetworkScope,
        anchor: Option<String>,
    ) {
        if target >= MAX_NETWORK_PAGES {
            return;
        }
        if target == 0 {
            self.network.loading = Some(NetworkLoading {
                scope,
                pages: Vec::new(),
                anchor,
            });
        }
        if let Some(loading) = self.network.loading.as_mut() {
            loading.pages.truncate(target);
            loading.pages.push(page);
            let current = loading.pages.last().unwrap();
            if let Some(anchor) = loading.anchor.as_ref()
                && !current.rows.iter().any(|row| &row.commit.oid == anchor)
                && let Some(cursor) = current.next.clone()
            {
                let request = NetworkRequest {
                    scope: loading.scope.clone(),
                    cursor: Some(cursor),
                };
                let anchor = loading.anchor.clone();
                self.request_network_page(request, target + 1, anchor);
                return;
            }
            let loading = self.network.loading.take().unwrap();
            self.network.generation = self.network.generation.wrapping_add(1);
            self.network.scope = loading.scope;
            self.network.pages = loading.pages;
            self.network.selected = loading.anchor;
        } else {
            self.network.pages.truncate(target);
            self.network.pages.push(page);
        }
        self.network.page = if self.network.selected.as_ref().is_some_and(|anchor| {
            !self.network.pages[target]
                .rows
                .iter()
                .any(|row| &row.commit.oid == anchor)
        }) {
            0
        } else {
            target
        };
        self.show_network_page(true);
    }

    fn network_row_identity(&self) -> Option<usize> {
        if !self.active_buffer().is_git_network() {
            return None;
        }
        let line = self.active_buffer().offset_to_row(self.active().head());
        let index = line.checked_sub(1)?;
        (index < self.network.pages.get(self.network.page)?.rows.len()).then_some(index)
    }

    pub(super) fn selected_network_oid(&self) -> Option<String> {
        let index = self.network_row_identity()?;
        self.network
            .pages
            .get(self.network.page)?
            .rows
            .get(index)
            .map(|row| row.commit.oid.clone())
    }

    fn show_network_page(&mut self, activate: bool) {
        let Some(page) = self.network.pages.get(self.network.page) else {
            return;
        };
        let freshness = if page.stale {
            " | stale refs: refresh to capture new roots"
        } else {
            ""
        };
        let limit = if page.limited {
            " | graph limit: narrow scope to HEAD or an exact ref"
        } else if page.next.is_some() {
            " | continues"
        } else {
            " | end"
        };
        let mut text = format!(
            "# {} | page {} | lanes 1-16{}{}",
            self.network.scope.label(),
            self.network.page + 1,
            freshness,
            limit
        );
        let graph_width = page
            .rows
            .iter()
            .map(crate::git::network::GraphRow::width)
            .max()
            .unwrap_or(1);
        let mut spans = Vec::new();
        let mut offset = text.chars().count() + 1;
        for row in &page.rows {
            use crate::git::network::{HASH_COLUMNS, display_label};
            use crate::snapshot::TextRole;
            let line = row.text_with_width(graph_width);
            spans.push((offset..offset + HASH_COLUMNS, TextRole::GitHash));
            let graph = offset + row.metadata_prefix().chars().count();
            graph_spans(&mut spans, &row.node, graph);
            let branch_start = graph + graph_width * 2 + 1;
            if let Some(lane) = row.lane {
                spans.push((
                    branch_start..branch_start + row.branch_label().chars().count(),
                    lane_role(row.node.cells[lane * 2].color),
                ));
            }
            if let Some(label) = row
                .commit
                .decorations
                .first()
                .filter(|label| label.starts_with("HEAD"))
            {
                let start = branch_start + row.branch_label().chars().count() + 1;
                let displayed_label = display_label(label);
                spans.push((
                    start..start + displayed_label.chars().count(),
                    TextRole::GitHead,
                ));
            }
            text.push('\n');
            text.push_str(&line);
            offset += line.chars().count() + 1;
        }
        if page.rows.is_empty() {
            text.push_str("\nNo commits (unborn HEAD or empty scope)");
        }
        let selected = activate
            .then(|| self.network.selected.take())
            .flatten()
            .and_then(|oid| page.rows.iter().position(|row| row.commit.oid == oid))
            .map_or(1, |index| index + 1);
        let buffer = self
            .network
            .buffer
            .filter(|buffer| !self.closed_buffers.contains(buffer))
            .unwrap_or_else(|| {
                self.buffers.push(Buffer::git_network(&text));
                self.syntax.push(None);
                self.buffers.len() - 1
            });
        self.network.buffer = Some(buffer);
        self.replace_virtual_preserving_positions(
            buffer,
            &text,
            Some(true),
            |buffer, pane| NetworkSelection::capture(buffer, &pane.selection),
            |buffer, selection| selection.resolve(buffer),
        );
        self.network.spans = spans;
        if activate && self.active().buffer != buffer {
            self.switch_buffer(buffer);
        }
        let offset = self.buffers[buffer]
            .line_to_offset(selected.min(self.buffers[buffer].len_lines().saturating_sub(1)));
        if activate {
            self.active_mut()
                .replace_selection(Selection::point(offset));
            self.active_mut().preserve_scroll = false;
            self.mode = Mode::Normal;
        }
    }

    pub(super) fn network_return_origin(&self) -> Option<NetworkReturnOrigin> {
        self.active_buffer()
            .is_git_network()
            .then(|| NetworkReturnOrigin {
                pane: self.active_pane,
                generation: self.network.generation,
                page: self.network.page,
                position: super::view_position::ViewPosition::capture_with(
                    self.active(),
                    NetworkSelection::capture(self.active_buffer(), &self.active().selection),
                ),
            })
    }

    pub(super) fn remember_network_return(&mut self, detail: usize, origin: NetworkReturnOrigin) {
        self.network
            .returns
            .insert(origin.pane, NetworkReturn { detail, origin });
    }

    pub(super) fn restore_network_return(&mut self, detail: usize) {
        let mut panes = self
            .network
            .returns
            .iter()
            .filter_map(|(&pane, value)| (value.detail == detail).then_some(pane))
            .collect::<Vec<_>>();
        panes.sort_by_key(|pane| (*pane != self.active_pane, *pane));
        for pane in panes {
            let value = self.network.returns.remove(&pane).unwrap();
            if value.origin.generation != self.network.generation
                || value.origin.page >= self.network.pages.len()
                || self
                    .panes
                    .get(&pane)
                    .is_none_or(|pane| Some(pane.buffer) != self.network.buffer)
            {
                continue;
            }
            self.network.page = value.origin.page;
            self.show_network_page(false);
            let selection = value
                .origin
                .position
                .selection
                .resolve(&self.buffers[self.panes[&pane].buffer]);
            value
                .origin
                .position
                .restore_with(self.panes.get_mut(&pane).unwrap(), selection);
        }
    }

    pub(super) fn note_network_detail_request(&mut self, id: GitRequestId) {
        if !self.active_buffer().is_git_network() {
            return;
        }
        self.network.latest_detail = Some(id);
        self.network.detail_requests.insert(
            id,
            NetworkDetailRequest {
                pane: self.active_pane,
                buffer: self.active().buffer,
                binding: self.active().binding_generation,
                selection: self.active().selection_revision,
            },
        );
    }

    pub(super) fn accept_network_detail_response(&mut self, id: Option<GitRequestId>) -> bool {
        let Some(id) = id else {
            return true;
        };
        let Some(origin) = self.network.detail_requests.remove(&id) else {
            return true;
        };
        self.network.latest_detail == Some(id)
            && self.active_pane == origin.pane
            && self.active().buffer == origin.buffer
            && self.active().binding_generation == origin.binding
            && self.active().selection_revision == origin.selection
    }

    pub(super) fn check_network_roots(&mut self) {
        if self.network.roots_check.is_some() {
            self.network.roots_dirty = true;
            return;
        }
        if self.network.pages.is_empty()
            || self
                .network
                .buffer
                .is_none_or(|buffer| self.closed_buffers.contains(&buffer))
        {
            return;
        }
        let Some(repository) = self.git.repository().cloned() else {
            return;
        };
        let scope = self.network.scope.clone();
        if self.ports.git_service.is_some() {
            if let Some(id) = self.request_git(GitOperation::NetworkRoots { repository, scope }) {
                self.network.roots_check = Some((id, self.network.generation));
            }
        } else if let Some(provider) = self.ports.git.as_deref()
            && let Ok((roots, limited)) = provider.network_roots(&repository, &scope)
        {
            self.mark_network_roots(&scope, roots, limited);
        }
    }

    pub(super) fn apply_network_roots(
        &mut self,
        request: Option<GitRequestId>,
        scope: NetworkScope,
        roots: Vec<crate::git::NetworkRoot>,
        limited: bool,
    ) {
        let Some((id, generation)) = self.network.roots_check else {
            return;
        };
        if request != Some(id) {
            return;
        }
        self.network.roots_check = None;
        if generation == self.network.generation {
            self.mark_network_roots(&scope, roots, limited);
        }
        if std::mem::take(&mut self.network.roots_dirty) {
            self.check_network_roots();
        }
    }

    pub(super) fn network_request_failed(&mut self, id: GitRequestId) {
        self.network.detail_requests.remove(&id);
        if self
            .network
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == id)
        {
            self.network.pending = None;
            self.network.loading = None;
        }
        if self
            .network
            .roots_check
            .is_some_and(|(pending, _)| pending == id)
        {
            self.network.roots_check = None;
            self.network.roots_dirty = false;
        }
    }

    fn mark_network_roots(
        &mut self,
        scope: &NetworkScope,
        roots: Vec<crate::git::NetworkRoot>,
        limited: bool,
    ) {
        if scope != &self.network.scope {
            return;
        }
        let stale = self
            .network
            .pages
            .first()
            .is_some_and(|page| page.roots != roots || page.roots_limited != limited);
        if stale {
            for page in &mut self.network.pages {
                page.stale = true;
            }
            self.show_network_page(false);
        }
    }

    pub(crate) fn network_role_at(
        &self,
        buffer: usize,
        offset: usize,
    ) -> Option<crate::snapshot::TextRole> {
        if self.network.buffer != Some(buffer) {
            return None;
        }
        let index = self
            .network
            .spans
            .partition_point(|(range, _)| range.end <= offset);
        self.network
            .spans
            .get(index)
            .filter(|(range, _)| range.contains(&offset))
            .map(|(_, role)| *role)
    }

    pub(super) fn choose_git_network_ref(&mut self) {
        let Some(page) = self.network.pages.get(self.network.page) else {
            self.action_failed("open the Git network before choosing a captured ref");
            return;
        };
        let mut references = page
            .roots
            .iter()
            .flat_map(|root| root.references.iter().cloned())
            .collect::<Vec<_>>();
        references.sort();
        references.dedup();
        if references.is_empty() {
            self.action_failed("this generation has no captured refs; open all refs first");
            return;
        }
        self.list_actions = references
            .iter()
            .cloned()
            .map(super::ListAction::GitNetworkRef)
            .collect();
        let items = references
            .into_iter()
            .enumerate()
            .map(|(index, reference)| super::PickerItem::new(reference, "", index))
            .collect();
        self.list = Some(super::ListPicker::new("Commit network scope", items));
    }

    pub(super) fn next_git_network_page(&mut self) {
        if !self.active_buffer().is_git_network() {
            self.action_failed("paging requires the Git network");
            return;
        }
        let next = self.network.page + 1;
        if next < self.network.pages.len() {
            self.network.page = next;
            self.show_network_page(true);
            return;
        }
        let Some(cursor) = self
            .network
            .pages
            .get(self.network.page)
            .and_then(|page| page.next.clone())
        else {
            self.action_failed(
                "the commit network has no next page; narrow scope at a graph limit",
            );
            return;
        };
        self.request_network_page(
            NetworkRequest {
                scope: self.network.scope.clone(),
                cursor: Some(cursor),
            },
            next,
            None,
        );
    }

    pub(super) fn previous_git_network_page(&mut self) {
        if !self.active_buffer().is_git_network() {
            self.action_failed("paging requires the Git network");
            return;
        }
        if let Some(page) = self.network.page.checked_sub(1) {
            self.network.page = page;
            self.show_network_page(true);
        }
    }
}

fn graph_spans(
    spans: &mut Vec<(std::ops::Range<usize>, crate::snapshot::TextRole)>,
    line: &crate::git::network::GraphLine,
    offset: usize,
) {
    for (column, cell) in line.cells.iter().enumerate() {
        if cell.character() != ' ' {
            spans.push((offset + column..offset + column + 1, lane_role(cell.color)));
        }
    }
}

fn lane_role(color: u8) -> crate::snapshot::TextRole {
    use crate::snapshot::TextRole;
    let roles = [
        TextRole::GitLane0,
        TextRole::GitLane1,
        TextRole::GitLane2,
        TextRole::GitLane3,
    ];
    roles[usize::from(color)]
}

#[cfg(test)]
#[path = "tests/git_network.rs"]
mod tests;
