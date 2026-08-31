// SPDX-License-Identifier: MPL-2.0

//! Project, directory, content, and open-resource picker coordination.

// Application-module dependencies:
use super::{
    App, Buffer, CONTENT_ENTRY_LIMIT, FilePicker, FilePickerEvent, FilePickerKind, FilePreview,
    FileScanner, FinderContentScan, FinderContentSource, FinderMatchSource, FinderMode,
    FinderTarget, Mode, PathBuf, Range, ResourceFinder, ResourceItem, ResourceKind, ResourceTarget,
    Result, Selection, TerminalSession, buffer_picker_columns, buffer_preview,
    resource_path_fields, scan_content, scan_files, terminal_preview,
};

use crate::file_picker::FileHits;
use crate::file_picker::line_hit;

const RESOURCE_CONTENT_SLICE_ROWS: usize = 128;

/// Applies the part of the unified content budget still available to disk
/// hits. Keeping this admission step outside `FilePicker::add_content` makes
/// the picker and the already-collected live resources share one ceiling.
pub(super) fn truncate_content_hits(entries: &mut Vec<FileHits>, mut available: usize) -> bool {
    let mut limited = false;
    for hits in entries.iter_mut() {
        limited |= hits.truncate(available);
        available = available.saturating_sub(hits.len());
    }
    entries.retain(|hits| !hits.is_empty());
    limited
}

impl App {
    pub(super) fn open_project_picker(&mut self) -> Result<()> {
        self.open_picker_at(self.project_root.clone(), FilePickerKind::Files)?;
        self.picker.as_mut().unwrap().enable_unified_finder();
        self.finder = Some(ResourceFinder::new(FinderMode::Names));
        self.rebuild_resource_finder();
        Ok(())
    }

    pub(super) fn open_directory_picker(&mut self) -> Result<()> {
        self.open_picker_at(self.active_directory(), FilePickerKind::Files)
    }

    pub(super) fn open_project_grep(&mut self) -> Result<()> {
        self.open_picker_at(self.project_root.clone(), FilePickerKind::Contents)?;
        self.picker.as_mut().unwrap().enable_unified_finder();
        self.finder = Some(ResourceFinder::new(FinderMode::Contents));
        self.start_content_scan();
        self.rebuild_resource_finder();
        Ok(())
    }

    pub(super) fn open_directory_grep(&mut self) -> Result<()> {
        self.open_picker_at(self.active_directory(), FilePickerKind::Contents)?;
        self.start_content_scan();
        Ok(())
    }

    pub(super) fn open_picker_at(&mut self, root: PathBuf, kind: FilePickerKind) -> Result<()> {
        let root = root.canonicalize().map_err(|error| {
            anyhow::anyhow!("failed to open picker at {}: {error}", root.display())
        })?;
        anyhow::ensure!(root.is_dir(), "{} is not a directory", root.display());
        if let Some(picker) = self.picker.take()
            && let Some(scanner) = &self.file_scanner
        {
            scanner.cancel(picker.scan_id);
        }
        self.finder = None;
        self.finder_content_scan = None;
        self.finder_content_dirty_terminals.clear();
        let scan_id = self.next_file_scan_id;
        self.next_file_scan_id = self.next_file_scan_id.wrapping_add(1).max(1);
        self.picker = Some(match kind {
            FilePickerKind::Files => FilePicker::new(scan_id, root.clone()),
            FilePickerKind::Contents => FilePicker::grep(scan_id, root.clone()),
        });
        if kind == FilePickerKind::Contents {
            return Ok(());
        }
        if let Some(scanner) = &self.file_scanner {
            scanner.scan(
                scan_id,
                root,
                self.project_root.clone(),
                self.state_root.clone(),
                self.config.editor.show_hidden_files,
            );
        } else {
            match scan_files(
                &root,
                &self.project_root,
                &self.state_root,
                self.config.editor.show_hidden_files,
            ) {
                Ok((paths, skipped)) => {
                    let picker = self.picker.as_mut().unwrap();
                    picker.add_paths(paths);
                    picker.finish(skipped, false);
                }
                Err(error) => self.picker.as_mut().unwrap().fail(error.to_string()),
            }
            self.refresh_file_picker_preview();
        }
        Ok(())
    }

    /// Starts, or restarts, the content scan behind the current query.
    ///
    /// A content scan belongs to one query: the scanner keeps only the lines
    /// that query matches, which is what makes the candidate ceiling a limit
    /// on results rather than on how far into the project the walk got. So a
    /// query the entries on hand cannot answer is not re-ranked, it is
    /// re-scanned, under a fresh id that leaves the previous scan's late
    /// batches to be dropped by the picker's id guard.
    ///
    /// Open buffers are searched from their live text and their paths are
    /// left out of the disk results, so unsaved edits stay authoritative.
    fn start_content_scan(&mut self) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let root = picker.root.clone();
        let previous_scan_id = picker.scan_id;
        if let Some(scanner) = &self.file_scanner {
            scanner.cancel(previous_scan_id);
        }
        let scan_id = self.next_file_scan_id;
        self.next_file_scan_id = self.next_file_scan_id.wrapping_add(1).max(1);
        let picker = self.picker.as_mut().expect("the picker was just borrowed");
        picker.restart_content_scan(scan_id);
        let query = picker.query.clone();
        if self.finder.is_none() {
            let live = self
                .buffers
                .iter()
                .enumerate()
                .filter(|(index, buffer)| {
                    !self.closed_buffers.contains(index)
                        && !buffer.is_directory()
                        && buffer
                            .path
                            .as_deref()
                            .is_some_and(|path| path.starts_with(&root))
                })
                .filter_map(|(_, buffer)| {
                    let path = buffer
                        .path
                        .as_deref()
                        .expect("filtered file buffer has a path");
                    let lines = crate::file_picker::line_hits(&buffer.to_string(), &query);
                    (!lines.is_empty()).then(|| crate::file_picker::FileHits {
                        path: path.to_path_buf(),
                        lines,
                    })
                })
                .scan(0usize, |held, hits| {
                    if *held >= CONTENT_ENTRY_LIMIT {
                        return None;
                    }
                    *held += hits.len();
                    Some(hits)
                })
                .collect();
            self.picker.as_mut().unwrap().add_content(live);
        }
        if let Some(scanner) = &self.file_scanner {
            scanner.scan_content(
                scan_id,
                root,
                self.project_root.clone(),
                self.state_root.clone(),
                self.config.editor.show_hidden_files,
                query,
            );
        } else {
            match scan_content(
                &root,
                &self.project_root,
                &self.state_root,
                self.config.editor.show_hidden_files,
                &query,
            ) {
                Ok((mut entries, skipped, limited)) => {
                    entries.retain(|hits| {
                        !self.buffers.iter().enumerate().any(|(index, buffer)| {
                            !self.closed_buffers.contains(&index)
                                && !buffer.is_directory()
                                && buffer.path.as_deref() == Some(hits.path.as_path())
                        })
                    });
                    let picker = self.picker.as_mut().unwrap();
                    picker.add_content(entries);
                    picker.finish(skipped, limited);
                }
                Err(error) => self.picker.as_mut().unwrap().fail(error.to_string()),
            }
            self.refresh_file_picker_preview();
        }
        self.merge_finder_matches();
    }

    /// Re-scans when the query has moved past what the entries on hand can
    /// answer. Every path that edits a content query funnels through here.
    pub(super) fn restart_content_scan_if_needed(&mut self) {
        if self
            .picker
            .as_ref()
            .is_some_and(FilePicker::content_rescan_needed)
        {
            self.start_content_scan();
        }
    }

    pub(super) fn close_file_picker(&mut self) {
        self.finder = None;
        self.finder_content_scan = None;
        self.finder_content_dirty_terminals.clear();
        if let Some(picker) = self.picker.take()
            && let Some(scanner) = &self.file_scanner
        {
            scanner.cancel(picker.scan_id);
        }
    }

    pub(super) fn refresh_file_picker_preview(&mut self) {
        let selected = self.picker.as_ref().and_then(|picker| {
            let found = picker.selected_match()?;
            let entry = picker.view(found.entry)?;
            Some((
                entry.path.to_path_buf(),
                entry.is_dir,
                entry.row.map(|row| {
                    (
                        row,
                        found
                            .positions
                            .iter()
                            .map(|position| entry.column + position)
                            .collect::<Vec<_>>(),
                    )
                }),
            ))
        });
        let preview = selected.map(|(path, is_dir, content_match)| {
            if is_dir {
                return FilePreview::from_directory(&path, self.config.editor.show_hidden_files);
            }
            let live_buffer = self
                .buffers
                .iter()
                .enumerate()
                .find(|(index, buffer)| {
                    !self.closed_buffers.contains(index)
                        && !buffer.is_directory()
                        && buffer.path.as_deref() == Some(path.as_path())
                })
                .map(|(_, buffer)| buffer);
            match (live_buffer, content_match) {
                (Some(buffer), Some((row, emphasis))) => {
                    FilePreview::snippet_from_lines(buffer.lines(), row, emphasis)
                }
                (Some(buffer), None) => FilePreview::from_lines(buffer.lines()),
                (None, Some((row, emphasis))) => {
                    FilePreview::snippet_from_path(&path, row, emphasis)
                }
                (None, None) => FilePreview::from_path(&path),
            }
        });
        if let Some(picker) = self.picker.as_mut() {
            picker.preview = preview;
        }
    }

    pub(super) fn rebuild_resource_finder(&mut self) {
        let Some(query) = self.picker.as_ref().map(|picker| picker.query.clone()) else {
            return;
        };
        let Some(mode) = self.finder.as_ref().map(|finder| finder.mode) else {
            return;
        };
        if mode == FinderMode::Contents {
            self.start_resource_content_scan();
            return;
        }
        self.finder_content_scan = None;
        self.finder_content_dirty_terminals.clear();
        let mut items = Vec::new();
        let active = self
            .active_terminal()
            .is_none()
            .then(|| self.active().buffer);
        for (index, buffer) in self.buffers.iter().enumerate() {
            if !self.buffer_is_discoverable(index) {
                continue;
            }
            let (label, detail) =
                buffer_picker_columns(buffer, &self.project_root, active == Some(index));
            let mut fields = vec![
                "buffer".to_owned(),
                "buffers".to_owned(),
                buffer.display_name(),
                buffer.pane_title(),
            ];
            if let Some(path) = buffer.path.as_deref() {
                fields.extend(resource_path_fields(
                    path,
                    &self.project_root,
                    self.home_directory.as_deref(),
                ));
            }
            let mut item = ResourceItem::new(
                label,
                detail,
                ResourceTarget::Buffer(index),
                ResourceKind::Buffer,
                fields,
            )
            .with_preview(buffer_preview(buffer));
            if let Some(path) = buffer.path.clone() {
                item = item.with_path(path);
            }
            items.push(item);
        }
        for session in self.terminals.iter() {
            let mut detail = format!("#{} · {}", session.id(), session.directory().display());
            if self
                .panes
                .values()
                .any(|pane| pane.terminal == Some(session.id()))
            {
                detail.push_str(" · shown");
            }
            if session.unread_activity() {
                detail.push_str(" · unread");
            }
            if session.bell() {
                detail.push_str(" · bell");
            }
            let mut fields = vec![
                "terminal".to_owned(),
                "terminals".to_owned(),
                "term".to_owned(),
                session.id().to_string(),
                session.name(),
                session.display_name(),
                session.launch_label().to_owned(),
            ];
            if let Some(name) = session.user_name() {
                fields.push(name.to_owned());
            }
            if let Some(title) = session.child_title() {
                fields.push(title.to_owned());
            }
            fields.extend(resource_path_fields(
                session.directory(),
                &self.project_root,
                self.home_directory.as_deref(),
            ));
            fields.extend(resource_path_fields(
                session.initial_directory(),
                &self.project_root,
                self.home_directory.as_deref(),
            ));
            items.push(
                ResourceItem::new(
                    session.display_name(),
                    detail,
                    ResourceTarget::Terminal(session.id()),
                    ResourceKind::Terminal,
                    fields,
                )
                .with_preview(terminal_preview(session)),
            );
        }
        if let (Some(finder), Some(picker)) = (self.finder.as_mut(), self.picker.as_ref()) {
            finder.replace_items(items, picker, &query);
        }
        self.refresh_finder_preview();
    }

    fn start_resource_content_scan(&mut self) {
        let Some(query) = self.picker.as_ref().map(|picker| picker.query.clone()) else {
            self.finder_content_scan = None;
            return;
        };
        let sources = self
            .buffers
            .iter()
            .enumerate()
            .filter(|(index, _)| self.buffer_is_discoverable(*index))
            .map(|(buffer, item)| FinderContentSource::Buffer {
                buffer,
                label: item.display_name(),
                path: item.path.clone(),
            })
            .chain(
                self.terminals
                    .iter()
                    .map(|terminal| FinderContentSource::Terminal {
                        terminal: terminal.id(),
                        label: terminal.display_name(),
                    }),
            )
            .collect::<Vec<_>>();
        let suppressed_paths = sources.iter().filter_map(|source| match source {
            FinderContentSource::Buffer { path, .. } => path.clone(),
            FinderContentSource::Terminal { .. } => None,
        });
        let Some((picker, finder)) = self.picker.as_ref().zip(self.finder.as_mut()) else {
            self.finder_content_scan = None;
            return;
        };
        finder.begin_content_scan(picker, &query, suppressed_paths);
        self.finder_content_dirty_terminals.clear();
        self.finder_content_scan = Some(FinderContentScan {
            query,
            sources,
            source: 0,
            row: 0,
            limited: false,
        });
        self.refresh_finder_preview();
    }

    /// Whether the event loop should schedule another bounded live-content
    /// pass. A new query replaces this cursor, which cancels the old pass
    /// without allowing stale rows into the finder.
    pub fn resource_finder_scan_pending(&self) -> bool {
        self.finder_content_scan.is_some()
    }

    pub(super) fn note_terminal_finder_change(&mut self, terminal: crate::terminal::TerminalId) {
        let Some(mode) = self.finder.as_ref().map(|finder| finder.mode) else {
            return;
        };
        if mode == FinderMode::Names {
            self.rebuild_resource_finder();
        } else if self.finder_content_scan.is_some() {
            self.finder_content_dirty_terminals.insert(terminal);
        } else {
            self.start_resource_content_scan();
        }
    }

    /// Advances live buffer and terminal matching without materializing the
    /// complete corpus or holding the input loop for an unbounded scan.
    pub fn advance_resource_finder_scan(&mut self) {
        let Some(mut scan) = self.finder_content_scan.take() else {
            return;
        };
        let mut visited = 0usize;
        let mut found = Vec::new();
        while scan.source < scan.sources.len() && visited < RESOURCE_CONTENT_SLICE_ROWS {
            if found.len()
                + self.finder.as_ref().map_or(0, |finder| finder.items.len())
                + self
                    .picker
                    .as_ref()
                    .map_or(0, |picker| picker.entries.len())
                >= CONTENT_ENTRY_LIMIT
            {
                scan.limited = true;
                scan.source = scan.sources.len();
                break;
            }
            let source = scan.sources[scan.source].clone();
            let rows = match &source {
                FinderContentSource::Buffer { buffer, .. } => self
                    .buffers
                    .get(*buffer)
                    .filter(|_| !self.closed_buffers.contains(buffer))
                    .map_or(0, |buffer| buffer.len_lines()),
                FinderContentSource::Terminal { terminal, .. } => self
                    .terminals
                    .get(*terminal)
                    .map_or(0, |terminal| terminal.plain_line_count()),
            };
            if scan.row >= rows {
                scan.source += 1;
                scan.row = 0;
                continue;
            }
            let row = scan.row;
            scan.row += 1;
            visited += 1;
            let line = match &source {
                FinderContentSource::Buffer { buffer, .. } => self
                    .buffers
                    .get(*buffer)
                    .map(|buffer| buffer.line_string(row)),
                FinderContentSource::Terminal { terminal, .. } => self
                    .terminals
                    .get(*terminal)
                    .and_then(|terminal| terminal.plain_line(row)),
            };
            let Some(hit) = line.as_deref().and_then(|line| line_hit(line, &scan.query)) else {
                continue;
            };
            let item = match source {
                FinderContentSource::Buffer {
                    buffer,
                    label,
                    path,
                } => {
                    let mut item = ResourceItem::content(
                        format!("{label}:{}", row + 1),
                        hit.text,
                        ResourceTarget::BufferLocation {
                            buffer,
                            row,
                            column: hit.column,
                        },
                        ResourceKind::Buffer,
                    );
                    if let Some(path) = path {
                        item = item.with_path(path);
                    }
                    item
                }
                FinderContentSource::Terminal { terminal, label } => ResourceItem::content(
                    format!("{label}:{}", row + 1),
                    hit.text,
                    ResourceTarget::TerminalLocation {
                        terminal,
                        row,
                        column: hit.column,
                    },
                    ResourceKind::Terminal,
                ),
            };
            found.push(item);
        }

        let query = scan.query.clone();
        if let (Some(finder), Some(picker)) = (self.finder.as_mut(), self.picker.as_ref()) {
            finder.append_items(found, picker, &query);
        }
        if scan.source >= scan.sources.len()
            && !scan.limited
            && !self.finder_content_dirty_terminals.is_empty()
        {
            let dirty = std::mem::take(&mut self.finder_content_dirty_terminals);
            if let (Some(finder), Some(picker)) = (self.finder.as_mut(), self.picker.as_ref()) {
                finder.remove_terminal_content(&dirty, picker, &query);
            }
            scan.sources
                .extend(dirty.into_iter().filter_map(|terminal| {
                    self.terminals
                        .get(terminal)
                        .map(|session| FinderContentSource::Terminal {
                            terminal,
                            label: session.display_name(),
                        })
                }));
        }
        let finished = scan.source >= scan.sources.len();
        if finished {
            self.finder_content_dirty_terminals.clear();
            if let (Some(finder), Some(picker)) = (self.finder.as_mut(), self.picker.as_ref()) {
                finder.finish_content_scan(picker, &query, scan.limited);
            }
        }
        if !finished {
            self.finder_content_scan = Some(scan);
        }
        self.refresh_finder_preview();
    }

    pub(super) fn rank_resource_finder(&mut self) {
        if self
            .finder
            .as_ref()
            .is_some_and(|finder| finder.mode == FinderMode::Contents)
        {
            self.rebuild_resource_finder();
            self.advance_resource_finder_scan();
            return;
        }
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let query = picker.query.clone();
        if let Some(finder) = self.finder.as_mut() {
            finder.rank(picker, &query);
        }
        self.refresh_finder_preview();
    }

    pub(super) fn toggle_finder_mode(&mut self) {
        let Some(mode) = self.finder.as_ref().map(|finder| finder.mode) else {
            return;
        };
        if let Some(picker) = self.picker.as_ref()
            && let Some(scanner) = &self.file_scanner
        {
            scanner.cancel(picker.scan_id);
        }
        let next = match mode {
            FinderMode::Names => FinderMode::Contents,
            FinderMode::Contents => FinderMode::Names,
        };
        if let Some(finder) = self.finder.as_mut() {
            finder.mode = next;
        }
        let scan_id = self.next_file_scan_id;
        self.next_file_scan_id = self.next_file_scan_id.wrapping_add(1).max(1);
        let kind = match next {
            FinderMode::Names => FilePickerKind::Files,
            FinderMode::Contents => FilePickerKind::Contents,
        };
        self.picker.as_mut().unwrap().switch_kind(scan_id, kind);
        match next {
            FinderMode::Contents => {
                self.start_content_scan();
                self.rebuild_resource_finder();
                self.advance_resource_finder_scan();
            }
            FinderMode::Names => {
                self.rebuild_resource_finder();
                self.start_file_scan();
            }
        }
    }

    fn start_file_scan(&mut self) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let scan_id = picker.scan_id;
        let root = picker.root.clone();
        if let Some(scanner) = &self.file_scanner {
            scanner.scan(
                scan_id,
                root,
                self.project_root.clone(),
                self.state_root.clone(),
                self.config.editor.show_hidden_files,
            );
        } else {
            match scan_files(
                &root,
                &self.project_root,
                &self.state_root,
                self.config.editor.show_hidden_files,
            ) {
                Ok((paths, skipped)) => {
                    let picker = self.picker.as_mut().unwrap();
                    picker.add_paths(paths);
                    picker.finish(skipped, false);
                }
                Err(error) => self.picker.as_mut().unwrap().fail(error.to_string()),
            }
            self.merge_finder_matches();
            self.refresh_finder_preview();
        }
    }

    pub(super) fn activate_finder_target(&mut self, target: FinderTarget) {
        self.close_file_picker();
        match target {
            FinderTarget::File(target) => {
                if let Err(error) = self.open_file(target.path.clone()) {
                    self.action_failed(error.to_string());
                } else {
                    self.select_picker_target(&target);
                }
            }
            FinderTarget::Resource(ResourceTarget::Buffer(buffer)) => {
                if buffer >= self.buffers.len() || self.closed_buffers.contains(&buffer) {
                    self.action_failed("that buffer is no longer open");
                } else {
                    self.switch_buffer(buffer);
                }
            }
            FinderTarget::Resource(ResourceTarget::BufferLocation {
                buffer,
                row,
                column,
            }) => {
                if buffer >= self.buffers.len() || self.closed_buffers.contains(&buffer) {
                    self.action_failed("that buffer is no longer open");
                } else {
                    self.switch_buffer(buffer);
                    let row = row.min(self.active_buffer().len_lines().saturating_sub(1));
                    let column = column.min(self.active_buffer().line_len(row));
                    let offset = self.active_buffer().line_to_offset(row) + column;
                    self.active_mut()
                        .replace_selection(Selection::single(Range::point(offset)));
                }
            }
            FinderTarget::Resource(ResourceTarget::Terminal(id)) => {
                if self.terminals.get(id).is_none() {
                    self.action_failed("that terminal is gone");
                } else {
                    self.show_terminal(id);
                }
            }
            FinderTarget::Resource(ResourceTarget::TerminalLocation {
                terminal,
                row,
                column: _,
            }) => {
                if self.terminals.get(terminal).is_none() {
                    self.action_failed("that terminal is gone");
                } else {
                    self.show_terminal(terminal);
                    let (_, rows) = self.pane_cells(self.active_pane);
                    let scroll_offset = self.config.editor.scroll_offset;
                    if let Some(session) = self.terminals.get_mut(terminal) {
                        session.begin_review();
                        session.goto_review_line(row + 1, false);
                        session.focus_review_selection(rows.max(1), scroll_offset);
                    }
                    self.mode = Mode::Normal;
                }
            }
        }
    }

    fn merge_finder_matches(&mut self) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let query = picker.query.clone();
        if let Some(finder) = self.finder.as_mut() {
            finder.merge_files(picker, &query);
        }
    }

    pub(super) fn refresh_finder_preview(&mut self) {
        let resource_preview = self
            .finder
            .as_ref()
            .zip(self.picker.as_ref())
            .and_then(|(finder, picker)| finder.selected_target(picker))
            .and_then(|target| match target {
                FinderTarget::Resource(ResourceTarget::BufferLocation { buffer, row, .. }) => self
                    .buffers
                    .get(buffer)
                    .map(|buffer| buffer_content_preview(buffer, row)),
                FinderTarget::Resource(ResourceTarget::TerminalLocation {
                    terminal, row, ..
                }) => self
                    .terminals
                    .get(terminal)
                    .map(|terminal| terminal_content_preview(terminal, row)),
                _ => None,
            });
        if let Some(finder) = self.finder.as_mut() {
            finder.set_selected_preview(resource_preview);
        }
        let selected_file = self
            .finder
            .as_ref()
            .and_then(ResourceFinder::selected_match)
            .and_then(|found| match found.source {
                FinderMatchSource::File(entry) => Some(entry),
                FinderMatchSource::Resource(_) => None,
            });
        if let Some(entry) = selected_file
            && let Some(selected) = self
                .picker
                .as_ref()
                .and_then(|picker| picker.matches.iter().position(|found| found.entry == entry))
        {
            self.picker.as_mut().unwrap().selected = selected;
            self.refresh_file_picker_preview();
        }
    }

    pub fn attach_file_scanner(&mut self, scanner: FileScanner) {
        self.file_scanner = Some(scanner);
    }

    pub fn apply_file_picker_event(&mut self, event: FilePickerEvent) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        match event {
            FilePickerEvent::Files { scan_id, paths } if scan_id == picker.scan_id => {
                picker.add_paths(paths);
            }
            FilePickerEvent::Content {
                scan_id,
                mut entries,
            } if scan_id == picker.scan_id => {
                entries.retain(|hits| {
                    !self.buffers.iter().enumerate().any(|(index, buffer)| {
                        !self.closed_buffers.contains(&index)
                            && !buffer.is_directory()
                            && buffer.path.as_deref() == Some(hits.path.as_path())
                    })
                });
                let available = CONTENT_ENTRY_LIMIT.saturating_sub(
                    self.finder.as_ref().map_or(0, |finder| finder.items.len())
                        + picker.entries.len(),
                );
                let shared_limit_reached = truncate_content_hits(&mut entries, available);
                picker.add_content(entries);
                picker.limited |= shared_limit_reached;
            }
            FilePickerEvent::Finished {
                scan_id,
                skipped,
                limited,
            } if scan_id == picker.scan_id => {
                // A truncated scan is the one case where typing on ahead was
                // allowed to narrow in memory but should not have been: the
                // scan stopped before the whole project, so matches for the
                // longer query may still be out there. Learning that only
                // now, start the scan the current query deserves.
                picker.finish(skipped, limited);
                self.restart_content_scan_if_needed();
            }
            FilePickerEvent::Failed { scan_id, message } if scan_id == picker.scan_id => {
                picker.fail(message);
            }
            FilePickerEvent::Files { .. }
            | FilePickerEvent::Content { .. }
            | FilePickerEvent::Finished { .. }
            | FilePickerEvent::Failed { .. } => return,
        }
        self.merge_finder_matches();
        if self.finder.is_some() {
            self.refresh_finder_preview();
        } else {
            self.refresh_file_picker_preview();
        }
    }
}

fn buffer_content_preview(buffer: &Buffer, row: usize) -> String {
    const CONTEXT: usize = 6;
    buffer
        .lines()
        .skip(row.saturating_sub(CONTEXT))
        .take(CONTEXT * 2 + 1)
        .map(|line| line.chars().take(512).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn terminal_content_preview(terminal: &TerminalSession, row: usize) -> String {
    const CONTEXT: usize = 6;
    let start = row.saturating_sub(CONTEXT);
    (start..terminal.plain_line_count().min(start + CONTEXT * 2 + 1))
        .filter_map(|row| terminal.plain_line(row))
        .collect::<Vec<_>>()
        .join("\n")
}
