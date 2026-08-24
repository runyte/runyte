// SPDX-License-Identifier: MPL-2.0

//! Project, directory, content, and open-resource picker coordination.

// Application-module dependencies:
use super::{
    App, CONTENT_ENTRY_LIMIT, FilePicker, FilePickerEvent, FilePickerKind, FilePreview,
    FileScanner, FinderMode, PathBuf, ResourceFinder, ResourceItem, ResourceKind, ResourceTarget,
    Result, buffer_picker_columns, buffer_preview, line_hits, resource_path_fields, scan_content,
    scan_files, terminal_preview,
};

impl App {
    pub(super) fn open_project_picker(&mut self) -> Result<()> {
        self.open_picker_at(self.project_root.clone(), FilePickerKind::Files)?;
        self.finder = Some(ResourceFinder::default());
        self.rebuild_resource_finder();
        Ok(())
    }

    pub(super) fn open_directory_picker(&mut self) -> Result<()> {
        self.open_picker_at(self.active_directory(), FilePickerKind::Files)
    }

    pub(super) fn open_project_grep(&mut self) -> Result<()> {
        self.open_picker_at(self.project_root.clone(), FilePickerKind::Contents)
    }

    pub(super) fn open_directory_grep(&mut self) -> Result<()> {
        self.open_picker_at(self.active_directory(), FilePickerKind::Contents)
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
        let scan_id = self.next_file_scan_id;
        self.next_file_scan_id = self.next_file_scan_id.wrapping_add(1).max(1);
        self.picker = Some(match kind {
            FilePickerKind::Files => FilePicker::new(scan_id, root.clone()),
            FilePickerKind::Contents => FilePicker::grep(scan_id, root.clone()),
        });
        if kind == FilePickerKind::Contents {
            self.start_content_scan();
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
                let lines = line_hits(&buffer.to_string(), &query);
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

    fn rebuild_resource_finder(&mut self) {
        let Some(query) = self.picker.as_ref().map(|picker| picker.query.clone()) else {
            return;
        };
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
            items.push(
                ResourceItem::new(
                    label,
                    detail,
                    ResourceTarget::Buffer(index),
                    ResourceKind::Buffer,
                    fields,
                )
                .with_preview(buffer_preview(buffer)),
            );
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
        if let Some(finder) = self.finder.as_mut() {
            finder.replace_items(items, &query);
        }
    }

    pub(super) fn rank_resource_finder(&mut self) {
        let Some(query) = self.picker.as_ref().map(|picker| picker.query.as_str()) else {
            return;
        };
        if let Some(finder) = self.finder.as_mut() {
            finder.rank(query);
        }
    }

    pub(super) fn toggle_finder_mode(&mut self) {
        let Some(mode) = self.finder.as_ref().map(|finder| finder.mode) else {
            return;
        };
        if mode == FinderMode::Files {
            self.rebuild_resource_finder();
        }
        if let Some(finder) = self.finder.as_mut() {
            finder.mode = match mode {
                FinderMode::Files => FinderMode::Resources,
                FinderMode::Resources => FinderMode::Files,
            };
        }
        if mode == FinderMode::Resources {
            self.refresh_file_picker_preview();
        }
    }

    pub(super) fn activate_resource_target(&mut self, target: ResourceTarget) {
        self.close_file_picker();
        match target {
            ResourceTarget::Buffer(buffer) => {
                if buffer >= self.buffers.len() || self.closed_buffers.contains(&buffer) {
                    self.error("that buffer is no longer open");
                } else {
                    self.switch_buffer(buffer);
                }
            }
            ResourceTarget::Terminal(id) => {
                if self.terminals.get(id).is_none() {
                    self.error("that terminal is gone");
                } else {
                    self.show_terminal(id);
                }
            }
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
                picker.add_content(entries);
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
        if self
            .finder
            .as_ref()
            .is_none_or(|finder| finder.mode == FinderMode::Files)
        {
            self.refresh_file_picker_preview();
        }
    }
}
