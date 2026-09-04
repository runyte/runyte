// SPDX-License-Identifier: MPL-2.0

//! Project, directory, content, and open-resource picker coordination.

// Application-module dependencies:
use super::{
    App, CONTENT_ENTRY_LIMIT, FilePicker, FilePickerEvent, FilePickerKind, FilePreview,
    FileScanner, FinderContentScan, FinderContentSource, FinderMatchSource, FinderMode,
    FinderTarget, Mode, PICKER_LIST_INTERVAL, PathBuf, Range, ResourceFinder, ResourceItem,
    ResourceKind, ResourceTarget, Result, Selection, TerminalContentMark,
    TerminalContentRetirement, TerminalSession, buffer_picker_columns, buffer_preview,
    resource_path_fields, scan_content, scan_files, terminal_preview,
};

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use crate::file_picker::{
    FileHits, FilePreviewRequest, PickerTarget, ScanScope, line_hit, line_hit_from_trimmed,
};
use crate::terminal::TerminalId;

const RESOURCE_CONTENT_SLICE_ROWS: usize = 128;
const RESOURCE_CONTENT_LINE_CHARACTERS: usize = 1_024;

type FilePreviewSelection = (PickerTarget, PathBuf, bool, Option<(usize, Vec<usize>)>);

/// Whether a ranked answer names the finder's current live matches, and
/// whether it is the complete answer for them.
///
/// The second is what lets the rows be read: an answer that arrived while the
/// finder's own half was still ranking covers only part of the list.
fn rank_answer_state(
    finder: Option<&ResourceFinder>,
    finder_revision: Option<u64>,
) -> (bool, bool) {
    let current = match (finder, finder_revision) {
        (None, None) => true,
        (Some(finder), Some(revision)) => finder.file_rank_revision() == revision,
        _ => false,
    };
    (
        current,
        current && finder.is_none_or(|finder| !finder.loading),
    )
}

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
    fn request_background_file_rank(&mut self, reset_resource_selection: bool) -> bool {
        let Some(scanner) = self.file_scanner.clone() else {
            return false;
        };
        let query = self
            .picker
            .as_ref()
            .map(|picker| picker.query.clone())
            .unwrap_or_default();
        let finder = self.finder.as_mut().map(|finder| {
            if finder.mode == FinderMode::Names {
                let discarded = finder.retire_background_corpus(true);
                scanner.discard_owned(discarded);
                finder.begin_name_rank(&query, reset_resource_selection);
            }
            finder.take_file_rank_context(&query)
        });
        let Some(picker) = self.picker.as_mut() else {
            return false;
        };
        picker.ranking = true;
        scanner.rank(picker.background_rank_request(finder));
        true
    }

    fn update_background_finder_context(&mut self) {
        let Some(scanner) = self.file_scanner.clone() else {
            return;
        };
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let query = picker.query.clone();
        let scan_id = picker.scan_id;
        let query_revision = picker.query_revision;
        let finder = self
            .finder
            .as_mut()
            .map(|finder| finder.take_file_rank_context(&query));
        scanner.update_finder_context(scan_id, query_revision, finder);
    }

    /// The scope the project's own finders scan with: every ignore file from
    /// the project root down.
    pub(super) fn project_scan_scope(&self) -> ScanScope {
        ScanScope::ignoring(&self.project_root)
    }

    pub(super) fn open_project_picker(&mut self) -> Result<()> {
        self.open_finder_at(self.project_root.clone(), self.project_scan_scope())
    }

    /// The project finder over every file the project holds, ignore files not
    /// consulted. Only the scope separates it from `open_project_picker`.
    pub(super) fn open_all_files_picker(&mut self) -> Result<()> {
        self.open_finder_at(self.project_root.clone(), ScanScope::Everything)
    }

    /// The same unfiltered finder rooted at a path that need not be inside
    /// the workspace.
    pub(super) fn open_path_picker(&mut self, root: PathBuf) -> Result<()> {
        self.open_finder_at(root, ScanScope::Everything)
    }

    /// Opens the unfiltered finder at a path as a person spelled it: `~` and
    /// a relative path mean what they mean everywhere else a path is typed.
    pub(super) fn open_finder_path(&mut self, path: &std::path::Path) -> Result<()> {
        self.open_path_picker(self.resolve_working_path(path.to_path_buf()))
    }

    /// Opens the unified name-mode finder: a file picker plus the open
    /// buffers and terminals merged into its list.
    fn open_finder_at(&mut self, root: PathBuf, scope: ScanScope) -> Result<()> {
        self.open_picker_at(root, scope, FilePickerKind::Files)?;
        self.picker.as_mut().unwrap().enable_unified_finder();
        self.finder = Some(ResourceFinder::new(FinderMode::Names));
        self.rebuild_resource_finder();
        Ok(())
    }

    pub(super) fn open_directory_picker(&mut self) -> Result<()> {
        self.open_picker_at(
            self.active_directory(),
            self.project_scan_scope(),
            FilePickerKind::Files,
        )
    }

    pub(super) fn open_project_grep(&mut self) -> Result<()> {
        self.open_picker_at(
            self.project_root.clone(),
            self.project_scan_scope(),
            FilePickerKind::Contents,
        )?;
        self.picker.as_mut().unwrap().enable_unified_finder();
        self.finder = Some(ResourceFinder::new(FinderMode::Contents));
        self.start_content_scan();
        self.rebuild_resource_finder();
        Ok(())
    }

    pub(super) fn open_directory_grep(&mut self) -> Result<()> {
        self.open_picker_at(
            self.active_directory(),
            self.project_scan_scope(),
            FilePickerKind::Contents,
        )?;
        self.start_content_scan();
        Ok(())
    }

    pub(super) fn open_picker_at(
        &mut self,
        root: PathBuf,
        scope: ScanScope,
        kind: FilePickerKind,
    ) -> Result<()> {
        let root = root.canonicalize().map_err(|error| {
            anyhow::anyhow!("failed to open picker at {}: {error}", root.display())
        })?;
        anyhow::ensure!(root.is_dir(), "{} is not a directory", root.display());
        if self.picker.is_some() || self.finder.is_some() || self.finder_content_scan.is_some() {
            self.close_file_picker();
        }
        let scan_id = self.next_file_scan_id;
        self.next_file_scan_id = self.next_file_scan_id.wrapping_add(1).max(1);
        self.picker = Some(match kind {
            FilePickerKind::Files => FilePicker::new(scan_id, root.clone(), scope.clone()),
            FilePickerKind::Contents => FilePicker::grep(scan_id, root.clone(), scope.clone()),
        });
        if kind == FilePickerKind::Contents {
            return Ok(());
        }
        if let Some(scanner) = &self.file_scanner {
            scanner.scan(
                scan_id,
                root,
                scope,
                self.state_root.clone(),
                self.config.editor.show_hidden_files,
            );
        } else {
            match scan_files(
                &root,
                &scope,
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
        self.request_background_file_rank(false);
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
        let scope = picker.scope.clone();
        let previous_scan_id = picker.scan_id;
        if let Some(scanner) = &self.file_scanner {
            scanner.cancel(previous_scan_id);
        }
        let scan_id = self.next_file_scan_id;
        self.next_file_scan_id = self.next_file_scan_id.wrapping_add(1).max(1);
        let picker = self.picker.as_mut().expect("the picker was just borrowed");
        let discarded = picker.restart_content_scan(scan_id);
        if let Some(scanner) = &self.file_scanner {
            scanner.discard_picker_corpus(discarded);
        }
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
                scope,
                self.state_root.clone(),
                self.config.editor.show_hidden_files,
                query,
            );
        } else {
            match scan_content(
                &root,
                &scope,
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
        if self.file_scanner.is_some() && self.finder.is_none() {
            self.request_background_file_rank(false);
        } else {
            if self.file_scanner.is_none() {
                self.merge_finder_matches();
            }
        }
    }

    /// Re-scans when the query has moved past what the entries on hand can
    /// answer. Every path that edits a content query funnels through here.
    ///
    /// A re-scan replaces the corpus the rows on screen are read from, so one
    /// per keystroke empties the list and fills it again for every character
    /// typed. Each keystroke schedules it instead, and pushes it out again:
    /// a burst of typing costs one walk of the project, taken once the query
    /// stops moving, which is also when the reader starts reading. Until it
    /// fires the rows narrow in memory against what the last scan collected,
    /// which is a subset of the answer rather than a different one.
    pub(super) fn restart_content_scan_if_needed(&mut self) {
        if !self
            .picker
            .as_ref()
            .is_some_and(FilePicker::content_rescan_needed)
        {
            self.content_rescan_due = None;
            return;
        }
        self.content_rescan_due = Some(Instant::now() + PICKER_LIST_INTERVAL);
    }

    /// Starts a scheduled re-scan once its query has been still long enough.
    ///
    /// It scans for the query as it stands now rather than the one that
    /// scheduled it, so a burst of typing costs one re-scan rather than one
    /// for each character of it.
    pub(super) fn restart_due_content_scan(&mut self, now: Instant) -> bool {
        if self.content_rescan_due.is_none_or(|due| now < due) {
            return false;
        }
        self.content_rescan_due = None;
        if !self
            .picker
            .as_ref()
            .is_some_and(FilePicker::content_rescan_needed)
        {
            return false;
        }
        self.start_content_scan();
        // A restart hands the background ranker a new scan id, which resets
        // it: the query it was answering, that query's revision, and the
        // finder's live matches all go with the entry table they described.
        // Re-state them here rather than in the callers, because the path
        // that learns from `Finished` that the scan was truncated restarts
        // without a keystroke behind it. Left unstated, the ranker answers
        // the empty query at revision zero, the picker's own revision guard
        // discards every one of those answers, and the finder keeps showing
        // rows whose entry indices name a table that no longer exists.
        if self.file_scanner.is_some() && self.finder.is_some() {
            self.rank_resource_finder();
        }
        true
    }

    /// Advances whatever the picker's own clocks have made due.
    ///
    /// The event loop calls this when [`App::picker_pacing_delay`] says one
    /// of them has come round; both change what the reader is looking at, so
    /// the caller draws afterwards.
    pub fn advance_picker_pacing(&mut self) -> bool {
        let rescanned = self.restart_due_content_scan(Instant::now());
        let published = self.publish_paced_picker_rows();
        rescanned || published
    }

    pub(super) fn close_file_picker(&mut self) {
        self.content_rescan_due = None;
        if let Some(held) = self.held_rank.take() {
            self.discard_rank_event(held);
        }
        self.picker_rows_published = None;
        let picker = self.picker.take();
        let scan_id = picker.as_ref().map(|picker| picker.scan_id);
        let discarded = (
            picker,
            self.finder.take(),
            self.finder_content_scan.take(),
            std::mem::replace(&mut self.finder_content_sources, Arc::from([])),
            std::mem::replace(
                &mut self.finder_content_suppressed_paths,
                Arc::new(HashSet::new()),
            ),
            std::mem::take(&mut self.finder_dirty_terminals),
            std::mem::take(&mut self.finder_terminal_marks),
        );
        if let Some(scanner) = &self.file_scanner {
            if let Some(scan_id) = scan_id {
                scanner.cancel(scan_id);
            }
            scanner.discard_owned(discarded);
            scanner.close_ranker();
        } else {
            drop(discarded);
        }
    }

    pub(super) fn refresh_file_picker_preview(&mut self) {
        let selected = self.picker.as_ref().and_then(|picker| {
            let found = picker.selected_match()?;
            let entry = picker.view(found.entry)?;
            Some((
                picker.selected_target()?,
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
        self.refresh_file_preview(selected);
    }

    fn refresh_file_preview(&mut self, selected: Option<FilePreviewSelection>) {
        let Some((target, path, is_dir, content_match)) = selected else {
            if let Some(picker) = self.picker.as_mut() {
                picker.preview = None;
                picker.clear_preview_request();
            }
            return;
        };
        let live_preview =
            (!is_dir)
                .then(|| {
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
                    match (live_buffer, content_match.as_ref()) {
                        (Some(buffer), Some((row, emphasis))) => Some(
                            FilePreview::snippet_from_lines(buffer.lines(), *row, emphasis.clone()),
                        ),
                        (Some(buffer), None) => Some(FilePreview::from_lines(buffer.lines())),
                        (None, _) => None,
                    }
                })
                .flatten();
        if let Some(preview) = live_preview {
            let picker = self.picker.as_mut().unwrap();
            picker.preview = Some(preview);
            picker.clear_preview_request();
            return;
        }
        if let Some(scanner) = self.file_scanner.clone() {
            let picker = self.picker.as_mut().unwrap();
            let Some(request_id) = picker.begin_preview_request(target, content_match.as_ref())
            else {
                return;
            };
            scanner.preview(FilePreviewRequest {
                scan_id: picker.scan_id,
                query_revision: picker.query_revision,
                request_id,
                path,
                is_dir,
                content_match,
                show_hidden: self.config.editor.show_hidden_files,
            });
        } else {
            let preview = if is_dir {
                FilePreview::from_directory(&path, self.config.editor.show_hidden_files)
            } else if let Some((row, emphasis)) = content_match {
                FilePreview::snippet_from_path(&path, row, emphasis)
            } else {
                FilePreview::from_path(&path)
            };
            self.picker.as_mut().unwrap().preview = Some(preview);
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
            self.request_background_file_rank(true);
            return;
        }
        self.retire_finder_content_state();
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
            );
            if let Some(path) = buffer.path.clone() {
                item = item.with_path(path);
            }
            items.push(item);
        }
        for session in self.terminals.iter() {
            let shown = self
                .panes
                .values()
                .any(|pane| pane.terminal == Some(session.id()));
            items.push(terminal_finder_item(
                session,
                shown,
                &self.project_root,
                self.home_directory.as_deref(),
            ));
        }
        if let (Some(finder), Some(picker)) = (self.finder.as_mut(), self.picker.as_ref()) {
            if let Some(scanner) = &self.file_scanner {
                let discarded = finder.retire_background_corpus(false);
                scanner.discard_owned(discarded);
                finder.replace_items_unmerged(items, &query);
            } else {
                finder.replace_items(items, picker, &query);
            }
        }
        if self.file_scanner.is_some() {
            self.request_background_file_rank(false);
        }
        self.refresh_finder_preview();
    }

    fn start_resource_content_scan(&mut self) {
        if self.picker.is_none() {
            self.finder_content_scan = None;
            return;
        }
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
                        from: 0,
                    }),
            )
            .collect::<Arc<[_]>>();
        self.finder_content_suppressed_paths = Arc::new(
            sources
                .iter()
                .filter_map(|source| match source {
                    FinderContentSource::Buffer { path, .. } => path.clone(),
                    FinderContentSource::Terminal { .. } => None,
                })
                .collect(),
        );
        self.finder_terminal_marks = self
            .terminals
            .iter()
            .map(|terminal| {
                (
                    terminal.id(),
                    TerminalContentMark {
                        retired: terminal.retired_lines(),
                        columns: terminal.columns(),
                    },
                )
            })
            .collect();
        self.finder_content_sources = sources.clone();
        self.begin_resource_content_scan(sources);
    }

    fn restart_resource_content_scan(&mut self) {
        self.begin_resource_content_scan(self.finder_content_sources.clone());
    }

    fn discard_finder_content_scan(&mut self) {
        let Some(scan) = self.finder_content_scan.take() else {
            return;
        };
        if let Some(scanner) = &self.file_scanner {
            scanner.discard_owned(scan);
        }
    }

    fn retire_finder_content_state(&mut self) {
        let discarded = (
            self.finder_content_scan.take(),
            std::mem::replace(&mut self.finder_content_sources, Arc::from([])),
            std::mem::replace(
                &mut self.finder_content_suppressed_paths,
                Arc::new(HashSet::new()),
            ),
            std::mem::take(&mut self.finder_dirty_terminals),
            std::mem::take(&mut self.finder_terminal_marks),
        );
        if let Some(scanner) = &self.file_scanner {
            scanner.discard_owned(discarded);
        }
    }

    fn begin_resource_content_scan(&mut self, sources: Arc<[FinderContentSource]>) {
        self.discard_finder_content_scan();
        let Some(query) = self.picker.as_ref().map(|picker| picker.query.clone()) else {
            return;
        };
        let suppressed_paths = self.finder_content_suppressed_paths.clone();
        let Some((picker, finder)) = self.picker.as_ref().zip(self.finder.as_mut()) else {
            return;
        };
        if let Some(scanner) = &self.file_scanner {
            let kept = finder.take_file_rows();
            let discarded = finder.retire_background_corpus(false);
            scanner.discard_owned(discarded);
            finder.begin_content_scan_unmerged(&query, suppressed_paths);
            finder.restore_file_rows(kept);
        } else {
            finder.begin_content_scan(picker, &query, suppressed_paths.iter().cloned());
        }
        // Nothing survives a new query, so this pass has nothing to refill.
        self.finder_dirty_terminals.clear();
        self.finder_content_scan = Some(FinderContentScan {
            query,
            sources,
            source: 0,
            row: 0,
            column: 0,
            retirements: Vec::new(),
            retirement: 0,
            limited: false,
            refilling: false,
            #[cfg(test)]
            drop_observer: None,
        });
        self.refresh_finder_preview();
    }

    /// Whether the event loop should schedule another bounded live-content
    /// pass. A new query replaces this cursor, which cancels the old pass
    /// without allowing stale rows into the finder.
    pub fn resource_finder_scan_pending(&self) -> bool {
        self.finder_content_scan.is_some()
            || self
                .finder
                .as_ref()
                .is_some_and(ResourceFinder::name_rank_pending)
    }

    /// Whether the pass in flight is refilling rows it has just dropped.
    ///
    /// A refresh drops a terminal's rows before reading them back, so between
    /// the two the list has a hole where results the reader was looking at
    /// used to be. Drawing that would move the selection and put the rows
    /// back a moment later, which is the churn this pacing exists to prevent,
    /// so the event loop holds its frame until the pass ends.
    pub fn finder_scan_refills(&self) -> bool {
        self.finder_content_scan
            .as_ref()
            .is_some_and(|scan| scan.refilling)
    }

    /// Records that a terminal's output no longer matches what the finder
    /// read from it. This is all a write does: reading the terminal back is
    /// the event loop's decision, taken on a slow tick.
    pub(super) fn note_terminal_finder_change(&mut self, terminal: crate::terminal::TerminalId) {
        if self.finder.is_none() {
            return;
        }
        self.finder_dirty_terminals.insert(terminal);
    }

    /// Whether any terminal has written something the finder has not read.
    pub fn finder_terminals_dirty(&self) -> bool {
        !self.finder_dirty_terminals.is_empty()
    }

    /// Re-reads the terminals that wrote something since the finder last
    /// looked, and reports whether anything came of it.
    ///
    /// A child writing continuously produces far more states than a reader
    /// can follow, and a list whose rows are replaced faster than they can be
    /// read is unusable however cheap each rebuild is. Doing this on a tick
    /// rather than on the write costs one refresh per interval instead of one
    /// per chunk.
    ///
    /// A pass already in flight keeps its cursor: replacing it here would
    /// abandon the sources it has not reached. The terminals stay dirty and
    /// the next tick picks them up.
    pub fn refresh_finder_terminals(&mut self) -> bool {
        if self.finder_dirty_terminals.is_empty()
            || self.finder_content_scan.is_some()
            || self.picker.as_ref().is_some_and(|picker| picker.ranking)
        {
            return false;
        }
        let Some(mode) = self.finder.as_ref().map(|finder| finder.mode) else {
            self.finder_dirty_terminals.clear();
            return false;
        };
        let dirty = std::mem::take(&mut self.finder_dirty_terminals);
        match mode {
            FinderMode::Names => {
                let mut changed = false;
                for terminal in dirty {
                    changed |= self.refresh_terminal_finder_item(terminal);
                }
                changed
            }
            FinderMode::Contents => {
                self.start_terminal_content_refresh(dirty);
                true
            }
        }
    }

    /// Re-reads one terminal's name-mode item, and reports whether the list
    /// changed because of it.
    ///
    /// Output is what marks a terminal dirty, but a name-mode item describes
    /// the session rather than its output, so most refreshes find exactly
    /// what they already had. Ranking those anyway would replace every row
    /// in the list once an interval for the whole time a child is writing.
    fn refresh_terminal_finder_item(&mut self, terminal: crate::terminal::TerminalId) -> bool {
        let Some(session) = self.terminals.get(terminal) else {
            return false;
        };
        let shown = self
            .panes
            .values()
            .any(|pane| pane.terminal == Some(terminal));
        let item = terminal_finder_item(
            session,
            shown,
            &self.project_root,
            self.home_directory.as_deref(),
        );
        let Some(query) = self.picker.as_ref().map(|picker| picker.query.clone()) else {
            return false;
        };
        if self
            .finder
            .as_ref()
            .is_none_or(|finder| !finder.terminal_item_differs(terminal, &item))
        {
            return false;
        }
        if self.file_scanner.is_some() {
            let Some((finder, picker)) = self.finder.as_mut().zip(self.picker.as_mut()) else {
                return false;
            };
            finder.preserve_selection(picker);
            picker.ranking = true;
            finder.replace_terminal_unmerged(terminal, item, &query);
        } else {
            let Some((finder, picker)) = self.finder.as_mut().zip(self.picker.as_ref()) else {
                return false;
            };
            finder.replace_terminal(terminal, item, picker, &query);
        }
        self.update_background_finder_context();
        self.refresh_finder_preview();
        true
    }

    /// Revisits the terminals that changed after an otherwise complete content
    /// scan. Every other live result, and a claimed selection, stay in place
    /// while their bounded rows are read again.
    pub(super) fn start_terminal_content_refresh(&mut self, terminals: HashSet<TerminalId>) {
        self.discard_finder_content_scan();
        let Some(query) = self.picker.as_ref().map(|picker| picker.query.clone()) else {
            return;
        };
        // A terminal that is gone still has rows to drop, so every dirty
        // session names what it keeps and only the surviving ones are read.
        let mut sources = Vec::new();
        let mut retirement_specs = Vec::with_capacity(terminals.len());
        for &terminal in &terminals {
            let Some(session) = self.terminals.get(terminal) else {
                self.finder_terminal_marks.remove(&terminal);
                retirement_specs.push((terminal, 0));
                continue;
            };
            let mark = TerminalContentMark {
                retired: session.retired_lines(),
                columns: session.columns(),
            };
            let scrollback = session.scrollback_rows();
            // Three things make earlier rows unusable rather than merely old,
            // and all of them start this session over from its first row: a
            // retired count that has gone backwards, which is a different
            // screen rather than later output; a width that has changed,
            // which rewrote every retained line in place; and a session
            // nothing has read yet.
            let from = match self.finder_terminal_marks.get(&terminal) {
                Some(read) if read.columns == mark.columns && mark.retired >= read.retired => {
                    let added = usize::try_from(mark.retired - read.retired).unwrap_or(usize::MAX);
                    scrollback.saturating_sub(added)
                }
                _ => 0,
            };
            let label = session.display_name();
            retirement_specs.push((terminal, from));
            sources.push(FinderContentSource::Terminal {
                terminal,
                label,
                from,
            });
            self.finder_terminal_marks.insert(terminal, mark);
        }
        let limited = self.finder.as_ref().is_some_and(|finder| finder.limited);
        if self.file_scanner.is_some() {
            let Some((finder, picker)) = self.finder.as_mut().zip(self.picker.as_mut()) else {
                return;
            };
            finder.preserve_selection(picker);
            picker.ranking = true;
            let retirements = retirement_specs
                .into_iter()
                .map(|(terminal, retained_until)| TerminalContentRetirement {
                    terminal,
                    items: finder.take_terminal_content_items(terminal),
                    item: 0,
                    retained_row: 0,
                    retained_until,
                })
                .collect();
            self.finder_content_scan = Some(FinderContentScan {
                row: sources.first().map_or(0, FinderContentSource::first_row),
                query,
                sources: sources.into(),
                source: 0,
                column: 0,
                retirements,
                retirement: 0,
                limited,
                refilling: true,
                #[cfg(test)]
                drop_observer: None,
            });
        } else {
            let kept = retirement_specs
                .into_iter()
                .map(|(terminal, retained_until)| {
                    let lines = self
                        .terminals
                        .get(terminal)
                        .map(|session| {
                            session
                                .retained_line_ids()
                                .take(retained_until)
                                .collect::<HashSet<_>>()
                        })
                        .unwrap_or_default();
                    (terminal, lines)
                })
                .collect();
            let Some((finder, picker)) = self.finder.as_mut().zip(self.picker.as_ref()) else {
                return;
            };
            finder.retain_terminal_content(&kept, picker);
            self.finder_content_scan = Some(FinderContentScan {
                row: sources.first().map_or(0, FinderContentSource::first_row),
                query,
                sources: sources.into(),
                source: 0,
                column: 0,
                retirements: Vec::new(),
                retirement: 0,
                limited,
                refilling: true,
                #[cfg(test)]
                drop_observer: None,
            });
        }
        self.refresh_finder_preview();
    }

    /// Advances live buffer and terminal matching without materializing the
    /// complete corpus or holding the input loop for an unbounded scan.
    pub fn advance_resource_finder_scan(&mut self) {
        let Some(mut scan) = self.finder_content_scan.take() else {
            let publish = self
                .finder
                .as_mut()
                .is_some_and(|finder| finder.advance_name_rank(RESOURCE_CONTENT_SLICE_ROWS));
            if publish {
                self.update_background_finder_context();
                self.refresh_finder_preview();
            }
            return;
        };
        let mut visited = 0usize;
        let mut found = Vec::new();
        let mut retired = Vec::new();
        while scan.retirement < scan.retirements.len() && visited < RESOURCE_CONTENT_SLICE_ROWS {
            let retirement = &mut scan.retirements[scan.retirement];
            if retirement.item >= retirement.items.len() {
                scan.retirement += 1;
                continue;
            }
            let (line_id, item) = retirement.items[retirement.item];
            let retained = (retirement.retained_row < retirement.retained_until)
                .then(|| {
                    self.terminals
                        .get(retirement.terminal)?
                        .retained_line_id(retirement.retained_row)
                })
                .flatten();
            visited += 1;
            match retained.map(|retained| retained.cmp(&line_id)) {
                Some(std::cmp::Ordering::Less) => retirement.retained_row += 1,
                Some(std::cmp::Ordering::Equal) => {
                    retirement.retained_row += 1;
                    retirement.item += 1;
                    if let Some(finder) = self.finder.as_mut() {
                        finder.keep_terminal_content_item(retirement.terminal, line_id, item);
                    }
                }
                Some(std::cmp::Ordering::Greater) | None => {
                    retirement.item += 1;
                    if let Some(finder) = self.finder.as_mut()
                        && let Some(item) = finder.retire_content_item(item, true)
                    {
                        retired.push(item);
                    }
                }
            }
        }
        if !retired.is_empty()
            && let Some(scanner) = &self.file_scanner
        {
            scanner.discard_owned(retired);
        }
        while scan.source < scan.sources.len() && visited < RESOURCE_CONTENT_SLICE_ROWS {
            if found.len()
                + self.finder.as_ref().map_or(0, |finder| {
                    if self.file_scanner.is_some() {
                        finder.content_item_count()
                    } else {
                        finder.items.len()
                    }
                })
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
                if let FinderContentSource::Terminal { terminal, .. } = source
                    && let Some(session) = self.terminals.get(terminal)
                {
                    if self.finder_dirty_terminals.contains(&terminal) {
                        // The retained-row sequence changed while this cursor
                        // was merging stable identities. Its row cursor was a
                        // reading of the old sequence, so no incremental mark
                        // may survive: the already-queued refresh must start
                        // at zero and repair both false keeps and false drops.
                        self.finder_terminal_marks.remove(&terminal);
                    } else {
                        self.finder_terminal_marks.insert(
                            terminal,
                            TerminalContentMark {
                                retired: session.retired_lines(),
                                columns: session.columns(),
                            },
                        );
                    }
                }
                scan.source += 1;
                scan.row = scan
                    .sources
                    .get(scan.source)
                    .map_or(0, FinderContentSource::first_row);
                scan.column = 0;
                continue;
            }
            let row = scan.row;
            let decoded = match &source {
                FinderContentSource::Buffer { buffer, .. } => {
                    let Some(buffer) = self.buffers.get(*buffer) else {
                        scan.row += 1;
                        scan.column = 0;
                        continue;
                    };
                    let line_len = buffer.line_len(row);
                    let chunk = buffer.line_slice_string(
                        row,
                        scan.column,
                        RESOURCE_CONTENT_LINE_CHARACTERS,
                    );
                    visited += 1;
                    let chunk_characters = chunk.chars().count();
                    let leading = chunk
                        .chars()
                        .take_while(|character| character.is_whitespace())
                        .count();
                    if leading == chunk_characters && scan.column + leading < line_len {
                        scan.column += leading;
                        continue;
                    }
                    let column = scan.column + leading;
                    scan.row += 1;
                    scan.column = 0;
                    if column >= line_len {
                        continue;
                    }
                    let text =
                        buffer.line_slice_string(row, column, RESOURCE_CONTENT_LINE_CHARACTERS);
                    line_hit_from_trimmed(&text, &scan.query, column).map(|hit| (hit, None))
                }
                FinderContentSource::Terminal { terminal, .. } => {
                    scan.row += 1;
                    scan.column = 0;
                    visited += 1;
                    self.terminals.get(*terminal).and_then(|session| {
                        let (line_id, line) = session.plain_line_with_id(row)?;
                        let hit = line_hit(&line, &scan.query)?;
                        Some((hit, Some((line_id, session.output_line_number(row)))))
                    })
                }
            };
            let Some((hit, terminal_line)) = decoded else {
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
                FinderContentSource::Terminal {
                    terminal, label, ..
                } => {
                    let Some((line_id, number)) = terminal_line else {
                        continue;
                    };
                    // The row index moves as bounded history fills and is
                    // evicted, and a refresh no longer re-reads the rows it
                    // already has, so a row keeps the number it was labelled
                    // with. Number by place in the child's whole output,
                    // which does not move.
                    ResourceItem::content(
                        format!("{label}:{number}"),
                        hit.text,
                        ResourceTarget::TerminalLocation {
                            terminal,
                            line_id,
                            column: hit.column,
                        },
                        ResourceKind::Terminal,
                    )
                }
            };
            found.push(item);
        }

        let query = scan.query.clone();
        let mut publish = false;
        if let (Some(finder), Some(picker)) = (self.finder.as_mut(), self.picker.as_ref()) {
            if self.file_scanner.is_some() {
                publish = finder.append_content_items_unmerged(found, &query);
            } else {
                finder.append_items(found, picker, &query);
            }
        }
        // Terminals that wrote while this pass ran stay dirty. Folding them
        // in here would let a running child keep one pass alive indefinitely,
        // which is the churn the refresh tick exists to bound.
        let finished =
            scan.retirement >= scan.retirements.len() && scan.source >= scan.sources.len();
        let repair = if finished && scan.refilling && self.file_scanner.is_some() {
            scan.sources
                .iter()
                .filter_map(|source| match source {
                    FinderContentSource::Terminal { terminal, .. }
                        if self.finder_dirty_terminals.contains(terminal) =>
                    {
                        Some(*terminal)
                    }
                    _ => None,
                })
                .collect::<HashSet<_>>()
        } else {
            HashSet::new()
        };
        if finished && let Some(finder) = self.finder.as_mut() {
            if self.file_scanner.is_some() {
                finder.finish_content_scan_unmerged(scan.limited);
                publish = true;
            } else {
                finder.finish_content_scan(scan.limited);
            }
        }
        if !repair.is_empty() {
            // The worker needs this slot delta before the repair starts
            // reusing completed-pass slots. Its result remains inert: the
            // repair advances the live revision before this turn returns,
            // and the next cursor remains a refill so no hole is drawn.
            self.update_background_finder_context();
            if let Some(scanner) = self.file_scanner.clone() {
                scanner.discard_owned(scan);
            }
            for terminal in &repair {
                self.finder_dirty_terminals.remove(terminal);
            }
            self.start_terminal_content_refresh(repair);
            return;
        }
        if !finished {
            self.finder_content_scan = Some(scan);
        }
        if publish || self.file_scanner.is_none() {
            self.update_background_finder_context();
            self.refresh_finder_preview();
        }
    }

    pub(super) fn rank_resource_finder(&mut self) {
        if self.file_scanner.is_some() {
            if self
                .finder
                .as_ref()
                .is_some_and(|finder| finder.mode == FinderMode::Contents)
            {
                self.restart_resource_content_scan();
                self.request_background_file_rank(true);
                return;
            }
            self.request_background_file_rank(true);
            return;
        }
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
        let discarded = self.picker.as_mut().unwrap().switch_kind(scan_id, kind);
        if let Some(scanner) = &self.file_scanner {
            scanner.discard_picker_corpus(discarded);
        }
        match next {
            FinderMode::Contents => {
                self.start_content_scan();
                self.rebuild_resource_finder();
                if self.file_scanner.is_none() {
                    self.advance_resource_finder_scan();
                }
            }
            FinderMode::Names => {
                self.start_file_scan();
                self.rebuild_resource_finder();
            }
        }
    }

    fn start_file_scan(&mut self) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let scan_id = picker.scan_id;
        let root = picker.root.clone();
        let scope = picker.scope.clone();
        if let Some(scanner) = &self.file_scanner {
            scanner.scan(
                scan_id,
                root,
                scope,
                self.state_root.clone(),
                self.config.editor.show_hidden_files,
            );
        } else {
            match scan_files(
                &root,
                &scope,
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
                line_id,
                column: _,
            }) => {
                if self.terminals.get(terminal).is_none() {
                    self.action_failed("that terminal is gone");
                    return;
                }
                let captured = self
                    .terminals
                    .get_mut(terminal)
                    .is_some_and(|session| session.begin_review_at_line(line_id));
                if !captured {
                    self.action_failed("that terminal line is no longer retained");
                    return;
                }
                self.terminals.enforce_memory_budget();
                if !self
                    .terminals
                    .get(terminal)
                    .is_some_and(TerminalSession::reviewing)
                {
                    self.action_failed("that terminal line exceeds the retained review budget");
                    return;
                }
                // Capture before moving the terminal: the destination pane may
                // be shorter, and resizing its live grid can trim bottom rows.
                // The immutable review keeps the selected identity intact.
                self.show_terminal(terminal);
                let (_, rows) = self.pane_cells(self.active_pane);
                let scroll_offset = self.config.editor.scroll_offset;
                if let Some(session) = self.terminals.get_mut(terminal) {
                    session.focus_review_selection(rows.max(1), scroll_offset);
                }
                self.mode = Mode::Normal;
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
        // A content match ranked the trimmed text of one row, so its
        // emphasis is relative to that text. Shifting by the row's own
        // indent puts the positions back in the line the preview shows.
        let selected =
            self.finder
                .as_ref()
                .zip(self.picker.as_ref())
                .and_then(|(finder, picker)| {
                    let emphasis = finder.selected_match()?.detail_emphasis.clone();
                    Some((finder.selected_target(picker)?, emphasis))
                });
        let resource_preview = selected.and_then(|(target, emphasis)| {
            let shifted = |column: usize| {
                emphasis
                    .iter()
                    .map(|position| position + column)
                    .collect::<Vec<_>>()
            };
            match target {
                FinderTarget::Resource(ResourceTarget::Buffer(buffer)) => self
                    .buffers
                    .get(buffer)
                    .map(|buffer| FilePreview::from_text(&buffer_preview(buffer))),
                FinderTarget::Resource(ResourceTarget::BufferLocation {
                    buffer,
                    row,
                    column,
                }) => self.buffers.get(buffer).map(|buffer| {
                    FilePreview::snippet_from_lines(buffer.lines(), row, shifted(column))
                }),
                FinderTarget::Resource(ResourceTarget::Terminal(terminal)) => self
                    .terminals
                    .get(terminal)
                    .map(|terminal| FilePreview::from_text(&terminal_preview(terminal))),
                FinderTarget::Resource(ResourceTarget::TerminalLocation {
                    terminal,
                    line_id,
                    column,
                }) => self.terminals.get(terminal).and_then(|terminal| {
                    terminal_content_preview(terminal, line_id, shifted(column))
                }),
                _ => None,
            }
        });
        if let Some(finder) = self.finder.as_mut() {
            finder.set_selected_preview(resource_preview);
        }
        let selected_file =
            self.finder
                .as_ref()
                .zip(self.picker.as_ref())
                .and_then(|(finder, picker)| {
                    let found = finder.selected_match()?;
                    let FinderMatchSource::File(entry) = found.source else {
                        return None;
                    };
                    Some(finder.file_entry(picker, entry).and_then(|entry| {
                        // The rows deliberately remain visible while a new
                        // query ranks, but their stored emphasis still answers
                        // the preceding query. Previewing one of those rows
                        // under the new query revision used to bless the stale
                        // spans as current. Re-score just this selected line —
                        // bounded to the content candidate size — so the cheap
                        // retained preview is truthful without re-ranking the
                        // list on the input thread.
                        let emphasis = match (entry.row, entry.text) {
                            (Some(_), Some(text)) => {
                                let mut matcher =
                                    crate::file_picker::FuzzyMatcher::for_lines(&picker.query);
                                matcher.score(text)?.1
                            }
                            _ => found.emphasis.clone(),
                        };
                        let content_match = entry.row.map(|row| {
                            (
                                row,
                                emphasis
                                    .iter()
                                    .map(|position| entry.column + position)
                                    .collect::<Vec<_>>(),
                            )
                        });
                        let target = PickerTarget {
                            path: entry.path.to_path_buf(),
                            row: entry.row,
                            column: entry.column
                                + entry
                                    .row
                                    .and_then(|_| emphasis.first().copied())
                                    .unwrap_or(0),
                        };
                        Some((
                            target,
                            entry.path.to_path_buf(),
                            entry.is_dir,
                            content_match,
                        ))
                    }))
                });
        if let Some(selected) = selected_file {
            // A content re-scan keeps one old corpus specifically so the
            // rows already on screen remain readable while their replacement
            // ranks. Preview that same scan-aware row; looking its bare entry
            // index up in the new picker's matches either finds nothing or,
            // worse, finds an unrelated row at the reused index.
            self.refresh_file_preview(selected);
        }
    }

    pub fn attach_file_scanner(&mut self, scanner: FileScanner) {
        self.file_scanner = Some(scanner);
    }

    /// Applies a scanner or ranker event, pacing the rows it would replace.
    ///
    /// Every other frontend-visible effect lands at once; only the ranked
    /// answer waits, and only for as long as the list under the reader is
    /// younger than [`PICKER_LIST_INTERVAL`].
    pub fn apply_file_picker_event(&mut self, event: FilePickerEvent) {
        let Some(event) = self.hold_paced_rank(event) else {
            return;
        };
        self.apply_file_picker_event_now(event);
    }

    /// Holds back the answer to the query on screen while its rows are young.
    ///
    /// A result the picker's own guards would discard passes straight
    /// through, so its buffers go back to the ranker at once rather than
    /// waiting out an interval to be thrown away. A newer answer replaces a
    /// held one rather than queueing behind it: the reader is shown the
    /// current state of the list, not every state it passed through.
    fn hold_paced_rank(&mut self, event: FilePickerEvent) -> Option<FilePickerEvent> {
        let FilePickerEvent::Ranked {
            scan_id,
            query_revision,
            matches,
            finder_matches,
            flushed,
            ..
        } = &event
        else {
            return Some(event);
        };
        let Some(picker) = self.picker.as_ref() else {
            return Some(event);
        };
        if *scan_id != picker.scan_id || *query_revision != picker.query_revision {
            return Some(event);
        }
        // Whether the reader has rows to be choosing from at all.
        let showing = self
            .finder
            .as_ref()
            .map_or(!picker.matches.is_empty(), |finder| {
                !finder.matches.is_empty()
            });
        // A content walk collects the lines one query matches, so while it is
        // still running an answer with no rows means it has not found any
        // yet, not that the query has none. Installing that would empty the
        // list and fill it again a moment later, which is what a re-scan on
        // every keystroke used to look like. The flush a finished scan asks
        // for is exempt: that one is the answer.
        // Only the half the reader is looking at counts: with a finder open
        // the list is its merged rows, and an answer carrying no finder half
        // leaves those rows alone rather than emptying them.
        let answered_nothing = match (self.finder.as_ref(), finder_matches.as_ref()) {
            (Some(_), Some(found)) => found.is_empty(),
            (Some(_), None) => false,
            (None, _) => matches.is_empty(),
        };
        if answered_nothing
            && !*flushed
            && showing
            && picker.loading
            && picker.kind == FilePickerKind::Contents
        {
            self.discard_rank_event(event);
            return None;
        }
        // An empty list has no rows to hold still, and a picker that has just
        // opened is waiting on the scanner rather than on pacing, so the
        // answer that first fills one lands as it arrives.
        let now = Instant::now();
        if !showing
            || self
                .picker_rows_published
                .is_none_or(|shown| now.saturating_duration_since(shown) >= PICKER_LIST_INTERVAL)
        {
            self.picker_rows_published = Some(now);
            return Some(event);
        }
        if let Some(replaced) = self.held_rank.replace(event) {
            self.discard_rank_event(replaced);
        }
        None
    }

    /// Whether the answer pacing is holding would let the rows be read.
    ///
    /// A key that reads the list publishes that answer before it runs, so
    /// what the reader is offered has to be what they will get: an answer
    /// still short of the finder's half, or of the flush a finished scan
    /// asked for, leaves the rows inert whichever key publishes it.
    pub(crate) fn held_rank_releases_rows(&self) -> bool {
        let Some(FilePickerEvent::Ranked {
            finder_revision,
            flushed,
            ..
        }) = self.held_rank.as_ref()
        else {
            return false;
        };
        let Some(picker) = self.picker.as_ref() else {
            return false;
        };
        rank_answer_state(self.finder.as_ref(), *finder_revision).1
            && (*flushed || !picker.awaiting_final_rank())
    }

    /// Shows the ranked answer pacing has been holding back, if there is one.
    ///
    /// Returns whether the rows moved, so a caller drawing on its own clock
    /// knows there is a frame worth publishing. A key that reads the list
    /// calls this before it runs: pacing holds an answer back from the
    /// reader, never from the reader's own keys.
    pub fn publish_paced_picker_rows(&mut self) -> bool {
        let Some(event) = self.held_rank.take() else {
            return false;
        };
        self.picker_rows_published = Some(Instant::now());
        self.apply_file_picker_event_now(event);
        true
    }

    /// Returns a ranked answer nothing will show to the ranker's free lists.
    fn discard_rank_event(&self, event: FilePickerEvent) {
        let FilePickerEvent::Ranked {
            matches,
            match_positions,
            finder_matches,
            finder_positions,
            ..
        } = event
        else {
            return;
        };
        if let Some(scanner) = self.file_scanner.as_ref() {
            scanner.discard_rank_result(
                matches,
                finder_matches.unwrap_or_default(),
                match_positions,
                finder_positions,
            );
        }
    }

    fn apply_file_picker_event_now(&mut self, event: FilePickerEvent) {
        let scanner = self.file_scanner.clone();
        if self.picker.is_none() {
            if let FilePickerEvent::Ranked {
                matches,
                match_positions,
                finder_matches,
                finder_positions,
                ..
            } = event
                && let Some(scanner) = scanner.as_ref()
            {
                scanner.discard_rank_result(
                    matches,
                    finder_matches.unwrap_or_default(),
                    match_positions,
                    finder_positions,
                );
            }
            return;
        }
        let picker = self.picker.as_mut().unwrap();
        match event {
            FilePickerEvent::Files { scan_id, paths } if scan_id == picker.scan_id => {
                if let Some(scanner) = scanner.as_ref() {
                    let candidates = picker.add_paths_unranked(paths);
                    scanner.add_rank_candidates(scan_id, candidates);
                    return;
                } else {
                    picker.add_paths(paths);
                }
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
                if let Some(scanner) = scanner.as_ref() {
                    let candidates = picker.add_content_unranked(entries);
                    scanner.add_rank_candidates(scan_id, candidates);
                    picker.limited |= shared_limit_reached;
                    return;
                } else {
                    picker.add_content(entries);
                }
                picker.limited |= shared_limit_reached;
            }
            FilePickerEvent::Ranked {
                scan_id,
                query_revision,
                matches,
                match_positions,
                finder_matches,
                finder_revision,
                finder_positions,
                flushed,
            } if scan_id == picker.scan_id && query_revision == picker.query_revision => {
                let (finder_current, finder_complete) =
                    rank_answer_state(self.finder.as_ref(), finder_revision);
                let old_matches = picker.apply_background_matches(
                    matches,
                    &match_positions,
                    finder_complete,
                    flushed,
                );
                let mut discarded_finder_matches = finder_matches.unwrap_or_default();
                if finder_current
                    && let (Some(finder), Some(_)) = (self.finder.as_mut(), finder_revision)
                {
                    discarded_finder_matches = finder.apply_background_matches(
                        scan_id,
                        std::mem::take(&mut discarded_finder_matches),
                        &finder_positions,
                    );
                }
                // Once both halves name the new corpus, the one a re-scan
                // replaced has no reader left.
                let retired_corpus = (finder_current || self.finder.is_none())
                    .then(|| {
                        self.picker
                            .as_mut()
                            .and_then(FilePicker::forget_previous_corpus)
                    })
                    .flatten();
                if let Some(scanner) = scanner.as_ref() {
                    if let Some(corpus) = retired_corpus {
                        scanner.discard_owned(corpus);
                    }
                    scanner.discard_rank_result(
                        old_matches,
                        discarded_finder_matches,
                        match_positions,
                        finder_positions,
                    );
                }
            }
            FilePickerEvent::Preview {
                scan_id,
                query_revision,
                request_id,
                preview,
            } if scan_id == picker.scan_id && query_revision == picker.query_revision => {
                if picker.preview_request_id() == Some(request_id) {
                    picker.preview = Some(preview);
                }
                return;
            }
            FilePickerEvent::Ranked {
                matches,
                match_positions,
                finder_matches,
                finder_positions,
                ..
            } => {
                if let Some(scanner) = scanner.as_ref() {
                    scanner.discard_rank_result(
                        matches,
                        finder_matches.unwrap_or_default(),
                        match_positions,
                        finder_positions,
                    );
                }
                return;
            }
            FilePickerEvent::Finished {
                scan_id,
                skipped,
                limited,
            } if scan_id == picker.scan_id => {
                if let Some(scanner) = scanner.as_ref() {
                    // The flush is the first ranking request for any tail
                    // smaller than RANK_PUBLISH_BATCH. Keep the rows inert
                    // until its Ranked event installs that complete answer:
                    // a publish already in flight answers a shorter list.
                    picker.begin_final_rank();
                    scanner.flush_rank(scan_id);
                }
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
            | FilePickerEvent::Preview { .. }
            | FilePickerEvent::Finished { .. }
            | FilePickerEvent::Failed { .. } => return,
        }
        if scanner.is_none() {
            self.merge_finder_matches();
        }
        if self.finder.is_some() {
            self.refresh_finder_preview();
        } else {
            self.refresh_file_picker_preview();
        }
    }
}

/// Scrollback around a matching terminal row, read by index.
///
/// Terminal history answers a row directly, so this takes the snippet's own
/// range rather than skipping a line iterator over everything before it.
fn terminal_content_preview(
    terminal: &TerminalSession,
    line_id: crate::terminal::TerminalLineId,
    emphasis: Vec<usize>,
) -> Option<FilePreview> {
    let row = terminal.retained_line_row(line_id)?;
    let rows = FilePreview::snippet_rows(row);
    let end = rows.end.min(terminal.plain_line_count());
    // The iterator is positioned with retained indices, but those indices
    // shift whenever bounded history evicts a row. Snippet labels use the
    // terminal's stable, zero-based output positions instead.
    let start_row = terminal
        .output_line_number(rows.start)
        .checked_sub(1)?
        .try_into()
        .ok()?;
    let focus_row = terminal
        .output_line_number(row)
        .checked_sub(1)?
        .try_into()
        .ok()?;
    Some(FilePreview::snippet_from_rows(
        (rows.start..end).filter_map(|row| terminal.plain_line(row)),
        start_row,
        focus_row,
        emphasis,
    ))
}

fn terminal_finder_item(
    session: &TerminalSession,
    shown: bool,
    project_root: &std::path::Path,
    home_directory: Option<&std::path::Path>,
) -> ResourceItem {
    let mut detail = format!("#{} · {}", session.id(), session.directory().display());
    if shown {
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
        project_root,
        home_directory,
    ));
    fields.extend(resource_path_fields(
        session.initial_directory(),
        project_root,
        home_directory,
    ));
    ResourceItem::new(
        session.display_name(),
        detail,
        ResourceTarget::Terminal(session.id()),
        ResourceKind::Terminal,
        fields,
    )
}
