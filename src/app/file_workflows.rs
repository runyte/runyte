// SPDX-License-Identifier: MPL-2.0

//! File, directory-buffer, pane, and side-by-side comparison workflows.

// Application-module dependencies:
use super::{
    App, Axis, Buffer, BufferKind, ContentAlignment, DiffSession, DiffSide,
    DirectoryReloadConfirmation, DirectoryView, DocumentSyntax, FileObservation,
    FileReloadConfirmation, FsConfirmation, FsOperation, FsPlan, GeneratedViewIdentity, HashSet,
    InputGrammar, Layout, MAX_DIFF_BYTES, MaximizedPane, MaximizedView, Mode, PaneDirectory, Path,
    PathBuf, PromptKind, Result, Selection, SelectionSemantics, Side, TerminalId, Transaction,
    TransferMode, bail, buffer_language, diff_row_for_identity, diff_row_identity, enclosing_area,
    ensure, expand_home_path, external_open, fs, open_or_new_at_identity, parse_buffer,
    resolved_operation_path, trailing_whitespace_changes,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReloadDispatch {
    Directory,
    GitStatus,
    GitBranches,
    GitWorktrees,
    GitLog,
    GitStash,
    File,
}

/// What a split reports, named for the border it draws rather than for the
/// axis it splits along.
fn split_status(axis: Axis) -> &'static str {
    match axis {
        Axis::Horizontal => "vertical split",
        Axis::Vertical => "horizontal split",
    }
}

pub(super) fn reload_dispatch(kind: &BufferKind) -> ReloadDispatch {
    match kind {
        BufferKind::Directory => ReloadDispatch::Directory,
        BufferKind::GitStatus => ReloadDispatch::GitStatus,
        BufferKind::GitBranches => ReloadDispatch::GitBranches,
        BufferKind::GitWorktrees => ReloadDispatch::GitWorktrees,
        BufferKind::GitLog => ReloadDispatch::GitLog,
        BufferKind::GitStash => ReloadDispatch::GitStash,
        _ => ReloadDispatch::File,
    }
}

impl App {
    pub(super) fn open_explorer(&mut self, requested: Option<PathBuf>) -> Result<()> {
        let root = if let Some(path) = requested {
            self.resolve_working_path(path)
        } else {
            self.working_directory.clone()
        };
        let root = fs::canonicalize(&root)
            .map_err(|error| anyhow::anyhow!("failed to open {}: {error}", root.display()))?;
        anyhow::ensure!(root.is_dir(), "{} is not a directory", root.display());
        self.open_file(root)
    }

    pub(super) fn open_active_directory_explorer(&mut self) -> Result<()> {
        let file = if matches!(self.active_buffer().kind, BufferKind::File) {
            self.active_buffer().path.clone()
        } else {
            None
        };
        let directory = self.active_directory();
        self.open_explorer(Some(directory))?;
        if let Some(file) = file {
            if let Some(confirmation) = &mut self.directory_reload_confirmation {
                confirmation.focus_entry = Some(file);
            } else {
                self.focus_directory_entry(&file);
            }
        }
        Ok(())
    }

    /// Files contribute their parent, explorers their current directory, and
    /// pathless views fall back to the directory controlled by `:cd`.
    pub(super) fn active_directory(&self) -> PathBuf {
        self.buffer_directory(self.active().buffer)
            .unwrap_or_else(|| self.working_directory.clone())
    }

    pub(super) fn quit_here_directory(&self) -> PathBuf {
        self.buffer_directory(self.active().buffer)
            .or_else(|| self.active().last_explorer_directory.clone())
            .unwrap_or_else(|| self.working_directory.clone())
    }

    pub(super) fn buffer_directory(&self, buffer: usize) -> Option<PathBuf> {
        let buffer = &self.buffers[buffer];
        buffer
            .path
            .as_deref()
            .and_then(|path| {
                if buffer.is_directory() {
                    Some(path)
                } else {
                    path.parent()
                }
            })
            .map(Path::to_path_buf)
    }

    pub(super) fn change_directory(&mut self, requested: PathBuf) -> Result<()> {
        let requested = self.resolve_working_path(requested);
        let directory = fs::canonicalize(&requested).map_err(|error| {
            anyhow::anyhow!(
                "failed to change directory to {}: {error}",
                requested.display()
            )
        })?;
        anyhow::ensure!(
            directory.is_dir(),
            "cannot change directory to {}: not a directory",
            directory.display()
        );

        self.working_directory = directory.clone();
        if self.active_buffer().is_directory() {
            self.open_file(directory.clone())?;
            // A dirty explorer leaves its own discard question in the status
            // line. The working directory has still changed; confirmation is
            // only about replacing the edited directory listing.
            if self.directory_reload_confirmation.is_some() {
                return Ok(());
            }
        }
        self.status(format!("working directory: {}", directory.display()));
        Ok(())
    }

    pub(super) fn resolve_working_path(&self, path: PathBuf) -> PathBuf {
        let path = expand_home_path(path, self.home_directory.as_deref());
        if path.is_absolute() {
            path
        } else {
            self.working_directory.join(path)
        }
    }

    pub(super) fn selected_directory_entry(&self) -> Result<Option<PathBuf>> {
        let row = self.cursor_position().row;
        self.active_buffer().directory_entry_path(row)
    }

    pub(super) fn open_directory_entry(&mut self) -> Result<()> {
        let Some(path) = self.selected_directory_entry()? else {
            self.status("no directory entry on this row");
            return Ok(());
        };
        self.open_file(path)?;
        // An explorer entry is an object to visit, not text to keep
        // extending. This also collapses a selection produced by `/` before
        // the next buffer or directory listing receives input. A dirty
        // explorer has not moved yet; its confirmation completes this step.
        if self.directory_reload_confirmation.is_none()
            && self.prompt_kind != PromptKind::ExternalProgram
        {
            self.enter_normal_mode();
        }
        Ok(())
    }

    pub(super) fn open_parent_directory(&mut self) -> Result<()> {
        let Some((child, parent)) = self
            .active_buffer()
            .path
            .as_deref()
            .and_then(|child| Path::parent(child).map(|parent| (child, parent)))
            .map(|(child, parent)| (child.to_path_buf(), parent.to_path_buf()))
        else {
            self.status("already at the filesystem root");
            return Ok(());
        };
        self.open_file(parent)?;
        if let Some(confirmation) = &mut self.directory_reload_confirmation {
            confirmation.focus_entry = Some(child);
        } else {
            self.focus_directory_entry(&child);
            self.enter_normal_mode();
        }
        Ok(())
    }

    /// Selects `path` in the active explorer when it is present in the
    /// projection. A hidden child can legitimately be absent when `-` opens a
    /// parent whose dotfiles are filtered, in which case the ordinary saved
    /// view (or the first row) remains intact.
    pub(super) fn focus_directory_entry(&mut self, path: &Path) {
        let buffer = self.active().buffer;
        if !self.buffers[buffer].is_directory() {
            return;
        }
        let row = (0..self.buffers[buffer].len_lines()).find(|row| {
            self.buffers[buffer]
                .directory_entry_path(*row)
                .ok()
                .flatten()
                .as_deref()
                == Some(path)
        });
        let Some(row) = row else {
            return;
        };
        let offset = self.buffers[buffer].line_to_offset(row);
        let pane = self.active_mut();
        pane.replace_selection(Selection::point(offset));
        pane.preserve_scroll = false;
    }

    pub(super) fn refresh_directory(&mut self) -> Result<()> {
        let buffer = self.active().buffer;
        if self.buffers[buffer].dirty {
            let confirmation = DirectoryReloadConfirmation {
                buffer,
                destination: None,
                focus_entry: None,
            };
            let message = confirmation.message();
            self.directory_reload_confirmation = Some(confirmation);
            self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
            self.status(message);
            return Ok(());
        }
        self.reload_directory_buffer(buffer)
    }

    /// Shows or hides dotfiles in every explorer.
    ///
    /// A listing is a projection of one preference, so a pane that is not
    /// active must not be left contradicting it: every clean explorer is
    /// re-read, the active one first so the visible pane is right even if a
    /// background listing has since gone missing. A dirty explorer keeps its
    /// text, since re-reading would discard edits that never reached a write
    /// plan, and the active one refuses the toggle outright rather than
    /// silently disagreeing with the preference it just changed.
    pub(super) fn toggle_hidden_files(&mut self) -> Result<()> {
        let active = self.active().buffer;
        // The binding is explorer-scoped, but the command palette reaches every
        // command by name from any view.
        anyhow::ensure!(
            self.buffers[active].is_directory(),
            "hidden files can only be shown or hidden in an explorer"
        );
        anyhow::ensure!(
            !self.buffers[active].dirty,
            "explorer has unsaved edits; write or refresh them before changing hidden files"
        );
        let show_hidden = !self.config.editor.show_hidden_files;
        self.config.editor.show_hidden_files = show_hidden;
        let listings = std::iter::once(active)
            .chain((0..self.buffers.len()).filter(|buffer| {
                *buffer != active
                    && self.buffers[*buffer].is_directory()
                    && !self.buffers[*buffer].dirty
            }))
            .collect::<Vec<_>>();
        for buffer in listings {
            self.reload_directory_buffer(buffer)?;
        }
        self.status(if show_hidden {
            "showing hidden files"
        } else {
            "hiding hidden files"
        });
        Ok(())
    }

    pub(super) fn reload_directory_buffer(&mut self, buffer: usize) -> Result<()> {
        let show_hidden = self.config.editor.show_hidden_files;
        self.buffers
            .get_mut(buffer)
            .ok_or_else(|| anyhow::anyhow!("directory buffer is gone"))?
            .reload_directory(show_hidden)?;
        self.settle_reloaded_directory(buffer);
        Ok(())
    }

    /// The bookkeeping a freshly read listing needs once it is in place. Split
    /// from the read so a listing read before its buffer joins the editor can
    /// still be settled the same way.
    fn settle_reloaded_directory(&mut self, buffer: usize) {
        self.clear_syntax_history(buffer);
        self.stale_syntax.remove(&buffer);
        self.syntax[buffer] = None;
        self.forget_directory_view(buffer);
        self.forget_directory_jumps(buffer);
        self.normalize_buffer(buffer);
        self.status("directory refreshed");
    }

    /// Drops the remembered view of whatever directory a buffer is showing,
    /// because a re-read listing may not have the row that was under the
    /// caret.
    pub(super) fn forget_directory_view(&mut self, buffer: usize) {
        if let Some(path) = self.buffers[buffer].path.as_deref() {
            self.directory_views.remove(path);
        }
    }

    fn remember_active_directory_view(&mut self) {
        let buffer = self.active().buffer;
        if self.buffers[buffer].is_directory()
            && let Some(path) = self.buffers[buffer].path.clone()
        {
            let view = DirectoryView::from_pane(self.active());
            self.directory_views.insert(path, view);
        }
    }

    /// Whether some pane other than the active one is showing or reserving a
    /// buffer.
    ///
    /// Showing counts as well as reserving, because a pane can display a
    /// directory buffer it never claimed — the startup explorer, before it has
    /// navigated anywhere — and retargeting that would move its listing out
    /// from under it just the same.
    fn claimed_by_another_pane(&self, buffer: usize) -> bool {
        self.panes.iter().any(|(id, pane)| {
            *id != self.active_pane
                && (pane.buffer == buffer || pane.directory_buffer == Some(buffer))
        })
    }

    /// The explorer this pane would reuse, without reading or adopting one.
    fn reusable_pane_directory_buffer(&self) -> Option<usize> {
        self.active().directory_buffer.or_else(|| {
            (0..self.buffers.len()).find(|index| {
                self.buffers[*index].is_directory() && !self.claimed_by_another_pane(*index)
            })
        })
    }

    /// This pane's explorer, adopting one no live pane is browsing with before
    /// opening another.
    ///
    /// Closing a pane leaves its explorer behind, and buffers are never
    /// removed, so adoption is what keeps the count at one per pane instead of
    /// letting orphans accumulate.
    ///
    /// This only says which buffer would be used, without deciding that it
    /// will be: a listing the pane cannot go on to enter must leave no buffer
    /// behind, so a newly read one is handed back rather than pushed.
    fn pane_directory_buffer(&mut self, path: &Path) -> Result<PaneDirectory> {
        if let Some(reusable) = self.reusable_pane_directory_buffer() {
            return Ok(PaneDirectory::Existing(reusable));
        }
        Ok(PaneDirectory::New(Box::new(Buffer::open_directory(
            path,
            self.config.editor.show_hidden_files,
        )?)))
    }

    fn adopt_directory_buffer(&mut self, buffer: Buffer) -> usize {
        self.syntax.push(None);
        self.buffers.push(buffer);
        self.buffers.len() - 1
    }

    fn contains_only_pending_cut(&self, buffer: usize) -> bool {
        let Some(register) = self
            .registers
            .get(&'"')
            .and_then(|register| register.directory.as_ref())
            .filter(|register| register.mode == TransferMode::Move)
        else {
            return false;
        };
        let sources = register
            .entries
            .iter()
            .map(|entry| entry.source.clone())
            .collect::<HashSet<_>>();
        self.contains_only_deletions(buffer, &sources)
    }

    pub(super) fn contains_only_deletions(
        &self,
        buffer: usize,
        sources: &HashSet<PathBuf>,
    ) -> bool {
        let Ok(plan) = self.buffers[buffer].directory_plan() else {
            return false;
        };
        !plan.operations().is_empty()
            && plan.operations().iter().all(|operation| {
                let FsOperation::Delete { path, .. } = operation else {
                    return false;
                };
                sources.contains(&plan.root().join(path))
            })
    }

    /// Points this pane's explorer at `path`, creating one if it has none.
    ///
    /// Returns `None` when the explorer holds unsaved edits to another
    /// directory, having asked whether to discard them; the navigation
    /// resumes from `handle_directory_reload_confirmation` if it is confirmed.
    fn retarget_pane_directory(&mut self, path: &Path) -> Result<Option<usize>> {
        // Every listing this reads is read before the pane commits to it, so a
        // directory that cannot be listed leaves the pane pointing where it
        // already was and adds no buffer to the editor.
        let buffer_id = match self.pane_directory_buffer(path)? {
            PaneDirectory::Existing(buffer_id) => buffer_id,
            PaneDirectory::New(mut buffer) => {
                // The re-read the entering branch below would have done, taken
                // while the buffer is still a local value with nothing
                // referring to it.
                buffer.reload_directory(self.config.editor.show_hidden_files)?;
                let buffer_id = self.adopt_directory_buffer(*buffer);
                self.settle_reloaded_directory(buffer_id);
                self.active_mut().directory_buffer = Some(buffer_id);
                return Ok(Some(buffer_id));
            }
        };
        if self.buffers[buffer_id].path.as_deref() == Some(path) {
            let entering = self.active().buffer != buffer_id;
            if entering && !self.buffers[buffer_id].dirty {
                self.reload_directory_buffer(buffer_id)?;
            }
            self.active_mut().directory_buffer = Some(buffer_id);
            return Ok(Some(buffer_id));
        }
        if self.buffers[buffer_id].dirty && !self.contains_only_pending_cut(buffer_id) {
            let confirmation = DirectoryReloadConfirmation {
                buffer: buffer_id,
                destination: Some(path.to_path_buf()),
                focus_entry: None,
            };
            let message = confirmation.message();
            self.directory_reload_confirmation = Some(confirmation);
            self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
            self.status(message);
            return Ok(None);
        }
        self.buffers[buffer_id].retarget_directory(path, self.config.editor.show_hidden_files)?;
        self.clear_syntax_history(buffer_id);
        self.stale_syntax.remove(&buffer_id);
        self.syntax[buffer_id] = None;
        self.forget_directory_jumps(buffer_id);
        self.active_mut().directory_buffer = Some(buffer_id);
        Ok(Some(buffer_id))
    }

    /// Asks which program should be given a binary file.
    pub(super) fn ask_for_external_program(&mut self, path: PathBuf) {
        if self.prompt_kind == PromptKind::ExternalProgram && self.external_target.is_some() {
            // A language server answering a goto with a binary file must not
            // replace the question already on screen, or a half-typed program
            // name would silently be applied to a file nobody asked about.
            self.action_failed(format!(
                "{} is not a text file; answer the open prompt first",
                path.display()
            ));
            return;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        self.external_target = Some(path);
        self.open_prompt(PromptKind::ExternalProgram);
        self.status(format!("{name} is not a text file"));
    }

    /// Hands a binary file to the default app or a remembered explicit choice.
    pub(super) fn open_externally(&mut self, path: &Path, program: String) {
        let program = program.trim().to_owned();
        if let Err(error) = external_open::launch(&program, path) {
            // Nothing is remembered, because a program that will not run is
            // not a hint worth offering back.
            self.error_from(
                "External program",
                "Program launch failed",
                error.to_string(),
            );
            return;
        }
        if program.is_empty() {
            self.status(format!(
                "opened {} with the system default application",
                path.display()
            ));
            return;
        }
        if let Err(error) = self.programs.remember(&program) {
            self.action_warning(
                "Program choice was not saved",
                format!("opened with {program}, but {error}"),
            );
            return;
        }
        self.status(format!("opened {} with {program}", path.display()));
    }

    /// Retires every jump into a directory buffer whose listing was replaced.
    ///
    /// Retargeting and reloading swap the whole text outside the transaction
    /// system, so nothing maps these offsets the way an edit would. The buffer
    /// index survives but the content behind it does not, and jumping back to
    /// a row of a directory that is no longer on screen means nothing.
    pub(super) fn forget_directory_jumps(&mut self, buffer: usize) {
        for pane in self.panes.values_mut() {
            pane.jumps.forget(buffer);
        }
    }

    pub(super) fn open_file(&mut self, path: PathBuf) -> Result<()> {
        let path = self.resolve_working_path(path);
        let requested_identity = crate::path_safety::path_identity(&path)?;
        self.remember_active_directory_view();
        let was_showing = self.active().buffer;
        let buffer_id = if path.is_dir() {
            match self.retarget_pane_directory(&path)? {
                Some(buffer_id) => buffer_id,
                None => return Ok(()),
            }
        } else if let Some(index) = self.live_buffer_for_path(&path) {
            index
        } else if external_open::looks_binary(&path) {
            // Before the buffer exists, so a file Runyte cannot edit never
            // becomes one it is holding open and cannot save.
            self.ask_for_external_program(path);
            return Ok(());
        } else {
            let buffer = match open_or_new_at_identity(
                &path,
                &requested_identity,
                self.config.editor.show_hidden_files,
            ) {
                Ok(buffer) => buffer,
                Err(error) if error.is::<crate::buffer::BinaryFileError>() => {
                    // The file changed after the bounded probe, or binary
                    // bytes appeared beyond it. The final read owns the
                    // classification and must still use the external opener.
                    self.ask_for_external_program(path);
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            self.syntax.push(parse_buffer(&buffer, &self.registry));
            self.buffers.push(buffer);
            // A newly opened file is the one moment its staged text has to be
            // read; every edit after this is diffed against what is held here.
            self.track_in_git(&path);
            self.buffers.len() - 1
        };
        // Only once the buffer is known to exist, so a path that failed to
        // open leaves no jump to nowhere. Retargeting the explorer the pane
        // was already showing leaves nothing to jump back to: the listing the
        // offsets belong to is gone, so no entry is recorded for it.
        if buffer_id != was_showing || !self.buffers[buffer_id].is_directory() {
            self.push_jump();
        }
        let directory_view = self.directory_views.get(&path).cloned();
        let launch_selection = self.take_pending_launch_selection(buffer_id);
        let pane = self.active_mut();
        pane.retarget(buffer_id);
        if let Some(view) = directory_view {
            pane.replace_selection(view.selection);
            pane.scroll_row = view.scroll_row;
            pane.scroll_wrap = view.scroll_wrap;
            pane.scroll_col = view.scroll_col;
        } else {
            pane.replace_selection(launch_selection.unwrap_or_else(|| Selection::point(0)));
            pane.scroll_row = 0;
            pane.scroll_wrap = 0;
            pane.scroll_col = 0;
        }
        pane.preserve_scroll = false;
        self.lsp_touch(buffer_id);
        self.status(format!("opened {}", path.display()));
        self.report_new_registry_errors();
        Ok(())
    }

    pub(crate) fn host_open_file(&mut self, path: PathBuf, activate: bool) -> Result<usize> {
        let opened = self.host_open_files(vec![path], activate)?;
        Ok(opened[0])
    }

    fn live_buffer_for_path(&self, path: &Path) -> Option<usize> {
        let identity = crate::path_safety::path_identity(path).ok()?;
        self.live_buffer_for_identity(&identity)
    }

    fn live_buffer_for_identity(&self, identity: &Path) -> Option<usize> {
        self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index)
                && buffer.path.as_deref().is_some_and(|candidate| {
                    crate::path_safety::path_identity(candidate)
                        .is_ok_and(|candidate| candidate == identity)
                }))
            .then_some(index)
        })
    }

    /// Opens every path or none of them.
    ///
    /// A request naming several paths is one request: a client that is told it
    /// failed has no buffer ids and no wait token, so any buffer left open
    /// behind that answer is state it cannot see, name, or close. Every path is
    /// therefore built into a buffer first, and only once all of them exist is
    /// anything pushed into the editor. The building is the validation — the
    /// same `open_or_new` the interactive editor uses — so there is no second
    /// notion of what makes a path openable that could drift away from the
    /// first.
    ///
    /// File activation is applied last, after the buffers are live. An active
    /// directory is the exception: the pane may reuse an explorer whose id is
    /// not the staged directory's, so it is entered after every path has been
    /// prepared but before staged buffers are committed. The explorer entry is
    /// atomic, and its actual id then replaces the staged slot.
    pub(crate) fn host_open_files(
        &mut self,
        paths: Vec<PathBuf>,
        activate: bool,
    ) -> Result<Vec<usize>> {
        self.host_open_files_with_refresh(paths, activate, None)
    }

    /// Opens files for a new editor-wait request, refreshing reused clean
    /// files unless another pending request still owns their current text.
    /// Activating a wait file is also an input boundary: an external editor
    /// request must not inherit Insert or another modal state from the buffer
    /// that happened to be active in an existing persistent session.
    pub(crate) fn host_open_wait_files(
        &mut self,
        paths: Vec<PathBuf>,
        activate: bool,
        pending_wait_buffers: &HashSet<usize>,
    ) -> Result<Vec<usize>> {
        let previous_buffer = self.active().buffer;
        let previous_selection = self.active().selection.clone();
        let previous_semantics = self.active().selection_semantics();
        let previous_selection_revision = self.active().selection_revision;
        let previous_mode = self.mode;
        let normal_selection = self.normal_mode_selection(
            previous_buffer,
            previous_selection.clone(),
            previous_semantics,
            previous_mode,
        );
        let normal_semantics = match self.grammar.kind() {
            crate::command::GrammarKind::Runyte => SelectionSemantics::Runyte,
            crate::command::GrammarKind::Vim => SelectionSemantics::HalfOpen,
        };
        let previous_directory = self.buffers[previous_buffer]
            .is_directory()
            .then(|| self.buffers[previous_buffer].path.clone())
            .flatten();
        let previous_directory_view = previous_directory
            .as_ref()
            .and_then(|path| self.directory_views.get(path).cloned());
        if activate {
            // Let a successful open record the position being left exactly as
            // an explicit transition to Normal would. A failed open restores
            // both values and the captured selection revision without
            // committing the Insert undo group.
            let pane = self.active_mut();
            pane.replace_selection(normal_selection);
            pane.mark_selection_semantics(normal_semantics);
        }
        let opened =
            match self.host_open_files_with_refresh(paths, activate, Some(pending_wait_buffers)) {
                Ok(opened) => opened,
                Err(error) => {
                    if activate {
                        let pane = self.active_mut();
                        pane.replace_selection(previous_selection);
                        pane.mark_selection_semantics(previous_semantics);
                        pane.selection_revision = previous_selection_revision;
                        if let Some(path) = previous_directory {
                            if let Some(view) = previous_directory_view {
                                self.directory_views.insert(path, view);
                            } else {
                                self.directory_views.remove(&path);
                            }
                        }
                    }
                    return Err(error);
                }
            };
        if activate {
            if matches!(previous_mode, Mode::Insert | Mode::Replace) {
                self.buffers[previous_buffer].commit_undo_group();
            }
            // The inherited mode belonged to the source. The activated wait
            // buffer starts from its own caret in Normal mode.
            self.mode = Mode::Normal;
            self.enter_normal_mode();
        }
        Ok(opened)
    }

    fn host_open_files_with_refresh(
        &mut self,
        paths: Vec<PathBuf>,
        activate: bool,
        pending_wait_buffers: Option<&HashSet<usize>>,
    ) -> Result<Vec<usize>> {
        let covered_terminal = activate.then(|| self.active_terminal()).flatten();
        enum Prepared {
            /// Already open before this request, so nothing is staged for it.
            Live(usize),
            /// Position in `staged`; repeated paths share one entry.
            Staged(usize),
        }

        let paths = paths
            .into_iter()
            .map(|path| self.resolve_working_path(path))
            .collect::<Vec<_>>();
        let identities = paths
            .iter()
            .map(|path| crate::path_safety::path_identity(path))
            .collect::<Result<Vec<_>>>()?;
        let mut staged: Vec<(PathBuf, PathBuf, Buffer, Option<DocumentSyntax>)> = Vec::new();
        let mut refreshed = Vec::new();
        let mut prepared = Vec::with_capacity(paths.len());
        for (path, identity) in paths.iter().zip(&identities) {
            ensure!(
                crate::path_safety::path_identity(path)? == *identity,
                "{} changed its resolved identity while the request was being prepared; retry the open",
                path.display()
            );
            if let Some(index) = self.live_buffer_for_path(path) {
                if let Some(pending_wait_buffers) = pending_wait_buffers
                    && !self.buffers[index].dirty
                    && self.buffers[index].kind == BufferKind::File
                    && !pending_wait_buffers.contains(&index)
                    && !refreshed
                        .iter()
                        .any(|(refreshed_index, _)| *refreshed_index == index)
                {
                    // Read every replacement before changing live state. A
                    // later invalid request path must leave this clean buffer
                    // and its revision untouched along with the new buffers.
                    let mut replacement = self.buffers[index].clone();
                    replacement.reload()?;
                    refreshed.push((index, replacement));
                }
                prepared.push(Prepared::Live(index));
                continue;
            }
            if let Some(slot) = staged
                .iter()
                .position(|(_, staged_identity, _, _)| staged_identity == identity)
            {
                prepared.push(Prepared::Staged(slot));
                continue;
            }
            ensure!(
                !external_open::looks_binary(path),
                "binary files cannot be opened through the workspace protocol"
            );
            let buffer =
                open_or_new_at_identity(path, identity, self.config.editor.show_hidden_files)?;
            let syntax = parse_buffer(&buffer, &self.registry);
            prepared.push(Prepared::Staged(staged.len()));
            staged.push((path.clone(), identity.clone(), buffer, syntax));
        }

        let first_is_directory = prepared.first().is_some_and(|prepared| match prepared {
            Prepared::Live(index) => self.buffers[*index].is_directory(),
            Prepared::Staged(slot) => staged[*slot].2.is_directory(),
        });
        let activated_directory = if activate && first_is_directory {
            let path = &paths[0];
            if let Some(buffer_id) = self.reusable_pane_directory_buffer() {
                ensure!(
                    self.buffers[buffer_id].path.as_deref() == Some(path)
                        || !self.buffers[buffer_id].dirty
                        || self.contains_only_pending_cut(buffer_id),
                    "cannot activate {} while the pane's explorer has unsaved edits",
                    path.display()
                );
            }
            // All request paths are valid by now, while no staged buffer is
            // live yet. Entering here lets the explorer choose its real,
            // pane-owned id; the atomic explorer path leaves no pane or buffer
            // mutation behind if its final listing read loses a filesystem
            // race.
            self.open_file(path.clone())?;
            let buffer_id = self.active().buffer;
            ensure!(
                self.buffers[buffer_id].is_directory()
                    && self.buffers[buffer_id].path.as_deref() == Some(path),
                "directory activation did not enter {}",
                path.display()
            );
            Some(buffer_id)
        } else {
            None
        };

        // Past this point nothing can fail on the request's own paths.
        let activated_slot = activated_directory.and_then(|_| match prepared.first() {
            Some(Prepared::Staged(slot)) => Some(*slot),
            Some(Prepared::Live(_)) | None => None,
        });
        for (index, replacement) in refreshed {
            let language = buffer_language(&self.buffers[index], &self.registry);
            self.buffers[index] = replacement;
            self.resync_replaced_buffer(index, language);
            self.normalize_buffer(index);
        }
        let mut staged_ids = vec![None; staged.len()];
        for (slot, (path, _, buffer, syntax)) in staged.into_iter().enumerate() {
            if Some(slot) == activated_slot {
                staged_ids[slot] = activated_directory;
                continue;
            }
            let is_directory = buffer.is_directory();
            self.syntax.push(syntax);
            self.buffers.push(buffer);
            let buffer_id = self.buffers.len() - 1;
            if !is_directory {
                // A newly opened file is the one moment its staged text has to
                // be read; every edit after this is diffed against what is
                // held here.
                self.track_in_git(&path);
            }
            self.lsp_touch(buffer_id);
            staged_ids[slot] = Some(buffer_id);
        }
        let mut opened = prepared
            .into_iter()
            .map(|prepared| match prepared {
                Prepared::Live(index) => index,
                Prepared::Staged(slot) => {
                    staged_ids[slot].expect("every staged buffer was committed or activated")
                }
            })
            .collect::<Vec<_>>();
        if let Some(buffer_id) = activated_directory {
            let first = &paths[0];
            for (path, opened) in paths.iter().zip(&mut opened) {
                if path == first {
                    *opened = buffer_id;
                }
            }
        } else if activate && let Some(path) = paths.first() {
            // The buffer exists by now, so this retargets the pane rather than
            // reading the path again.
            self.open_file(path.clone())?;
        }
        // Recorded after the activation, because retargeting the pane is what
        // took the terminal off it. Nobody in the editor asked this pane to
        // stop showing its terminal — a program did, usually one running in
        // that very terminal — so the pane owes it a return once the document
        // the request brought is finished with.
        if let Some(id) = covered_terminal
            && self.terminals.get(id).is_some()
            && let Some(pane) = self.panes.get_mut(&self.active_pane)
            && pane.terminal.is_none()
        {
            pane.covered_terminal = Some((pane.buffer, id));
        }
        Ok(opened)
    }

    pub(crate) fn host_save_buffer(&mut self, buffer: usize) -> Result<()> {
        ensure!(
            buffer < self.buffers.len() && !self.closed_buffers.contains(&buffer),
            "unknown or closed buffer"
        );
        self.buffers[buffer].commit_undo_group();
        self.save_buffer(buffer, None, false)
    }

    pub(crate) fn host_close_buffer(&mut self, buffer: usize, discard: bool) -> Result<()> {
        ensure!(
            buffer < self.buffers.len() && !self.closed_buffers.contains(&buffer),
            "unknown or closed buffer"
        );
        ensure!(
            !self.buffers[buffer].dirty || discard,
            "modified buffers must be saved or explicitly discarded before closing"
        );
        if discard {
            self.buffers[buffer].mark_saved();
        }
        self.close_buffer(buffer);
        Ok(())
    }

    pub(crate) fn host_buffer_is_closed(&self, buffer: usize) -> bool {
        self.closed_buffers.contains(&buffer)
    }

    pub(super) fn switch_buffer(&mut self, buffer_id: usize) {
        if buffer_id >= self.buffers.len() || self.closed_buffers.contains(&buffer_id) {
            return;
        }
        if buffer_id == self.active().buffer {
            // The buffer still names the document covered by a terminal. A
            // request to switch to that document is therefore not a no-op:
            // reveal it through the same transition as `leave-terminal`.
            if self.active_terminal().is_some() {
                self.leave_terminal();
            }
            return;
        }
        self.dismiss_popups();
        self.push_jump();
        // A pane must not walk away still reserving the explorer it left, or
        // no pane could ever display or adopt that buffer again. Switching to
        // a directory buffer adopts it, unless another pane owns it, in which
        // case this pane simply gives its own up and takes a fresh explorer
        // the next time it navigates — retargeting a buffer someone else is
        // browsing with would move their directory out from under them.
        let explorer = if self.buffers[buffer_id].is_directory() {
            (!self.claimed_by_another_pane(buffer_id)).then_some(buffer_id)
        } else {
            self.active().directory_buffer
        };
        let selection = self
            .take_pending_launch_selection(buffer_id)
            .unwrap_or_else(|| Selection::point(0));
        let pane = self.active_mut();
        pane.retarget(buffer_id);
        pane.directory_buffer = explorer;
        pane.replace_selection(selection);
        pane.scroll_row = 0;
        pane.scroll_wrap = 0;
        pane.scroll_col = 0;
        pane.preserve_scroll = false;
        self.lsp_touch(buffer_id);
        self.status(format!("buffer {}", self.buffers[buffer_id].display_name()));
    }

    pub(super) fn open_scratch_buffer(&mut self) {
        self.buffers.push(Buffer::scratch());
        self.syntax.push(None);
        let buffer = self.buffers.len() - 1;
        self.switch_buffer(buffer);
    }

    /// Opens a generated page whose content the pane places, rather than one
    /// laid out by the columns its text was written in.
    ///
    /// The alignment belongs to the buffer, so reopening the page or resizing
    /// the pane re-centres it without regenerating a character of text.
    pub(super) fn open_virtual_page(
        &mut self,
        identity: GeneratedViewIdentity,
        name: String,
        text: &str,
        alignment: ContentAlignment,
    ) -> usize {
        self.open_virtual_content(identity, name, text, false, alignment)
    }

    /// Opens a read-only buffer whose text is a unified diff, so a frontend
    /// can read the leading character of each line rather than showing a
    /// patch as undifferentiated prose.
    pub(super) fn open_virtual_diff(
        &mut self,
        identity: GeneratedViewIdentity,
        name: String,
        text: &str,
    ) -> usize {
        self.open_virtual_content(identity, name, text, true, ContentAlignment::default())
    }

    fn open_virtual_content(
        &mut self,
        identity: GeneratedViewIdentity,
        name: String,
        text: &str,
        diff: bool,
        alignment: ContentAlignment,
    ) -> usize {
        let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index)
                && buffer.generated_view_identity() == Some(&identity))
            .then_some(index)
        });
        let buffer = if let Some(existing) = existing {
            self.buffers[existing].replace_virtual_text(text);
            existing
        } else {
            self.buffers.push(
                if diff {
                    Buffer::virtual_diff_identified(identity, name, text)
                } else {
                    Buffer::virtual_text_identified(identity, name, text)
                }
                .aligned(alignment),
            );
            // Virtual buffers are durable projections, never highlighted.
            self.syntax.push(None);
            self.buffers.len() - 1
        };
        self.push_jump();
        let pane = self.active_mut();
        pane.retarget(buffer);
        pane.replace_selection(Selection::point(0));
        pane.scroll_row = 0;
        pane.scroll_wrap = 0;
        pane.scroll_col = 0;
        self.mode = Mode::Normal;
        buffer
    }

    /// Reprojects a diff without moving a caret whose hunk and line still
    /// exist. Row fallback is deterministic when the selected line vanished.
    pub(super) fn replace_virtual_preserving_row(&mut self, buffer: usize, text: &str) {
        let old = self.buffers[buffer].to_string();
        let selections = self
            .panes
            .iter()
            .filter(|(_, pane)| pane.buffer == buffer)
            .map(|(pane_id, pane)| {
                let head = pane.head();
                let row = self.buffers[buffer].offset_to_row(head);
                // Carry the column as well as the row, so a refresh puts the
                // cursor back where it was on the line rather than dragging
                // it to the first column.
                let column = head.saturating_sub(self.buffers[buffer].line_to_offset(row));
                (*pane_id, row, column, diff_row_identity(&old, row))
            })
            .collect::<Vec<_>>();
        self.buffers[buffer].replace_virtual_text(text);
        for (pane_id, old_row, column, identity) in selections {
            let row = identity
                .as_ref()
                .and_then(|identity| diff_row_for_identity(text, identity))
                .unwrap_or_else(|| old_row.min(self.buffers[buffer].len_lines().saturating_sub(1)));
            // The replacement row may be shorter than the one the cursor came
            // from, so the column lands at its end rather than past it.
            let offset = self.buffers[buffer].line_to_offset(row)
                + column.min(self.buffers[buffer].line_len(row));
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.replace_selection(Selection::point(offset));
            }
        }
    }

    pub(super) fn save(&mut self, path: Option<PathBuf>, replace: bool) -> Result<()> {
        let buffer_id = self.active().buffer;
        self.buffers[buffer_id].commit_undo_group();
        self.save_buffer(buffer_id, path, replace)
    }

    pub(super) fn save_buffer(
        &mut self,
        buffer_id: usize,
        path: Option<PathBuf>,
        replace: bool,
    ) -> Result<()> {
        if let Some(reason) = self.buffers[buffer_id].read_only_reason() {
            self.action_warning("Save refused", reason);
            return Ok(());
        }
        if self.buffers[buffer_id].is_commit_message() {
            if path.is_some() {
                self.action_failed("a commit message cannot be written to a path");
                return Ok(());
            }
            self.commit_staged(buffer_id);
            return Ok(());
        }
        if self.buffers[buffer_id].is_directory() {
            if path.is_some() {
                self.action_failed("directory buffers cannot be written to another path");
                return Ok(());
            }
            match self.buffers[buffer_id].directory_plan() {
                Ok(plan) if plan.is_empty() => {
                    self.reload_directory_buffer(buffer_id)?;
                    self.status("directory has no filesystem changes");
                }
                Ok(plan) if self.plan_invalidates_a_pasted_cut_source(buffer_id, &plan) => {
                    self.action_warning(
                        "Save refused",
                        "a cut from this explorer is pending in another explorer; write the destination first",
                    );
                }
                Ok(plan) => {
                    let count = plan.operations().len();
                    self.fs_confirmation = Some(FsConfirmation {
                        buffer: buffer_id,
                        plan,
                        selected: 0,
                    });
                    self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
                    self.status(format!(
                        "review {count} filesystem operation{} before applying",
                        if count == 1 { "" } else { "s" }
                    ));
                }
                Err(error) => self.action_warning("Save refused", error.to_string()),
            }
            return Ok(());
        }
        let path = path.map(|path| self.resolve_working_path(path));
        let saving_current_path = path.as_deref().map_or_else(
            || self.buffers[buffer_id].path.is_some(),
            |destination| self.buffers[buffer_id].owns_path_identity(destination),
        );
        if !replace
            && saving_current_path
            && self.buffers[buffer_id].external_file_status().is_stale()
            && self.buffers[buffer_id].external_file_status()
                != crate::buffer::ExternalFileStatus::Deleted
        {
            self.action_warning(
                "Save refused",
                "file changed on disk; Space b d compares, Space r reloads, and :write! replaces it",
            );
            return Ok(());
        }
        let destination = path.as_deref().or(self.buffers[buffer_id].path.as_deref());
        let expected_identity = destination
            .map(crate::path_safety::path_identity)
            .transpose()?;
        if let Some(identity) = expected_identity.as_deref()
            && let Some(owner) = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
                (index != buffer_id
                    && !self.closed_buffers.contains(&index)
                    && buffer.path.as_deref().is_some_and(|candidate| {
                        crate::path_safety::path_identity(candidate)
                            .is_ok_and(|candidate| candidate == identity)
                    }))
                .then_some(index)
            })
        {
            self.action_warning(
                "Save refused",
                format!(
                    "{} is already open in buffer {}; close that buffer before writing this one there",
                    destination
                        .expect("a checked identity came from a destination")
                        .display(),
                    self.buffers[owner].display_name()
                ),
            );
            return Ok(());
        }
        if self.config.editor.trim_trailing_whitespace
            && (path.is_some() || self.buffers[buffer_id].path.is_some())
        {
            self.trim_trailing_whitespace(buffer_id);
        }
        let previous_git_path = path.as_ref().and_then(|destination| {
            self.buffers[buffer_id]
                .path
                .clone()
                .filter(|previous| previous != destination)
        });
        let result = if let Some(path) = path {
            self.invalidate_partial_guards(buffer_id);
            self.buffers[buffer_id].save_as_checked(
                path,
                replace,
                expected_identity.expect("save-as has a destination identity"),
            )
        } else if let Some(expected_identity) = expected_identity {
            self.buffers[buffer_id].save_checked(replace, expected_identity)
        } else {
            self.buffers[buffer_id].save(replace)
        };
        match result {
            Ok(save_outcome) => {
                if let Some(previous) = previous_git_path {
                    self.git.forget(&previous);
                }
                // `:write <path>` can give a scratch buffer a language for the
                // first time, or change the one it already had.
                self.clear_syntax_history(buffer_id);
                self.stale_syntax.remove(&buffer_id);
                self.syntax[buffer_id] = parse_buffer(&self.buffers[buffer_id], &self.registry);
                self.lsp_save(buffer_id);
                // Writing changes what Git reports without changing any
                // buffer, and `:write <path>` can put a file under Git that
                // was not there a moment ago.
                if let Some(path) = self.buffers[buffer_id].path.clone() {
                    self.reconcile_git_after_file_write(&path);
                }
                let path = self.buffers[buffer_id]
                    .path
                    .as_ref()
                    .map_or("[scratch]".into(), |path| path.display().to_string());
                match save_outcome {
                    crate::buffer::SaveOutcome::Durable => self.status(format!("wrote {path}")),
                    crate::buffer::SaveOutcome::CommittedWithWarning(warning) => {
                        self.action_warning("Save completed with warning", warning);
                    }
                }
                self.report_new_registry_errors();
                Ok(())
            }
            Err(error) => {
                let save_conflict = crate::buffer::is_save_conflict(&error);
                if saving_current_path
                    && let Some(observation) = self.buffers[buffer_id].observe_now(buffer_id)
                {
                    self.apply_file_observation(observation);
                }
                if save_conflict {
                    let message = error.to_string();
                    if self.buffers[buffer_id].external_file_status().is_stale() {
                        self.action_warning_unretained(message);
                    } else {
                        self.action_warning("Save refused", message);
                    }
                } else {
                    self.error_from("Runyte", "Save failed", error.to_string());
                }
                Ok(())
            }
        }
    }

    /// A cut which has reached another explorer is a single move owned by
    /// that destination's plan. Letting another plan delete or relocate its
    /// source, including an ancestor directory, makes the confirmed move
    /// stale and strands the destination buffer.
    fn plan_invalidates_a_pasted_cut_source(&self, buffer: usize, plan: &FsPlan) -> bool {
        let pasted_move_sources = self
            .buffers
            .iter()
            .enumerate()
            .filter(|(candidate, _)| {
                *candidate != buffer && !self.closed_buffers.contains(candidate)
            })
            .flat_map(|(_, buffer)| buffer.pending_directory_move_sources())
            .collect::<HashSet<_>>();
        plan.operations().iter().any(|operation| {
            let invalidated = match operation {
                FsOperation::Delete { path, .. }
                | FsOperation::Rename { from: path, .. }
                | FsOperation::Move { from: path, .. } => {
                    resolved_operation_path(plan.root(), path)
                }
                FsOperation::Create { .. } | FsOperation::Copy { .. } => return false,
            };
            pasted_move_sources
                .iter()
                .any(|source| source.starts_with(&invalidated))
        })
    }

    fn trim_trailing_whitespace(&mut self, buffer_id: usize) {
        let buffer = &self.buffers[buffer_id];
        let changes = trailing_whitespace_changes(buffer, 0..buffer.len_lines());
        if !changes.is_empty() {
            self.apply_to_buffer(buffer_id, &Transaction::new(changes));
        }
    }

    pub(super) fn reload_file(&mut self) -> Result<()> {
        let buffer_id = self.active().buffer;
        let was_dirty = self.buffers[buffer_id].dirty;
        let Some(observation) = self.buffers[buffer_id].observe_now(buffer_id) else {
            bail!("buffer is not a file");
        };
        self.apply_file_observation(observation.clone());
        if !was_dirty {
            return match &observation.observation {
                FileObservation::Text { .. } => {
                    self.install_file_reload(buffer_id, &observation.observation)
                }
                _ => self.report_unreloadable_observation(&observation.observation),
            };
        }
        if !self.buffers[buffer_id].dirty {
            match &observation.observation {
                FileObservation::Text { text, .. } => {
                    // Convergence adopted the disk state without clearing history.
                    if self.buffers[buffer_id].to_string() == text.as_ref() {
                        self.status("buffer already agrees with disk");
                        return Ok(());
                    }
                    return self.install_file_reload(buffer_id, &observation.observation);
                }
                _ => return self.report_unreloadable_observation(&observation.observation),
            }
        }
        if !matches!(observation.observation, FileObservation::Text { .. }) {
            return self.report_unreloadable_observation(&observation.observation);
        }
        let confirmation = FileReloadConfirmation {
            buffer: buffer_id,
            path: observation.path,
            generation: observation.generation,
            observation: observation.observation,
        };
        self.status(
            confirmation.message(self.buffers[buffer_id].external_file_status().is_stale()),
        );
        self.file_reload_confirmation = Some(confirmation);
        self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
        Ok(())
    }

    pub(super) fn install_file_reload(
        &mut self,
        buffer_id: usize,
        observation: &FileObservation,
    ) -> Result<()> {
        let language_before = buffer_language(&self.buffers[buffer_id], &self.registry);
        self.buffers[buffer_id].reload_from_observation(observation)?;
        self.resync_replaced_buffer(buffer_id, language_before);
        self.normalize_buffer(buffer_id);
        let path = self.buffers[buffer_id]
            .path
            .as_ref()
            .expect("a reloaded file buffer has a path");
        self.status(format!("reloaded {}", path.display()));
        self.report_new_registry_errors();
        Ok(())
    }

    fn report_unreloadable_observation(&mut self, observation: &FileObservation) -> Result<()> {
        let message = match observation {
            FileObservation::Deleted => "file was deleted on disk; there is nothing to reload",
            FileObservation::Binary { .. } => {
                "file became binary on disk; the text buffer was preserved"
            }
            FileObservation::Unreadable { .. } => {
                "file is unreadable on disk; the text buffer was preserved"
            }
            FileObservation::Text { .. } => return Ok(()),
        };
        self.action_failed(message);
        Ok(())
    }

    pub(super) fn reload_active(&mut self) -> Result<()> {
        match reload_dispatch(&self.active_buffer().kind) {
            ReloadDispatch::Directory => self.refresh_directory(),
            ReloadDispatch::GitStatus => {
                self.refresh_git();
                Ok(())
            }
            ReloadDispatch::GitBranches => {
                self.open_git_branches();
                Ok(())
            }
            ReloadDispatch::GitWorktrees => {
                self.open_git_worktrees();
                Ok(())
            }
            ReloadDispatch::GitLog => {
                self.open_git_log();
                Ok(())
            }
            ReloadDispatch::GitStash => {
                self.open_git_stashes();
                Ok(())
            }
            ReloadDispatch::File => self.reload_file(),
        }
    }

    pub(super) fn split(&mut self, axis: Axis, path: Option<PathBuf>) -> Result<()> {
        if let Some(maximized) = self.maximized {
            bail!("leave {} before creating a split", maximized.view.label());
        }
        let old = self.active_pane;
        let new = self.next_pane;
        self.next_pane += 1;
        let mut pane = self.panes[&old].clone();
        // Expansion is an interaction chain owned by the pane where it was
        // started. A split copies the current selection but starts its own
        // empty chain, avoiding coupling through the application-wide mode.
        pane.syntax_history.clear();
        // The new pane browses with an explorer of its own, so navigating in
        // one split cannot retarget the directory the other is showing. Until
        // it navigates it shares the view it was split from, which is what
        // makes a split of an explorer show the same directory twice.
        pane.directory_buffer = None;
        // A split of a terminal pane shows that pane's buffer, not a second
        // view of the same child. One pty has one size, and two panes sizing
        // it would fight over every resize; the terminal list is how a session
        // is put in front of another pane deliberately.
        pane.terminal = None;
        // A covered terminal is owed a return by the one pane that gave it
        // up, for the same reason. Two panes holding the claim would race to
        // reveal one session.
        pane.covered_terminal = None;
        self.panes.insert(new, pane);
        self.record_pane_opened(new);
        self.layout.split(old, new, axis);
        self.activate_pane(new);
        if let Some(path) = path {
            self.open_file(path)?;
        }
        self.status(split_status(axis));
        Ok(())
    }

    /// The split the `Space w v/s` and `Ctrl-w v/s` commands make.
    ///
    /// The terminal question is answered here rather than in `split` because
    /// `split` is the pane primitive: `:diff-this`, the tutorial, and the Git
    /// comparison all split in order to retarget both panes themselves, and
    /// they need the plain copy. Only a person asking for a window gets an
    /// explorer instead.
    ///
    /// Terminal Insert's explicit review boundary is preserved either way.
    /// The child stays live in its original pane, and the newly active pane
    /// uses Normal mode without asking the terminal session to capture a
    /// review snapshot.
    pub(super) fn split_window(&mut self, axis: Axis) -> Result<()> {
        let insert_mode = self.mode == Mode::Insert;
        // What the source pane is showing right now is the question, not
        // which buffer the pane kept behind a terminal.
        let from_terminal = self.active_terminal().is_some();
        let terminal_input = insert_mode && from_terminal;
        self.split(axis, None)?;
        debug_assert!(!terminal_input || self.active_terminal().is_none());
        if from_terminal {
            // The buffer the terminal's pane retained is that pane's history,
            // not a document the person asked for a second view of. Splitting
            // a terminal is a request for somewhere to work, so the new pane
            // starts where `Space E` would put it.
            self.open_explorer(None)?;
            // `open_file` reports what it opened, and the split is what was
            // asked for.
            self.status(split_status(axis));
        }
        self.finish_insert_window_command(insert_mode, terminal_input);
        Ok(())
    }

    /// Marks the active buffer for comparison, or compares it with the buffer
    /// marked before it.
    ///
    /// Two calls make a view: the first names one side and the second names
    /// the other. Which panes they are in does not matter, because the second
    /// call arranges them — a buffer that is not on screen gets a split of its
    /// own, and two that already are keep the panes they are in.
    pub(super) fn diff_this(&mut self) {
        let pane_id = self.active_pane;
        let buffer = self.panes[&pane_id].buffer;
        let name = self.buffers[buffer].display_name();

        if self.buffers[buffer].is_directory() {
            self.action_failed("a directory listing cannot be compared");
            return;
        }
        if self.buffers[buffer].len_bytes() > MAX_DIFF_BYTES {
            self.action_failed(format!("{name} is too large to compare"));
            return;
        }

        if self.diffs.iter().any(|session| session.has_buffer(buffer)) {
            self.action_failed(format!(
                "{name} is already being compared; :diff-off closes it"
            ));
            return;
        }

        // A buffer marked and then closed leaves a mark pointing at nothing.
        // Treating that as no mark at all is what makes the second call the
        // one that opens a view rather than one that fails.
        let marked = self
            .pending_diff
            .filter(|marked| !self.closed_buffers.contains(marked));
        let Some(marked) = marked else {
            self.pending_diff = Some(buffer);
            self.status(format!(
                "marked {name}; run :diff-this in the buffer to compare it with"
            ));
            return;
        };
        if marked == buffer {
            self.pending_diff = None;
            self.status(format!("{name} is no longer marked for comparison"));
            return;
        }
        // Checked again here rather than only when it was marked, because the
        // buffer has been editable in the meantime.
        if self.buffers[marked].len_bytes() > MAX_DIFF_BYTES {
            let marked = self.buffers[marked].display_name();
            self.pending_diff = None;
            self.action_failed(format!("{marked} is too large to compare"));
            return;
        }

        let Some((left, right)) = self.diff_sides(marked, buffer) else {
            self.action_failed("comparing needs room for two panes");
            return;
        };
        self.pending_diff = None;
        // Collapsed regions hide lines, and a hidden line cannot sit level
        // with anything, so a comparison starts from the whole of both files.
        for side in [left, right] {
            if let Some(pane) = self.panes.get_mut(&side.pane) {
                pane.folds.clear();
                pane.preserve_scroll = false;
            }
        }
        let left_text = self.buffers[left.buffer].to_string();
        let right_text = self.buffers[right.buffer].to_string();
        let session = DiffSession::new(left, right, &left_text, &right_text);
        let equal = session.alignment().is_equal();
        self.diffs.push(session);
        let left_name = self.buffers[left.buffer].display_name();
        let right_name = self.buffers[right.buffer].display_name();
        if equal {
            self.status(format!("{left_name} and {right_name} are identical"));
        } else {
            self.status(format!("comparing {left_name} with {right_name}"));
        }
    }

    /// Compares one immutable, freshly observed disk revision on the left
    /// with the authoritative editable buffer on the right.
    pub(super) fn diff_disk(&mut self) {
        let source = self.active().buffer;
        if self.buffers[source].kind != BufferKind::File {
            self.action_failed(":diff-disk requires an ordinary file buffer");
            return;
        }
        if self.maximized.is_some() {
            self.action_failed("leave the maximized view before comparing");
            return;
        }
        let existing_disk_diff = self.diffs.iter().position(|diff| {
            diff.has_buffer(source)
                && [Side::Left, Side::Right].into_iter().any(|side| {
                    let candidate = diff.side(side).buffer;
                    matches!(
                        self.buffers[candidate].generated_view_identity(),
                        Some(GeneratedViewIdentity::DiskSnapshot { source_buffer, .. })
                            if *source_buffer == source
                    )
                })
        });
        if self.diffs.iter().any(|diff| diff.has_buffer(source)) && existing_disk_diff.is_none() {
            self.action_failed("this buffer is already being compared; :diff-off closes it");
            return;
        }
        if self.buffers[source].len_bytes() > MAX_DIFF_BYTES {
            self.action_failed("the Runyte buffer is too large to compare");
            return;
        }
        let Some(event) = self.buffers[source].observe_now(source) else {
            self.action_failed(":diff-disk requires an ordinary file buffer");
            return;
        };
        self.apply_file_observation(event.clone());
        let FileObservation::Text { text, .. } = &event.observation else {
            let message = match &event.observation {
                FileObservation::Deleted => {
                    "the file was deleted; there is no disk text to compare"
                }
                FileObservation::Binary { .. } => {
                    "the disk version is binary and cannot be compared"
                }
                FileObservation::Unreadable { .. } => {
                    "the disk version is unreadable and cannot be compared"
                }
                FileObservation::Text { .. } => unreachable!(),
            };
            self.action_failed(message);
            return;
        };
        if text.len() > MAX_DIFF_BYTES {
            self.action_failed("the disk version is too large to compare");
            return;
        }

        let path = event.path.clone();
        let identity = GeneratedViewIdentity::DiskSnapshot {
            source_buffer: source,
            revision: Buffer::observed_revision_key(&event.observation),
        };
        let snapshot =
            Buffer::virtual_text_identified(identity, format!("[disk] {}", path.display()), text);
        if let Some(index) = existing_disk_diff {
            let left = self.diffs[index].side(Side::Left);
            let right = self.diffs[index].side(Side::Right);
            debug_assert_eq!(right.buffer, source);
            self.buffers[left.buffer] = snapshot;
            self.syntax[left.buffer] = None;
            let source_text = self.buffers[source].to_string();
            let session = DiffSession::new(left, right, text, &source_text);
            let equal = session.alignment().is_equal();
            self.diffs[index] = session;
            self.status(if equal {
                format!("{} and its disk version are identical", path.display())
            } else {
                format!("comparing disk with {}", path.display())
            });
            return;
        }
        self.buffers.push(snapshot);
        self.syntax.push(None);
        let disk = self.buffers.len() - 1;
        let Some((left, right)) = self.diff_sides(disk, source) else {
            self.closed_buffers.insert(disk);
            self.action_failed("comparing needs room for two panes");
            return;
        };
        debug_assert_eq!(left.buffer, disk);
        debug_assert_eq!(right.buffer, source);
        for side in [left, right] {
            if let Some(pane) = self.panes.get_mut(&side.pane) {
                pane.folds.clear();
                pane.preserve_scroll = false;
            }
        }
        let right_text = self.buffers[source].to_string();
        let session = DiffSession::new(left, right, text, &right_text);
        let equal = session.alignment().is_equal();
        self.diffs.push(session);
        self.status(if equal {
            format!("{} and its disk version are identical", path.display())
        } else {
            format!("comparing disk with {}", path.display())
        });
    }

    /// Settles which pane shows which buffer, and which of them is the left.
    ///
    /// Left and right are read off the screen rather than off the order the
    /// two buffers were marked in, so the side that is coloured as added is
    /// always the one the person sees on the right.
    fn diff_sides(&mut self, marked: usize, active: usize) -> Option<(DiffSide, DiffSide)> {
        let active_pane = self.active_pane;
        let marked_pane = match self
            .panes
            .iter()
            .find(|(pane_id, pane)| **pane_id != active_pane && pane.buffer == marked)
            .map(|(pane_id, _)| *pane_id)
        {
            Some(pane_id) => pane_id,
            None => {
                // Nothing else is showing the marked buffer, so it needs a
                // pane. A split always appends its new pane after the one it
                // came from, so the marked buffer takes over the original and
                // the buffer the person is in moves into the new one. That
                // reads the way the two commands were typed — the buffer
                // marked first is the one on the left — and the split carried
                // the person's selection with it, so they keep their place.
                self.split(Axis::Horizontal, None).ok()?;
                let opened = self.active_pane;
                let pane = self.panes.get_mut(&active_pane)?;
                pane.retarget(marked);
                pane.replace_selection(Selection::point(0));
                pane.scroll_row = 0;
                pane.scroll_wrap = 0;
                pane.scroll_col = 0;
                return Some((
                    DiffSide {
                        pane: active_pane,
                        buffer: marked,
                    },
                    DiffSide {
                        pane: opened,
                        buffer: active,
                    },
                ));
            }
        };

        let marked = DiffSide {
            pane: marked_pane,
            buffer: marked,
        };
        let active = DiffSide {
            pane: active_pane,
            buffer: active,
        };
        // The layout tree is walked first-then-second and laid out the same
        // way, so a pane's position in that walk is where it sits on screen.
        // Reading the order from the tree rather than from `areas` is what
        // makes it right for a pane split into existence a moment ago, which
        // has no geometry until the next frame.
        let mut ordered = Vec::new();
        self.layout.panes(&mut ordered);
        let position = |pane: usize| ordered.iter().position(|id| *id == pane);
        Some(if position(active.pane) < position(marked.pane) {
            (active, marked)
        } else {
            (marked, active)
        })
    }

    /// Closes the comparison the active pane or buffer belongs to.
    pub(super) fn diff_off(&mut self) {
        let pane_id = self.active_pane;
        let buffer = self.panes[&pane_id].buffer;
        let before = self.diffs.len();
        self.diffs
            .retain(|session| !session.has_pane(pane_id) && !session.has_buffer(buffer));
        if self.diffs.len() != before {
            self.status("comparison closed");
            return;
        }
        if self.pending_diff.take().is_some() {
            self.status("no longer marked for comparison");
            return;
        }
        self.status("no comparison is open");
    }

    pub(super) fn close_pane(&mut self) {
        if let Some(maximized) = self.maximized {
            self.status(format!(
                "leave {} before closing the pane",
                maximized.view.label()
            ));
            return;
        }
        if self.panes.len() == 1 {
            self.status("Cannot close the last pane. To quit runyte type :quit");
            return;
        }
        self.remove_pane(self.active_pane);
    }

    /// Removes one pane while preserving a valid focus and layout.
    ///
    /// Unlike the user-facing close command, lifecycle callers may name an
    /// inactive pane and may run while a maximized view is showing the pane
    /// whose process just ended. The last pane remains an invariant of the editor.
    fn remove_pane(&mut self, closing: usize) -> bool {
        if self.panes.len() == 1 || !self.panes.contains_key(&closing) {
            return false;
        }
        let mut ordered = Vec::new();
        self.layout.panes(&mut ordered);
        let focus_changed = self.active_pane == closing;
        if focus_changed {
            let index = ordered.iter().position(|id| *id == closing).unwrap_or(0);
            self.activate_pane(ordered[(index + 1) % ordered.len()]);
        }
        let layout = self.layout.clone().without(closing).unwrap();
        self.layout = layout;
        self.panes.remove(&closing);
        self.areas.remove(&closing);
        self.pane_opened_at.remove(&closing);
        self.pane_activated_at.remove(&closing);
        if self
            .maximized
            .is_some_and(|maximized| maximized.pane == closing)
        {
            self.maximized = None;
        }
        if focus_changed {
            self.finish_pane_focus(closing, false);
        }
        true
    }

    fn next_window(&mut self) {
        if let Some(maximized) = self.maximized {
            self.status(format!(
                "{} keeps the current pane maximized",
                maximized.view.label()
            ));
            return;
        }
        let mut panes = Vec::new();
        self.layout.panes(&mut panes);
        if let Some(index) = panes.iter().position(|pane| *pane == self.active_pane) {
            self.activate_pane(panes[(index + 1) % panes.len()]);
        }
    }

    pub(super) fn only_window(&mut self) {
        if let Some(maximized) = self.maximized {
            self.status(format!(
                "pane is already maximized by {}",
                maximized.view.label()
            ));
            return;
        }
        let active = self.active_pane;
        self.panes.retain(|pane, _| *pane == active);
        self.layout = Layout::Pane(active);
        self.areas.retain(|pane, _| *pane == active);
        self.pane_opened_at.retain(|pane, _| *pane == active);
        self.pane_activated_at.retain(|pane, _| *pane == active);
        self.status("only window");
    }

    /// Levels the split tree: every pane the same width, and then every pane
    /// sharing a column the same height.
    ///
    /// The tree itself is untouched, so which pane sits beside or above which
    /// is exactly what it was; only the boundaries between them move. A
    /// maximized pane hides the tree rather than replacing it, so the levelled
    /// layout is what the next toggle reveals.
    pub(super) fn equalize_panes(&mut self) {
        if matches!(self.layout, Layout::Pane(_)) {
            self.status("only one pane");
            return;
        }
        self.layout.equalize();
        self.status("equalized panes");
    }

    /// The maximized presentation the named pane is currently drawn with, if
    /// it is the one being maximized.
    pub(crate) fn maximized_view(&self, pane: usize) -> Option<MaximizedView> {
        self.maximized
            .filter(|maximized| maximized.pane == pane)
            .map(|maximized| maximized.view)
    }

    /// Toggles one maximized presentation without changing the underlying
    /// split tree. While active, frame preparation exposes only this pane at
    /// the full editor geometry; the next toggle reveals the untouched tree.
    ///
    /// The two views are one state rather than two, so asking for the other
    /// one switches to it instead of stacking a second maximization on top of
    /// the first. Only the view that is already showing toggles off.
    pub(super) fn toggle_maximized(&mut self, view: MaximizedView) {
        if self
            .maximized
            .is_some_and(|maximized| maximized.view == view)
        {
            self.maximized = None;
            match view {
                MaximizedView::Zen => self.status("zen mode disabled"),
                MaximizedView::Fullscreen => self.status("full-screen view disabled"),
            }
            return;
        }
        self.maximized = Some(MaximizedPane {
            pane: self.active_pane,
            view,
        });
        match view {
            MaximizedView::Zen => self.status(format!(
                "zen mode enabled at {} columns",
                self.config.editor.zen_width
            )),
            MaximizedView::Fullscreen => self.status("full-screen view enabled"),
        }
    }

    fn advance_pane_history(&mut self) -> u64 {
        self.pane_history_clock = self.pane_history_clock.wrapping_add(1);
        self.pane_history_clock
    }

    pub(super) fn record_pane_opened(&mut self, pane: usize) {
        let order = self.advance_pane_history();
        self.pane_opened_at.insert(pane, order);
    }

    pub(super) fn activate_pane(&mut self, pane: usize) {
        let order = self.advance_pane_history();
        self.pane_activated_at.insert(pane, order);
        self.active_pane = pane;
    }

    /// Gives a pointer press ownership of its pane without carrying document
    /// Insert across a focus boundary. A live terminal destination starts
    /// input, while a reviewed terminal keeps its snapshot and a press in the
    /// current document pane can still reposition its Insert caret. The drag
    /// path remains free to enter Select mode.
    pub(super) fn activate_pane_from_pointer(&mut self, pane: usize) {
        if pane != self.active_pane && matches!(self.mode, Mode::Insert | Mode::Replace) {
            self.enter_normal_mode();
        }
        self.activate_pane(pane);
        if let Some(id) = self.terminal_of_pane(pane) {
            self.settle_terminal_focus(id);
        }
    }

    fn pane_focus_rank(&self, pane: usize) -> (bool, u64, u64, usize) {
        let activated = self.pane_activated_at.get(&pane).copied();
        (
            activated.is_some(),
            activated.unwrap_or_default(),
            self.pane_opened_at.get(&pane).copied().unwrap_or_default(),
            pane,
        )
    }

    fn pane_neighbor(&self, dx: i32, dy: i32) -> Option<usize> {
        let current = self.areas.get(&self.active_pane).copied()?;
        let current_right = current.x.saturating_add(current.width);
        let current_bottom = current.y.saturating_add(current.height);
        self.areas
            .iter()
            .filter(|(id, _)| **id != self.active_pane)
            .filter_map(|(id, rect)| {
                let right = rect.x.saturating_add(rect.width);
                let bottom = rect.y.saturating_add(rect.height);
                let vertical_overlap = current.y.max(rect.y) < current_bottom.min(bottom);
                let horizontal_overlap = current.x.max(rect.x) < current_right.min(right);
                let shares_edge = if dx != 0 {
                    ((dx < 0 && right == current.x) || (dx > 0 && rect.x == current_right))
                        && vertical_overlap
                } else {
                    ((dy < 0 && bottom == current.y) || (dy > 0 && rect.y == current_bottom))
                        && horizontal_overlap
                };
                shares_edge.then_some(*id)
            })
            .max_by_key(|pane| self.pane_focus_rank(*pane))
    }

    pub(super) fn focus(&mut self, dx: i32, dy: i32) {
        // A maximized pane has no neighbours to move to, and saying so is the
        // rule rather than a consequence of the frame's geometry: `self.areas`
        // holds one pane only from the next `prepare_view` onwards, so a
        // replayed macro that maximizes and then moves would otherwise read
        // the previous frame's rectangles and focus a pane nobody can see.
        if let Some(maximized) = self.maximized {
            self.status(format!(
                "{} keeps the current pane maximized",
                maximized.view.label()
            ));
            return;
        }
        // `active_pane` is public for frontend ownership and tests, so observe
        // the current value at this semantic boundary even if it was assigned
        // without going through `activate_pane`.
        self.activate_pane(self.active_pane);
        if let Some(id) = self.pane_neighbor(dx, dy) {
            self.activate_pane(id);
            if self.mode == Mode::Insert && self.active_buffer().is_read_only() {
                self.mode = Mode::Normal;
            }
        }
    }

    pub(super) fn focus_from_terminal_insert(&mut self, dx: i32, dy: i32) {
        let terminal_input = self.mode == Mode::Insert && self.active_terminal().is_some();
        let replacing = self.mode == Mode::Replace;
        let previous_pane = self.active_pane;
        let replace_focus_moves = replacing
            && self.maximized.is_none()
            && self
                .pane_neighbor(dx, dy)
                .is_some_and(|pane| pane != previous_pane);
        if replace_focus_moves {
            self.enter_normal_mode();
        }
        self.focus(dx, dy);
        self.finish_pane_focus(previous_pane, terminal_input);
        if replacing && self.active_pane != previous_pane && self.active_terminal().is_none() {
            if self.active_buffer().is_read_only() {
                self.mode = Mode::Normal;
            } else {
                self.enter_replace_mode();
            }
        }
    }

    pub(super) fn next_window_from_terminal_insert(&mut self) {
        let terminal_input = self.mode == Mode::Insert && self.active_terminal().is_some();
        let replacing = self.mode == Mode::Replace;
        let previous_pane = self.active_pane;
        let mut panes = Vec::new();
        self.layout.panes(&mut panes);
        let replace_focus_moves = replacing
            && self.maximized.is_none()
            && panes
                .iter()
                .position(|pane| *pane == previous_pane)
                .is_some_and(|index| panes[(index + 1) % panes.len()] != previous_pane);
        if replace_focus_moves {
            self.enter_normal_mode();
        }
        self.next_window();
        self.finish_pane_focus(previous_pane, terminal_input);
        if replacing && self.active_pane != previous_pane && self.active_terminal().is_none() {
            if self.active_buffer().is_read_only() {
                self.mode = Mode::Normal;
            } else {
                self.enter_replace_mode();
            }
        }
    }

    /// Settles a user-driven pane focus change at the destination's natural
    /// mode. Live terminals resume input, reviewed terminals keep their
    /// snapshot, and documents reached directly from Terminal Insert remain
    /// protected by Normal mode.
    fn finish_pane_focus(&mut self, previous_pane: usize, terminal_input: bool) {
        if self.active_pane == previous_pane {
            return;
        }
        if let Some(id) = self.active_terminal() {
            self.settle_terminal_focus(id);
            self.grammar.reset();
        } else if terminal_input {
            self.enter_normal_mode();
        }
    }

    /// Chooses the mode of a terminal that gained focus without interpreting
    /// focus itself as an input request. Review belongs to the terminal
    /// session, so it survives application-wide mode changes in other panes.
    pub(super) fn settle_terminal_focus(&mut self, id: TerminalId) {
        let Some(session) = self.terminals.get_mut(id) else {
            return;
        };
        if !session.live() {
            session.begin_review();
            self.mode = Mode::Normal;
        } else if session.reviewing() {
            self.mode = Mode::Normal;
        } else {
            session.scroll_to_live();
            self.mode = Mode::Insert;
        }
    }

    /// Settles the destination of a window command begun from Insert mode.
    ///
    /// Mode is application-wide, while review belongs to one terminal
    /// session. A terminal reached from another terminal can therefore still
    /// own a snapshot captured before the source — terminal or document —
    /// entered Insert. Pane focus preserves that snapshot; a document reached
    /// from Terminal Insert instead uses Normal so the next key cannot edit it
    /// accidentally.
    fn finish_insert_window_command(&mut self, insert_mode: bool, terminal_input: bool) {
        if !insert_mode {
            return;
        }
        if let Some(id) = self.active_terminal() {
            self.settle_terminal_focus(id);
        } else if terminal_input {
            self.enter_normal_mode();
        }
    }

    pub(super) fn resize_pane_edge(&mut self, dx: i32, dy: i32, delta: i16) -> Result<()> {
        let Some(neighbor) = self.pane_neighbor(dx, dy) else {
            self.status("active pane has no boundary in that direction");
            return Ok(());
        };
        let Some(area) = enclosing_area(self.areas.values().copied()) else {
            self.status("pane layout is not ready");
            return Ok(());
        };
        self.layout
            .resize_between_cells(self.active_pane, neighbor, area, delta);
        self.status(format!(
            "resized pane by {} cell{}",
            delta.unsigned_abs(),
            if delta.unsigned_abs() == 1 { "" } else { "s" }
        ));
        Ok(())
    }
}
