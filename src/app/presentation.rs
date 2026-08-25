// SPDX-License-Identifier: MPL-2.0

//! Pane preparation, semantic overlay snapshots, and presentation-facing state.

// Application-module dependencies:
#[cfg(unix)]
use super::WorkspaceRow;
use super::{
    App, BindingScope, Buffer, CompletionSource, ConfirmationOverlay, ContentAlignment,
    ContentLayout, DiffProjection, DiffSession, FinderMode, FrameGeometry, GeneratedViewIdentity,
    HelpTopic, ListPurpose, MaximizedView, Mode, Pane, Path, Position, PreparedPane, PreparedRow,
    PreparedView, PromptKind, Rect, ResourceKind, ResourceTarget, Selection, SettingType, Side,
    StashMutation, adjust_scroll, adjust_scroll_wrapped, diff_projection, fold_hiding_row,
    move_projected_start_backward, project_aligned_rows, project_visible_rows,
    selection_for_launch_position,
};

impl App {
    pub fn active(&self) -> &Pane {
        &self.panes[&self.active_pane]
    }

    /// View coordinate of the active pane's primary caret.
    pub fn cursor_position(&self) -> Position {
        self.active().cursor(self.active_buffer())
    }

    pub(super) fn take_pending_launch_selection(&mut self, buffer: usize) -> Option<Selection> {
        let position = self.launch_positions.remove(&buffer)?;
        Some(selection_for_launch_position(
            &self.buffers[buffer],
            position,
        ))
    }

    pub(super) fn apply_pending_launch_position(&mut self, buffer: usize) {
        if let Some(selection) = self.take_pending_launch_selection(buffer) {
            self.active_mut().replace_selection(selection);
        }
    }

    /// The live diff a pane is one side of, if it is.
    pub fn diff_session(&self, pane_id: usize) -> Option<&DiffSession> {
        self.diffs.iter().find(|session| session.has_pane(pane_id))
    }

    /// How a pane projects its rows when it is one side of a live diff.
    ///
    /// Every consumer of the row projection goes through this, so what is
    /// drawn, what a click resolves to, where a jump label sits, and where
    /// `H`/`M`/`L` land are all the same rows.
    pub(super) fn diff_projection(&self, pane_id: usize) -> Option<DiffProjection<'_>> {
        diff_projection(&self.diffs, pane_id)
    }

    /// Whether a pane must ignore the configured soft wrap.
    ///
    /// Alignment is line-based, so a wrapped line takes a different number of
    /// screen rows on each side and the two views drift apart while still
    /// being correctly aligned. A diff pane therefore does not wrap. The
    /// config document also has its own fixed-width column wrapping, so
    /// applying visual soft wrap again would turn its padded rows into empty
    /// continuation rows.
    pub(super) fn pane_soft_wrap(&self, pane_id: usize) -> bool {
        let buffer = self
            .panes
            .get(&pane_id)
            .and_then(|pane| self.buffers.get(pane.buffer));
        let is_settings = buffer.is_some_and(Buffer::is_settings);
        // A document whose lines are long enough to make wrapping the frame's
        // dominant cost is shown unwrapped instead. Wrapping is measured per
        // logical line, so one minified line is one pass over the whole file
        // for every frame it is on screen.
        let viable = buffer.is_none_or(Buffer::soft_wrap_viable);
        self.config.editor.soft_wrap
            && self.diff_session(pane_id).is_none()
            && !is_settings
            && viable
    }

    /// Brings every live diff up to date and settles where both sides start.
    ///
    /// Run before any pane is projected, because a pane's rows depend on the
    /// alignment and on the aligned start its session agreed on.
    fn prepare_diffs(&mut self) {
        // A session whose panes or buffers have gone is over. Closing a pane
        // or a buffer is not a diff operation, so this is where that shows up
        // rather than in every command that can close one.
        self.diffs.retain(|session| {
            [Side::Left, Side::Right].into_iter().all(|side| {
                let side = session.side(side);
                self.panes
                    .get(&side.pane)
                    .is_some_and(|pane| pane.buffer == side.buffer)
            })
        });

        for index in 0..self.diffs.len() {
            let left = self.diffs[index].side(Side::Left);
            let right = self.diffs[index].side(Side::Right);
            let revisions = (
                self.buffers[left.buffer].revision(),
                self.buffers[right.buffer].revision(),
            );
            if self.diffs[index].needs_update(revisions) {
                let left = self.buffers[left.buffer].to_string();
                let right = self.buffers[right.buffer].to_string();
                self.diffs[index].update(revisions, &left, &right);
            }
        }
    }

    /// Puts both sides of every comparison on the same aligned row.
    ///
    /// This runs after the panes have been projected rather than before,
    /// because the leader's scroll is not settled until its own caret has been
    /// accounted for. Both sides are then re-projected from the position they
    /// agreed on, so neither side is ever a frame behind the other.
    fn settle_diff_scroll(&mut self, prepared: &mut [PreparedPane]) {
        for index in 0..self.diffs.len() {
            // The active pane leads and the other follows. When neither is
            // active the leader is the left side, so a session that nobody is
            // looking at still has one definite position.
            let leader = self.diffs[index]
                .side_of_pane(self.active_pane)
                .unwrap_or(Side::Left);
            let leader_pane = self.diffs[index].side(leader).pane;
            let Some(scroll_row) = prepared
                .iter()
                .find(|pane| pane.pane_id == leader_pane && pane.drawable)
                .map(|pane| pane.scroll_row)
            else {
                continue;
            };
            let aligned = self.diffs[index]
                .alignment()
                .aligned_row(leader, scroll_row);
            self.diffs[index].set_aligned_start(aligned);

            for side in [Side::Left, Side::Right] {
                let target = self.diffs[index].side(side);
                let row = self.diffs[index].row_at_or_above(side, aligned);
                if let Some(pane) = self.panes.get_mut(&target.pane) {
                    pane.scroll_row = row;
                    pane.scroll_wrap = 0;
                    pane.scroll_col = 0;
                }
                let Some(entry) = prepared
                    .iter_mut()
                    .find(|pane| pane.pane_id == target.pane && pane.drawable)
                else {
                    continue;
                };
                entry.scroll_row = row;
                entry.scroll_wrap = 0;
                entry.scroll_col = 0;
                entry.rows = project_aligned_rows(
                    &self.buffers[target.buffer],
                    DiffProjection {
                        alignment: self.diffs[index].alignment(),
                        side,
                        start: aligned,
                    },
                    entry.body_height,
                );
            }
        }
    }

    /// Prepares layout, gutter, wrapping, and scroll state before rendering.
    ///
    /// This is the only frame lifecycle step allowed to mutate view state.
    /// Rendering consumes the returned owned values and an immutable `App`.
    pub fn prepare_view(&mut self, geometry: FrameGeometry) -> PreparedView {
        self.sync_word_index();
        let pane_ids_to_reveal = self.panes.keys().copied().collect::<Vec<_>>();
        for pane_id in pane_ids_to_reveal {
            self.reveal_pane_selection_from_folds(pane_id);
        }
        self.prepare_diffs();
        self.areas.clear();
        if let Some(maximized) = self
            .maximized
            .filter(|maximized| self.panes.contains_key(&maximized.pane))
        {
            self.areas.insert(maximized.pane, geometry.editor);
        } else {
            self.layout.rectangles(geometry.editor, &mut self.areas);
        }
        let mut pane_ids = self.areas.keys().copied().collect::<Vec<_>>();
        pane_ids.sort_unstable();
        let mut prepared = Vec::with_capacity(pane_ids.len());

        for pane_id in pane_ids {
            let area = self.areas[&pane_id];
            let buffer_id = self.panes[&pane_id].buffer;
            // A terminal whose session has gone is not a terminal any more.
            // Resolving it here keeps every later question — geometry, title,
            // key routing — reading one answer.
            let terminal = self.panes[&pane_id]
                .terminal
                .filter(|id| self.terminals.get(*id).is_some());
            self.panes.get_mut(&pane_id).unwrap().terminal = terminal;
            if area.width < 3 || area.height < 3 {
                let pane = &self.panes[&pane_id];
                prepared.push(PreparedPane {
                    pane_id,
                    area,
                    body: Rect::default(),
                    buffer_id,
                    terminal,
                    drawable: false,
                    body_width: 0,
                    body_height: 0,
                    line_digits: 0,
                    signs: false,
                    changes: false,
                    gutter_width: 0,
                    text_width: 0,
                    content_indent: 0,
                    scroll_row: pane.scroll_row,
                    scroll_wrap: pane.scroll_wrap,
                    scroll_col: pane.scroll_col,
                    wrap_width: pane.wrap_width,
                    rows: Vec::new(),
                });
                continue;
            }

            let body = Rect {
                x: area.x + 1,
                y: area.y + 1,
                width: area.width - 2,
                height: area.height - 2,
            };
            let body_width = body.width as usize;
            let body_height = body.height as usize;
            if let Some(id) = terminal {
                // The pane's inside is the child's screen, whole: no gutter,
                // no line numbers, no content indent. This is also the one
                // place that knows the pane's new shape, so it is where the
                // child is told about it.
                if let Some(session) = self.terminals.get_mut(id)
                    && session.resize(body_width, body_height)
                {
                    session.focus_review_selection(body_height, self.config.editor.scroll_offset);
                }
                prepared.push(PreparedPane {
                    pane_id,
                    area,
                    body,
                    buffer_id,
                    terminal,
                    drawable: true,
                    body_width,
                    body_height,
                    line_digits: 0,
                    signs: false,
                    changes: false,
                    gutter_width: 0,
                    text_width: body_width,
                    content_indent: 0,
                    scroll_row: 0,
                    scroll_wrap: 0,
                    scroll_col: 0,
                    wrap_width: body_width.max(1),
                    rows: Vec::new(),
                });
                continue;
            }
            let line_digits = self.buffers[buffer_id].len_lines().max(1).to_string().len();
            let signs = self.buffers[buffer_id]
                .path
                .as_deref()
                .is_some_and(|path| !self.diagnostics.for_path(path).is_empty());
            // Marks are brought up to date before the width that reserves room
            // for them is decided, and the column stays for as long as the
            // file has a staged text — a gutter that appeared and vanished as
            // lines were edited would shift the whole pane sideways.
            self.update_git_marks(buffer_id);
            let folds = self.resolved_folds(pane_id);
            // A diff pane always enables change symbols, because the
            // comparison is what they are there to show whether or not the
            // file has a staged text behind it.
            let changes = self.git_tracks(buffer_id) || self.diff_session(pane_id).is_some();
            // The wrap arrow, fold triangle, and change symbol normally share
            // one indicator cell. A folded anchor that is itself changed
            // needs both of the latter, so that pane receives one additional
            // cell rather than hiding either claim.
            let split_fold_change = self.config.editor.line_numbers
                && folds
                    .iter()
                    .any(|fold| self.git_change(buffer_id, fold.anchor_row).is_some());
            let gutter_width = if self.config.editor.line_numbers {
                // Change symbols reuse the indicator cell between the line
                // number and separator unless a changed fold anchor needs a
                // second one. The final cell remains a margin.
                line_digits + 3 + usize::from(signs) + usize::from(split_fold_change)
            } else {
                usize::from(signs) + usize::from(changes)
            };
            // A centred page takes its margin out of the text column before
            // anything else is measured against it, so wrapping, clipping, and
            // hint placement all work in the width the text actually has.
            let layout = if self.maximized_view(pane_id) == Some(MaximizedView::Zen) {
                ContentLayout::viewport(self.config.editor.zen_width)
            } else {
                self.buffers[buffer_id].content_layout()
            };
            let content_indent = layout.indent(body_width.saturating_sub(gutter_width));
            let text_width = layout.width(body_width.saturating_sub(gutter_width));
            let cursor = self.panes[&pane_id].cursor(&self.buffers[buffer_id]);
            let soft_wrap = self.pane_soft_wrap(pane_id);
            let scroll_offset = self.config.editor.scroll_offset;
            let tab_width = self.config.editor.tab_width;
            // Resolved before the pane is borrowed mutably, and against the
            // sessions alone so the two borrows stay disjoint.
            let diff = diff_projection(&self.diffs, pane_id);
            let buffer = &self.buffers[buffer_id];
            let pane = self.panes.get_mut(&pane_id).unwrap();
            pane.wrap_width = text_width.max(1);
            if soft_wrap && folds.is_empty() {
                adjust_scroll_wrapped(
                    pane,
                    buffer,
                    cursor,
                    body_height,
                    text_width,
                    scroll_offset,
                    tab_width,
                );
            } else if soft_wrap {
                // The generic wrapper measures every logical row between the
                // viewport and caret. A collapsed range can make that gap
                // enormous, so folded panes use only the bounded projection
                // below.
                pane.scroll_row = pane.scroll_row.min(buffer.last_row());
                let start_count = crate::wrap::segments(
                    &buffer.line_string(pane.scroll_row),
                    text_width.max(1),
                    tab_width,
                )
                .len();
                pane.scroll_wrap = pane.scroll_wrap.min(start_count.saturating_sub(1));
                pane.scroll_col = 0;
            } else {
                pane.scroll_wrap = 0;
                adjust_scroll(pane, cursor, body_height, text_width, scroll_offset);
            }
            if let Some(fold) = fold_hiding_row(&folds, pane.scroll_row) {
                pane.scroll_row = fold.anchor_row;
                pane.scroll_wrap = 0;
            }
            let cursor_segment = if soft_wrap {
                crate::wrap::segment_index(
                    &buffer.line_string(cursor.row),
                    cursor.col,
                    text_width.max(1),
                    tab_width,
                )
            } else {
                0
            };
            if !pane.preserve_scroll {
                let visible = project_visible_rows(
                    buffer,
                    &folds,
                    pane.scroll_row,
                    pane.scroll_wrap,
                    body_height,
                    text_width.max(1),
                    tab_width,
                    soft_wrap,
                    diff,
                );
                let cursor_screen_row = visible.iter().position(|row| {
                    row.document_row == Some(cursor.row)
                        && row.segment.is_none_or(|segment| {
                            cursor.col >= segment.start
                                && (cursor.col < segment.end
                                    || cursor.col == segment.end
                                        && segment.end == buffer.line_len(cursor.row))
                        })
                });
                let margin = scroll_offset.min(body_height / 2);
                let desired = match cursor_screen_row {
                    Some(index) if index < margin => Some(margin),
                    Some(index) if index >= body_height.saturating_sub(margin) => {
                        Some(body_height.saturating_sub(margin + 1))
                    }
                    None => Some(margin),
                    _ => None,
                };
                if let Some(amount) = desired {
                    (pane.scroll_row, pane.scroll_wrap) = move_projected_start_backward(
                        buffer,
                        &folds,
                        cursor.row,
                        cursor_segment,
                        amount,
                        text_width.max(1),
                        tab_width,
                        soft_wrap,
                    );
                }
            }
            let mut rows = project_visible_rows(
                buffer,
                &folds,
                pane.scroll_row,
                pane.scroll_wrap,
                body_height,
                text_width.max(1),
                tab_width,
                soft_wrap,
                diff,
            );
            // A page that asks to be centred down the pane is projected from
            // its own first row instead: if the whole of it fits there is
            // nothing off-screen to scroll to, so it is held at the top and
            // the leftover height is split above and below it. A page too tall
            // to fit is left where the scroll position put it, and scrolls
            // like anything else.
            if layout.centers_vertically() {
                let whole = project_visible_rows(
                    buffer,
                    &folds,
                    0,
                    0,
                    body_height + 1,
                    text_width.max(1),
                    tab_width,
                    soft_wrap,
                    diff,
                );
                if whole.len() <= body_height {
                    // Vertical only: a page can be short enough to hold still
                    // and still be wider than the pane, and scrolling sideways
                    // is the only way to read the rest of it.
                    pane.scroll_row = 0;
                    pane.scroll_wrap = 0;
                    let above = layout.top(whole.len(), body_height);
                    rows = std::iter::repeat_n(PreparedRow::padding(), above)
                        .chain(whole)
                        .collect();
                    rows.resize(body_height, PreparedRow::padding());
                }
            }
            prepared.push(PreparedPane {
                pane_id,
                area,
                body,
                buffer_id,
                terminal,
                drawable: true,
                body_width,
                body_height,
                line_digits,
                signs,
                changes,
                gutter_width,
                text_width,
                content_indent,
                scroll_row: pane.scroll_row,
                scroll_wrap: pane.scroll_wrap,
                scroll_col: pane.scroll_col,
                wrap_width: pane.wrap_width,
                rows,
            });
        }

        self.settle_diff_scroll(&mut prepared);
        PreparedView {
            geometry,
            panes: prepared,
        }
    }

    pub fn active_buffer(&self) -> &Buffer {
        &self.buffers[self.active().buffer]
    }

    /// Directory requested by `:quit-here`, if that command passed the normal
    /// quit safety checks.
    pub fn quit_directory(&self) -> Option<&Path> {
        self.quit_directory.as_deref()
    }

    /// Enables `:quit-here` after the launcher has supplied a cwd handoff
    /// channel. Without this, exiting would silently behave like plain quit.
    pub fn enable_quit_directory_handoff(&mut self) {
        self.set_quit_directory_handoff(true);
    }

    /// Tracks whether whoever owns the terminal can receive a chosen directory.
    ///
    /// A persistent host outlives its clients and each one may or may not have
    /// been launched through the shell wrapper, so this follows the attached
    /// client rather than the host's own invocation.
    pub fn set_quit_directory_handoff(&mut self, enabled: bool) {
        self.quit_directory_handoff = enabled;
    }

    pub fn key_binding_scope(&self) -> BindingScope {
        // A terminal pane's buffer is the document behind it, not what the
        // keys are acting on. Reading its scope would give a pane showing a
        // shell the bindings of whatever explorer or Git view it will go back
        // to, which is the one wrong answer available here.
        if self.active_terminal().is_some() {
            return BindingScope::Terminal;
        }
        if self.active_buffer().is_directory() {
            BindingScope::Directory
        } else if self.active_buffer().is_settings() {
            BindingScope::Settings
        } else if self.active_buffer().is_git_status() {
            BindingScope::GitStatus
        } else if self.active_buffer().is_git_branches() {
            BindingScope::GitBranches
        } else if self.active_buffer().is_git_worktrees() {
            BindingScope::GitWorktrees
        } else if self.active_buffer().is_git_log() {
            BindingScope::GitLog
        } else if self.active_buffer().is_git_blame() {
            BindingScope::GitBlame
        } else if self.active_buffer().is_git_stash() {
            BindingScope::GitStash
        } else if self.active_buffer().is_workspace_search() {
            BindingScope::WorkspaceSearch
        } else if self.active_buffer().is_help() || self.active_buffer().is_manual() {
            BindingScope::Help
        } else if self.active_buffer().is_commit_message() {
            BindingScope::CommitMessage
        } else if self.active_buffer().is_diff() {
            BindingScope::Diff
        } else {
            BindingScope::Global
        }
    }

    /// Renders help for the active view into the shared help buffer and
    /// focuses it.
    ///
    /// The text is a snapshot of the view help was opened from, so it is
    /// rendered before the pane retargets: once the help buffer is active, the
    /// scope would describe help itself.
    ///
    /// Help does not depend on the mode, so every route in reaches the same
    /// document and the palette's own mode cannot leak into the answer.
    pub(super) fn open_help(&mut self) {
        let topic = if self.active_terminal().is_some() {
            HelpTopic::Terminal
        } else if self.active_buffer().is_notifications() {
            HelpTopic::Notifications
        } else {
            HelpTopic::for_context(self.key_binding_scope())
        };
        let text = crate::help::render(
            topic,
            self.grammar.kind(),
            self.key_binding_scope(),
            self.keymap(),
            self.active_buffer().is_read_only(),
        );
        let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_help()).then_some(index)
        });
        let buffer = match existing {
            Some(existing) => {
                self.buffers[existing].replace_virtual_text(&text);
                existing
            }
            None => {
                self.buffers.push(Buffer::help(&text));
                self.syntax.push(None);
                self.buffers.len() - 1
            }
        };
        self.push_jump();
        let pane = self.active_mut();
        pane.retarget(buffer);
        pane.replace_selection(Selection::point(0));
        pane.scroll_row = 0;
        pane.scroll_wrap = 0;
        pane.scroll_col = 0;
        self.mode = Mode::Normal;
    }

    /// Opens the general manual and optionally places its requested section at
    /// the top of the active pane. It is a different reusable buffer from
    /// contextual view help, so consulting one never overwrites the other.
    pub(super) fn open_manual(&mut self, requested: Option<&str>) {
        let topic = match requested.map(crate::manual::ManualTopic::resolve) {
            Some(Some(topic)) => Some(topic),
            Some(None) => {
                self.error(format!(
                    "unknown help topic: {}; available topics: {}",
                    requested.unwrap(),
                    crate::manual::available_topics()
                ));
                return;
            }
            None => None,
        };
        let text = crate::manual::render();
        let buffer = self.open_virtual_page(
            GeneratedViewIdentity::Manual,
            "[help]".to_owned(),
            &text,
            ContentAlignment::default(),
        );
        let offset = topic.map_or(0, |topic| crate::manual::topic_offset(&text, topic));
        let row = self.buffers[buffer].offset_to_row(offset);
        let pane = self.active_mut();
        pane.replace_selection(Selection::point(offset));
        pane.scroll_row = row;
        pane.scroll_wrap = 0;
        pane.scroll_col = 0;
    }

    /// Opens the small product front page as an ordinary read-only buffer,
    /// centred in whatever pane it lands in.
    pub(super) fn open_about(&mut self) {
        let text = crate::about::render();
        self.open_virtual_page(
            GeneratedViewIdentity::About,
            "[about]".to_owned(),
            &text,
            ContentAlignment::CENTERED,
        );
    }

    /// Whether a modal overlay owns the next key before normal/select dispatch.
    ///
    /// The terminal event loop uses the same boundary as `handle_key` so a key
    /// that closes an overlay cannot also produce a normal-mode key hint.
    pub fn has_input_overlay(&self) -> bool {
        self.picker.is_some()
            || self.fs_confirmation.is_some()
            || self.directory_reload_confirmation.is_some()
            || self.buffer_discard_confirmation.is_some()
            || self.git_discard_confirmation.is_some()
            || self.git_stash_confirmation.is_some()
            || self.git_branch_deletion.is_some()
            || self.git_pull_rebase.is_some()
            || self.git_worktree_removal.is_some()
            || self.list.is_some()
            || self.context_action_menu.is_some()
            || self.program_action_menu.is_some()
            || self.session_action_menu.is_some()
            || self.terminal_action_menu.is_some()
            || self.path_popup.is_some()
            || self.path_action_menu.is_some()
    }

    pub(crate) fn setting_choices_open(&self) -> bool {
        self.settings_view.is_some()
    }

    /// Describes the one pending yes/no decision without borrowing the status
    /// line. Service feedback and action echoes may change while a decision is
    /// open; its popup must continue to name the exact operation Enter accepts.
    fn confirmation_overlay(&self) -> Option<ConfirmationOverlay> {
        if let Some(confirmation) = &self.directory_reload_confirmation {
            return Some(ConfirmationOverlay {
                title: "Discard directory edits",
                accept: if confirmation.destination.is_some() {
                    "discard and open"
                } else {
                    "discard and refresh"
                },
                message: confirmation.message(),
            });
        }
        if let Some(buffer) = self.buffer_discard_confirmation {
            let name = self
                .buffers
                .get(buffer)
                .map_or_else(|| "buffer".to_owned(), Buffer::display_name);
            return Some(ConfirmationOverlay {
                title: "Discard buffer changes",
                accept: "discard changes",
                message: format!("Discard changes to {name}?\nEnter confirms.\nEscape cancels."),
            });
        }
        if let Some(confirmation) = &self.git_discard_confirmation {
            return Some(ConfirmationOverlay {
                title: "Discard Git changes",
                accept: "discard changes",
                message: confirmation.message(),
            });
        }
        if let Some(confirmation) = &self.git_stash_confirmation {
            let (title, accept) = match &confirmation.mutation {
                StashMutation::Create { .. } => ("Create stash", "create stash"),
                StashMutation::Apply { .. } => ("Apply stash", "apply stash"),
                StashMutation::Drop { .. } => ("Drop stash", "drop stash"),
            };
            return Some(ConfirmationOverlay {
                title,
                accept,
                message: confirmation.message.clone(),
            });
        }
        if let Some(confirmation) = &self.git_branch_deletion {
            return Some(ConfirmationOverlay {
                title: "Delete branch",
                accept: "delete branch",
                message: confirmation.message(),
            });
        }
        if let Some(confirmation) = &self.git_pull_rebase {
            return Some(ConfirmationOverlay {
                title: "Replay commits",
                accept: "replay commits",
                message: confirmation.message(),
            });
        }
        self.git_worktree_removal
            .as_ref()
            .map(|confirmation| ConfirmationOverlay {
                title: "Remove worktree",
                accept: "remove worktree",
                message: confirmation.message(),
            })
    }

    /// Captures every application overlay without leaking live application
    /// state to a frontend. The rows are bounded so a client snapshot cannot
    /// grow with an unbounded result set.
    pub fn overlay_snapshots(&self) -> Vec<crate::snapshot::OverlaySnapshot> {
        use crate::snapshot::{
            OverlayAction, OverlayIdentity, OverlayInput, OverlayKind, OverlayLayout,
            OverlayPreview, OverlayPurpose, OverlayRow, OverlaySnapshot,
        };

        const ROW_LIMIT: usize = 512;
        fn bounded(
            kind: OverlayKind,
            title: impl Into<String>,
            query: impl Into<String>,
            rows: Vec<OverlayRow>,
            selected: Option<usize>,
            message: Option<String>,
        ) -> OverlaySnapshot {
            let total_rows = rows.len();
            let row_offset = selected
                .map(|selected| selected.saturating_sub(ROW_LIMIT / 2))
                .unwrap_or_default()
                .min(total_rows.saturating_sub(ROW_LIMIT));
            let rows = rows
                .into_iter()
                .skip(row_offset)
                .take(ROW_LIMIT)
                .collect::<Vec<_>>();
            let (purpose, input, layout, actions) = match kind {
                OverlayKind::FilesystemConfirmation => (
                    OverlayPurpose::Confirmation,
                    OverlayInput::None,
                    OverlayLayout::Standard,
                    vec![
                        OverlayAction::new("Enter", "apply with trash"),
                        OverlayAction::new("P", "apply with permanent deletion"),
                        OverlayAction::new("↑/↓", "review operations"),
                        OverlayAction::new("Esc", "cancel"),
                    ],
                ),
                OverlayKind::FilePicker => (
                    OverlayPurpose::Picker,
                    OverlayInput::Filter,
                    OverlayLayout::Preview,
                    vec![
                        OverlayAction::new("Enter", "open"),
                        OverlayAction::new("Ctrl-t", "toggle preview"),
                        OverlayAction::new("Esc", "cancel"),
                    ],
                ),
                OverlayKind::ResultList => (
                    OverlayPurpose::Picker,
                    OverlayInput::Filter,
                    OverlayLayout::Standard,
                    vec![OverlayAction::new("Esc", "cancel")],
                ),
                OverlayKind::BufferActions | OverlayKind::ProgramActions => (
                    OverlayPurpose::Choice,
                    OverlayInput::None,
                    OverlayLayout::Standard,
                    vec![
                        OverlayAction::new("Enter", "run"),
                        OverlayAction::new("Esc", "cancel"),
                    ],
                ),
                OverlayKind::Confirmation => (
                    OverlayPurpose::Confirmation,
                    OverlayInput::None,
                    OverlayLayout::Standard,
                    vec![
                        OverlayAction::new("Enter", "confirm"),
                        OverlayAction::new("Esc", "cancel"),
                    ],
                ),
                OverlayKind::CommandPalette => (
                    OverlayPurpose::CommandPalette,
                    OverlayInput::Text,
                    OverlayLayout::Standard,
                    vec![
                        OverlayAction::new("Enter", "accept"),
                        OverlayAction::new("Esc", "cancel"),
                    ],
                ),
                OverlayKind::ProgramHints => (
                    OverlayPurpose::Picker,
                    OverlayInput::Text,
                    OverlayLayout::Standard,
                    vec![
                        OverlayAction::new("Enter", "open"),
                        OverlayAction::new("Tab", "actions"),
                        OverlayAction::new("Esc", "cancel"),
                    ],
                ),
                OverlayKind::Path => (
                    OverlayPurpose::Info,
                    OverlayInput::None,
                    OverlayLayout::Standard,
                    vec![
                        OverlayAction::new("Tab", "actions"),
                        OverlayAction::new("Esc", "close"),
                    ],
                ),
                OverlayKind::PathActions => (
                    OverlayPurpose::Info,
                    OverlayInput::None,
                    OverlayLayout::Standard,
                    vec![
                        OverlayAction::new("j/k, ↑/↓, or Shift-Tab", "select"),
                        OverlayAction::new("mnemonic/Enter", "copy"),
                        OverlayAction::new("Tab/Esc", "back"),
                    ],
                ),
                OverlayKind::Prompt => (
                    OverlayPurpose::Input,
                    OverlayInput::Text,
                    OverlayLayout::Setting,
                    vec![
                        OverlayAction::new("Enter", "save"),
                        OverlayAction::new("Esc", "cancel"),
                    ],
                ),
                OverlayKind::Completion => (
                    OverlayPurpose::Context,
                    OverlayInput::None,
                    OverlayLayout::Anchored,
                    vec![
                        OverlayAction::new("Enter", "accept"),
                        OverlayAction::new("Esc", "dismiss"),
                    ],
                ),
                OverlayKind::Signature | OverlayKind::Hover => (
                    OverlayPurpose::Context,
                    OverlayInput::None,
                    OverlayLayout::Anchored,
                    vec![OverlayAction::new("any key", "dismiss")],
                ),
                OverlayKind::KeyHints => (
                    OverlayPurpose::Context,
                    OverlayInput::None,
                    OverlayLayout::Bottom,
                    vec![OverlayAction::new("Esc", "dismiss")],
                ),
            };
            OverlaySnapshot {
                kind,
                purpose,
                input,
                layout,
                actions,
                title: title.into(),
                query: query.into(),
                selected: selected
                    .filter(|selected| *selected < total_rows)
                    .map(|selected| selected - row_offset),
                scroll_anchor: selected.filter(|selected| *selected < total_rows),
                row_offset,
                rows,
                message,
                omitted_rows: total_rows.saturating_sub(ROW_LIMIT),
                total_rows,
                query_cursor: None,
                show_preview: false,
                preview_title: None,
                preview: None,
            }
        }
        fn row(
            identity: impl Into<OverlayIdentity>,
            label: impl Into<String>,
            detail: impl Into<String>,
        ) -> OverlayRow {
            OverlayRow {
                identity: identity.into(),
                label: label.into(),
                detail: detail.into(),
                available: true,
                dimmed: false,
                muted: Vec::new(),
                emphasis: Vec::new(),
            }
        }

        let mut overlays = Vec::new();
        if let Some(confirmation) = &self.fs_confirmation {
            overlays.push(bounded(
                OverlayKind::FilesystemConfirmation,
                format!("Filesystem plan · {}", confirmation.plan.root().display()),
                "",
                confirmation
                    .plan
                    .lines()
                    .into_iter()
                    .enumerate()
                    .map(|(index, line)| row(index, line, ""))
                    .collect(),
                Some(confirmation.selected),
                Some("Enter apply · P permanently delete · Esc cancel".to_owned()),
            ));
        }
        if let Some(picker) = &self.picker {
            if let Some(finder) = &self.finder
                && finder.mode == FinderMode::Resources
            {
                let mut snapshot = bounded(
                    OverlayKind::FilePicker,
                    format!("Find · {}", finder.mode.title()),
                    picker.query.clone(),
                    finder
                        .matches
                        .iter()
                        .filter_map(|found| finder.items.get(found.item).map(|item| (found, item)))
                        .map(|(found, item)| {
                            let identity = match item.target {
                                ResourceTarget::Buffer(buffer) => format!("buffer:{buffer}"),
                                ResourceTarget::Terminal(id) => format!("terminal:{id}"),
                            };
                            let mut row = row(identity, item.label.clone(), item.detail.clone());
                            row.emphasis = found.emphasis.clone();
                            row
                        })
                        .collect(),
                    (!finder.matches.is_empty()).then_some(finder.selected),
                    None,
                );
                snapshot.query_cursor = Some(picker.query_cursor);
                snapshot.layout = OverlayLayout::Preview;
                snapshot.actions = vec![
                    OverlayAction::new("Enter", "open"),
                    OverlayAction::new("Tab", "files"),
                    OverlayAction::new("Ctrl-t", "toggle preview"),
                    OverlayAction::new("Esc", "cancel"),
                ];
                snapshot.show_preview = picker.show_preview;
                snapshot.preview_title = finder.selected_item().map(|item| match item.kind {
                    ResourceKind::Buffer => "Contents".to_owned(),
                    ResourceKind::Terminal => "Output".to_owned(),
                });
                snapshot.preview = Some(finder.selected_preview().map_or(
                    OverlayPreview::Empty,
                    |preview| {
                        OverlayPreview::Text(preview.split('\n').map(str::to_owned).collect())
                    },
                ));
                overlays.push(snapshot);
            } else {
                let mut snapshot = bounded(
                    OverlayKind::FilePicker,
                    if self.finder.is_some() {
                        format!("Find · Files · {}", picker.root.display())
                    } else {
                        format!("{} · {}", picker.kind.title(), picker.root.display())
                    },
                    picker.query.clone(),
                    picker
                        .matches
                        .iter()
                        .filter_map(|found| picker.view(found.entry).map(|entry| (found, entry)))
                        .map(|(found, entry)| {
                            let label = entry.label();
                            let identity = format!(
                                "{}:{}",
                                entry.path.display(),
                                entry.row.map_or(0, |row| row + 1)
                            );
                            let mut row = row(identity, label, "");
                            row.emphasis = entry.match_positions_in_label(&found.positions);
                            row
                        })
                        .collect(),
                    (!picker.matches.is_empty()).then_some(picker.selected),
                    picker
                        .error
                        .clone()
                        .or_else(|| picker.loading.then(|| "Scanning files…".to_owned())),
                );
                snapshot.query_cursor = Some(picker.query_cursor);
                snapshot.show_preview = picker.show_preview;
                snapshot.preview_title = Some("Preview".to_owned());
                snapshot.preview = Some(match picker.preview.as_ref() {
                    Some(crate::file_picker::FilePreview::Text(lines)) => {
                        OverlayPreview::Text(lines.clone())
                    }
                    Some(crate::file_picker::FilePreview::Snippet(snippet)) => {
                        OverlayPreview::Snippet {
                            lines: snippet.lines.clone(),
                            start_row: snippet.start_row,
                            focus_row: snippet.focus_row,
                            emphasis: snippet.emphasis.clone(),
                        }
                    }
                    Some(crate::file_picker::FilePreview::Binary) => OverlayPreview::Binary,
                    Some(crate::file_picker::FilePreview::Directory(lines)) => {
                        OverlayPreview::Text(lines.clone())
                    }
                    Some(crate::file_picker::FilePreview::Unreadable(error)) => {
                        OverlayPreview::Unavailable(error.clone())
                    }
                    None => OverlayPreview::Empty,
                });
                if self.finder.is_some() {
                    snapshot
                        .actions
                        .insert(1, OverlayAction::new("Tab", "buffers + terminals"));
                }
                overlays.push(snapshot);
            }
        }
        if let Some(picker) = &self.list {
            let visible = picker.visible_indices();
            let report = picker.purpose == ListPurpose::Report;
            let report_offset = picker.report_offset.min(visible.len().saturating_sub(1));
            let all_rows = visible
                .iter()
                .filter_map(|index| picker.items.get(*index))
                .map(|item| {
                    let mut row = row(item.index, item.label.clone(), item.detail.clone());
                    row.dimmed = item.is_dimmed();
                    if picker.has_preview() {
                        row.emphasis = picker.item_label_emphasis(item);
                    }
                    row
                })
                .collect::<Vec<_>>();
            let total_rows = all_rows.len();
            let rows = if report {
                all_rows
                    .into_iter()
                    .skip(report_offset)
                    .take(ROW_LIMIT)
                    .collect()
            } else {
                all_rows
            };
            let selected = (!report && !visible.is_empty()).then_some(picker.selected);
            let mut snapshot = bounded(
                OverlayKind::ResultList,
                picker.title.clone(),
                picker.filter.clone(),
                rows,
                selected,
                None,
            );
            snapshot.purpose = match picker.purpose {
                ListPurpose::Picker => OverlayPurpose::Picker,
                ListPurpose::Choice => OverlayPurpose::Choice,
                ListPurpose::Manager => OverlayPurpose::Manager,
                ListPurpose::Report => OverlayPurpose::Report,
            };
            if picker.purpose == ListPurpose::Report {
                snapshot.selected = None;
                snapshot.scroll_anchor = (total_rows > 0).then_some(report_offset);
                snapshot.row_offset = report_offset;
                snapshot.total_rows = total_rows;
                snapshot.omitted_rows = total_rows.saturating_sub(snapshot.rows.len());
            }
            snapshot.input = if picker.accepts_filter_input() {
                OverlayInput::Filter
            } else {
                OverlayInput::None
            };
            snapshot.layout = if self.settings_view.is_some() {
                OverlayLayout::SettingChoice
            } else if picker.has_preview() {
                OverlayLayout::Preview
            } else {
                OverlayLayout::Standard
            };
            snapshot.actions.clear();
            if picker.has_tags() {
                snapshot
                    .actions
                    .push(OverlayAction::new("Tab", picker.tag_label()));
            }
            if let Some(action) = &picker.primary_action {
                snapshot.actions.push(OverlayAction::new("Enter", action));
            }
            if let Some((key, action)) = &picker.secondary_action {
                snapshot.actions.push(OverlayAction::new(key, action));
            }
            if picker.purpose == ListPurpose::Report {
                snapshot.actions.push(OverlayAction::new("↑/↓", "scroll"));
                snapshot.actions.push(OverlayAction::new("Esc", "dismiss"));
            } else {
                if picker.has_preview() {
                    snapshot
                        .actions
                        .push(OverlayAction::new("Ctrl-t", "toggle preview"));
                }
                snapshot.actions.push(OverlayAction::new("Esc", "cancel"));
            }
            if picker.has_preview() {
                snapshot.show_preview = picker.show_preview;
                snapshot.preview_title = picker.preview_title().map(str::to_owned);
                snapshot.preview =
                    picker
                        .selected_preview()
                        .map(|preview| OverlayPreview::MatchedText {
                            lines: preview.split('\n').map(str::to_owned).collect(),
                            emphasis: picker.selected_preview_emphasis(),
                        });
            }
            overlays.push(snapshot);
        }
        if let Some(menu) = &self.context_action_menu {
            let mut snapshot = bounded(
                OverlayKind::BufferActions,
                format!("Actions · {}", self.active_buffer().display_name()),
                "",
                menu.actions
                    .iter()
                    .map(|action| {
                        let context = match action.context {
                            crate::keymap::ActionContext::Row => "row",
                            crate::keymap::ActionContext::Buffer => "buffer",
                        };
                        row(
                            action.mnemonic.label(),
                            action.mnemonic.label(),
                            format!("{context} · {}", action.description),
                        )
                    })
                    .collect(),
                Some(menu.selected),
                None,
            );
            snapshot.actions = vec![
                OverlayAction::new("j/k, ↑/↓, or Shift-Tab", "select"),
                OverlayAction::new("mnemonic/Enter", "run"),
                OverlayAction::new("Tab/Esc/Ctrl-c", "cancel"),
            ];
            overlays.push(snapshot);
        }
        if let Some(popup) = &self.path_popup {
            let (kind, rows, selected) = match &self.path_action_menu {
                Some(menu) => (
                    OverlayKind::PathActions,
                    menu.actions
                        .iter()
                        .map(|action| {
                            row(
                                action.mnemonic().to_string(),
                                action.mnemonic().to_string(),
                                action.label(),
                            )
                        })
                        .collect(),
                    Some(menu.selected),
                ),
                None => (OverlayKind::Path, Vec::new(), None),
            };
            overlays.push(bounded(
                kind,
                "Path",
                "",
                rows,
                selected,
                Some(popup.path.clone()),
            ));
        }
        if let Some(menu) = &self.buffer_action_menu {
            overlays.push(bounded(
                OverlayKind::BufferActions,
                self.buffers
                    .get(menu.buffer)
                    .map_or_else(|| "Buffer actions".to_owned(), Buffer::display_name),
                "",
                menu.actions
                    .iter()
                    .map(|action| row(action.label(), action.label(), ""))
                    .collect(),
                Some(menu.selected),
                None,
            ));
        }
        #[cfg(unix)]
        if let Some(menu) = &self.session_action_menu {
            overlays.push(bounded(
                OverlayKind::BufferActions,
                self.workspace_rows
                    .get(menu.row)
                    .map_or_else(|| "Session actions".to_owned(), WorkspaceRow::display_name),
                "",
                menu.actions
                    .iter()
                    .map(|action| {
                        row(
                            action.label().to_ascii_lowercase(),
                            action.label(),
                            action.description(),
                        )
                    })
                    .collect(),
                Some(menu.selected),
                None,
            ));
        }
        if let Some(menu) = &self.terminal_action_menu {
            overlays.push(bounded(
                OverlayKind::BufferActions,
                self.terminals.get(menu.id).map_or_else(
                    || "Terminal actions".to_owned(),
                    |session| format!("Terminal actions · {}", session.display_name()),
                ),
                "",
                menu.actions
                    .iter()
                    .map(|action| {
                        row(
                            action.label().to_ascii_lowercase(),
                            action.label(),
                            action.description(),
                        )
                    })
                    .collect(),
                Some(menu.selected),
                None,
            ));
        }

        if let Some(confirmation) = self.confirmation_overlay() {
            let mut overlay = bounded(
                OverlayKind::Confirmation,
                confirmation.title,
                "",
                Vec::new(),
                None,
                Some(confirmation.message),
            );
            overlay.actions = vec![
                OverlayAction::new("Enter", confirmation.accept),
                OverlayAction::new("Esc", "cancel"),
            ];
            overlays.push(overlay);
        }

        if self.mode == Mode::Command {
            if self.prompt_kind == PromptKind::Command {
                if let Some(hints) = self.matching_path_hints() {
                    overlays.push(bounded(
                        OverlayKind::CommandPalette,
                        "Paths",
                        self.command.clone(),
                        hints
                            .iter()
                            .map(|hint| {
                                row(
                                    hint.value.clone(),
                                    hint.value.clone(),
                                    format!(
                                        "{} · {}",
                                        if hint.is_directory {
                                            "directory"
                                        } else {
                                            "file"
                                        },
                                        hint.detail
                                    ),
                                )
                            })
                            .collect(),
                        (!hints.is_empty()).then_some(self.command_selection),
                        None,
                    ));
                } else {
                    let matches = self.matching_commands();
                    overlays.push(bounded(
                        OverlayKind::CommandPalette,
                        "Commands",
                        self.command.clone(),
                        matches
                            .iter()
                            .map(|matched| {
                                let category = matched.category.label();
                                let label = format!("[{category}] :{}", matched.usage());
                                let command_start = category.chars().count() + 3;
                                let available = matched.availability.is_available();
                                let aliases = matched.other_names().join(", ");
                                let availability = matched.availability.reason().map_or_else(
                                    || "available".to_owned(),
                                    |reason| format!("unavailable: {reason}"),
                                );
                                let mut row = row(
                                    matched.spec.name,
                                    label,
                                    format!(
                                        "{} · aliases: {} · {availability}",
                                        matched.spec.description,
                                        if aliases.is_empty() { "-" } else { &aliases }
                                    ),
                                );
                                row.available = available;
                                row.muted = (0..command_start).collect();
                                if available {
                                    row.emphasis =
                                        (command_start..row.label.chars().count()).collect();
                                }
                                row
                            })
                            .collect(),
                        (!matches.is_empty()).then_some(self.command_selection),
                        None,
                    ));
                }
            } else if self.prompt_kind == PromptKind::ExternalProgram {
                let choices = self.matching_program_choices();
                overlays.push(bounded(
                    OverlayKind::ProgramHints,
                    "Open with · Enter open · Tab actions",
                    self.command.clone(),
                    choices
                        .iter()
                        .map(|choice| {
                            let detail = match (choice.is_default, choice.system) {
                                (true, true) => "default · system opener",
                                (true, false) => "default",
                                (false, true) => "system opener",
                                (false, false) => "",
                            };
                            row(choice.program.clone(), choice.program.clone(), detail)
                        })
                        .collect(),
                    (!choices.is_empty())
                        .then_some(self.command_selection.min(choices.len().saturating_sub(1))),
                    self.external_target
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned()),
                ));
                if let Some(menu) = &self.program_action_menu {
                    overlays.push(bounded(
                        OverlayKind::ProgramActions,
                        format!("{} actions", menu.choice.program),
                        "",
                        menu.actions
                            .iter()
                            .map(|action| row(action.label(), action.label(), ""))
                            .collect(),
                        Some(menu.selected),
                        None,
                    ));
                }
            } else if matches!(self.prompt_kind, PromptKind::SettingValue(_)) {
                let title = match self.prompt_kind {
                    PromptKind::SettingValue(setting) => match setting.descriptor().value_type {
                        SettingType::Integer { minimum, maximum } => format!(
                            "{} · integer {minimum}–{maximum} · Enter save · Esc cancel",
                            setting.descriptor().key
                        ),
                        SettingType::Text => {
                            format!("{} · Enter save · Esc cancel", setting.descriptor().key)
                        }
                        SettingType::Grammar
                        | SettingType::Boolean
                        | SettingType::Theme
                        | SettingType::WorkspaceMode => setting.descriptor().key.to_owned(),
                    },
                    _ => format!("{:?}", self.prompt_kind),
                };
                let message = matches!(self.prompt_kind, PromptKind::SettingValue(_))
                    .then(|| self.status_error.then(|| self.status.clone()))
                    .flatten();
                let mut snapshot = bounded(
                    OverlayKind::Prompt,
                    title,
                    self.command.clone(),
                    Vec::new(),
                    None,
                    message,
                );
                snapshot.query_cursor = Some(self.command_cursor);
                overlays.push(snapshot);
            }
        }
        if let Some(completion) = &self.completion {
            let visible = completion.visible_indices();
            if !visible.is_empty() {
                let title = match completion.source {
                    CompletionSource::Language => "LSP Complete",
                    CompletionSource::Path | CompletionSource::Word => "Complete",
                };
                let mut snapshot = bounded(
                    OverlayKind::Completion,
                    title,
                    completion.filter.clone(),
                    visible
                        .iter()
                        .filter_map(|index| completion.items.get(*index))
                        .map(|item| {
                            row(
                                item.label.clone(),
                                item.label.clone(),
                                format!("{} · {}", item.kind, item.detail),
                            )
                        })
                        .collect(),
                    Some(completion.selected),
                    None,
                );
                // Every source can open on its own — Word for any
                // three-character prefix, Language after `.`/`:`, Path after
                // `/` — so only Tab accepts; Enter is reserved for its usual
                // newline everywhere.
                snapshot.actions = vec![
                    OverlayAction::new("↑/↓, Ctrl-n/p", "navigate"),
                    OverlayAction::new("Tab", "accept"),
                    OverlayAction::new("Esc", "dismiss"),
                ];
                overlays.push(snapshot);
            }
        }
        if let Some(signature) = &self.signature {
            overlays.push(bounded(
                OverlayKind::Signature,
                "Signature",
                "",
                signature
                    .signatures
                    .iter()
                    .enumerate()
                    .map(|(index, signature)| {
                        let mut row = row(
                            index,
                            signature.label.clone(),
                            signature.documentation.clone(),
                        );
                        if let Some((start, end)) = signature.active_parameter
                            && (end as usize) <= signature.label.len()
                            && signature.label.is_char_boundary(start as usize)
                            && signature.label.is_char_boundary(end as usize)
                        {
                            let start = signature.label[..start as usize].chars().count();
                            let end = signature.label[..end as usize].chars().count();
                            row.emphasis.extend(start..end);
                        }
                        row
                    })
                    .collect(),
                None,
                None,
            ));
        }
        if let Some(hover) = &self.hover {
            let mut snapshot = bounded(
                OverlayKind::Hover,
                "Documentation",
                "",
                hover
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(index, line)| row(index, line.clone(), ""))
                    .collect(),
                None,
                None,
            );
            snapshot.rows.truncate(self.hover_visible_rows());
            snapshot.omitted_rows = snapshot.total_rows.saturating_sub(snapshot.rows.len());
            snapshot.actions = if snapshot.omitted_rows > 0 {
                vec![
                    OverlayAction::new("Enter", "open complete documentation"),
                    OverlayAction::new("other key", "dismiss and continue"),
                ]
            } else {
                vec![OverlayAction::new("any key", "dismiss and continue")]
            };
            overlays.push(snapshot);
        }
        overlays
    }
}
