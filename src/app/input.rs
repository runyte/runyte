// SPDX-License-Identifier: MPL-2.0

//! Input ownership, key dispatch, command execution, prompts, and confirmations.

// Application-module dependencies:
#[cfg(unix)]
use super::parse_session_number;
use super::{
    ActiveGrammar, App, AppCapabilitySnapshot, ApplyReport, ArgumentKind, Axis, BTreeMap, Buffer,
    BufferKind, COMMAND_PATH_HINT_LIMIT, COMMANDS, Capabilities, Change, ColonCommand,
    CommandArguments, CommandAvailability, CommandExecutionContext, CommandId, CommandInvocation,
    CommandMatch, CommandOutcome, CommandOutcomeHint, CommandRefusal, CommandState,
    CommandUnavailable, CompletionSource, ContentAlignment, DEFAULT_MACRO_REGISTER,
    DeletionAuthorization, DeletionMode, DelimiterPair, DiffScope, EditorCommand, EditorIntent,
    EntryKind, FileObservation, FilePicker, FinderTarget, FsOperation, GeneratedViewIdentity,
    GrammarContext, GrammarNotice, GrammarOutput, HOVER_PEEK_ROWS, HashSet, HelpInvocation,
    InputEvent, InputGrammar, Instant, InvocationParameters, KeyCode, KeySequence, KeyStroke,
    Keymap, LineDirection, ListPicker, LspCommand, MaximizedView, Mode, Modifiers, Motion, Offset,
    Path, PathBuf, PathHint, PickerTarget, PointerButton, PointerDrag, PointerEvent,
    PointerEventKind, PointerOutcome, PreparedView, ProgramAction, ProgramActionMenu,
    ProgramChoice, PromptKind, Range, RangeIntent, RequestKind, Result, SearchMode, SearchQuery,
    Selection, SelectionSemantics, SettingId, SettingType, SettingValue, SignatureContext,
    StashScope, SyntaxObject, SyntaxObjectPart, SyntaxSelectionTransform, SystemClipboard,
    Transaction, ViewAlignment, VimMotion, VimOperator, VimRangeTarget, VimTextObject,
    buffer_language, char_to_byte, display_path, enclosing_area, expand_home_path, external_open,
    hint_is_not_before, hover_content_rows, is_path_separator, is_path_token_boundary,
    is_terminal_normal_key, is_word, is_word_completion_character, keymap_for, mapped_applied_path,
    operative_span, parse_colon_command, persistent_session_availability, pointer_pane,
    pointer_resize_pair, prompt_backspace, prompt_delete, prompt_delete_range, prompt_insert,
    prompt_word_backward, prompt_word_forward, quote_path_hint, rect_contains, resolve_command,
    resolved_operation_path, row_characters, unclosed_or_complete_quoted_path,
};

impl App {
    pub(super) fn key_text(&self, template: &str) -> String {
        crate::key_spelling::resolve(template, self.keymap())
            .expect("actionable-message key markers must resolve")
            .text
    }

    /// Returns the registry used by this editor instance.
    ///
    /// Dispatch remains behavior-compatible in the foundation commit; Feature
    /// B will route execution through this same registry.
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// Replaces the binding registry. Primarily useful to deterministic
    /// embedders and tests that need a keymap smaller than the default one.
    pub fn set_keymap(&mut self, keymap: std::sync::Arc<Keymap>) {
        self.keymap = keymap;
    }

    /// Points the registry at the variant the current configuration asks for.
    ///
    /// Called wherever `config` moves — a preview, a save, a rolled-back
    /// preview — because the keymap is a reading of configuration rather than
    /// state of its own, and a stale one would answer keys the reader has
    /// already turned off.
    pub(super) fn sync_keymap(&mut self) {
        self.keymap = self
            .configured_keymaps
            .as_ref()
            .map(|maps| {
                std::sync::Arc::clone(&maps[usize::from(self.config.editor.fast_pane_keys)])
            })
            .unwrap_or_else(|| keymap_for(self.config.editor.fast_pane_keys));
    }

    /// Whether this key moves between panes on its own right now.
    ///
    /// A terminal in Insert mode owns every key its child could want, so a key
    /// only reaches Runyte when something above names it. `Ctrl-w` is named
    /// there permanently; these four join it exactly while the option is on.
    fn is_fast_pane_key(&self, key: KeyStroke) -> bool {
        self.config.editor.fast_pane_keys && crate::keymap::is_fast_pane_key(key)
    }

    /// Replaces the host clipboard boundary. Primarily useful to deterministic
    /// embedders and tests that must not touch a person's real clipboard.
    pub fn set_system_clipboard(&mut self, clipboard: Box<dyn SystemClipboard>) {
        self.ports.replace_clipboard(clipboard);
    }

    /// Replaces the recoverable-deletion boundary. Primarily useful to
    /// deterministic embedders and tests that must not touch platform trash.
    pub fn set_trash_backend(&mut self, trash: Box<dyn crate::fs_plan::TrashBackend>) {
        self.ports.replace_trash(trash);
    }

    pub(crate) fn replace_active_selection(&mut self, selection: Selection) {
        let buffer = self.active().buffer;
        self.active_mut().replace_selection(selection);
        self.normalize_buffer(buffer);
        self.active_mut()
            .mark_selection_semantics(SelectionSemantics::Runyte);
    }

    pub fn pending_sequence(&self) -> &KeySequence {
        self.grammar.pending_sequence()
    }

    pub fn pending_count(&self) -> Option<usize> {
        self.grammar.pending_count()
    }

    pub const fn grammar_kind(&self) -> crate::command::GrammarKind {
        self.grammar.kind()
    }

    /// Registry mode used by key discovery for the active grammar.
    pub fn key_hint_mode(&self) -> Option<Mode> {
        // Character-taking commands own the next key before the registry can
        // interpret it as a command or prefix. In particular, the first Space
        // in `r Space Space` is replacement text; only the second Space starts
        // the application command tree.
        if self.grammar.awaiting_character().is_some() {
            return None;
        }
        match self.grammar.kind() {
            crate::command::GrammarKind::Runyte
                if matches!(self.mode, Mode::Normal | Mode::Select) =>
            {
                Some(self.mode)
            }
            crate::command::GrammarKind::Vim
                if matches!(self.mode, Mode::Normal | Mode::Select) =>
            {
                Some(Mode::Normal)
            }
            crate::command::GrammarKind::Runyte | crate::command::GrammarKind::Vim => None,
        }
    }

    /// Registry mode a frontend should use while observing this key.
    pub fn key_hint_mode_for_key(&self, key: KeyStroke) -> Option<Mode> {
        if matches!(self.mode, Mode::Insert | Mode::Replace)
            && (key.canonical_for_binding() == self.keymap.window_prefix()
                || !self.grammar.pending_sequence().is_empty())
        {
            return Some(self.mode);
        }
        self.key_hint_mode()
    }

    /// Takes the persistent-session meaning of an editor-level exit.
    ///
    /// Direct workspace switches clear `should_quit` through
    /// `take_workspace_switch` before this is read. The fallback covers older
    /// internal paths that request an ordinary safe quit by setting the public
    /// application flag directly.
    pub fn take_persistent_exit_request(&mut self) -> Option<super::PersistentExitRequest> {
        if !std::mem::take(&mut self.should_quit) {
            return None;
        }
        Some(
            self.persistent_exit_request
                .take()
                .unwrap_or(super::PersistentExitRequest::Quit { force: false }),
        )
    }

    /// Captures every optional capability consulted by the command palette.
    /// One owned value drives the complete projection so rows cannot mix
    /// availability observed before and after an asynchronous service event.
    pub fn command_capabilities(&self) -> AppCapabilitySnapshot {
        let buffer_id = self.active().buffer;
        let language_id = self
            .buffers
            .get(buffer_id)
            .and_then(|buffer| buffer_language(buffer, &self.registry));
        let syntax = if self.syntax.get(buffer_id).is_some_and(Option::is_some) {
            CommandAvailability::Available
        } else if self.stale_syntax.contains_key(&buffer_id) {
            CommandAvailability::Unavailable(
                "syntax tree is being refreshed for this buffer".to_owned(),
            )
        } else if let Some(error) = language_id.and_then(|language| {
            self.registry
                .errors()
                .into_iter()
                .find(|error| error.language == language)
        }) {
            CommandAvailability::Unavailable(format!("syntax unavailable: {error}"))
        } else {
            CommandAvailability::Unavailable("syntax is unavailable for this buffer".to_owned())
        };
        let lsp_manager = if !self.config.lsp.enable {
            CommandAvailability::Unavailable("language servers are disabled in settings".to_owned())
        } else if !self.lsp_workspace_allowed {
            CommandAvailability::Unavailable(
                "LSP is disabled for this workspace; use :lsp-trust".to_owned(),
            )
        } else if !self.ports.has_lsp() {
            CommandAvailability::Unavailable("language-server manager is not attached".to_owned())
        } else {
            CommandAvailability::Available
        };
        let lsp_document = match self.language_of(buffer_id) {
            None => CommandAvailability::Unavailable(
                "the active buffer has no recognized language".to_owned(),
            ),
            Some(_) if !self.config.lsp.enable => CommandAvailability::Unavailable(
                "language servers are disabled in settings".to_owned(),
            ),
            Some(_) if !self.lsp_workspace_allowed => CommandAvailability::Unavailable(
                "LSP is disabled for this workspace; use :lsp-trust".to_owned(),
            ),
            Some(language) if !self.config.lsp.servers.contains_key(&language) => {
                CommandAvailability::Unavailable(format!(
                    "no language server is configured for {language}"
                ))
            }
            Some(_) if !self.ports.has_lsp() => CommandAvailability::Unavailable(
                "language-server manager is not attached".to_owned(),
            ),
            Some(language) if !self.lsp_servers.contains_key(&language) => {
                CommandAvailability::Unavailable(format!(
                    "the {language} language server is not ready"
                ))
            }
            Some(_) if !self.lsp_documents.contains_key(&buffer_id) => {
                CommandAvailability::Unavailable(
                    "the active file is not attached to its language server".to_owned(),
                )
            }
            Some(_) => CommandAvailability::Available,
        };
        let git_project = if !self.has_git() {
            CommandAvailability::Unavailable("no `git` executable was found".to_owned())
        } else if let Some(message) = self.git_state.discovery_failure_message() {
            CommandAvailability::Unavailable(message)
        } else if self.git.repository().is_some() {
            CommandAvailability::Available
        } else if self.git_state.discovery_complete() {
            CommandAvailability::Unavailable("this project is not in a Git repository".to_owned())
        } else {
            CommandAvailability::Unavailable(
                "Git repository discovery is still in progress".to_owned(),
            )
        };
        let git_refresh = if self.has_git()
            && self.git_state.discovery_complete()
            && self.git_state.discovery_error().is_some()
        {
            CommandAvailability::Available
        } else {
            git_project.clone()
        };
        AppCapabilitySnapshot {
            syntax,
            lsp_manager,
            lsp_document,
            git_project,
            git_refresh,
            persistent_session: persistent_session_availability(
                cfg!(unix),
                self.persistent_session,
            ),
        }
    }

    pub fn matching_commands(&self) -> Vec<CommandMatch> {
        let capabilities = self.command_capabilities();
        self.matching_commands_with_capabilities(&capabilities)
    }

    pub(super) fn matching_commands_with_capabilities(
        &self,
        capabilities: &AppCapabilitySnapshot,
    ) -> Vec<CommandMatch> {
        let trimmed = self.command.trim();
        let query = trimmed.split_whitespace().next().unwrap_or_default();
        if query.is_empty() {
            // Nothing typed yet is a table of contents, so it lists each
            // command once under its canonical name.
            let mut commands = COMMANDS
                .iter()
                .map(|spec| CommandMatch::canonical(spec, capabilities))
                .collect::<Vec<_>>();
            commands.sort_by_key(|matched| matched.name);
            return commands;
        }
        if trimmed.chars().any(char::is_whitespace) && resolve_command(query).is_some() {
            return resolve_command(query)
                .map(|spec| CommandMatch::canonical(spec, capabilities))
                .into_iter()
                .collect();
        }
        let terms = trimmed
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        let mut matches = COMMANDS
            .iter()
            .filter_map(|spec| {
                let matched_name = spec.names().find(|name| name.starts_with(query));
                let haystack = format!(
                    "{} {} {} {} {}",
                    spec.name,
                    spec.aliases.join(" "),
                    spec.usage,
                    spec.description,
                    spec.category().label()
                )
                .to_lowercase();
                (matched_name.is_some() || terms.iter().all(|term| haystack.contains(term))).then(
                    || {
                        matched_name.map_or_else(
                            || CommandMatch::canonical(spec, capabilities),
                            |name| CommandMatch::new(spec, name, capabilities),
                        )
                    },
                )
            })
            .collect::<Vec<_>>();
        // Direct canonical/alias prefixes remain the first completion even
        // when the same short query also occurs in several descriptions.
        matches.sort_by_key(|matched| usize::from(!matched.name.starts_with(query)));
        matches
    }

    /// Files and directories matching the argument of a path-valued command.
    ///
    /// `None` means the palette is still completing command names. `Some`
    /// means a path argument owns the rows, including when no entry matches.
    pub fn matching_path_hints(&self) -> Option<Vec<PathHint>> {
        let command_len = self.command.chars().count();
        let cursor_at_end = self.command_cursor == command_len;
        let cursor_before_closing_quote = self.command_cursor + 1 == command_len
            && self
                .command
                .chars()
                .last()
                .is_some_and(|character| matches!(character, '\'' | '"'));
        if !cursor_at_end && !cursor_before_closing_quote {
            return None;
        }
        let (name, argument) = self.command.split_once(char::is_whitespace)?;
        let spec = resolve_command(name)?;
        if !matches!(
            spec.arguments,
            CommandArguments::Required(ArgumentKind::Path)
                | CommandArguments::Optional(ArgumentKind::Path)
        ) {
            return None;
        }
        Some(self.path_hints_for(argument))
    }

    /// Files and directories matching one typed path, wherever it was typed.
    ///
    /// Shared by the palette's path arguments and by the finder-path prompt,
    /// so both spell `~`, a relative path, and a trailing separator the same
    /// way and bound their rows the same way.
    fn path_hints_for(&self, argument: &str) -> Vec<PathHint> {
        let argument = argument.trim_start();
        let raw = unclosed_or_complete_quoted_path(argument);
        let separator = std::path::MAIN_SEPARATOR;
        if raw == "~"
            && let Some(home) = &self.home_directory
        {
            return vec![PathHint {
                value: format!("~{separator}"),
                name: format!("~{separator}"),
                detail: display_path(home),
                is_directory: true,
            }];
        }

        let typed = PathBuf::from(raw);
        let expanded = expand_home_path(typed, self.home_directory.as_deref());
        let ends_in_separator = raw.ends_with(is_path_separator);
        let (directory, prefix) = if raw.is_empty() {
            (self.working_directory.clone(), "")
        } else if ends_in_separator {
            (self.resolve_hint_path(expanded), "")
        } else {
            let prefix = Path::new(raw)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(raw);
            let parent = expanded.parent().unwrap_or_else(|| Path::new(""));
            (self.resolve_hint_path(parent.to_path_buf()), prefix)
        };

        let base_end = raw.rfind(is_path_separator).map_or(0, |index| index + 1);
        let display_base = &raw[..base_end];
        let Some(entries) = self.path_listings.borrow_mut().read(&directory) else {
            return Vec::new();
        };
        let show_hidden = self.config.editor.show_hidden_files || prefix.starts_with('.');
        // Where each row's detail column starts, resolved once: naming the
        // directory per row would ask the operating system for the working
        // directory once per row.
        let detail_base = PathBuf::from(display_path(&directory));
        // Keyed by the order the rows are shown in, so the bound below drops
        // the last row rather than whichever entries the filesystem happened
        // to return late. Reading the whole directory before bounding is what
        // lets a typed prefix find its matches in a directory larger than the
        // bound. The exact spelling is part of the key only to separate two
        // names that differ just in case.
        let mut kept = BTreeMap::<(bool, String, String), PathHint>::new();
        for entry in entries.iter() {
            let name = entry.name.as_str();
            if !name.starts_with(prefix) || (!show_hidden && name.starts_with('.')) {
                continue;
            }
            let is_directory = entry.is_directory;
            // Once the bound is full, only a name that sorts before the last
            // row kept can change the answer. The key holds the row without
            // the base every row shares, so this compares just the name.
            if kept.len() >= COMMAND_PATH_HINT_LIMIT
                && kept.last_key_value().is_some_and(|(last, _)| {
                    hint_is_not_before(name, is_directory, separator, last)
                })
            {
                continue;
            }
            let row = row_characters(name, is_directory, separator).collect::<String>();
            let folded = row.chars().flat_map(char::to_lowercase).collect::<String>();
            kept.insert(
                (!is_directory, folded, row.clone()),
                PathHint {
                    value: format!("{display_base}{row}"),
                    name: row.clone(),
                    detail: detail_base.join(name).display().to_string(),
                    is_directory,
                },
            );
            if kept.len() > COMMAND_PATH_HINT_LIMIT {
                kept.pop_last();
            }
        }
        kept.into_values().collect()
    }

    /// Files and directories matching the finder-path prompt's text.
    ///
    /// The whole prompt is the path, so there is no command name to strip and
    /// no argument gate to pass.
    pub fn finder_path_hints(&self) -> Option<Vec<PathHint>> {
        (self.prompt_kind == PromptKind::FinderPath
            && self.command_cursor == self.command.chars().count())
        .then(|| self.path_hints_for(&self.command))
    }

    fn resolve_hint_path(&self, path: PathBuf) -> PathBuf {
        if path.is_absolute() {
            path
        } else {
            self.working_directory.join(path)
        }
    }

    fn command_hint_count(&self) -> usize {
        self.matching_path_hints()
            .map_or_else(|| self.matching_commands().len(), |hints| hints.len())
    }

    /// Handles one frontend-neutral input event.
    ///
    /// Literal text stays one event and one edit transaction. Macro recording
    /// stores the same raw event ordering that arrived at this boundary.
    pub fn handle_input(&mut self, input: InputEvent) -> Result<()> {
        if self.macro_replay.is_some() {
            if is_macro_replay_cancel(&input) {
                self.cancel_macro_replay();
            } else {
                self.macro_replay_progress_status();
            }
            return Ok(());
        }
        self.handle_input_inner(input, false)
    }

    pub(super) fn handle_replayed_input(&mut self, input: InputEvent) -> Result<()> {
        self.handle_input_inner(input, true)
    }

    fn handle_input_inner(&mut self, input: InputEvent, replaying: bool) -> Result<()> {
        // A selected-line request belongs to the exact interaction state that
        // produced it. Later keyboard or text intent makes that selection
        // stale even when it did not edit the buffer.
        let overlay_owns_input = self.has_input_overlay();
        let interactive = !matches!(input, InputEvent::Pointer(_));
        let previous_action = self.active_action_id;
        if interactive {
            let action = self.next_action_id;
            self.next_action_id = self.next_action_id.wrapping_add(1).max(1);
            self.active_action_id = Some(action);
            self.invalidate_all_partial_guards();
            // Filtering and navigating an open popup continue the command
            // that opened it. Keep teaching that command until input returns
            // to the editor; background notifications never replace it.
            if !overlay_owns_input {
                self.action_feedback = None;
            }
        }
        self.last_interaction = Instant::now();
        self.status_error = false;
        let recording_before = self.recording_macro;
        let recordable = !matches!(input, InputEvent::Pointer(_));
        let recorded_input = input.clone();
        let result = match input {
            InputEvent::Key(key) => self.handle_key_stroke(key),
            InputEvent::Text(text) => self.handle_text(&text),
            InputEvent::Pointer(_) => Ok(()),
        };
        if result.is_ok() {
            self.retire_detached_ephemeral_buffers();
        }
        if result.is_ok()
            && recordable
            && !replaying
            && let Some(register) = recording_before
            && self.recording_macro == Some(register)
        {
            self.macro_staging.push(recorded_input);
            // An unresolved sequence may yet turn out to be the one that stops
            // the recording, so it only joins the macro once it has resolved
            // into something else.
            if self.grammar.pending_sequence().is_empty() {
                let staged = std::mem::take(&mut self.macro_staging);
                self.macros.entry(register).or_default().extend(staged);
            }
        }
        if result.is_ok() {
            self.reconcile_tutorial();
        }
        self.active_action_id = previous_action;
        result
    }

    /// Convenience entry point for embedders and tests producing one key.
    pub fn handle_key(&mut self, key: KeyStroke) -> Result<()> {
        self.handle_input(key.into())
    }

    /// Applies one owned pointer event through the exact fold/wrap projection
    /// that produced the visible frame.
    ///
    /// A future frontend can supply the same [`PreparedView`] without knowing
    /// anything about terminal cells or Crossterm event types.
    pub fn handle_pointer(&mut self, event: PointerEvent, view: &PreparedView) -> Result<()> {
        self.handle_pointer_repeated(event, view, 1).map(|_| ())
    }

    /// Applies a bounded run of identical wheel events. Frontends use this to
    /// preserve scroll distance while avoiding one transport and PTY queue
    /// operation per physical wheel report.
    pub fn handle_pointer_repeated(
        &mut self,
        event: PointerEvent,
        view: &PreparedView,
        repetitions: u16,
    ) -> Result<PointerOutcome> {
        if self.macro_replay.is_some() {
            return Ok(PointerOutcome::Unchanged);
        }
        // Crossterm's mouse capture includes passive any-motion events. They
        // carry no editor intent and must not clear status, hints, or trigger
        // any semantic work when this owned boundary is used by another
        // frontend.
        if event.kind == PointerEventKind::Moved {
            self.forward_terminal_pointer(event, view, 1);
            return Ok(PointerOutcome::Unchanged);
        }
        self.last_interaction = Instant::now();
        if self.mode == Mode::Command || self.has_input_overlay() {
            self.invalidate_all_partial_guards();
            self.status_error = false;
            self.pointer_drag = None;
            return Ok(PointerOutcome::Changed);
        }
        let active_pane = self.active_pane;
        if self.forward_terminal_pointer(event, view, repetitions) {
            return Ok(if self.active_pane != active_pane {
                self.invalidate_all_partial_guards();
                self.status_error = false;
                PointerOutcome::Changed
            } else {
                PointerOutcome::Unchanged
            });
        }
        self.invalidate_all_partial_guards();
        self.status_error = false;
        match event.kind {
            PointerEventKind::Down(PointerButton::Left) => {
                if self.jump.take().is_some() {
                    self.status("jump cancelled");
                }
                // A pointer press cancels any modal prefix/operator/register
                // state before it establishes a new spatial interaction.
                self.grammar.reset();
                if let Some((first, second, axis)) =
                    pointer_resize_pair(view, event.column, event.row)
                {
                    self.pointer_drag = Some(PointerDrag::Resize {
                        first,
                        second,
                        axis,
                        last_column: event.column,
                        last_row: event.row,
                    });
                    return Ok(PointerOutcome::Changed);
                }
                let Some(pane_id) = pointer_pane(view, event.column, event.row) else {
                    self.pointer_drag = None;
                    return Ok(PointerOutcome::Changed);
                };
                self.activate_pane_from_pointer(pane_id);
                if let Some(terminal) = self.terminal_of_pane(pane_id) {
                    // Live cells still belong to the child. Once review has
                    // captured them, however, they are an immutable
                    // Runyte-owned surface and accept the same character-cell
                    // selection gesture as a document.
                    let rows = usize::from(view.pane(pane_id).unwrap().body.height);
                    let screen_row = usize::from(event.row - view.pane(pane_id).unwrap().body.y);
                    let screen_column =
                        usize::from(event.column - view.pane(pane_id).unwrap().body.x);
                    let extend = event.modifiers.contains(Modifiers::SHIFT);
                    let Some(offset) = self.terminals.get(terminal).and_then(|session| {
                        session.review_offset_at_view_cell(rows, screen_row, screen_column)
                    }) else {
                        self.pointer_drag = None;
                        return Ok(PointerOutcome::Changed);
                    };
                    let anchor = if extend {
                        self.terminals
                            .get(terminal)
                            .and_then(|session| session.review_selection_anchor())
                            .unwrap_or(offset)
                    } else {
                        offset
                    };
                    if let Some(session) = self.terminals.get_mut(terminal) {
                        session.set_review_selection(anchor, offset);
                    }
                    self.pointer_drag = Some(PointerDrag::TerminalSelection {
                        pane: pane_id,
                        terminal,
                        anchor,
                    });
                    self.mode = if anchor == offset {
                        Mode::Normal
                    } else {
                        Mode::Select
                    };
                    self.dismiss_popups();
                    return Ok(PointerOutcome::Changed);
                }
                // Focus has already settled, so this mode is the one the
                // press leaves behind. A Shift-click is extending a selection
                // rather than placing an Insert caret, so its head addresses a
                // character even when the pane was in Insert mode.
                let extend = event.modifiers.contains(Modifiers::SHIFT);
                let insert = matches!(self.mode, Mode::Insert | Mode::Replace) && !extend;
                let Some(offset) =
                    self.pointer_offset(view, pane_id, event.column, event.row, insert)
                else {
                    self.pointer_drag = None;
                    return Ok(PointerOutcome::Changed);
                };
                let previous_mode = self.mode;
                let anchor = if extend {
                    self.pointer_anchor(pane_id)
                } else {
                    offset
                };
                let (selection, semantics) = self.pointer_selection(anchor, offset);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.replace_selection(selection);
                pane.mark_selection_semantics(semantics);
                self.pointer_drag = Some(PointerDrag::Selection {
                    pane: pane_id,
                    buffer: pane.buffer,
                    anchor,
                });
                if extend && anchor != offset {
                    self.mode = Mode::Select;
                } else if matches!(previous_mode, Mode::Insert | Mode::Replace) {
                    self.mode = previous_mode;
                    if previous_mode == Mode::Replace {
                        self.replace_session = Some(super::ReplaceSession {
                            buffer: self.active().buffer,
                            steps: Vec::new(),
                        });
                    }
                } else if anchor == offset {
                    self.mode = Mode::Normal;
                } else {
                    self.mode = Mode::Select;
                }
                self.dismiss_popups();
            }
            PointerEventKind::Drag(PointerButton::Left) => match self.pointer_drag {
                Some(PointerDrag::Selection {
                    pane,
                    buffer,
                    anchor,
                }) => {
                    if self
                        .panes
                        .get(&pane)
                        .is_none_or(|candidate| candidate.buffer != buffer)
                    {
                        self.pointer_drag = None;
                        return Ok(PointerOutcome::Changed);
                    }
                    // A drag builds a selection whatever mode it started in,
                    // and a selection covers characters.
                    let Some(offset) =
                        self.pointer_offset(view, pane, event.column, event.row, false)
                    else {
                        return Ok(PointerOutcome::Changed);
                    };
                    self.activate_pane(pane);
                    let (selection, semantics) = self.pointer_selection(anchor, offset);
                    let candidate = self.panes.get_mut(&pane).unwrap();
                    candidate.replace_selection(selection);
                    candidate.mark_selection_semantics(semantics);
                    self.mode = if anchor == offset {
                        Mode::Normal
                    } else {
                        Mode::Select
                    };
                }
                Some(PointerDrag::TerminalSelection {
                    pane,
                    terminal,
                    anchor,
                }) => {
                    if self.terminal_of_pane(pane) != Some(terminal) {
                        self.pointer_drag = None;
                        return Ok(PointerOutcome::Changed);
                    }
                    let Some(prepared) = view.pane(pane) else {
                        self.pointer_drag = None;
                        return Ok(PointerOutcome::Changed);
                    };
                    if !rect_contains(prepared.body, event.column, event.row) {
                        return Ok(PointerOutcome::Changed);
                    }
                    let screen_row = usize::from(event.row.saturating_sub(prepared.body.y));
                    let screen_column = usize::from(event.column.saturating_sub(prepared.body.x));
                    let Some(offset) = self.terminals.get(terminal).and_then(|session| {
                        session.review_offset_at_view_cell(
                            usize::from(prepared.body.height),
                            screen_row,
                            screen_column,
                        )
                    }) else {
                        return Ok(PointerOutcome::Changed);
                    };
                    self.activate_pane(pane);
                    if let Some(session) = self.terminals.get_mut(terminal) {
                        session.set_review_selection(anchor, offset);
                    }
                    self.mode = if anchor == offset {
                        Mode::Normal
                    } else {
                        Mode::Select
                    };
                }
                Some(PointerDrag::Resize {
                    first,
                    second,
                    axis,
                    last_column,
                    last_row,
                }) => {
                    let cells = match axis {
                        Axis::Horizontal => i32::from(event.column) - i32::from(last_column),
                        Axis::Vertical => i32::from(event.row) - i32::from(last_row),
                    };
                    let delta = cells.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
                    if delta != 0 {
                        self.layout.resize_between_cells(
                            first,
                            second,
                            view.geometry.editor,
                            delta,
                        );
                    }
                    self.pointer_drag = Some(PointerDrag::Resize {
                        first,
                        second,
                        axis,
                        last_column: event.column,
                        last_row: event.row,
                    });
                }
                None => {}
            },
            PointerEventKind::Up(_) => self.pointer_drag = None,
            PointerEventKind::ScrollUp | PointerEventKind::ScrollDown => {
                let Some(pane) = pointer_pane(view, event.column, event.row) else {
                    return Ok(PointerOutcome::Changed);
                };
                if !self.panes.contains_key(&pane) {
                    return Ok(PointerOutcome::Changed);
                }
                // A wheel over a terminal moves its scrollback. The pane's
                // buffer is the document behind it, and scrolling that would
                // move a viewport nobody can see.
                if let Some(id) = self.terminal_of_pane(pane)
                    && let Some(session) = self.terminals.get_mut(id)
                {
                    let lines = 3usize.saturating_mul(usize::from(repetitions));
                    if event.kind == PointerEventKind::ScrollUp {
                        session.scroll_back(lines);
                    } else {
                        session.scroll_forward(lines);
                    }
                    return Ok(PointerOutcome::Changed);
                }
                let direction = if event.kind == PointerEventKind::ScrollUp {
                    -1
                } else {
                    1
                };
                for _ in 0..3usize.saturating_mul(usize::from(repetitions)) {
                    self.scroll_pane(pane, direction);
                }
            }
            PointerEventKind::ScrollLeft | PointerEventKind::ScrollRight => {
                let Some(pane) = pointer_pane(view, event.column, event.row) else {
                    return Ok(PointerOutcome::Changed);
                };
                if !self.panes.contains_key(&pane) {
                    return Ok(PointerOutcome::Changed);
                }
                let soft_wrap = self.pane_soft_wrap(pane);
                let buffer = self.panes[&pane].buffer;
                let prefix_width = self.buffers[buffer].row_prefix_width();
                if prefix_width > 0 || !soft_wrap {
                    let candidate = self.panes.get_mut(&pane).unwrap();
                    let columns = 3usize.saturating_mul(usize::from(repetitions));
                    if event.kind == PointerEventKind::ScrollLeft {
                        let text_columns = columns.min(candidate.scroll_col);
                        candidate.scroll_col -= text_columns;
                        candidate.row_prefix_scroll = candidate
                            .row_prefix_scroll
                            .saturating_sub(columns - text_columns);
                    } else {
                        let prefix_columns =
                            columns.min(prefix_width.saturating_sub(candidate.row_prefix_scroll));
                        candidate.row_prefix_scroll += prefix_columns;
                        if !soft_wrap {
                            candidate.scroll_col = candidate
                                .scroll_col
                                .saturating_add(columns - prefix_columns);
                        }
                    }
                    candidate.preserve_scroll = true;
                }
            }
            PointerEventKind::Down(PointerButton::Right) => {
                self.pointer_drag = None;
                if !self.pointer_over_active_selection(view, event.column, event.row) {
                    return Ok(PointerOutcome::Changed);
                }
                self.jump = None;
                self.grammar.reset();
                let terminal_review = self
                    .terminal_of_pane(self.active_pane)
                    .and_then(|terminal| self.terminals.get(terminal))
                    .is_some_and(|session| session.reviewing());
                let state = CommandState::capture(self);
                self.execute_editor_command(EditorCommand::ClipboardYank)?;
                let mut outcome = state.outcome(self, CommandOutcomeHint::Infer);
                if terminal_review && matches!(outcome, CommandOutcome::Status(_)) {
                    outcome = CommandOutcome::Status("yanked to system clipboard".to_owned());
                }
                self.report_completed_action(
                    "right mouse click",
                    "Yank to the system clipboard",
                    outcome,
                );
            }
            PointerEventKind::Down(PointerButton::Middle)
            | PointerEventKind::Drag(PointerButton::Middle | PointerButton::Right)
            | PointerEventKind::Moved => {}
        }
        Ok(PointerOutcome::Changed)
    }

    /// Whether a pointer cell belongs to text the active surface would copy.
    ///
    /// Document selections retain their own inclusive or half-open semantics,
    /// while terminal review exposes the exact cell spans it highlights. A
    /// secondary press only invokes clipboard yank when it lands on one of
    /// those spans, so it never moves the selection it is about to copy.
    fn pointer_over_active_selection(&self, view: &PreparedView, column: u16, row: u16) -> bool {
        let Some(prepared) = view.pane(self.active_pane) else {
            return false;
        };
        if !rect_contains(prepared.body, column, row) {
            return false;
        }
        if let Some(id) = self.terminal_of_pane(self.active_pane) {
            let Some(session) = self.terminals.get(id).filter(|session| session.reviewing()) else {
                return false;
            };
            let terminal = session.view(usize::from(prepared.body.height));
            let pointer_row = usize::from(row - prepared.body.y);
            let pointer_column = usize::from(column - prepared.body.x);
            let highlighted = terminal.highlights.iter().any(|highlight| {
                matches!(
                    highlight.kind,
                    crate::terminal::TerminalHighlightKind::Selection
                        | crate::terminal::TerminalHighlightKind::ActiveMatch
                ) && highlight.row == pointer_row
                    && pointer_column >= highlight.start_column
                    && pointer_column < highlight.end_column
            });
            let caret = terminal.cursor.is_some_and(|(caret_row, caret_column)| {
                let width = terminal
                    .rows
                    .get(caret_row)
                    .and_then(|cells| cells.get(caret_column))
                    .map_or(1, |cell| usize::from(cell.width.max(1)));
                caret_row == pointer_row
                    && pointer_column >= caret_column
                    && pointer_column < caret_column.saturating_add(width)
            });
            return highlighted || caret;
        }

        let Some(offset) = self.pointer_text_offset(view, self.active_pane, column, row) else {
            return false;
        };
        let pane = self.active();
        let buffer = self.active_buffer();
        let half_open = matches!(
            pane.selection_semantics(),
            SelectionSemantics::HalfOpen | SelectionSemantics::VimLinewise
        );
        pane.selection.ranges().iter().any(|range| {
            let (from, to) = if half_open {
                (range.from(), range.to())
            } else {
                operative_span(buffer, range)
            };
            offset >= from && offset < to
        })
    }

    /// The buffer offset of the exact text cell under the pointer.
    ///
    /// Caret placement deliberately clamps the gutter, content margin, and
    /// blank cells after a row onto the nearest character. Selection hit
    /// testing must not: those cells carry no selection highlight and a
    /// secondary press on them is not a press on selected text.
    pub(super) fn pointer_text_offset(
        &self,
        view: &PreparedView,
        pane_id: usize,
        column: u16,
        row: u16,
    ) -> Option<Offset> {
        let pane = view.pane(pane_id)?;
        if !pane.drawable || !rect_contains(pane.body, column, row) {
            return None;
        }
        let live_pane = self.panes.get(&pane_id)?;
        if live_pane.buffer != pane.buffer_id {
            return None;
        }
        let projected = pane.rows.get(usize::from(row - pane.body.y))?;
        let document_row = projected.document_row?;
        let buffer = self.buffers.get(pane.buffer_id)?;
        let line = buffer.line_string(document_row);
        let text_x = pane.body.x.saturating_add(
            (pane.gutter_width + pane.content_indent + pane.row_prefix_width) as u16,
        );
        let screen_cell = usize::from(column.checked_sub(text_x)?);
        let segment = projected.segment;
        let (start, end, cells) = segment.map_or_else(
            || {
                let start = pane.scroll_col.min(buffer.line_len(document_row));
                let end = buffer.line_len(document_row);
                let cells =
                    crate::wrap::cells_from_column(&line, start, end, self.config.editor.tab_width);
                (start, end, cells)
            },
            |segment| {
                (
                    segment.start,
                    segment.end,
                    segment.end_cell.saturating_sub(segment.start_cell),
                )
            },
        );
        if screen_cell == cells {
            let offset = buffer.line_to_offset(document_row) + end;
            let final_segment =
                segment.is_none_or(|segment| segment.end == buffer.line_len(document_row));
            let caret_drawn = pane_id == self.active_pane
                && final_segment
                && cells < pane.text_width.saturating_sub(pane.row_prefix_width)
                && live_pane
                    .selection
                    .ranges()
                    .iter()
                    .any(|range| range.head == offset);
            return caret_drawn.then_some(offset);
        }
        if screen_cell > cells {
            return None;
        }
        let character = if segment.is_some() {
            crate::wrap::column_for_cell_from(
                &line,
                start,
                screen_cell,
                self.config.editor.tab_width,
            )
        } else {
            crate::wrap::column_for_scrolled_cell(
                &line,
                start,
                screen_cell,
                self.config.editor.tab_width,
            )
        }
        .min(end);
        Some(buffer.line_to_offset(document_row) + character)
    }

    fn forward_terminal_pointer(
        &mut self,
        event: PointerEvent,
        view: &PreparedView,
        repetitions: u16,
    ) -> bool {
        let Some(pane) = view
            .panes
            .iter()
            .find(|pane| rect_contains(pane.body, event.column, event.row))
        else {
            return false;
        };
        let Some(id) = self.terminal_of_pane(pane.pane_id) else {
            return false;
        };
        let Some(session) = self.terminals.get(id) else {
            return false;
        };
        // Review is Runyte-owned immutable history. Its wheel and selection
        // gestures must not leak into the live child behind it even if that
        // child left mouse reporting enabled.
        if session.reviewing() || !session.sgr_mouse_reporting() {
            return false;
        }
        if matches!(event.kind, PointerEventKind::Down(_)) {
            self.activate_pane_from_pointer(pane.pane_id);
        }
        let column = event.column.saturating_sub(pane.body.x);
        let row = event.row.saturating_sub(pane.body.y);
        if let Some(session) = self.terminals.get_mut(id) {
            let _ = session.send_mouse_repeated(event, column, row, repetitions);
        }
        // Once the child requests pointer reports, every event inside its body
        // belongs to it even when its bounded input queue is momentarily full.
        true
    }

    /// The selection a pointer press or drag installs, given the character
    /// cell it was anchored on and the one it is over now.
    ///
    /// A pointer names a character, not a boundary between two of them, so a
    /// drag covers the pressed cell and the cell under the pointer and
    /// everything between. That is Runyte's own inclusive range model, which
    /// is why a pointer selection carries Runyte semantics rather than the
    /// half-open ones a syntax range or a Vim operator produces. The Vim
    /// grammar writes the same span down with its leading end one past the
    /// last covered character, so it is converted rather than reshaped.
    fn pointer_selection(&self, anchor: Offset, head: Offset) -> (Selection, SelectionSemantics) {
        let selection = Selection::single(Range::new(anchor, head));
        if self.grammar.kind() == crate::command::GrammarKind::Runyte {
            return (selection, SelectionSemantics::Runyte);
        }
        // A bare Vim caret is an empty half-open range: Normal mode there
        // draws over a character without selecting it.
        if anchor == head {
            return (selection, SelectionSemantics::HalfOpen);
        }
        (
            self.vim_inclusive_to_half_open(selection, true),
            SelectionSemantics::HalfOpen,
        )
    }

    /// The character cell a Shift-click extends from.
    ///
    /// An inclusive range is anchored on a character it covers whichever way
    /// it runs, so its anchor is already a cell. A half-open one — a Vim
    /// selection, or a delimiter text object under either grammar — keeps its
    /// anchor one past the covered character when it runs backward.
    fn pointer_anchor(&self, pane: usize) -> Offset {
        let pane = &self.panes[&pane];
        if pane.selection_semantics() == SelectionSemantics::Runyte {
            return pane.selection.primary().anchor;
        }
        self.vim_half_open_to_inclusive(pane.selection.clone())
            .primary()
            .anchor
    }

    /// The character a pointer at `column`, `row` names in `pane_id`.
    ///
    /// `insert` carries the one difference between the two caret models the
    /// editor already has: an Insert caret may sit past the last character of
    /// a row, so that clicking the blank area beyond a line appends to it,
    /// while every other caret addresses a character and stops on the last
    /// one. Keyboard motion has always clamped this way; the pointer is given
    /// the same rule rather than a second one of its own.
    fn pointer_offset(
        &self,
        view: &PreparedView,
        pane_id: usize,
        column: u16,
        row: u16,
        insert: bool,
    ) -> Option<Offset> {
        let pane = view.pane(pane_id)?;
        if !pane.drawable || !rect_contains(pane.body, column, row) {
            return None;
        }
        let live_pane = self.panes.get(&pane_id)?;
        if live_pane.buffer != pane.buffer_id {
            return None;
        }
        let projected = pane.rows.get(usize::from(row - pane.body.y))?;
        // Filler belongs to no line, so clicking it is like clicking the blank
        // area past the end of a buffer: it selects nothing rather than
        // silently snapping to a neighbour the person did not aim at.
        let document_row = projected.document_row?;
        let buffer = self.buffers.get(pane.buffer_id)?;
        let line = buffer.line_string(document_row);
        // Content alignment moves the text, not the buffer, so a pointer is
        // translated back through the same indent before it names a column.
        // Anything landing in the margin names the first column of the row,
        // exactly as a click on the gutter does.
        let text_x = pane.body.x.saturating_add(
            (pane.gutter_width + pane.content_indent + pane.row_prefix_width) as u16,
        );
        let screen_cell = usize::from(column.saturating_sub(text_x));
        let start = projected
            .segment
            .map(|segment| segment.start)
            .unwrap_or(pane.scroll_col);
        let mut character = crate::wrap::column_for_cell_from(
            &line,
            start,
            screen_cell,
            self.config.editor.tab_width,
        );
        if let Some(segment) = projected.segment {
            character = character.min(segment.end);
        }
        character = character.min(self.buffers[pane.buffer_id].line_len(document_row));
        let offset = self.buffers[pane.buffer_id].line_to_offset(document_row) + character;
        Some(self.buffers[pane.buffer_id].clamp_offset(offset, insert))
    }

    fn handle_key_stroke(&mut self, mut key: KeyStroke) -> Result<()> {
        // The effective leader opens most application surfaces, so the same
        // key closes a modal overlay that already owns input. Route it through Escape
        // instead of clearing state here: settings previews, confirmations,
        // and nested action menus each retain their existing cancellation
        // semantics. Exact-text confirmations are the exception because a
        // branch or path can legitimately contain a space.
        if key.canonical_for_binding() == self.keymap.leader()
            && self.space_dismisses_input_overlay()
        {
            key = KeyStroke::new(KeyCode::Escape, Modifiers::NONE);
        }
        if self.fs_confirmation.is_some() {
            return self.handle_fs_confirmation(key);
        }
        if self.directory_reload_confirmation.is_some() {
            return self.handle_directory_reload_confirmation(key);
        }
        if self.file_reload_confirmation.is_some() {
            return self.handle_file_reload_confirmation(key);
        }
        if self.git_discard_confirmation.is_some() {
            return self.handle_git_discard_confirmation(key);
        }
        if self.git_stash_confirmation.is_some() {
            return self.handle_git_stash_confirmation(key);
        }
        if self.git_branch_switch.is_some() {
            return self.handle_branch_switch_confirmation(key);
        }
        if self.git_branch_deletion.is_some() {
            return self.handle_branch_deletion_confirmation(key);
        }
        if self.git_pull_rebase.is_some() {
            return self.handle_pull_rebase_confirmation(key);
        }
        if self.git_worktree_removal.is_some() {
            return self.handle_worktree_removal_confirmation(key);
        }
        if self.buffer_discard_confirmation.is_some() {
            return self.handle_buffer_discard_confirmation(key);
        }
        if self.context_action_menu.is_some() {
            return self.handle_context_action_key(key);
        }
        if self.program_action_menu.is_some() {
            return self.handle_program_action_key(key);
        }
        if self.path_action_menu.is_some() {
            return self.handle_path_action_key(key);
        }
        if self.path_popup.is_some() {
            return self.handle_path_popup_key(key);
        }
        if self.picker.is_some() {
            return self.handle_picker(key);
        }
        if self.list.is_some() {
            return self.handle_list_key(key);
        }
        // Live jump labels own the keyboard: every key either names a label or
        // dismisses them.
        if self.jump.is_some() {
            return self.handle_jump_labels(key);
        }
        // A hover popup is a one-shot overlay: the next keystroke dismisses it
        // and is then handled normally, so reading documentation never eats an
        // edit. Long documentation advertises Enter as the one exception: it
        // opens the complete text as a reusable read-only buffer.
        if matches!(key.code, KeyCode::Enter)
            && key.modifiers.is_empty()
            && self.mode != Mode::Command
            && self.completion.is_none()
            && self.grammar.pending_sequence().is_empty()
            && self
                .hover
                .as_ref()
                .is_some_and(|hover| hover.lines.len() > self.hover_visible_rows())
        {
            let text = self.hover.take().unwrap().lines.join("\n");
            self.open_virtual_page(
                GeneratedViewIdentity::Documentation,
                "[documentation]".to_owned(),
                &text,
                ContentAlignment::default(),
            );
            return Ok(());
        }
        self.hover = None;

        // A terminal in Insert mode owns every key except Ctrl-\\, the
        // registered effective window prefix, and the single-key pane moves
        // while those are configured on. Pending prefix suffixes continue through
        // the same declarative grammar as every other view.
        if self.mode == Mode::Insert
            && let Some(id) = self.active_terminal()
            && self.grammar.pending_sequence().is_empty()
            && !is_terminal_normal_key(key)
            && key.canonical_for_binding() != self.keymap.window_prefix()
            && !self.is_fast_pane_key(key)
        {
            return self.handle_terminal_key(id, key);
        }

        if key.code == KeyCode::Tab
            && key.modifiers.is_empty()
            && matches!(self.mode, Mode::Normal | Mode::Select)
            && self.grammar.pending_sequence().is_empty()
            && self.grammar.awaiting_character().is_none()
        {
            if self.open_context_actions() {
                return Ok(());
            }
            if !self.has_language_server() {
                self.status("No actions available for this selection");
                return Ok(());
            }
        }

        match self.mode {
            Mode::Command => self.handle_command(key),
            Mode::Insert | Mode::Replace | Mode::Normal | Mode::Select => {
                self.handle_editor_input(InputEvent::Key(key))
            }
        }
    }

    fn space_dismisses_input_overlay(&self) -> bool {
        if self.exact_confirmation_accepts_space() {
            return false;
        }
        // A nested action menu is the topmost overlay even when the picker
        // beneath it already has a query.
        if self.context_action_menu.is_some()
            || self.program_action_menu.is_some()
            || self.path_action_menu.is_some()
            || self.buffer_action_menu.is_some()
            || self.terminal_action_menu.is_some()
            || self.session_action_menu.is_some()
        {
            return true;
        }
        if let Some(picker) = self.picker.as_ref() {
            // An initial Space is the symmetric close gesture. Once a file or
            // content query exists, spaces retain their established role as
            // separators between fuzzy terms.
            return picker.query.is_empty();
        }
        if let Some(list) = self.list.as_ref() {
            // A filterable list is the same interaction as the finder above
            // and follows the same rule, so a commit search can be narrowed
            // with more than one word. A report has no filter for a space to
            // belong to, so Space still closes it outright.
            return !list.accepts_filter_input() || list.filter.is_empty();
        }
        self.has_input_overlay()
    }

    fn exact_confirmation_accepts_space(&self) -> bool {
        self.git_branch_switch.is_some()
            || self
                .git_branch_deletion
                .as_ref()
                .is_some_and(|confirmation| confirmation.typed())
            || self
                .git_worktree_removal
                .as_ref()
                .is_some_and(|confirmation| confirmation.typed())
    }

    pub(super) fn hover_visible_rows(&self) -> usize {
        enclosing_area(self.areas.values().copied())
            .map(|area| hover_content_rows(area.height))
            .unwrap_or(HOVER_PEEK_ROWS)
    }

    fn handle_text(&mut self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        if let Some(confirmation) = self.git_branch_switch.as_mut() {
            insert_confirmation_text(&mut confirmation.input, &mut confirmation.cursor, text);
            self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
            return Ok(());
        }
        if let Some(confirmation) = self.git_branch_deletion.as_mut() {
            if confirmation.typed() {
                insert_confirmation_text(&mut confirmation.input, &mut confirmation.cursor, text);
                self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
            }
            return Ok(());
        }
        if let Some(confirmation) = self.git_worktree_removal.as_mut() {
            if confirmation.typed() {
                insert_confirmation_text(&mut confirmation.input, &mut confirmation.cursor, text);
                self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
            }
            return Ok(());
        }
        if self.fs_confirmation.is_some()
            || self.directory_reload_confirmation.is_some()
            || self.buffer_discard_confirmation.is_some()
            || self.context_action_menu.is_some()
            || self.program_action_menu.is_some()
        {
            return Ok(());
        }
        if let Some(picker) = self.picker.as_mut() {
            if self.file_scanner.is_some() {
                picker.insert_query_text_unranked(text);
            } else {
                picker.insert_query_text(text);
            }
            self.restart_content_scan_if_needed();
            self.rank_resource_finder();
            if self.finder.is_some() {
                self.refresh_finder_preview();
            } else {
                self.refresh_file_picker_preview();
            }
            return Ok(());
        }
        if self.list.is_some() {
            if self.buffer_action_menu.is_none()
                && self.terminal_action_menu.is_none()
                && self
                    .list
                    .as_ref()
                    .is_some_and(ListPicker::accepts_filter_input)
            {
                let list = self.list.as_mut().unwrap();
                for character in text.chars() {
                    list.push_filter(character);
                }
                self.preview_selected_setting_value();
            }
            return Ok(());
        }
        if self.jump.take().is_some() {
            self.status("jump cancelled");
            return Ok(());
        }
        self.hover = None;

        // A paste into a terminal is the child's, bracketed if it asked for
        // that, so a multi-line paste into a shell is data rather than a
        // sequence of commands it runs as they arrive.
        if let Some(id) = self.active_terminal()
            && matches!(self.mode, Mode::Insert)
            && let Some(session) = self.terminals.get_mut(id)
        {
            if !session.send_text(text) {
                self.action_failed("terminal input queue is full or the paste exceeds 1 MiB");
            }
            return Ok(());
        }

        match self.mode {
            Mode::Insert | Mode::Replace => {
                return self.handle_editor_input(InputEvent::Text(text.to_owned()));
            }
            Mode::Command => {
                let cursor = char_to_byte(&self.command, self.command_cursor);
                self.command.insert_str(cursor, text);
                self.command_cursor += text.chars().count();
                self.command_selection = 0;
            }
            Mode::Normal | Mode::Select => {}
        }
        Ok(())
    }

    fn handle_editor_input(&mut self, mut input: InputEvent) -> Result<()> {
        loop {
            if matches!(self.mode, Mode::Insert | Mode::Replace)
                // A terminal is pane content in front of its backing buffer.
                // The terminal-specific gate in `handle_key_stroke` only lets
                // Runyte-owned keys reach this grammar, so a read-only backing
                // buffer must not reject those commands before they dispatch.
                && self.active_terminal().is_none()
                && let Some(reason) = self.active_buffer().read_only_reason()
                && matches!(
                    self.grammar.kind(),
                    crate::command::GrammarKind::Runyte | crate::command::GrammarKind::Vim
                )
            {
                self.mode = Mode::Normal;
                self.action_failed(reason);
                return Ok(());
            }
            if let InputEvent::Key(key) = &input
                && matches!(self.mode, Mode::Insert | Mode::Replace)
                && self.grammar.pending_sequence().is_empty()
                && self.handle_completion_key(*key)
            {
                return Ok(());
            }

            let context = GrammarContext::new(self.mode, self.key_binding_scope(), &self.keymap)
                .with_recording_macro(self.recording_macro.is_some());
            let GrammarOutput {
                intents,
                reprocess,
                post_action,
                resolved_binding,
            } = self.grammar.translate(input, context)?;
            let mut command_outcome = None;
            for intent in intents {
                command_outcome = self.apply_editor_intent(intent)?.or(command_outcome);
                self.reconcile_search_selection_presentation();
            }
            self.grammar.complete(post_action, self.mode);
            if let Some((sequence, target)) = resolved_binding {
                let outcome = command_outcome.unwrap_or_else(|| {
                    if self.status_error {
                        CommandOutcome::UserError(self.status.clone())
                    } else {
                        CommandOutcome::Completed
                    }
                });
                if !matches!(
                    outcome,
                    CommandOutcome::UserError(_) | CommandOutcome::Unavailable(_)
                ) {
                    self.note_tutorial_action(target.id(), &sequence.to_string());
                }
                self.report_completed_action(&sequence.to_string(), target.description(), outcome);
            }
            if let Some(mode) = self.grammar.preferred_mode()
                && matches!(self.mode, Mode::Normal | Mode::Select)
            {
                self.mode = mode;
            }

            let Some(next) = reprocess else {
                return Ok(());
            };
            if !matches!(self.mode, Mode::Insert | Mode::Replace) {
                return Ok(());
            }
            input = next;
        }
    }

    pub(crate) fn pristine_search_selection(&self, pane: usize) -> bool {
        self.search_selection.is_some_and(|presentation| {
            presentation.pane == pane
                && self
                    .panes
                    .get(&pane)
                    .is_some_and(|pane| pane.selection_revision == presentation.revision)
        })
    }

    pub(crate) fn awaiting_character_command(&self) -> Option<EditorCommand> {
        self.grammar.awaiting_character()
    }

    /// A selection motion turns search results into ordinary selections. Keep
    /// the remembered query for `n`/`N`, but stop describing or drawing the
    /// ranges as though they were still the exact matches search installed.
    fn reconcile_search_selection_presentation(&mut self) {
        let Some(presentation) = self.search_selection else {
            return;
        };
        if self.pristine_search_selection(presentation.pane) {
            return;
        }
        self.search_selection = None;
        if self.mode == Mode::Select {
            let count = self.active().selection.len();
            self.status(format!(
                "{count} selection{}",
                if count == 1 { "" } else { "s" }
            ));
        }
    }

    fn apply_editor_intent(&mut self, intent: EditorIntent) -> Result<Option<CommandOutcome>> {
        if let EditorIntent::Range(range) = &intent {
            let repetitions = match range {
                RangeIntent::SelectLine { count, .. }
                | RangeIntent::VimMotion { count, .. }
                | RangeIntent::VimVisualLine { count }
                | RangeIntent::VimRepeatSearch { count, .. }
                | RangeIntent::VimSearchWord { count, .. } => count.get(),
                RangeIntent::VimOperator { target, .. } => match target {
                    VimRangeTarget::Characters { count }
                    | VimRangeTarget::Motion { count, .. }
                    | VimRangeTarget::Line { count, .. } => count.get(),
                    VimRangeTarget::Syntax { .. } => 1,
                },
                RangeIntent::VimVisualOperator { .. }
                | RangeIntent::VimSyntaxSelection { .. }
                | RangeIntent::VimReplace { .. } => 1,
            };
            if !self.reserve_macro_replay_range_work(repetitions) {
                return Ok(None);
            }
        }
        match intent {
            EditorIntent::Command(invocation) => {
                return Ok(Some(self.execute(invocation)?));
            }
            EditorIntent::InsertText(text) => {
                if let Some(reason) = self.active_buffer().read_only_reason() {
                    self.action_failed(reason);
                    return Ok(None);
                }
                if self.mode == Mode::Replace {
                    self.replace_mode_text(&text);
                } else {
                    self.insert_text(&text);
                }
                for character in text.chars() {
                    self.after_insert(character);
                }
            }
            EditorIntent::Range(RangeIntent::SelectLine { direction, count }) => {
                let command = match direction {
                    LineDirection::Down => EditorCommand::SelectLine,
                    LineDirection::Up => EditorCommand::SelectLineUp,
                };
                for _ in 0..count.get() {
                    self.execute_editor_command(command)?;
                }
            }
            EditorIntent::Range(RangeIntent::VimMotion {
                motion,
                count,
                explicit_count,
                extend,
            }) => self.apply_vim_motion(motion, count.get(), explicit_count, extend),
            EditorIntent::Range(RangeIntent::VimOperator {
                operator,
                target,
                register,
            }) => {
                self.apply_vim_operator(operator, target, register)?;
            }
            EditorIntent::Range(RangeIntent::VimVisualOperator { operator, register }) => {
                self.apply_vim_visual_operator(operator, register)?;
            }
            EditorIntent::Range(RangeIntent::VimVisualLine { count }) => {
                self.enter_vim_visual_line(count.get());
            }
            EditorIntent::Range(RangeIntent::VimSyntaxSelection { object, around }) => {
                self.select_vim_syntax(object, around)?;
            }
            EditorIntent::Range(RangeIntent::VimReplace { character }) => {
                self.select_vim_characters(1);
                if self
                    .active()
                    .selection
                    .ranges()
                    .iter()
                    .any(|range| !range.is_empty())
                {
                    self.replace_with_char(character);
                    self.active_mut()
                        .mark_selection_semantics(SelectionSemantics::HalfOpen);
                }
            }
            EditorIntent::Range(RangeIntent::VimRepeatSearch { previous, count }) => {
                self.repeat_search_count(previous, count.get());
            }
            EditorIntent::Range(RangeIntent::VimSearchWord { previous, count }) => {
                self.search_selection_direction(previous, count.get());
            }
            EditorIntent::Notice(notice) => match notice {
                GrammarNotice::PendingSequence(sequence) => self.status(format!("{sequence} …")),
                GrammarNotice::Count(count) => self.status(format!("{count} …")),
                GrammarNotice::SequenceCancelled => self.status("key sequence cancelled"),
                GrammarNotice::NoBinding(sequence) => {
                    self.error_unretained(format!("No binding: {sequence}"));
                }
                GrammarNotice::AwaitingCharacter(command) => {
                    self.status(command.metadata().description);
                }
                GrammarNotice::CharacterInputCancelled => {
                    self.status("character input cancelled");
                }
                GrammarNotice::ExpectedCharacter => self.action_failed("expected a character"),
                GrammarNotice::InvalidRegister {
                    register,
                    macros_only,
                } => self.action_failed(if macros_only {
                    format!("macro register must be a-z, not '{register}'")
                } else {
                    format!("register must be a-z, A-Z, quote, or underscore, not '{register}'")
                }),
                GrammarNotice::CountNotSupported(target) => {
                    self.action_failed(format!(
                        "{} does not support a count",
                        target.description()
                    ));
                }
                GrammarNotice::UnavailableBinding {
                    target,
                    availability,
                } => {
                    let reason = availability
                        .reason()
                        .expect("unavailable grammar notice has a reason");
                    let state = match availability {
                        crate::keymap::BindingAvailability::Planned(_) => "planned",
                        crate::keymap::BindingAvailability::Unsupported(_) => "unsupported",
                        crate::keymap::BindingAvailability::Implemented => {
                            unreachable!("implemented binding cannot be unavailable")
                        }
                    };
                    self.action_failed(format!("{} is {state}: {reason}", target.description()));
                }
            },
        }
        Ok(None)
    }

    pub(super) fn vim_inclusive_to_half_open(
        &self,
        selection: Selection,
        inclusive: bool,
    ) -> Selection {
        let end = self.active_buffer().len_chars();
        selection.transform(|range| {
            if range.is_empty() {
                if inclusive {
                    let row = self.active_buffer().offset_to_row(range.head);
                    let row_end = self.active_buffer().line_to_offset(row)
                        + self.active_buffer().line_len(row);
                    return Range::new(range.head, (range.head + 1).min(row_end));
                }
                return range;
            }
            if range.anchor <= range.head {
                Range::new(
                    range.anchor,
                    range.head.saturating_add(usize::from(inclusive)).min(end),
                )
            } else {
                Range::new(
                    range.anchor.saturating_add(usize::from(inclusive)).min(end),
                    range.head,
                )
            }
        })
    }

    pub(super) fn vim_half_open_to_inclusive(&self, selection: Selection) -> Selection {
        selection.transform(|range| {
            if range.is_empty() {
                range
            } else if range.anchor < range.head {
                Range::new(range.anchor, range.head - 1)
            } else {
                Range::new(range.anchor - 1, range.head)
            }
        })
    }

    fn vim_motion_kind(motion: VimMotion) -> Option<Motion> {
        match motion {
            VimMotion::Left => Some(Motion::Left),
            VimMotion::Right => Some(Motion::Right),
            VimMotion::Up => Some(Motion::Up),
            VimMotion::Down => Some(Motion::Down),
            VimMotion::WordForward => Some(Motion::WordForward),
            VimMotion::WordBackward => Some(Motion::WordBack),
            VimMotion::WordEnd => Some(Motion::WordEnd),
            VimMotion::WordEndBackward => Some(Motion::WordEndBack),
            VimMotion::LongWordForward => Some(Motion::LongWordForward),
            VimMotion::LongWordBackward => Some(Motion::LongWordBack),
            VimMotion::LongWordEnd => Some(Motion::LongWordEnd),
            VimMotion::LongWordEndBackward => Some(Motion::LongWordEndBack),
            VimMotion::LineStart => Some(Motion::LineStart),
            VimMotion::FirstNonWhitespace => Some(Motion::FirstNonWhitespace),
            VimMotion::LastNonWhitespace => Some(Motion::LastNonWhitespace),
            VimMotion::LineEnd => Some(Motion::LineEnd),
            VimMotion::FileStart => Some(Motion::FileStart),
            VimMotion::FileEnd => Some(Motion::FileEnd),
            VimMotion::PageUp => Some(Motion::PageUp),
            VimMotion::PageDown => Some(Motion::PageDown),
            VimMotion::HalfPageUp => Some(Motion::HalfPageUp),
            VimMotion::HalfPageDown => Some(Motion::HalfPageDown),
            VimMotion::WindowTop => Some(Motion::WindowTop),
            VimMotion::WindowCenter => Some(Motion::WindowCenter),
            VimMotion::WindowBottom => Some(Motion::WindowBottom),
            VimMotion::FindNext(_)
            | VimMotion::FindPrevious(_)
            | VimMotion::TillNext(_)
            | VimMotion::TillPrevious(_)
            | VimMotion::MatchBracket => None,
        }
    }

    fn apply_vim_motion(
        &mut self,
        motion: VimMotion,
        count: usize,
        explicit_count: bool,
        extend: bool,
    ) {
        if extend && self.active().selection_semantics() == SelectionSemantics::VimLinewise {
            self.move_vim_linewise_selection(motion, count, explicit_count);
            return;
        }
        if !extend {
            self.collapse_vim_normal_selection();
        }
        self.move_vim_selection(motion, count, explicit_count, extend, extend);
    }

    fn collapse_vim_normal_selection(&mut self) {
        if self
            .active()
            .selection
            .ranges()
            .iter()
            .any(|range| !range.is_empty())
        {
            let selection = if matches!(
                self.active().selection_semantics(),
                SelectionSemantics::HalfOpen | SelectionSemantics::VimLinewise
            ) {
                self.vim_half_open_to_inclusive(self.active().selection.clone())
            } else {
                self.active().selection.clone()
            };
            self.active_mut().replace_selection(selection.collapse());
        }
        self.active_mut()
            .mark_selection_semantics(SelectionSemantics::HalfOpen);
    }

    fn select_vim_characters(&mut self, count: usize) {
        self.collapse_vim_normal_selection();
        let buffer = self.active_buffer();
        let selection = self.active().selection.transform(|range| {
            let row = buffer.offset_to_row(range.head);
            let row_end = buffer.line_to_offset(row) + buffer.line_len(row);
            Range::new(range.head, range.head.saturating_add(count).min(row_end))
        });
        self.active_mut().replace_selection(selection);
        self.active_mut()
            .mark_selection_semantics(SelectionSemantics::HalfOpen);
    }

    fn move_vim_selection(
        &mut self,
        motion: VimMotion,
        count: usize,
        explicit_count: bool,
        extend: bool,
        inclusive: bool,
    ) {
        if extend
            && matches!(
                self.active().selection_semantics(),
                SelectionSemantics::HalfOpen
            )
        {
            let selection = self.vim_half_open_to_inclusive(self.active().selection.clone());
            self.active_mut().replace_selection(selection);
        }

        let selection_before_motion = self.active().selection.clone();

        if matches!(motion, VimMotion::FileStart | VimMotion::FileEnd) && explicit_count {
            let buffer = self.active_buffer();
            let row = count.saturating_sub(1).min(buffer.last_row());
            let target = buffer.clamp_offset(buffer.line_to_offset(row), false);
            let selection = if extend {
                self.active()
                    .selection
                    .transform(|range| range.extend_to(target))
            } else {
                Selection::point(target)
            };
            self.active_mut().replace_selection(selection);
        } else if matches!(motion, VimMotion::Left | VimMotion::Right) {
            for _ in 0..count {
                let buffer = self.active_buffer();
                let selection = self.active().selection.transform(|range| {
                    let position = buffer.position_of(range.head);
                    let head = if motion == VimMotion::Left {
                        if position.col > 0 {
                            range.head - 1
                        } else {
                            range.head
                        }
                    } else if position.col + 1 < buffer.line_len(position.row) {
                        range.head + 1
                    } else {
                        range.head
                    };
                    if extend {
                        range.extend_to(head)
                    } else {
                        Range::point(head)
                    }
                });
                self.active_mut().replace_selection(selection);
            }
        } else if motion == VimMotion::LineEnd && count > 1 {
            let buffer = self.active_buffer();
            let selection = self.active().selection.transform(|range| {
                let row = buffer
                    .offset_to_row(range.head)
                    .saturating_add(count - 1)
                    .min(buffer.last_row());
                let start = buffer.line_to_offset(row);
                let end = start + buffer.line_len(row);
                let head = buffer.clamp_offset(end, false);
                if extend {
                    range.extend_to(head)
                } else {
                    Range::point(head)
                }
            });
            self.active_mut().replace_selection(selection);
        } else if matches!(
            motion,
            VimMotion::WindowTop | VimMotion::WindowCenter | VimMotion::WindowBottom
        ) {
            let shared = Self::vim_motion_kind(motion).expect("window motion has shared form");
            self.motion_with_extension(shared, extend);
            let step = match motion {
                VimMotion::WindowTop => Some(Motion::Down),
                VimMotion::WindowBottom => Some(Motion::Up),
                VimMotion::WindowCenter => None,
                _ => unreachable!(),
            };
            if let Some(step) = step {
                for _ in 1..count {
                    self.motion_with_extension(step, extend);
                }
            }
        } else if let Some(motion) = Self::vim_motion_kind(motion) {
            for _ in 0..count {
                self.motion_with_extension(motion, extend);
            }
        } else {
            let prior_mode = self.mode;
            self.mode = if extend { Mode::Select } else { Mode::Normal };
            for index in 0..count {
                match motion {
                    VimMotion::FindNext(character) => self.find_character(character, true, false),
                    VimMotion::FindPrevious(character) => {
                        self.find_character(character, false, false)
                    }
                    VimMotion::TillNext(character) => {
                        self.find_character(character, true, index + 1 == count)
                    }
                    VimMotion::TillPrevious(character) => {
                        self.find_character(character, false, index + 1 == count)
                    }
                    VimMotion::MatchBracket => self.match_bracket(),
                    _ => unreachable!("ordinary Vim motions have a shared Motion"),
                }
            }
            self.mode = prior_mode;
        }

        let fallible_target = matches!(
            motion,
            VimMotion::FindNext(_)
                | VimMotion::FindPrevious(_)
                | VimMotion::TillNext(_)
                | VimMotion::TillPrevious(_)
                | VimMotion::MatchBracket
        );
        if extend && (!fallible_target || self.active().selection != selection_before_motion) {
            let selection =
                self.vim_inclusive_to_half_open(self.active().selection.clone(), inclusive);
            self.active_mut().replace_selection(selection);
        }
        self.active_mut()
            .mark_selection_semantics(SelectionSemantics::HalfOpen);
    }

    fn select_vim_lines(&mut self, direction: LineDirection, count: usize) {
        let buffer = self.active_buffer();
        let ranges = self
            .active()
            .selection
            .ranges()
            .iter()
            .map(|range| {
                let row = buffer.offset_to_row(range.head);
                let other = match direction {
                    LineDirection::Down => row
                        .saturating_add(count.saturating_sub(1))
                        .min(buffer.last_row()),
                    LineDirection::Up => row.saturating_sub(count.saturating_sub(1)),
                };
                let first = row.min(other);
                let last = row.max(other);
                let from = buffer.line_to_offset(first);
                let to = if last < buffer.last_row() {
                    buffer.line_to_offset(last + 1)
                } else {
                    buffer.len_chars()
                };
                Range::new(from, to)
            })
            .collect::<Vec<_>>();
        let primary = self.active().selection.primary_index();
        self.active_mut()
            .replace_selection(Selection::new(ranges, primary));
        self.active_mut()
            .mark_selection_semantics(SelectionSemantics::VimLinewise);
    }

    fn vim_linewise_range(buffer: &Buffer, anchor_row: usize, head_row: usize) -> Range {
        let row_end = |row: usize| {
            if row < buffer.last_row() {
                buffer.line_to_offset(row + 1)
            } else {
                buffer.len_chars()
            }
        };
        if head_row >= anchor_row {
            Range::new(buffer.line_to_offset(anchor_row), row_end(head_row))
        } else {
            Range::new(row_end(anchor_row), buffer.line_to_offset(head_row))
        }
    }

    fn vim_linewise_rows(buffer: &Buffer, range: Range) -> (usize, usize) {
        if range.anchor <= range.head {
            let anchor_row = buffer.offset_to_row(range.anchor);
            let head = range.head.saturating_sub(1).min(buffer.len_chars());
            (anchor_row, buffer.offset_to_row(head))
        } else {
            let anchor = range.anchor.saturating_sub(1).min(buffer.len_chars());
            (
                buffer.offset_to_row(anchor),
                buffer.offset_to_row(range.head),
            )
        }
    }

    fn enter_vim_visual_line(&mut self, count: usize) {
        let mut selection = self.active().selection.clone();
        if self.mode == Mode::Select
            && self.active().selection_semantics() == SelectionSemantics::HalfOpen
        {
            selection = self.vim_half_open_to_inclusive(selection);
        }
        let buffer = self.active_buffer();
        let ranges = selection
            .ranges()
            .iter()
            .map(|range| {
                let anchor_row = buffer.offset_to_row(range.anchor);
                let current_head_row = buffer.offset_to_row(range.head);
                let forward = range.anchor <= range.head;
                let head_row = if forward {
                    current_head_row
                        .saturating_add(count.saturating_sub(1))
                        .min(buffer.last_row())
                } else {
                    current_head_row.saturating_sub(count.saturating_sub(1))
                };
                Self::vim_linewise_range(buffer, anchor_row, head_row)
            })
            .collect::<Vec<_>>();
        let primary = selection.primary_index();
        self.active_mut()
            .replace_selection(Selection::new(ranges, primary));
        self.active_mut()
            .mark_selection_semantics(SelectionSemantics::VimLinewise);
        self.mode = Mode::Select;
    }

    fn move_vim_linewise_selection(
        &mut self,
        motion: VimMotion,
        count: usize,
        explicit_count: bool,
    ) {
        let buffer = self.active_buffer();
        let anchors_and_points = self
            .active()
            .selection
            .ranges()
            .iter()
            .map(|range| {
                let (anchor_row, head_row) = Self::vim_linewise_rows(buffer, *range);
                (anchor_row, Range::point(buffer.line_to_offset(head_row)))
            })
            .collect::<Vec<_>>();
        let primary = self.active().selection.primary_index();
        self.active_mut().replace_selection(Selection::new(
            anchors_and_points.iter().map(|(_, point)| *point).collect(),
            primary,
        ));
        self.move_vim_selection(motion, count, explicit_count, false, false);

        let buffer = self.active_buffer();
        let ranges = anchors_and_points
            .iter()
            .zip(self.active().selection.ranges())
            .map(|((anchor_row, _), target)| {
                let head_row = buffer.offset_to_row(target.head);
                Self::vim_linewise_range(buffer, *anchor_row, head_row)
            })
            .collect::<Vec<_>>();
        self.active_mut()
            .replace_selection(Selection::new(ranges, primary));
        self.active_mut()
            .mark_selection_semantics(SelectionSemantics::VimLinewise);
    }

    fn select_vim_syntax(&mut self, object: VimTextObject, around: bool) -> Result<bool> {
        let object = match object {
            VimTextObject::Function => SyntaxObject::Function,
            VimTextObject::Class => SyntaxObject::Class,
            VimTextObject::Parameter => SyntaxObject::Parameter,
        };
        let part = if around {
            SyntaxObjectPart::Around
        } else {
            SyntaxObjectPart::Inside
        };
        let revision = self.active().selection_revision;
        self.select_syntax_object(object, part)?;
        let produced = self.active().selection_revision != revision;
        if produced
            && self
                .active()
                .selection
                .ranges()
                .iter()
                .any(|range| !range.is_empty())
        {
            if self.active().selection_semantics() == SelectionSemantics::Runyte {
                let selection =
                    self.vim_inclusive_to_half_open(self.active().selection.clone(), true);
                self.active_mut().replace_selection(selection);
            }
            self.active_mut()
                .mark_selection_semantics(SelectionSemantics::HalfOpen);
        }
        Ok(produced)
    }

    fn apply_vim_operator(
        &mut self,
        operator: VimOperator,
        target: VimRangeTarget,
        register: Option<char>,
    ) -> Result<()> {
        if let Some(reason) = self.active_buffer().read_only_reason()
            && operator != VimOperator::Yank
        {
            self.action_failed(reason);
            return Ok(());
        }
        self.collapse_vim_normal_selection();
        match target {
            VimRangeTarget::Characters { count } => {
                self.select_vim_characters(count.get());
            }
            VimRangeTarget::Line { direction, count } => {
                self.select_vim_lines(direction, count.get());
            }
            VimRangeTarget::Syntax { object, around } => {
                if !self.select_vim_syntax(object, around)? {
                    return Ok(());
                }
            }
            VimRangeTarget::Motion { motion, count } => {
                let motion = if operator == VimOperator::Change
                    && motion == VimMotion::WordForward
                    && !self
                        .active_buffer()
                        .char_at(self.active().selection.primary().head)
                        .is_some_and(char::is_whitespace)
                {
                    VimMotion::WordEnd
                } else {
                    motion
                };
                if matches!(motion, VimMotion::FileStart | VimMotion::FileEnd) {
                    let current = self
                        .active_buffer()
                        .offset_to_row(self.active().selection.primary().head);
                    let target = if count.get() > 1 {
                        count
                            .get()
                            .saturating_sub(1)
                            .min(self.active_buffer().last_row())
                    } else if motion == VimMotion::FileStart {
                        0
                    } else {
                        self.active_buffer().last_row()
                    };
                    self.select_vim_lines(
                        if target < current {
                            LineDirection::Up
                        } else {
                            LineDirection::Down
                        },
                        current.abs_diff(target) + 1,
                    );
                } else {
                    let inclusive = matches!(
                        motion,
                        VimMotion::WordEnd
                            | VimMotion::WordEndBackward
                            | VimMotion::LongWordEnd
                            | VimMotion::LongWordEndBackward
                            | VimMotion::LineEnd
                            | VimMotion::LastNonWhitespace
                            | VimMotion::FindNext(_)
                            | VimMotion::FindPrevious(_)
                            | VimMotion::TillNext(_)
                            | VimMotion::TillPrevious(_)
                            | VimMotion::MatchBracket
                    );
                    self.move_vim_selection(motion, count.get(), false, true, inclusive);
                    if motion == VimMotion::Right
                        && self.active().selection.ranges().iter().all(Range::is_empty)
                    {
                        let buffer = self.active_buffer();
                        let selection = self.active().selection.transform(|range| {
                            let row = buffer.offset_to_row(range.head);
                            let end = buffer.line_to_offset(row) + buffer.line_len(row);
                            Range::new(range.head, (range.head + 1).min(end))
                        });
                        self.active_mut().replace_selection(selection);
                    }
                }
            }
        }
        if self.active().selection.ranges().iter().all(Range::is_empty) {
            self.mode = Mode::Normal;
            return Ok(());
        }
        if let Some(register) = register {
            self.select_register(register);
        }
        self.finish_vim_operator(operator);
        Ok(())
    }

    fn apply_vim_visual_operator(
        &mut self,
        operator: VimOperator,
        register: Option<char>,
    ) -> Result<()> {
        if let Some(reason) = self.active_buffer().read_only_reason()
            && operator != VimOperator::Yank
        {
            self.action_failed(reason);
            return Ok(());
        }
        if self.active().selection.ranges().iter().all(Range::is_empty) {
            let buffer = self.active_buffer();
            let selection = self.active().selection.transform(|range| {
                let row = buffer.offset_to_row(range.head);
                let end = buffer.line_to_offset(row) + buffer.line_len(row);
                Range::new(range.head, (range.head + 1).min(end))
            });
            self.active_mut().replace_selection(selection);
        }
        if self.active().selection_semantics() != SelectionSemantics::VimLinewise {
            if self.active().selection_semantics() == SelectionSemantics::Runyte {
                let selection =
                    self.vim_inclusive_to_half_open(self.active().selection.clone(), true);
                self.active_mut().replace_selection(selection);
            }
            self.active_mut()
                .mark_selection_semantics(SelectionSemantics::HalfOpen);
        }
        if let Some(register) = register {
            self.select_register(register);
        }
        self.finish_vim_operator(operator);
        Ok(())
    }

    fn finish_vim_operator(&mut self, operator: VimOperator) {
        match operator {
            VimOperator::Delete => self.delete_selection_or_char(false, false),
            VimOperator::Change
                if self.active().selection_semantics() == SelectionSemantics::VimLinewise =>
            {
                self.change_vim_lines()
            }
            VimOperator::Change => self.delete_selection_or_char(true, false),
            VimOperator::Yank => {
                self.yank(false);
                let selection = self
                    .active()
                    .selection
                    .transform(|range| Range::point(range.from()));
                self.active_mut().replace_selection(selection);
                self.mode = Mode::Normal;
            }
            VimOperator::Indent | VimOperator::Unindent => {
                self.indent(operator == VimOperator::Unindent);
                let selection = self
                    .active()
                    .selection
                    .transform(|range| Range::point(range.from()));
                self.active_mut().replace_selection(selection);
                self.mode = Mode::Normal;
            }
        }
        self.active_mut()
            .mark_selection_semantics(SelectionSemantics::HalfOpen);
    }

    fn change_vim_lines(&mut self) {
        let buffer_id = self.active().buffer;
        let register = self.yank_value(true);
        self.write_selected_register(register);
        self.buffers[buffer_id].begin_undo_group();
        let buffer = self.active_buffer();
        let spans = self.operative_spans();
        let primary = self.active().selection.primary_index();
        let insertion_points = spans
            .iter()
            .map(|(from, _)| Range::point(*from))
            .collect::<Vec<_>>();
        let changes = spans
            .into_iter()
            .filter_map(|(from, to)| {
                let delete_to = if to > from && buffer.char_at(to - 1) == Some('\n') {
                    if to - from >= 2 && buffer.char_at(to - 2) == Some('\r') {
                        to - 2
                    } else {
                        to - 1
                    }
                } else {
                    to
                };
                (from < delete_to).then(|| Change::new(from, delete_to, ""))
            })
            .collect();
        self.edit(Transaction::new(changes));
        self.active_mut()
            .replace_selection(Selection::new(insertion_points, primary));
        self.mode = Mode::Insert;
        self.normalize_buffer(buffer_id);
    }

    fn handle_fs_confirmation(&mut self, key: KeyStroke) -> Result<()> {
        let control = key.modifiers.contains(Modifiers::CONTROL);
        match (key.code, key.modifiers) {
            (KeyCode::Escape, _) => {
                self.fs_confirmation = None;
                self.status("filesystem plan cancelled");
            }
            (KeyCode::Char('c'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.fs_confirmation = None;
                self.status("filesystem plan cancelled");
            }
            (KeyCode::Enter, _) => self.apply_fs_confirmation(DeletionMode::Trash),
            (KeyCode::Char('P'), modifiers) if modifiers.is_empty() => {
                self.apply_fs_confirmation(DeletionMode::Permanent);
            }
            (KeyCode::Down, _) => {
                let confirmation = self.fs_confirmation.as_mut().unwrap();
                let last = confirmation.plan.operations().len().saturating_sub(1);
                confirmation.selected = confirmation.selected.saturating_add(1).min(last);
            }
            (KeyCode::Char('n'), _) if control => {
                let confirmation = self.fs_confirmation.as_mut().unwrap();
                let last = confirmation.plan.operations().len().saturating_sub(1);
                confirmation.selected = confirmation.selected.saturating_add(1).min(last);
            }
            (KeyCode::Up, _) => {
                let confirmation = self.fs_confirmation.as_mut().unwrap();
                confirmation.selected = confirmation.selected.saturating_sub(1);
            }
            (KeyCode::Char('p'), _) if control => {
                let confirmation = self.fs_confirmation.as_mut().unwrap();
                confirmation.selected = confirmation.selected.saturating_sub(1);
            }
            (KeyCode::PageDown, _) => {
                let confirmation = self.fs_confirmation.as_mut().unwrap();
                let last = confirmation.plan.operations().len().saturating_sub(1);
                confirmation.selected = confirmation.selected.saturating_add(10).min(last);
            }
            (KeyCode::Char('d'), _) if control => {
                let confirmation = self.fs_confirmation.as_mut().unwrap();
                let last = confirmation.plan.operations().len().saturating_sub(1);
                confirmation.selected = confirmation.selected.saturating_add(10).min(last);
            }
            (KeyCode::PageUp, _) => {
                let confirmation = self.fs_confirmation.as_mut().unwrap();
                confirmation.selected = confirmation.selected.saturating_sub(10);
            }
            (KeyCode::Char('u'), _) if control => {
                let confirmation = self.fs_confirmation.as_mut().unwrap();
                confirmation.selected = confirmation.selected.saturating_sub(10);
            }
            (KeyCode::Home, _) => self.fs_confirmation.as_mut().unwrap().selected = 0,
            (KeyCode::End, _) => {
                let confirmation = self.fs_confirmation.as_mut().unwrap();
                confirmation.selected = confirmation.plan.operations().len().saturating_sub(1);
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_directory_reload_confirmation(&mut self, key: KeyStroke) -> Result<()> {
        match (key.code, key.modifiers) {
            (KeyCode::Escape, _) => {
                self.directory_reload_confirmation = None;
                self.status("directory refresh cancelled");
            }
            (KeyCode::Char('c'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.directory_reload_confirmation = None;
                self.status("directory refresh cancelled");
            }
            (KeyCode::Enter, _) => {
                let Some(confirmation) = self.directory_reload_confirmation.take() else {
                    return Ok(());
                };
                match confirmation.destination {
                    // Discarding is what the navigation was waiting on, so it
                    // resumes through the ordinary path with a clean buffer.
                    Some(path) => {
                        self.buffers[confirmation.buffer].mark_saved();
                        self.open_file(path)?;
                        if let Some(entry) = confirmation.focus_entry {
                            self.focus_directory_entry(&entry);
                        }
                        self.enter_normal_mode();
                    }
                    None => self.reload_directory_buffer(confirmation.buffer)?,
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_git_discard_confirmation(&mut self, key: KeyStroke) -> Result<()> {
        match (key.code, key.modifiers) {
            (KeyCode::Escape, _) => {
                self.git_discard_confirmation = None;
                self.status("discard cancelled; nothing was changed");
            }
            (KeyCode::Char('c'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.git_discard_confirmation = None;
                self.status("discard cancelled; nothing was changed");
            }
            (KeyCode::Enter, _) => {
                let Some(confirmation) = self.git_discard_confirmation.take() else {
                    return Ok(());
                };
                self.apply_git_discard(confirmation.paths)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_branch_deletion_confirmation(&mut self, key: KeyStroke) -> Result<()> {
        match (key.code, key.modifiers) {
            (KeyCode::Escape, _) => {
                self.git_branch_deletion = None;
                self.status("delete cancelled; the branch is still there");
            }
            (KeyCode::Char('c'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.git_branch_deletion = None;
                self.status("delete cancelled; the branch is still there");
            }
            (KeyCode::Enter, _) => {
                let valid = self
                    .git_branch_deletion
                    .as_ref()
                    .is_some_and(|confirmation| {
                        !confirmation.typed() || confirmation.input == confirmation.plan.branch
                    });
                if !valid {
                    self.action_failed("type the exact branch name before deleting it");
                    return Ok(());
                }
                let Some(confirmation) = self.git_branch_deletion.take() else {
                    return Ok(());
                };
                let authorization = if confirmation.typed() {
                    DeletionAuthorization::Typed
                } else {
                    DeletionAuthorization::Enter
                };
                match confirmation.cascade {
                    Some(cascade) => {
                        self.apply_branch_cascade(confirmation.plan, cascade, authorization)
                    }
                    None => self.apply_branch_deletion(confirmation.plan, authorization),
                }
            }
            _ => {
                if let Some(confirmation) = self.git_branch_deletion.as_mut()
                    && confirmation.typed()
                {
                    edit_confirmation_text(&mut confirmation.input, &mut confirmation.cursor, key);
                    self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
                }
            }
        }
        Ok(())
    }

    fn handle_branch_switch_confirmation(&mut self, key: KeyStroke) -> Result<()> {
        match (key.code, key.modifiers) {
            (KeyCode::Escape, _) => {
                if let Some(confirmation) = self.git_branch_switch.take() {
                    self.status(confirmation.action.cancelled_message());
                }
            }
            (KeyCode::Char('c'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                if let Some(confirmation) = self.git_branch_switch.take() {
                    self.status(confirmation.action.cancelled_message());
                }
            }
            (KeyCode::Enter, _) => {
                let valid = self
                    .git_branch_switch
                    .as_ref()
                    .is_some_and(|confirmation| confirmation.input == confirmation.action.branch());
                if !valid {
                    self.action_failed("type the exact branch name before switching branches");
                    return Ok(());
                }
                let Some(confirmation) = self.git_branch_switch.take() else {
                    return Ok(());
                };
                self.apply_branch_switch(confirmation.repository, confirmation.action);
            }
            _ => {
                if let Some(confirmation) = self.git_branch_switch.as_mut() {
                    edit_confirmation_text(&mut confirmation.input, &mut confirmation.cursor, key);
                    self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
                }
            }
        }
        Ok(())
    }

    fn handle_pull_rebase_confirmation(&mut self, key: KeyStroke) -> Result<()> {
        let cancelled = match (key.code, key.modifiers) {
            (KeyCode::Escape, _) => true,
            (KeyCode::Char('c'), modifiers) if modifiers.contains(Modifiers::CONTROL) => true,
            (KeyCode::Enter, _) => {
                if self.git_pull_rebase.take().is_some() {
                    self.rebase_onto_upstream();
                }
                return Ok(());
            }
            _ => return Ok(()),
        };
        if cancelled && let Some(confirmation) = self.git_pull_rebase.take() {
            // The fetch behind the refused pull already ran, so the branch list
            // now shows the drift even though nothing was replayed.
            self.status(format!(
                "{} was left as it is, still {} commit(s) behind {}",
                confirmation.branch, confirmation.behind, confirmation.upstream
            ));
        }
        Ok(())
    }

    fn handle_worktree_removal_confirmation(&mut self, key: KeyStroke) -> Result<()> {
        match (key.code, key.modifiers) {
            (KeyCode::Escape, _) => {
                let Some(confirmation) = self.git_worktree_removal.take() else {
                    return Ok(());
                };
                self.status(format!(
                    "remove cancelled; worktree {} was kept and no branch was deleted",
                    crate::git::display_path(&confirmation.plan.path)
                ));
            }
            (KeyCode::Char('c'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                let Some(confirmation) = self.git_worktree_removal.take() else {
                    return Ok(());
                };
                self.status(format!(
                    "remove cancelled; worktree {} was kept and no branch was deleted",
                    crate::git::display_path(&confirmation.plan.path)
                ));
            }
            (KeyCode::Enter, _) => {
                let valid = self
                    .git_worktree_removal
                    .as_ref()
                    .is_some_and(|confirmation| {
                        !confirmation.typed() || confirmation.input == confirmation.expected()
                    });
                if !valid {
                    self.action_failed(
                        "type the exact branch name or worktree path before removing it",
                    );
                    return Ok(());
                }
                let Some(confirmation) = self.git_worktree_removal.take() else {
                    return Ok(());
                };
                let authorization = if confirmation.typed() {
                    DeletionAuthorization::Typed
                } else {
                    DeletionAuthorization::Enter
                };
                self.apply_worktree_removal(confirmation.plan, authorization);
            }
            _ => {
                if let Some(confirmation) = self.git_worktree_removal.as_mut()
                    && confirmation.typed()
                {
                    edit_confirmation_text(&mut confirmation.input, &mut confirmation.cursor, key);
                    self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
                }
            }
        }
        Ok(())
    }

    fn handle_buffer_discard_confirmation(&mut self, key: KeyStroke) -> Result<()> {
        match (key.code, key.modifiers) {
            (KeyCode::Escape, _) => {
                self.buffer_discard_confirmation = None;
                self.status("discard cancelled");
            }
            (KeyCode::Char('c'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.buffer_discard_confirmation = None;
                self.status("discard cancelled");
            }
            (KeyCode::Enter, _) => {
                let Some(buffer) = self.buffer_discard_confirmation.take() else {
                    return Ok(());
                };
                self.discard_buffer_changes(buffer)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_file_reload_confirmation(&mut self, key: KeyStroke) -> Result<()> {
        match (key.code, key.modifiers) {
            (KeyCode::Escape, _) => {
                self.file_reload_confirmation = None;
                self.status("reload cancelled");
            }
            (KeyCode::Char('c'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.file_reload_confirmation = None;
                self.status("reload cancelled");
            }
            (KeyCode::Enter, _) => {
                let Some(mut confirmation) = self.file_reload_confirmation.take() else {
                    return Ok(());
                };
                if confirmation.buffer >= self.buffers.len()
                    || self.closed_buffers.contains(&confirmation.buffer)
                {
                    self.action_failed("the file buffer was closed; reload cancelled");
                    return Ok(());
                }
                let Some(current) =
                    self.buffers[confirmation.buffer].observe_now(confirmation.buffer)
                else {
                    self.action_failed(
                        "the buffer is no longer an ordinary file; reload cancelled",
                    );
                    return Ok(());
                };
                if current.generation != confirmation.generation
                    || current.path != confirmation.path
                {
                    self.action_failed("the file baseline changed; review reload again");
                    return Ok(());
                }
                if current.observation != confirmation.observation {
                    self.apply_file_observation(current.clone());
                    if matches!(current.observation, FileObservation::Text { .. })
                        && self.buffers[confirmation.buffer]
                            .external_file_status()
                            .is_stale()
                    {
                        confirmation.observation = current.observation;
                        self.file_reload_confirmation = Some(confirmation);
                    }
                    self.action_failed("the file changed again on disk; review reload again");
                    return Ok(());
                }
                self.install_file_reload(confirmation.buffer, &confirmation.observation)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn apply_fs_confirmation(&mut self, deletion: DeletionMode) {
        let Some(confirmation) = self.fs_confirmation.take() else {
            return;
        };
        let root = confirmation.plan.root().to_path_buf();
        match confirmation
            .plan
            .apply_with_trash(deletion, self.ports.trash())
        {
            Ok(report) => {
                let count = report.applied.len();
                let warning =
                    self.reconcile_applied_filesystem(&root, confirmation.buffer, &report, true);
                let mut status = format!(
                    "applied {count} filesystem operation{}",
                    if count == 1 { "" } else { "s" }
                );
                if let Some(warning) = warning {
                    status.push_str(" · ");
                    status.push_str(&warning);
                }
                self.status(status);
            }
            Err(error) => {
                let warning = self.reconcile_applied_filesystem(
                    &root,
                    confirmation.buffer,
                    &error.report,
                    false,
                );
                let mut message = error.to_string();
                if let Some(warning) = warning {
                    message.push_str(" · ");
                    message.push_str(&warning);
                }
                if !error.report.recovery.is_empty() || !error.report.applied.is_empty() {
                    self.error_from("Filesystem", "Filesystem plan needs recovery", message);
                } else {
                    self.action_failed(message);
                }
            }
        }
    }

    pub(super) fn reconcile_applied_filesystem(
        &mut self,
        root: &Path,
        initiating_buffer: usize,
        report: &ApplyReport,
        completed: bool,
    ) -> Option<String> {
        let mut warnings = Vec::new();
        let mut affected_directories = HashSet::new();
        let mut deleted_paths = Vec::new();
        let mut moved_sources = HashSet::new();
        for recovery in &report.recovery {
            if let Some(parent) = recovery.original.parent() {
                affected_directories.insert(parent.to_path_buf());
            }
            if recovery.kind == crate::fs_plan::RecoveryKind::Original {
                // The old path may now belong to a different file. Leave open
                // text and its accepted disk baseline intact; ordinary save
                // conflict checks must still compare against that baseline.
                warnings.push(format!(
                    "original at {} requires recovery; retained location {}",
                    recovery.original.display(),
                    recovery.retained.display()
                ));
            }
        }
        for operation in &report.applied {
            match operation {
                FsOperation::Create { path, .. } | FsOperation::Copy { to: path, .. } => {
                    if let Some(parent) = resolved_operation_path(root, path).parent() {
                        affected_directories.insert(parent.to_path_buf());
                    }
                }
                FsOperation::Rename { from, to, .. } | FsOperation::Move { from, to, .. } => {
                    let source = resolved_operation_path(root, from);
                    if let Some(parent) = source.parent() {
                        affected_directories.insert(parent.to_path_buf());
                    }
                    if matches!(operation, FsOperation::Move { .. }) {
                        moved_sources.insert(source);
                    }
                    if let Some(parent) = resolved_operation_path(root, to).parent() {
                        affected_directories.insert(parent.to_path_buf());
                    }
                }
                FsOperation::Delete { path, kind } => {
                    let path = resolved_operation_path(root, path);
                    if let Some(parent) = path.parent() {
                        affected_directories.insert(parent.to_path_buf());
                    }
                    deleted_paths.push((path, *kind));
                }
            }
        }
        affected_directories = affected_directories
            .into_iter()
            .map(|directory| {
                mapped_applied_path(root, &directory, &report.applied).unwrap_or(directory)
            })
            .collect();

        let retargeted = self
            .buffers
            .iter()
            .enumerate()
            .filter_map(|(index, buffer)| {
                let path = buffer.path.as_deref()?;
                let mapped = mapped_applied_path(root, path, &report.applied)?;
                (mapped != path).then_some((index, path.to_path_buf(), mapped))
            })
            .collect::<Vec<_>>();
        let mut git_paths = Vec::new();
        for (buffer_id, previous, mapped) in retargeted {
            if self.closed_buffers.contains(&buffer_id) {
                continue;
            }
            self.buffers[buffer_id].retarget_path(mapped.clone());
            if self.buffers[buffer_id].kind == BufferKind::File {
                self.git.forget(&previous);
                git_paths.push(mapped);
            }
            self.reparse_whole(buffer_id);
            // `lsp_touch` compares both path and language against the opened
            // document, closing the old URI before opening the new identity.
            self.lsp_touch(buffer_id);
        }

        let mut reload = Vec::new();
        let mut rebase = Vec::new();
        for (index, buffer) in self.buffers.iter().enumerate() {
            if self.closed_buffers.contains(&index) {
                continue;
            }
            let Some(path) = buffer.path.as_deref() else {
                continue;
            };
            if deleted_paths.iter().any(|(deleted, kind)| {
                path == deleted || (*kind == EntryKind::Directory && path.starts_with(deleted))
            }) {
                warnings.push(format!("open buffer has stale path {}", path.display()));
            }
            if !buffer.is_directory() {
                continue;
            }
            let affected = index == initiating_buffer
                || affected_directories.contains(path)
                || affected_directories
                    .iter()
                    .any(|directory| directory.starts_with(path));
            if !affected {
                continue;
            }
            if index == initiating_buffer && !completed {
                warnings.push(
                    "directory edits retained after partial application; refresh before retrying"
                        .to_owned(),
                );
            } else if buffer.dirty
                && index != initiating_buffer
                && !self.contains_only_deletions(index, &moved_sources)
            {
                rebase.push((index, path.to_path_buf()));
            } else {
                reload.push(index);
            }
        }
        for (index, path) in rebase {
            match self.buffers[index].rebase_directory_after_external_removals(&moved_sources) {
                Ok(true) => {}
                Ok(false) | Err(_) => warnings.push(format!(
                    "affected explorer {} kept unsaved edits; refresh it before saving",
                    path.display()
                )),
            }
        }
        let view = self.listing_view();
        for index in reload {
            self.forget_directory_view(index);
            self.forget_directory_jumps(index);
            let refresh = if index == initiating_buffer && completed {
                self.buffers[index].accept_directory_plan(self.config.editor.show_hidden_files)
            } else {
                self.buffers[index].reload_directory(view)
            };
            match refresh {
                Ok(()) => {
                    self.clear_syntax_history(index);
                    self.stale_syntax.remove(&index);
                    self.syntax[index] = None;
                    self.normalize_buffer(index);
                }
                Err(error) => warnings.push(format!(
                    "could not refresh explorer {}: {error}",
                    self.buffers[index].display_name()
                )),
            }
        }
        if !report.applied.is_empty() {
            // These writes are editor-owned and already complete. Reconcile
            // directly rather than depending on an optional native watcher.
            self.reconcile_git_after_filesystem(git_paths);
        }
        warnings.sort();
        warnings.dedup();
        (!warnings.is_empty()).then(|| warnings.join(" · "))
    }

    /// Keeps the completion and signature popups in step with typing.
    ///
    /// Completion is requested on a trigger character rather than on every
    /// keystroke: an open popup filters locally as the word grows, so asking
    /// again per letter would be a round trip that changes nothing.
    ///
    /// Which characters those are is the server's own answer, read from the
    /// handshake, so a language whose calls are not written `f(a, b)` is asked
    /// where its own author said to ask. Both are `false` without a server.
    fn after_insert(&mut self, character: char) {
        let showing_signature = self.signature.is_some();
        let (completion_trigger, signature_trigger) =
            self.active_server_capabilities()
                .map_or((false, false), |capabilities| {
                    (
                        capabilities.triggers_completion(character),
                        capabilities.triggers_signature_help(character, showing_signature),
                    )
                });
        if let Some(session) = self.explicit_completion_session() {
            if character.is_whitespace() {
                self.completion = None;
            } else if completion_trigger {
                self.restart_explicit_lsp_completion(session);
            } else if is_word(character) {
                self.refresh_explicit_completion_filter();
            } else {
                let head = self.active().head();
                if let Some(state) = self.completion.as_mut() {
                    state.anchor = head;
                    state.filter.clear();
                    state.selected = 0;
                }
            }
            self.after_insert_signature(character, signature_trigger, showing_signature);
            return;
        }
        let was_path_completion = self.path_completion_active();
        if let Some(source) = self.completion.as_ref().map(|state| state.source) {
            let keeps_popup = match source {
                CompletionSource::Language => character.is_alphanumeric() || character == '_',
                CompletionSource::Path => !is_path_token_boundary(character) && character != '/',
                CompletionSource::Word => is_word_completion_character(character),
            };
            // A path popup is rebuilt from the directory below rather than
            // narrowed in place. Its items are only the bounded best of a
            // listing collected for the shorter prefix, so filtering them
            // would answer the longer one from a set that never contained
            // every match for it.
            if !keeps_popup || source == CompletionSource::Path {
                self.completion = None;
            } else if let Some(state) = self.completion.as_mut() {
                state.filter.push(character);
                state.selected = 0;
                if state.visible_indices().is_empty() {
                    self.completion = None;
                }
            }
        }
        if character == '/' || self.completion.is_none() {
            self.path_completion();
        }
        if was_path_completion || self.path_completion_active() {
            return;
        }
        self.word_completion(character);
        if completion_trigger {
            self.lsp_completion();
        }
        self.after_insert_signature(character, signature_trigger, showing_signature);
    }

    /// Asks for signature help, or dismisses the popup, for one typed
    /// character.
    ///
    /// A server that lists `)` among its retrigger characters is asked again
    /// there, which is what makes `f(g(a), b)` return to `f`'s signature
    /// rather than lose it: only the server knows which call the caret is
    /// inside now, and only the context tells it that this `)` closed an inner
    /// call rather than opening a fresh one. When the server named nothing,
    /// the popup is closed locally instead, so it can never be left open over
    /// a call that has ended.
    fn after_insert_signature(&mut self, character: char, trigger: bool, showing: bool) {
        if trigger {
            self.lsp_signature(SignatureContext {
                trigger: Some(character),
                retrigger: showing,
            });
        } else if character == ')' {
            self.signature = None;
        }
    }

    /// What the active buffer's server advertised, or `None` when it has no
    /// server or none has finished its handshake yet.
    fn active_server_capabilities(&self) -> Option<&Capabilities> {
        let language = self.language_of(self.active().buffer)?;
        Some(&self.lsp_servers.get(&language)?.capabilities)
    }

    /// Whether the active buffer has a server that finished its handshake.
    pub(super) fn has_language_server(&self) -> bool {
        self.active_server_capabilities().is_some()
    }

    fn handle_command(&mut self, key: KeyStroke) -> Result<()> {
        if self.prompt_kind != PromptKind::Command {
            return self.handle_search_prompt(key);
        }
        if key.modifiers.contains(Modifiers::CONTROL) {
            return self.handle_prompt_control(key);
        }
        if key.modifiers.contains(Modifiers::ALT) {
            match key.code {
                KeyCode::Char('b') => {
                    self.command_cursor = prompt_word_backward(&self.command, self.command_cursor);
                }
                KeyCode::Char('f') => {
                    self.command_cursor = prompt_word_forward(&self.command, self.command_cursor);
                }
                _ => {}
            }
            return Ok(());
        }
        match key.code {
            KeyCode::Escape => {
                self.close_prompt();
            }
            KeyCode::Enter => {
                let command = self.command.trim().to_owned();
                if command.is_empty() {
                    return Ok(());
                }
                let name = command.split_whitespace().next().unwrap_or_default();
                let Some(spec) = resolve_command(name) else {
                    self.complete_selected_command();
                    return Ok(());
                };
                let availability = self.command_capabilities().command_availability(spec);
                if let CommandAvailability::Unavailable(reason) = availability {
                    // Deliberately does not call report_completed_action: the
                    // palette is still open (the same "correctable input"
                    // treatment as a schema error below), and the
                    // interaction line is the palette's own text until it
                    // closes. Echoing here would mean closing it out from
                    // under whoever is still typing, which is exactly the
                    // "notifications never replace [the prompt]" rule this
                    // stays retained-only for; see the resolved issue file.
                    self.mark_unavailable(format!("{} is unavailable: {reason}", spec.description));
                    return Ok(());
                }
                let has_argument = command
                    .split_once(char::is_whitespace)
                    .is_some_and(|(_, argument)| !argument.trim().is_empty());
                if spec.arguments.is_required() && !has_argument {
                    self.command = format!("{} ", spec.name);
                    self.command_cursor = self.command.chars().count();
                    self.command_selection = 0;
                    return Ok(());
                }
                // Schema errors are correctable input, so unlike a successful
                // command they leave the palette and its text open. This is
                // also where argumentless commands reject accidental extras.
                let invocation = match parse_colon_command(&command) {
                    Ok(invocation) => invocation,
                    Err(error) => {
                        self.action_failed(error.to_string());
                        return Ok(());
                    }
                };
                self.command.clear();
                self.command_cursor = 0;
                self.command_selection = 0;
                self.mode = Mode::Normal;
                let outcome = self.execute(invocation)?;
                self.report_completed_action(&format!(":{name}"), spec.description, outcome);
                if let Some(mode) = self.grammar.preferred_mode()
                    && matches!(self.mode, Mode::Normal | Mode::Select)
                {
                    self.mode = mode;
                }
            }
            KeyCode::Backspace => {
                prompt_backspace(&mut self.command, &mut self.command_cursor);
                self.command_selection = 0;
            }
            KeyCode::Up | KeyCode::BackTab => self.select_previous_command(),
            KeyCode::Down => self.select_next_command(),
            KeyCode::Home => self.command_selection = 0,
            KeyCode::End => {
                self.command_selection = self.command_hint_count().saturating_sub(1);
            }
            KeyCode::Tab => self.complete_selected_command(),
            KeyCode::Left => {
                self.command_cursor = self.command_cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                self.command_cursor = (self.command_cursor + 1).min(self.command.chars().count());
            }
            KeyCode::Char(ch) => {
                prompt_insert(&mut self.command, self.command_cursor, ch);
                self.command_cursor += 1;
                self.command_selection = 0;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_picker(&mut self, key: KeyStroke) -> Result<()> {
        if key.code == KeyCode::Escape
            || key.code == KeyCode::Char('c') && key.modifiers.contains(Modifiers::CONTROL)
        {
            self.close_file_picker();
            return Ok(());
        }
        // Pacing holds a ranked answer back from the reader, never from the
        // reader's own keys. Every key but a query edit reads the list —
        // moving the selection, opening a row, switching mode — and reads it
        // as the ranker last left it, so nothing is accepted from a list that
        // has already been answered.
        if !edits_picker_query(key) {
            self.publish_paced_picker_rows();
        }

        if self.finder.is_some() && key.code == KeyCode::Tab && key.modifiers.is_empty() {
            self.toggle_finder_mode();
            return Ok(());
        }
        if self.finder.is_some() {
            return self.handle_resource_picker(key);
        }

        let page = 10;
        let control = key.modifiers.contains(Modifiers::CONTROL);
        let mut selection_changed = false;
        let mut query_changed = false;
        let result: Result<Option<PickerTarget>> = match (key.code, key.modifiers) {
            (KeyCode::Char('p'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.picker.as_mut().unwrap().up();
                selection_changed = true;
                Ok(None)
            }
            (KeyCode::Char('n'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.picker.as_mut().unwrap().down();
                selection_changed = true;
                Ok(None)
            }
            (KeyCode::Char('u'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.picker.as_mut().unwrap().page_up(page);
                selection_changed = true;
                Ok(None)
            }
            (KeyCode::Char('d'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.picker.as_mut().unwrap().page_down(page);
                selection_changed = true;
                Ok(None)
            }
            (KeyCode::Char('s'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                let target = self.picker.as_ref().and_then(FilePicker::selected_target);
                if let Some(target) = target {
                    self.close_file_picker();
                    self.split(Axis::Vertical, Some(target.path.clone()))?;
                    self.select_picker_target(&target);
                }
                return Ok(());
            }
            (KeyCode::Char('v'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                let target = self.picker.as_ref().and_then(FilePicker::selected_target);
                if let Some(target) = target {
                    self.close_file_picker();
                    self.split(Axis::Horizontal, Some(target.path.clone()))?;
                    self.select_picker_target(&target);
                }
                return Ok(());
            }
            (KeyCode::Char('t'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                let picker = self.picker.as_mut().unwrap();
                picker.show_preview = !picker.show_preview;
                Ok(None)
            }
            (KeyCode::Down | KeyCode::Tab, _) => {
                self.picker.as_mut().unwrap().down();
                selection_changed = true;
                Ok(None)
            }
            (KeyCode::Up | KeyCode::BackTab, _) => {
                self.picker.as_mut().unwrap().up();
                selection_changed = true;
                Ok(None)
            }
            (KeyCode::PageUp, _) => {
                self.picker.as_mut().unwrap().page_up(page);
                selection_changed = true;
                Ok(None)
            }
            (KeyCode::PageDown, _) => {
                self.picker.as_mut().unwrap().page_down(page);
                selection_changed = true;
                Ok(None)
            }
            (KeyCode::Home, _) => {
                self.picker.as_mut().unwrap().first();
                selection_changed = true;
                Ok(None)
            }
            (KeyCode::End, _) => {
                self.picker.as_mut().unwrap().last();
                selection_changed = true;
                Ok(None)
            }
            (KeyCode::Left, _) => {
                self.picker.as_mut().unwrap().query_left();
                Ok(None)
            }
            (KeyCode::Char('b'), _) if control => {
                self.picker.as_mut().unwrap().query_left();
                Ok(None)
            }
            (KeyCode::Right, _) => {
                self.picker.as_mut().unwrap().query_right();
                Ok(None)
            }
            (KeyCode::Char('f'), _) if control => {
                self.picker.as_mut().unwrap().query_right();
                Ok(None)
            }
            (KeyCode::Backspace, _) => {
                let picker = self.picker.as_mut().unwrap();
                query_changed = picker.query_cursor > 0;
                if self.file_scanner.is_some() {
                    picker.backspace_query_unranked();
                } else {
                    picker.backspace_query();
                }
                selection_changed = query_changed;
                Ok(None)
            }
            (KeyCode::Char('h'), _) if control => {
                let picker = self.picker.as_mut().unwrap();
                query_changed = picker.query_cursor > 0;
                if self.file_scanner.is_some() {
                    picker.backspace_query_unranked();
                } else {
                    picker.backspace_query();
                }
                selection_changed = query_changed;
                Ok(None)
            }
            (KeyCode::Delete, _) => {
                let picker = self.picker.as_mut().unwrap();
                query_changed = picker.query_cursor < picker.query.chars().count();
                if self.file_scanner.is_some() {
                    picker.delete_query_unranked();
                } else {
                    picker.delete_query();
                }
                selection_changed = query_changed;
                Ok(None)
            }
            (KeyCode::Char('w'), _) if control => {
                let picker = self.picker.as_mut().unwrap();
                query_changed = picker.query_cursor > 0;
                if self.file_scanner.is_some() {
                    picker.delete_query_word_unranked();
                } else {
                    picker.delete_query_word();
                }
                selection_changed = query_changed;
                Ok(None)
            }
            (KeyCode::Char('k'), _) if control => {
                let picker = self.picker.as_mut().unwrap();
                query_changed = picker.query_cursor < picker.query.chars().count();
                if self.file_scanner.is_some() {
                    picker.delete_query_end_unranked();
                } else {
                    picker.delete_query_end();
                }
                selection_changed = query_changed;
                Ok(None)
            }
            (KeyCode::Char('a'), _) if control => {
                self.picker.as_mut().unwrap().query_cursor = 0;
                Ok(None)
            }
            (KeyCode::Char('e'), _) if control => {
                let picker = self.picker.as_mut().unwrap();
                picker.query_cursor = picker.query.chars().count();
                Ok(None)
            }
            (KeyCode::Enter, _) => Ok(self.picker.as_ref().and_then(FilePicker::selected_target)),
            (KeyCode::Char(character), _)
                if !key
                    .modifiers
                    .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER) =>
            {
                if self.file_scanner.is_some() {
                    self.picker
                        .as_mut()
                        .unwrap()
                        .insert_query_unranked(character);
                } else {
                    self.picker.as_mut().unwrap().insert_query(character);
                }
                selection_changed = true;
                query_changed = true;
                Ok(None)
            }
            _ => Ok(None),
        };
        self.restart_content_scan_if_needed();
        if query_changed {
            self.rank_resource_finder();
        }
        if selection_changed {
            self.refresh_file_picker_preview();
        }
        if let Some(target) = result? {
            self.close_file_picker();
            self.open_file(target.path.clone())?;
            self.select_picker_target(&target);
        }
        Ok(())
    }

    fn handle_resource_picker(&mut self, key: KeyStroke) -> Result<()> {
        let page = 10;
        let control = key.modifiers.contains(Modifiers::CONTROL);
        let mut query_changed = false;
        match (key.code, key.modifiers) {
            (KeyCode::Char('p'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.finder.as_mut().unwrap().up();
            }
            (KeyCode::Char('n'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.finder.as_mut().unwrap().down();
            }
            (KeyCode::Char('u'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.finder.as_mut().unwrap().page_up(page);
            }
            (KeyCode::Char('d'), modifiers) if modifiers.contains(Modifiers::CONTROL) => {
                self.finder.as_mut().unwrap().page_down(page);
            }
            (KeyCode::Down, _) => self.finder.as_mut().unwrap().down(),
            (KeyCode::Up | KeyCode::BackTab, _) => self.finder.as_mut().unwrap().up(),
            (KeyCode::PageUp, _) => self.finder.as_mut().unwrap().page_up(page),
            (KeyCode::PageDown, _) => self.finder.as_mut().unwrap().page_down(page),
            (KeyCode::Home, _) => self.finder.as_mut().unwrap().first(),
            (KeyCode::End, _) => self.finder.as_mut().unwrap().last(),
            (KeyCode::Left, _) => {
                self.picker.as_mut().unwrap().query_left();
            }
            (KeyCode::Char('b'), _) if control => {
                self.picker.as_mut().unwrap().query_left();
            }
            (KeyCode::Right, _) => {
                self.picker.as_mut().unwrap().query_right();
            }
            (KeyCode::Char('f'), _) if control => {
                self.picker.as_mut().unwrap().query_right();
            }
            (KeyCode::Backspace, _) => {
                let picker = self.picker.as_mut().unwrap();
                query_changed = picker.query_cursor > 0;
                if self.file_scanner.is_some() {
                    picker.backspace_query_unranked();
                } else {
                    picker.backspace_query();
                }
            }
            (KeyCode::Char('h'), _) if control => {
                let picker = self.picker.as_mut().unwrap();
                query_changed = picker.query_cursor > 0;
                if self.file_scanner.is_some() {
                    picker.backspace_query_unranked();
                } else {
                    picker.backspace_query();
                }
            }
            (KeyCode::Delete, _) => {
                let picker = self.picker.as_mut().unwrap();
                query_changed = picker.query_cursor < picker.query.chars().count();
                if self.file_scanner.is_some() {
                    picker.delete_query_unranked();
                } else {
                    picker.delete_query();
                }
            }
            (KeyCode::Char('w'), _) if control => {
                let picker = self.picker.as_mut().unwrap();
                query_changed = picker.query_cursor > 0;
                if self.file_scanner.is_some() {
                    picker.delete_query_word_unranked();
                } else {
                    picker.delete_query_word();
                }
            }
            (KeyCode::Char('k'), _) if control => {
                let picker = self.picker.as_mut().unwrap();
                query_changed = picker.query_cursor < picker.query.chars().count();
                if self.file_scanner.is_some() {
                    picker.delete_query_end_unranked();
                } else {
                    picker.delete_query_end();
                }
            }
            (KeyCode::Char('a'), _) if control => {
                self.picker.as_mut().unwrap().query_cursor = 0;
            }
            (KeyCode::Char('e'), _) if control => {
                let picker = self.picker.as_mut().unwrap();
                picker.query_cursor = picker.query.chars().count();
            }
            (KeyCode::Char('s'), _) if control => {
                let target = self
                    .finder
                    .as_ref()
                    .zip(self.picker.as_ref())
                    .and_then(|(finder, picker)| finder.selected_target(picker));
                if let Some(FinderTarget::File(target)) = target {
                    self.close_file_picker();
                    self.split(Axis::Vertical, Some(target.path.clone()))?;
                    self.select_picker_target(&target);
                }
                return Ok(());
            }
            (KeyCode::Char('v'), _) if control => {
                let target = self
                    .finder
                    .as_ref()
                    .zip(self.picker.as_ref())
                    .and_then(|(finder, picker)| finder.selected_target(picker));
                if let Some(FinderTarget::File(target)) = target {
                    self.close_file_picker();
                    self.split(Axis::Horizontal, Some(target.path.clone()))?;
                    self.select_picker_target(&target);
                }
                return Ok(());
            }
            (KeyCode::Char('t'), _) if control => {
                let picker = self.picker.as_mut().unwrap();
                picker.show_preview = !picker.show_preview;
            }
            (KeyCode::Enter, _) => {
                if let (Some(finder), Some(picker)) = (self.finder.as_ref(), self.picker.as_ref())
                    && let Some(target) = finder.selected_target(picker)
                {
                    self.activate_finder_target(target);
                }
                return Ok(());
            }
            (KeyCode::Char(character), _)
                if !key
                    .modifiers
                    .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER) =>
            {
                if self.file_scanner.is_some() {
                    self.picker
                        .as_mut()
                        .unwrap()
                        .insert_query_unranked(character);
                } else {
                    self.picker.as_mut().unwrap().insert_query(character);
                }
                query_changed = true;
            }
            _ => {}
        }
        self.restart_content_scan_if_needed();
        if query_changed {
            self.rank_resource_finder();
        }
        self.refresh_finder_preview();
        Ok(())
    }

    pub(super) fn select_picker_target(&mut self, target: &PickerTarget) {
        let Some(row) = target.row else {
            return;
        };
        let buffer_id = self.active().buffer;
        if self.buffers[buffer_id].path.as_deref() != Some(target.path.as_path()) {
            return;
        }
        let row = row.min(self.buffers[buffer_id].len_lines().saturating_sub(1));
        let column = target.column.min(self.buffers[buffer_id].line_len(row));
        let offset = self.buffers[buffer_id].line_to_offset(row) + column;
        self.active_mut()
            .replace_selection(Selection::point(offset));
    }

    fn execute_editor_command(&mut self, command: EditorCommand) -> Result<()> {
        use EditorCommand as Command;

        // Before the read-only check, because a terminal is not read only —
        // it is not a document at all, and the buffer whose permissions that
        // check reads is the one hidden behind it.
        if let Some(id) = self.active_terminal() {
            let transient_line_selection = self.line_select.is_some();
            if !matches!(command, Command::SelectLine | Command::SelectLineUp)
                && let Some(mode) = self.line_select.take()
            {
                self.mode = mode;
            }
            if self.execute_terminal_command(id, command, transient_line_selection) {
                return Ok(());
            }
        }
        if let Some(reason) = self.active_buffer().read_only_reason()
            && command.is_mutating()
        {
            self.mode = Mode::Normal;
            self.action_failed(reason);
            return Ok(());
        }

        // Explicit view alignment is honored until the next editor command.
        // Alignment and scroll commands set this back to true below.
        self.active_mut().preserve_scroll = false;
        // A popup is anchored to a caret position; any command that is not
        // about the popup itself may move that caret, so it goes.
        let preserves_explicit_completion = self.explicit_completion_session().is_some()
            && matches!(
                command,
                Command::DeleteCharBackward | Command::DeleteCharForward
            );
        if !matches!(command, Command::TriggerCompletion) && !preserves_explicit_completion {
            self.completion = None;
        }
        if !matches!(command, Command::ShowDocumentation) {
            self.hover = None;
        }
        let transient_line_selection = self.line_select.is_some();
        // A line selection lives only for as long as `x`/`X` keep arriving.
        // Anything else ends it and hands the mode back, so `j` after `x` is a
        // plain motion rather than an extension.
        if !matches!(command, Command::SelectLine | Command::SelectLineUp)
            && let Some(mode) = self.line_select.take()
        {
            self.mode = mode;
        }
        match command {
            Command::EnterNormalMode => self.enter_normal_mode(),
            Command::OpenCommandPalette => self.open_prompt(PromptKind::Command),
            Command::MoveLeft => self.motion(Motion::Left),
            Command::MoveRight => self.motion(Motion::Right),
            Command::MoveUp => self.motion(Motion::Up),
            Command::MoveDown => self.motion(Motion::Down),
            Command::MoveLineStart => self.motion(Motion::LineStart),
            Command::MoveLineEnd => self.motion(Motion::LineEnd),
            Command::MoveFirstNonWhitespace => self.motion(Motion::FirstNonWhitespace),
            // The file boundaries are the two motions that can cross an
            // arbitrary distance, so they are the two that record a jump.
            Command::MoveFileStart => {
                self.push_jump();
                self.motion(Motion::FileStart);
            }
            Command::MoveFileEnd => {
                self.push_jump();
                self.motion(Motion::FileEnd);
            }
            Command::MoveWordForward => self.motion(Motion::WordForward),
            Command::MoveWordBackward => self.motion(Motion::WordBack),
            Command::MoveWordEnd => self.motion(Motion::WordEnd),
            Command::MoveLongWordForward => self.motion(Motion::LongWordForward),
            Command::MoveLongWordBackward => self.motion(Motion::LongWordBack),
            Command::MoveLongWordEnd => self.motion(Motion::LongWordEnd),
            Command::GotoNextParagraph => self.motion(Motion::NextParagraph),
            Command::GotoPreviousParagraph => self.motion(Motion::PreviousParagraph),
            Command::FindNextChar
            | Command::FindPreviousChar
            | Command::FindTillNextChar
            | Command::FindTillPreviousChar
            | Command::ReplaceChar
            | Command::SelectRegister
            | Command::RecordMacro
            | Command::ReplayMacro => {
                unreachable!("character-taking commands are intercepted before execution")
            }
            Command::StopMacroRecording => self.stop_macro_recording(),
            Command::RecordDefaultMacro => self.record_default_macro(),
            Command::ReplayDefaultMacro => {
                self.replay_macro(DEFAULT_MACRO_REGISTER, 1)?;
            }
            Command::ListMacros => self.open_macro_list(),
            Command::PageUp => self.motion(Motion::PageUp),
            Command::PageDown => self.motion(Motion::PageDown),
            Command::HalfPageUp => self.motion(Motion::HalfPageUp),
            Command::HalfPageDown => self.motion(Motion::HalfPageDown),
            Command::EnterInsertMode => self.enter_insert(false),
            Command::EnterReplaceMode => self.enter_replace_mode(),
            Command::AppendAfter => self.enter_insert(true),
            Command::InsertLineStart => {
                self.motion(Motion::LineStart);
                self.enter_insert(false);
            }
            Command::InsertLineEnd => {
                let buffer = self.active_buffer();
                let selection = self.active().selection.transform(|range| {
                    let row = buffer.offset_to_row(range.head);
                    Range::point(buffer.line_to_offset(row) + buffer.line_len(row))
                });
                self.active_mut().replace_selection(selection);
                self.mode = Mode::Insert;
            }
            Command::OpenLineBelow => self.open_line(false),
            Command::OpenLineAbove => self.open_line(true),
            Command::ToggleCase => self.toggle_case(),
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            Command::Yank => self.yank(transient_line_selection),
            Command::YankLine => self.yank_line(),
            Command::PasteAfter => self.paste(false),
            Command::PasteBefore => self.paste(true),
            Command::Indent => self.indent(false),
            Command::Unindent => self.indent(true),
            Command::ToggleComments => self.toggle_comments(),
            Command::DeleteSelection => {
                self.delete_selection_or_char(false, transient_line_selection)
            }
            Command::ChangeSelection => self.delete_selection_or_char(true, false),
            Command::EnterSelectMode => self.toggle_select_mode(),
            Command::SelectLine => self.select_line(true),
            Command::SelectLineUp => self.select_line(false),
            Command::SelectAll => self.select_all(),
            Command::MatchBracket => self.match_bracket(),
            Command::CollapseSelection => self.collapse_selection(),
            Command::FlipSelection => self.flip_selection(),
            Command::ExpandSyntaxSelection => self.expand_syntax_selection()?,
            Command::ShrinkSyntaxSelection => self.shrink_syntax_selection()?,
            Command::SelectSyntaxParent => {
                self.transform_syntax_selection(SyntaxSelectionTransform::Parent)?
            }
            Command::SelectSyntaxChild => {
                self.transform_syntax_selection(SyntaxSelectionTransform::FirstNamedChild)?
            }
            Command::SelectPreviousSyntaxSibling => {
                self.transform_syntax_selection(SyntaxSelectionTransform::PreviousNamedSibling)?
            }
            Command::SelectNextSyntaxSibling => {
                self.transform_syntax_selection(SyntaxSelectionTransform::NextNamedSibling)?
            }
            Command::SelectSyntaxFunction => {
                self.select_syntax_object(SyntaxObject::Function, SyntaxObjectPart::Around)?
            }
            Command::SelectInsideSyntaxFunction => {
                self.select_syntax_object(SyntaxObject::Function, SyntaxObjectPart::Inside)?
            }
            Command::SelectSyntaxClass => {
                self.select_syntax_object(SyntaxObject::Class, SyntaxObjectPart::Around)?
            }
            Command::SelectInsideSyntaxClass => {
                self.select_syntax_object(SyntaxObject::Class, SyntaxObjectPart::Inside)?
            }
            Command::SelectSyntaxParameter => {
                self.select_syntax_object(SyntaxObject::Parameter, SyntaxObjectPart::Around)?
            }
            Command::SelectInsideSyntaxParameter => {
                self.select_syntax_object(SyntaxObject::Parameter, SyntaxObjectPart::Inside)?
            }
            Command::SelectAroundParentheses => {
                self.select_delimiter(Some(DelimiterPair::Parentheses), SyntaxObjectPart::Around)?
            }
            Command::SelectInsideParentheses => {
                self.select_delimiter(Some(DelimiterPair::Parentheses), SyntaxObjectPart::Inside)?
            }
            Command::SelectAroundSquareBrackets => self.select_delimiter(
                Some(DelimiterPair::SquareBrackets),
                SyntaxObjectPart::Around,
            )?,
            Command::SelectInsideSquareBrackets => self.select_delimiter(
                Some(DelimiterPair::SquareBrackets),
                SyntaxObjectPart::Inside,
            )?,
            Command::SelectAroundBraces => {
                self.select_delimiter(Some(DelimiterPair::Braces), SyntaxObjectPart::Around)?
            }
            Command::SelectInsideBraces => {
                self.select_delimiter(Some(DelimiterPair::Braces), SyntaxObjectPart::Inside)?
            }
            Command::SelectAroundAngleBrackets => {
                self.select_delimiter(Some(DelimiterPair::AngleBrackets), SyntaxObjectPart::Around)?
            }
            Command::SelectInsideAngleBrackets => {
                self.select_delimiter(Some(DelimiterPair::AngleBrackets), SyntaxObjectPart::Inside)?
            }
            Command::SelectAroundDoubleQuotes => {
                self.select_delimiter(Some(DelimiterPair::DoubleQuotes), SyntaxObjectPart::Around)?
            }
            Command::SelectInsideDoubleQuotes => {
                self.select_delimiter(Some(DelimiterPair::DoubleQuotes), SyntaxObjectPart::Inside)?
            }
            Command::SelectAroundSingleQuotes => {
                self.select_delimiter(Some(DelimiterPair::SingleQuotes), SyntaxObjectPart::Around)?
            }
            Command::SelectInsideSingleQuotes => {
                self.select_delimiter(Some(DelimiterPair::SingleQuotes), SyntaxObjectPart::Inside)?
            }
            Command::SelectAroundBackticks => {
                self.select_delimiter(Some(DelimiterPair::Backticks), SyntaxObjectPart::Around)?
            }
            Command::SelectInsideBackticks => {
                self.select_delimiter(Some(DelimiterPair::Backticks), SyntaxObjectPart::Inside)?
            }
            Command::SelectAroundClosestDelimiter => {
                self.select_delimiter(None, SyntaxObjectPart::Around)?
            }
            Command::SelectInsideClosestDelimiter => {
                self.select_delimiter(None, SyntaxObjectPart::Inside)?
            }
            Command::GotoPreviousSyntaxFunction => {
                self.navigate_syntax_object(SyntaxObject::Function, false)?
            }
            Command::GotoNextSyntaxFunction => {
                self.navigate_syntax_object(SyntaxObject::Function, true)?
            }
            Command::GotoPreviousSyntaxClass => {
                self.navigate_syntax_object(SyntaxObject::Class, false)?
            }
            Command::GotoNextSyntaxClass => {
                self.navigate_syntax_object(SyntaxObject::Class, true)?
            }
            Command::GotoPreviousSyntaxParameter => {
                self.navigate_syntax_object(SyntaxObject::Parameter, false)?
            }
            Command::GotoNextSyntaxParameter => {
                self.navigate_syntax_object(SyntaxObject::Parameter, true)?
            }
            Command::DocumentOutline => self.open_document_outline()?,
            Command::ToggleSyntaxFold => self.toggle_syntax_fold(),
            Command::FoldAllSyntax => self.fold_all_syntax(),
            Command::UnfoldAllSyntax => self.unfold_all_syntax(),
            Command::SplitSelectionAtLineEnds => self.split_selection_at_line_edges(true),
            Command::SplitSelectionAtLineStarts => self.split_selection_at_line_edges(false),
            Command::KeepPrimarySelection => self.keep_primary_selection(),
            Command::RemovePrimarySelection => self.remove_primary_selection(),
            Command::CopySelectionDown => self.copy_selection(true),
            Command::CopySelectionUp => self.copy_selection(false),
            Command::CopySelectionDownPadded => self.copy_selection_padded(true),
            Command::CopySelectionUpPadded => self.copy_selection_padded(false),
            Command::RotateSelectionForward => self.rotate_selection(true),
            Command::RotateSelectionBackward => self.rotate_selection(false),
            Command::RotateSelectionContentsForward => self.rotate_selection_contents(true),
            Command::RotateSelectionContentsBackward => self.rotate_selection_contents(false),
            Command::KeepMatchingSelections => {
                self.open_prompt(PromptKind::FilterSelections { keep: true })
            }
            Command::RemoveMatchingSelections => {
                self.open_prompt(PromptKind::FilterSelections { keep: false })
            }
            Command::AlignSelections => self.align_selections(),
            Command::TrimSelections => self.trim_selections(),
            Command::TrimTrailingWhitespace => self.trim_trailing_whitespace_in_selection(),
            Command::HardWrap => self.hard_wrap_selections(self.config.editor.hard_wrap_width),
            Command::Reflow => self.reflow_selections(self.config.editor.hard_wrap_width),
            Command::JoinSelections => self.open_prompt(PromptKind::JoinDelimiter),
            Command::FormatTable => self.format_selected_tables(),
            Command::Search => self.open_prompt(PromptKind::Search(SearchMode::Insensitive)),
            Command::SearchRegex => self.open_prompt(PromptKind::Search(SearchMode::Regex)),
            Command::SearchForward => self.open_prompt(PromptKind::SearchForward),
            Command::SearchBackward => self.open_prompt(PromptKind::SearchBackward),
            Command::SearchNext => self.repeat_search(false),
            Command::SearchPrevious => self.repeat_search(true),
            Command::SearchSelection => self.search_selection(),
            Command::AlignViewCenter => self.align_view(ViewAlignment::Center),
            Command::AlignViewTop => self.align_view(ViewAlignment::Top),
            Command::AlignViewBottom => self.align_view(ViewAlignment::Bottom),
            Command::AlignViewMiddle => self.align_view_middle(),
            Command::ScrollViewDown => self.scroll_view(1),
            Command::ScrollViewUp => self.scroll_view(-1),
            Command::GotoWindowTop => self.motion(Motion::WindowTop),
            Command::GotoWindowCenter => self.motion(Motion::WindowCenter),
            Command::GotoWindowBottom => self.motion(Motion::WindowBottom),
            Command::GotoFile => self.goto_file_under_cursor()?,
            Command::GotoWord => self.label_visible_words(),
            Command::ToggleSoftWrap => {
                self.config.editor.soft_wrap = !self.config.editor.soft_wrap;
                self.status(if self.config.editor.soft_wrap {
                    "soft wrap enabled"
                } else {
                    "soft wrap disabled"
                });
            }
            Command::ToggleWhitespace => {
                self.config.editor.render_whitespace = !self.config.editor.render_whitespace;
                self.status(if self.config.editor.render_whitespace {
                    "whitespace markers enabled"
                } else {
                    "whitespace markers disabled"
                });
            }
            Command::ToggleMarkdownRender => self.toggle_markdown_render(),
            Command::ToggleZen => self.toggle_maximized(MaximizedView::Zen),
            Command::ToggleFullscreen => self.toggle_maximized(MaximizedView::Fullscreen),
            Command::OpenExplorer => self.open_active_directory_explorer()?,
            Command::OpenWorkingDirectoryExplorer => self.open_explorer(None)?,
            Command::OpenFilePicker => self.open_project_picker()?,
            Command::OpenAllFilesPicker => self.open_all_files_picker()?,
            Command::OpenPathFilePicker => self.open_prompt(PromptKind::FinderPath),
            Command::OpenDirectoryFilePicker => self.open_directory_picker()?,
            Command::OpenFuzzyGrep => self.open_project_grep()?,
            Command::OpenDirectoryFuzzyGrep => self.open_directory_grep()?,
            Command::OpenSettings => self.open_settings_buffer(),
            Command::OpenThemeSettings => self.open_setting_values(SettingId::Theme),
            Command::OpenDirectoryEntry => self.open_directory_entry()?,
            Command::OpenChangedFile => self.open_changed_file()?,
            Command::StageAllChangedFiles => self.stage_all_changed_files(),
            Command::CheckoutBranch => self.checkout_selected_branch(),
            Command::CreateBranch => self.create_branch_prompt(),
            Command::DeleteBranch => self.delete_selected_branch(),
            Command::PullBranch => self.pull_current_branch(),
            Command::PushBranch => self.push_selected_branch(),
            Command::OpenWorktree => self.open_selected_worktree(),
            Command::CreateWorktree => self.create_branch_worktree_prompt(),
            Command::CreateNewWorktree => self.create_worktree_prompt(true),
            Command::RemoveWorktree => self.remove_selected_worktree(),
            Command::NextGitLogPage => self.next_git_log_page(),
            Command::PreviousGitLogPage => self.previous_git_log_page(),
            Command::OpenGitCommit => self.open_selected_git_commit(),
            Command::OpenWorkspaceSearchResult => self.open_workspace_search_result()?,
            Command::OpenParentDirectory => self.open_parent_directory()?,
            Command::RefreshDirectory => self.refresh_directory()?,
            Command::ToggleHiddenFiles => self.toggle_hidden_files()?,
            Command::ToggleDirectoryDetails => self.toggle_directory_details()?,
            Command::ChooseExplorerOrder => self.choose_explorer_order()?,
            Command::SplitVertical => self.split_window(Axis::Horizontal)?,
            Command::SplitHorizontal => self.split_window(Axis::Vertical)?,
            Command::Save => self.save(None, false)?,
            Command::ForceSave => self.save(None, true)?,
            Command::ShowHelp => self.open_help(),
            Command::ShowAbout => self.open_about(),
            Command::ShowTutorial => self.open_tutorial(None)?,
            Command::ActivateSetting => self.activate_selected_setting(),
            Command::FocusWindowLeft => self.focus_from_terminal_insert(-1, 0),
            Command::FocusWindowDown => self.focus_from_terminal_insert(0, 1),
            Command::FocusWindowUp => self.focus_from_terminal_insert(0, -1),
            Command::FocusWindowRight => self.focus_from_terminal_insert(1, 0),
            Command::NextWindow => self.next_window_from_terminal_insert(),
            Command::SwapWindow => self.swap_window(),
            Command::CloseWindow => self.close_pane(),
            Command::OnlyWindow => self.only_window(),
            Command::EqualizeWindows => self.equalize_panes(),
            Command::DeleteWordBackward if self.mode == Mode::Replace => {
                self.restore_replace_word()
            }
            Command::DeleteWordBackward => self.delete_word_backward(),
            Command::DeleteWordForward => self.delete_word_forward(),
            Command::DeleteToLineStart if self.mode == Mode::Replace => self.restore_replace_line(),
            Command::DeleteToLineStart => self.delete_to_line_start(),
            Command::DeleteToLineEnd => self.delete_to_line_end(),
            Command::DeleteCharBackward => {
                if self.mode != Mode::Replace {
                    self.edit_backspace();
                } else {
                    self.restore_replace_step();
                }
                self.refresh_explicit_completion_filter();
            }
            Command::DeleteCharForward => {
                self.edit_delete();
                self.refresh_explicit_completion_filter();
            }
            Command::InsertNewline if self.mode == Mode::Replace => self.replace_mode_text("\n"),
            Command::InsertNewline => self.edit_newline(),
            Command::InsertTab if self.mode == Mode::Replace => {
                self.replace_mode_text(&" ".repeat(self.config.editor.tab_width.max(1)))
            }
            Command::InsertTab => self.insert_indentation(),
            Command::InsertLiteralTab if self.mode == Mode::Replace => self.replace_mode_text("\t"),
            Command::InsertLiteralTab => self.insert_char('\t'),
            Command::CommitUndoCheckpoint => {
                let buffer_id = self.active().buffer;
                self.buffers[buffer_id].commit_undo_group();
            }
            Command::GotoDefinition => {
                let position = self.lsp_cursor();
                self.lsp_goto(RequestKind::Definition(position));
            }
            Command::GotoDeclaration => {
                let position = self.lsp_cursor();
                self.lsp_goto(RequestKind::Declaration(position));
            }
            Command::GotoTypeDefinition => {
                let position = self.lsp_cursor();
                self.lsp_goto(RequestKind::TypeDefinition(position));
            }
            Command::GotoImplementation => {
                let position = self.lsp_cursor();
                self.lsp_goto(RequestKind::Implementation(position));
            }
            Command::GotoReferences => {
                let position = self.lsp_cursor();
                self.lsp_goto(RequestKind::References(position));
            }
            Command::ShowDocumentation => self.lsp_hover(),
            Command::DocumentSymbols => self.lsp_document_symbols(),
            Command::WorkspaceSymbols => self.lsp_workspace_symbols(),
            Command::Diagnostics => self.open_diagnostics_picker(),
            Command::RenameSymbol => self.lsp_rename_prompt(),
            Command::CodeAction => self.lsp_code_actions(),
            Command::TriggerCompletion => {
                if self.has_language_server() && !matches!(self.mode, Mode::Insert | Mode::Replace)
                {
                    self.enter_insert(false);
                }
                self.start_explicit_lsp_completion();
            }
            Command::JumpBackward => self.jump(true),
            Command::JumpForward => self.jump(false),
            Command::JumpBackwardBuffer => self.jump_in(true, true),
            Command::JumpForwardBuffer => self.jump_in(false, true),
            Command::NewBuffer => self.open_scratch_buffer(),
            Command::OpenBufferPicker => self.open_buffer_picker(),
            Command::GlobalSearch => {
                self.open_prompt(PromptKind::GlobalSearch(SearchMode::Insensitive))
            }
            Command::GlobalSearchRegex => {
                self.open_prompt(PromptKind::GlobalSearch(SearchMode::Regex))
            }
            Command::OpenTerminal => self.open_terminal(None),
            Command::OpenTerminalFileDirectory
            | Command::OpenTerminalDirectoryRoot
            | Command::OpenTerminalSelectedDirectory
            | Command::OpenTerminalSessionDirectory => self.action_failed(format!(
                "{} is a typed command",
                command.metadata().description.to_lowercase()
            )),
            Command::OpenTerminalList => self.open_terminal_list(),
            Command::RenameTerminal => self.open_terminal_rename_prompt(),
            Command::ShowTerminal => self.action_failed(format!(
                "{} is a typed command",
                command.metadata().description.to_lowercase()
            )),
            Command::LeaveTerminal => self.leave_terminal(),
            Command::CopyTerminalOutput => self.copy_terminal_output(),
            Command::SendToTerminal => self.send_to_terminal(),
            Command::ClipboardPasteAfter => self.clipboard_paste(false),
            Command::ClipboardPasteBefore => self.clipboard_paste(true),
            Command::ClipboardYank => self.clipboard_yank(),
            Command::ClipboardPaste => self.clipboard_paste_any(),
            Command::ShellPipe => {
                self.action_failed(format!(
                    "{} is not available",
                    command.metadata().description
                ));
            }
        }
        self.reveal_active_selection_from_folds();
        Ok(())
    }

    fn handle_search_prompt(&mut self, key: KeyStroke) -> Result<()> {
        if key.modifiers.contains(Modifiers::CONTROL) {
            return self.handle_prompt_control(key);
        }
        if key.modifiers.contains(Modifiers::ALT) {
            match key.code {
                KeyCode::Char('b') => {
                    self.command_cursor = prompt_word_backward(&self.command, self.command_cursor);
                }
                KeyCode::Char('f') => {
                    self.command_cursor = prompt_word_forward(&self.command, self.command_cursor);
                }
                _ => {}
            }
            return Ok(());
        }
        match key.code {
            KeyCode::Escape => self.close_prompt(),
            KeyCode::Enter => {
                let kind = self.prompt_kind;
                let value = if kind == PromptKind::ExternalProgram {
                    self.selected_program_choice()
                        .map_or_else(|| self.command.clone(), |choice| choice.launch_value())
                } else {
                    self.command.clone()
                };
                if let PromptKind::SettingValue(setting) = kind {
                    let value = match setting.descriptor().value_type {
                        SettingType::Integer { minimum, maximum } => {
                            let Ok(number) = value.trim().parse::<usize>() else {
                                self.action_failed(format!(
                                    "{} must be an integer from {minimum} through {maximum}",
                                    setting.descriptor().title
                                ));
                                return Ok(());
                            };
                            SettingValue::Integer(number)
                        }
                        SettingType::Text => SettingValue::Text(value),
                        SettingType::Grammar
                        | SettingType::Boolean
                        | SettingType::Theme
                        | SettingType::WorkspaceMode
                        | SettingType::ExplorerSort => {
                            self.action_failed("this setting must be chosen from its list");
                            return Ok(());
                        }
                    };
                    if let Err(error) = setting.validate(&value, &self.config) {
                        self.action_failed(error.to_string());
                        return Ok(());
                    }
                    if self.persist_selected_setting(setting, value) {
                        self.close_prompt();
                    }
                    return Ok(());
                }
                // Taken before `close_prompt`, which drops them so an abandoned
                // prompt cannot leave a file, or a branch, waiting behind the
                // next one.
                let target = self.external_target.take();
                #[cfg(unix)]
                let session_rename_target = self.session_rename_target.take();
                #[cfg(unix)]
                let session_number_target = self.session_number_target.take();
                let terminal_rename_target = self.terminal_rename_target.take();
                let start_point = self.git_branch_start.take();
                let worktree_start = self.git_worktree_start.take();
                let worktree_new_branch = self.git_worktree_new_branch.take();
                let worktree_upstream = self.git_worktree_upstream.take();
                self.close_prompt();
                if kind == PromptKind::ExternalProgram {
                    if let Some(target) = target {
                        self.open_externally(&target, value);
                    }
                } else if kind == PromptKind::Rename {
                    if value.trim().is_empty() {
                        self.action_failed("rename needs a new name");
                    } else {
                        self.lsp_rename(value);
                    }
                } else if kind == PromptKind::SessionRename {
                    #[cfg(unix)]
                    if let Some(target) = session_rename_target {
                        self.rename_session(target, value);
                    }
                    #[cfg(not(unix))]
                    self.action_failed("persistent mode is not yet supported on this platform");
                } else if kind == PromptKind::SessionNumber {
                    #[cfg(unix)]
                    if let Some(target) = session_number_target {
                        match parse_session_number(&value) {
                            Ok(number) => self.number_session(target, number),
                            Err(error) => self.action_failed(error),
                        }
                    }
                    #[cfg(not(unix))]
                    self.action_failed("persistent mode is not yet supported on this platform");
                } else if kind == PromptKind::TerminalRename {
                    if let Some(id) = terminal_rename_target {
                        if value.trim().is_empty() {
                            self.action_failed("a terminal name cannot be empty");
                        } else {
                            self.rename_terminal_id(id, &value);
                        }
                        // Back to where the rename was asked for. The list is
                        // rebuilt after the rename so it shows the new name,
                        // and no pane has changed what it is showing. An empty
                        // list has nothing to return to, and its own "no
                        // terminals" message would bury the failure that got
                        // here.
                        if !self.terminals.is_empty() {
                            self.open_terminal_list();
                        }
                    }
                } else if kind == PromptKind::NewBranch {
                    if let Some(start_point) = start_point {
                        self.create_branch(value, start_point);
                    }
                } else if kind == PromptKind::NewWorktreeBranch {
                    let name = value.trim().to_owned();
                    if name.is_empty() {
                        self.action_failed("a new worktree branch needs a name");
                    } else if let Some(start) = worktree_start {
                        self.git_worktree_start = Some(start);
                        self.git_worktree_new_branch = Some(name);
                        self.git_worktree_upstream = worktree_upstream;
                        self.open_prompt(PromptKind::WorktreeDestination);
                    }
                } else if kind == PromptKind::WorktreeDestination {
                    if let Some(start) = worktree_start {
                        self.create_worktree(value, start, worktree_new_branch, worktree_upstream);
                    }
                } else if kind == PromptKind::JoinDelimiter {
                    // Deliberately before the shared empty-value refusal below:
                    // an empty delimiter is the default answer, not a mistake.
                    self.join_selections(&value);
                } else if let PromptKind::GlobalSearch(mode) = kind {
                    if value.is_empty() {
                        self.action_failed("global search pattern is empty");
                    } else {
                        self.open_global_search(&value, mode);
                    }
                } else if let PromptKind::TerminalSearch(mode) = kind {
                    if value.is_empty() {
                        self.action_failed("terminal search pattern is empty");
                    } else {
                        self.search_terminal_review(&value, mode);
                    }
                } else if kind == PromptKind::FinderPath {
                    if value.is_empty() {
                        self.action_failed("finder path is empty");
                    } else if let Err(error) = self
                        .open_finder_path(Path::new(unclosed_or_complete_quoted_path(value.trim())))
                    {
                        self.action_failed(error.to_string());
                    }
                } else if let PromptKind::FilterSelections { keep } = kind {
                    if value.is_empty() {
                        self.action_failed("filter pattern is empty");
                    } else {
                        self.filter_selections(keep, &value);
                    }
                } else {
                    // Reaching here means `kind` is one of the three search
                    // prompts: everything else was matched above. Wrapped in
                    // `CommandState`/`report_completed_action` the same way a
                    // key-bound search (`n`, `N`, `*`, `#`) already is, so a
                    // failed or empty search echoes on the interaction line
                    // instead of only reaching the retained notification.
                    let state = CommandState::capture(self);
                    if value.is_empty() {
                        self.action_failed("search pattern is empty");
                    } else if let PromptKind::Search(mode) = kind {
                        let region = self.scoping_region();
                        self.commit_search(SearchQuery {
                            pattern: value,
                            mode,
                            region,
                            forward: true,
                        });
                    } else {
                        // The Vim grammar's directional single-match search.
                        let forward = kind == PromptKind::SearchForward;
                        self.search = SearchQuery {
                            pattern: value,
                            mode: SearchMode::Sensitive,
                            region: None,
                            forward,
                        };
                        self.find_search(forward, true, 1);
                    }
                    let (spelling, description) = match kind {
                        PromptKind::Search(SearchMode::Insensitive) => {
                            ("s", "Search for text, ignoring case")
                        }
                        PromptKind::Search(SearchMode::Sensitive) => {
                            ("S", "Search for text, matching case")
                        }
                        PromptKind::Search(SearchMode::Regex) => {
                            ("/", "Search with a regular expression")
                        }
                        PromptKind::SearchForward => ("/", "Search forward"),
                        PromptKind::SearchBackward => ("?", "Search backward"),
                        _ => unreachable!(
                            "handle_search_prompt's catch-all only reaches search prompt kinds: {kind:?}"
                        ),
                    };
                    let outcome = state.outcome(self, CommandOutcomeHint::Infer);
                    self.report_completed_action(spelling, description, outcome);
                }
            }
            KeyCode::Backspace => {
                prompt_backspace(&mut self.command, &mut self.command_cursor);
                self.command_selection = 0;
            }
            KeyCode::Delete => {
                prompt_delete(&mut self.command, self.command_cursor);
                self.command_selection = 0;
            }
            KeyCode::Left => self.command_cursor = self.command_cursor.saturating_sub(1),
            KeyCode::Right => {
                self.command_cursor = (self.command_cursor + 1).min(self.command.chars().count());
            }
            KeyCode::Home => self.command_cursor = 0,
            KeyCode::End => self.command_cursor = self.command.chars().count(),
            KeyCode::Up | KeyCode::BackTab if self.prompt_kind == PromptKind::ExternalProgram => {
                self.command_selection = self.command_selection.saturating_sub(1);
            }
            KeyCode::Down if self.prompt_kind == PromptKind::ExternalProgram => {
                let last = self.matching_program_choices().len().saturating_sub(1);
                self.command_selection = (self.command_selection + 1).min(last);
            }
            KeyCode::Tab if self.prompt_kind == PromptKind::ExternalProgram => {
                self.open_program_actions();
            }
            KeyCode::Up | KeyCode::BackTab if self.prompt_kind == PromptKind::FinderPath => {
                self.command_selection = self.command_selection.saturating_sub(1);
            }
            KeyCode::Down if self.prompt_kind == PromptKind::FinderPath => {
                let last = self
                    .finder_path_hints()
                    .map_or(0, |hints| hints.len().saturating_sub(1));
                self.command_selection = (self.command_selection + 1).min(last);
            }
            KeyCode::Tab if self.prompt_kind == PromptKind::FinderPath => {
                self.complete_selected_finder_path();
            }
            KeyCode::Char(ch) => {
                prompt_insert(&mut self.command, self.command_cursor, ch);
                self.command_cursor += 1;
                self.command_selection = 0;
            }
            _ => {}
        }
        Ok(())
    }

    /// Cached programs still reachable from what has been typed.
    pub fn matching_programs(&self) -> Vec<&str> {
        if self.prompt_kind != PromptKind::ExternalProgram {
            return Vec::new();
        }
        self.programs.matching(&self.command)
    }

    /// Selectable external-program rows, with the configured default first.
    pub fn matching_program_choices(&self) -> Vec<ProgramChoice> {
        if self.prompt_kind != PromptKind::ExternalProgram {
            return Vec::new();
        }
        let query = self.command.trim();
        let system = external_open::system_default_program();
        let custom_default = self.programs.default_program();
        let default = custom_default.or(system);
        let mut choices = Vec::new();
        let mut push = |program: &str, is_system: bool| {
            if !query.is_empty() && !program.starts_with(query)
                || choices
                    .iter()
                    .any(|choice: &ProgramChoice| choice.program == program)
            {
                return;
            }
            choices.push(ProgramChoice {
                program: program.to_owned(),
                system: is_system,
                remembered: self
                    .programs
                    .programs()
                    .iter()
                    .any(|known| known == program),
                is_default: default == Some(program),
            });
        };
        if let Some(program) = default {
            push(program, system == Some(program));
        }
        if let Some(program) = system {
            push(program, true);
        }
        for program in self.programs.programs() {
            push(program, false);
        }
        choices
    }

    fn selected_program_choice(&self) -> Option<ProgramChoice> {
        self.matching_program_choices()
            .get(self.command_selection)
            .cloned()
    }

    fn open_program_actions(&mut self) {
        let Some(choice) = self.selected_program_choice() else {
            return;
        };
        let mut actions = Vec::new();
        if choice.remembered {
            actions.push(ProgramAction::Delete);
        }
        if !choice.is_default {
            actions.push(ProgramAction::SetDefault);
        }
        if actions.is_empty() {
            self.status("the system opener is already the default");
            return;
        }
        self.program_action_menu = Some(ProgramActionMenu {
            choice,
            actions,
            selected: 0,
        });
    }

    fn handle_program_action_key(&mut self, key: KeyStroke) -> Result<()> {
        let control = key.modifiers.contains(Modifiers::CONTROL);
        match (key.code, control) {
            (KeyCode::Escape, _) | (KeyCode::Char('c'), true) | (KeyCode::Tab, _) => {
                self.program_action_menu = None;
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                let menu = self.program_action_menu.as_mut().unwrap();
                if !menu.actions.is_empty() {
                    menu.selected = (menu.selected + 1) % menu.actions.len();
                }
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) | (KeyCode::BackTab, _) => {
                let menu = self.program_action_menu.as_mut().unwrap();
                if !menu.actions.is_empty() {
                    menu.selected = (menu.selected + menu.actions.len() - 1) % menu.actions.len();
                }
            }
            (KeyCode::Enter, _) => {
                let chosen = self.program_action_menu.as_ref().and_then(|menu| {
                    menu.selected_action()
                        .map(|action| (menu.choice.clone(), action))
                });
                if let Some((choice, action)) = chosen {
                    self.run_program_action(&choice, action);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn run_program_action(&mut self, choice: &ProgramChoice, action: ProgramAction) {
        let result = match action {
            ProgramAction::Delete => self
                .programs
                .forget(&choice.program)
                .map(|_| format!("forgot {}", choice.program)),
            ProgramAction::SetDefault => {
                let default = (!choice.system).then_some(choice.program.as_str());
                self.programs
                    .set_default(default)
                    .map(|()| format!("{} is now the default application", choice.program))
            }
        };
        match result {
            Ok(status) => {
                self.program_action_menu = None;
                self.command.clear();
                self.command_cursor = 0;
                self.command_selection = 0;
                self.status(status);
            }
            Err(error) => self.error_from("Runyte", "Program choice failed", error.to_string()),
        }
    }

    fn handle_prompt_control(&mut self, key: KeyStroke) -> Result<()> {
        match key.code {
            KeyCode::Char('c') => self.close_prompt(),
            KeyCode::Char('s') => return self.save(None, false),
            KeyCode::Char('b') => self.command_cursor = self.command_cursor.saturating_sub(1),
            KeyCode::Char('f') => {
                self.command_cursor = (self.command_cursor + 1).min(self.command.chars().count());
            }
            KeyCode::Char('a') => self.command_cursor = 0,
            KeyCode::Char('e') => self.command_cursor = self.command.chars().count(),
            KeyCode::Char('w') => {
                let start = prompt_word_backward(&self.command, self.command_cursor);
                prompt_delete_range(&mut self.command, start, self.command_cursor);
                self.command_cursor = start;
            }
            KeyCode::Char('u') => {
                prompt_delete_range(&mut self.command, 0, self.command_cursor);
                self.command_cursor = 0;
            }
            KeyCode::Char('k') => {
                let end = self.command.chars().count();
                prompt_delete_range(&mut self.command, self.command_cursor, end);
            }
            KeyCode::Char('h') => {
                prompt_backspace(&mut self.command, &mut self.command_cursor);
            }
            KeyCode::Char('d') => {
                prompt_delete(&mut self.command, self.command_cursor);
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn open_prompt(&mut self, kind: PromptKind) {
        if self.mode != Mode::Command {
            self.prompt_origin_mode = self.mode;
        }
        self.mode = Mode::Command;
        self.prompt_kind = kind;
        self.command.clear();
        self.command_cursor = 0;
        self.command_selection = 0;
        self.program_action_menu = None;
        self.prompt_revision = self.prompt_revision.wrapping_add(1);
    }

    pub(super) fn open_prompt_with_value(&mut self, kind: PromptKind, value: String) {
        self.open_prompt(kind);
        self.command_cursor = value.chars().count();
        self.command = value;
        self.prompt_revision = self.prompt_revision.wrapping_add(1);
    }

    pub(super) fn close_prompt(&mut self) {
        #[cfg(unix)]
        let session_manager_return_target = self.session_manager_return_target.take();
        // Still set only when the prompt is being abandoned: a submitted
        // rename takes its own target before closing.
        let abandoned_terminal_rename = self.terminal_rename_target.take().is_some();
        self.command.clear();
        self.command_cursor = 0;
        self.command_selection = 0;
        self.program_action_menu = None;
        self.prompt_kind = PromptKind::Command;
        // Abandoning the prompt abandons the file with it, so a later prompt
        // cannot inherit a target nobody asked about. The branch a new one
        // would have started from is dropped for the same reason.
        self.external_target = None;
        #[cfg(unix)]
        {
            self.session_rename_target = None;
            self.session_number_target = None;
        }
        self.git_branch_start = None;
        self.git_worktree_start = None;
        self.git_worktree_new_branch = None;
        self.git_worktree_upstream = None;
        self.mode = self.grammar.preferred_mode().unwrap_or(Mode::Normal);
        if abandoned_terminal_rename && !self.terminals.is_empty() {
            self.open_terminal_list();
        }
        #[cfg(unix)]
        if let Some(target) = session_manager_return_target {
            self.rebuild_workspace_picker();
            if let Some(selected) = self
                .workspace_rows
                .iter()
                .position(|row| row.project_root == target)
                && let Some(picker) = self.list.as_mut()
            {
                picker.selected = selected;
            }
            self.request_selected_workspace_preview();
        }
    }

    pub(super) fn normal_mode_selection(
        &self,
        buffer_id: usize,
        selection: Selection,
        semantics: SelectionSemantics,
        mode: Mode,
    ) -> Selection {
        if matches!(mode, Mode::Insert | Mode::Replace) {
            let buffer = &self.buffers[buffer_id];
            let vim = self.grammar.kind() == crate::command::GrammarKind::Vim;
            selection.transform(|range| {
                let mut head = range.head;
                if vim {
                    let row = buffer.offset_to_row(head);
                    let start = buffer.line_to_offset(row);
                    if head > start {
                        head -= 1;
                    }
                }
                Range::point(buffer.clamp_offset(head, false))
            })
        } else if self.grammar.kind() == crate::command::GrammarKind::Vim
            && mode == Mode::Select
            && matches!(
                semantics,
                SelectionSemantics::HalfOpen | SelectionSemantics::VimLinewise
            )
        {
            self.vim_half_open_to_inclusive(selection).collapse()
        } else {
            selection.collapse()
        }
    }

    pub(super) fn enter_normal_mode(&mut self) {
        let buffer_id = self.active().buffer;
        if matches!(self.mode, Mode::Insert | Mode::Replace) {
            self.buffers[buffer_id].commit_undo_group();
        }
        let selection = self.normal_mode_selection(
            buffer_id,
            self.active().selection.clone(),
            self.active().selection_semantics(),
            self.mode,
        );
        self.active_mut().replace_selection(selection);
        self.replace_session = None;
        self.mode = Mode::Normal;
        match self.grammar.kind() {
            crate::command::GrammarKind::Runyte => self
                .active_mut()
                .mark_selection_semantics(SelectionSemantics::Runyte),
            crate::command::GrammarKind::Vim => self
                .active_mut()
                .mark_selection_semantics(SelectionSemantics::HalfOpen),
        }
        self.grammar.reset();
        self.dismiss_popups();
    }

    fn enter_insert(&mut self, after: bool) {
        let buffer_id = self.active().buffer;
        let buffer = &self.buffers[buffer_id];
        let selection = self.active().selection.transform(|range| {
            if after {
                let row = buffer.offset_to_row(range.to());
                let row_end = buffer.line_to_offset(row) + buffer.line_len(row);
                Range::point((range.to() + 1).min(row_end))
            } else {
                Range::point(range.from())
            }
        });
        self.active_mut().replace_selection(selection);
        self.buffers[buffer_id].begin_undo_group();
        self.mode = Mode::Insert;
    }

    /// Executes one validated semantic invocation. Key bindings, the command
    /// palette, and headless callers all enter through this boundary.
    /// User input and environmental operation failures are returned as typed
    /// outcomes; `Result::Err` is reserved for a fatal invariant failure at
    /// the application boundary.
    pub fn execute(&mut self, invocation: CommandInvocation) -> Result<CommandOutcome> {
        // Protocol and headless semantic commands bypass `handle_input`, but
        // they can move the originating selection just as surely as a key.
        self.invalidate_all_partial_guards();
        self.action_feedback = None;
        // Direct and protocol callers get the same semantic error reset as a
        // terminal key. Retained feedback lives in the notification center;
        // status_error remains only part of command-outcome inference.
        self.status_error = false;
        let before = CommandState::capture(self);
        let (id, parameters, execution, unavailable) = invocation.into_parts();
        if let Some(unavailable) = unavailable {
            let CommandId::Editor(command) = id else {
                unreachable!("only editor registry bindings carry availability")
            };
            let (availability, reason) = match unavailable {
                CommandUnavailable::Planned(reason) => ("planned", reason),
                CommandUnavailable::Unsupported(reason) => ("unsupported", reason),
            };
            self.mark_unavailable(format!(
                "{} is {availability}: {reason}",
                command.metadata().description
            ));
            return Ok(CommandOutcome::Unavailable(self.status.clone()));
        }
        let hint = match id {
            CommandId::Colon(ColonCommand::Format) => {
                if self.has_language_server() {
                    CommandOutcomeHint::Asynchronous
                } else {
                    CommandOutcomeHint::Unavailable
                }
            }
            CommandId::Colon(ColonCommand::LspRestart | ColonCommand::LspStatus) => {
                if self.ports.has_lsp() {
                    CommandOutcomeHint::Asynchronous
                } else {
                    CommandOutcomeHint::Unavailable
                }
            }
            CommandId::Editor(EditorCommand::ShellPipe) => CommandOutcomeHint::Unavailable,
            CommandId::Editor(EditorCommand::RenameSymbol) => {
                if self.has_language_server() {
                    CommandOutcomeHint::Infer
                } else {
                    CommandOutcomeHint::Unavailable
                }
            }
            CommandId::Editor(
                EditorCommand::GotoDefinition
                | EditorCommand::GotoDeclaration
                | EditorCommand::GotoTypeDefinition
                | EditorCommand::GotoImplementation
                | EditorCommand::GotoReferences
                | EditorCommand::ShowDocumentation
                | EditorCommand::DocumentSymbols
                | EditorCommand::WorkspaceSymbols
                | EditorCommand::CodeAction
                | EditorCommand::TriggerCompletion,
            ) => {
                if self.has_language_server() {
                    CommandOutcomeHint::Asynchronous
                } else {
                    CommandOutcomeHint::Unavailable
                }
            }
            _ => CommandOutcomeHint::Infer,
        };
        let execution_result = match id {
            CommandId::Editor(command) => {
                self.execute_editor_invocation(command, parameters, execution)
            }
            CommandId::Colon(command) => self.execute_colon_invocation(command, parameters),
        };
        if let Err(error) = execution_result {
            if let Some(refusal) = error.downcast_ref::<CommandRefusal>() {
                self.failure_from(
                    refusal.class(),
                    "Runyte",
                    "Command failed",
                    error.to_string(),
                );
            } else {
                self.error_from("Runyte", "Command failed", error.to_string());
            }
            return Ok(CommandOutcome::UserError(self.status.clone()));
        }
        self.retire_detached_ephemeral_buffers();
        let outcome = before.outcome(self, hint);
        Ok(outcome)
    }

    fn execute_editor_invocation(
        &mut self,
        command: EditorCommand,
        parameters: InvocationParameters,
        execution: CommandExecutionContext,
    ) -> Result<()> {
        match (command, parameters) {
            (EditorCommand::ShowHelp, InvocationParameters::Help(request)) => {
                match request {
                    HelpInvocation::ActiveView => {
                        return self.execute_editor_command(EditorCommand::ShowHelp);
                    }
                    HelpInvocation::Manual(topic) => self.open_manual(topic.as_deref()),
                }
                return Ok(());
            }
            (EditorCommand::ShowTutorial, InvocationParameters::OptionalText(request)) => {
                return self.open_tutorial(request.as_deref());
            }
            (EditorCommand::Save, InvocationParameters::OptionalPath(path)) => {
                return match path {
                    Some(path) => self.save(Some(path), false),
                    None => self.execute_editor_command(EditorCommand::Save),
                };
            }
            (EditorCommand::ForceSave, InvocationParameters::OptionalPath(path)) => {
                return self.save(path, true);
            }
            (EditorCommand::SplitVertical, InvocationParameters::OptionalPath(path)) => {
                return match path {
                    Some(path) => self.split(Axis::Horizontal, Some(path)),
                    None => self.execute_editor_command(EditorCommand::SplitVertical),
                };
            }
            (EditorCommand::SplitHorizontal, InvocationParameters::OptionalPath(path)) => {
                return match path {
                    Some(path) => self.split(Axis::Vertical, Some(path)),
                    None => self.execute_editor_command(EditorCommand::SplitHorizontal),
                };
            }
            (EditorCommand::OpenThemeSettings, InvocationParameters::OptionalText(name)) => {
                // A named theme is a direct switch persisted to the config;
                // without one, the settings menu is the chooser.
                return match name {
                    Some(name) => self.set_theme(&name),
                    None => self.execute_editor_command(EditorCommand::OpenThemeSettings),
                };
            }
            (EditorCommand::OpenTerminal, InvocationParameters::OptionalText(command)) => {
                self.open_terminal(command);
                return Ok(());
            }
            (
                EditorCommand::OpenTerminalFileDirectory,
                InvocationParameters::OptionalText(command),
            ) => {
                self.open_terminal_file_directory(command);
                return Ok(());
            }
            (
                EditorCommand::OpenTerminalDirectoryRoot,
                InvocationParameters::OptionalText(command),
            ) => {
                self.open_terminal_directory_root(command);
                return Ok(());
            }
            (
                EditorCommand::OpenTerminalSelectedDirectory,
                InvocationParameters::OptionalText(command),
            ) => {
                self.open_terminal_selected_directory(command);
                return Ok(());
            }
            (
                EditorCommand::OpenTerminalSessionDirectory,
                InvocationParameters::OptionalText(Some(target)),
            ) => {
                self.open_terminal_session_directory(&target);
                return Ok(());
            }
            (EditorCommand::ShowTerminal, InvocationParameters::OptionalText(Some(target))) => {
                self.show_terminal_target(&target);
                return Ok(());
            }
            (EditorCommand::RenameTerminal, InvocationParameters::OptionalText(Some(name))) => {
                self.rename_active_terminal(&name);
                return Ok(());
            }
            (EditorCommand::SendToTerminal, InvocationParameters::OptionalText(target)) => {
                self.send_to_terminal_target(target.as_deref());
                return Ok(());
            }
            (
                EditorCommand::OpenWorkingDirectoryExplorer,
                InvocationParameters::OptionalPath(path),
            ) => {
                return match path {
                    Some(path) => self.open_explorer(Some(path)),
                    None => {
                        self.execute_editor_command(EditorCommand::OpenWorkingDirectoryExplorer)
                    }
                };
            }
            (EditorCommand::OpenPathFilePicker, InvocationParameters::OptionalPath(path)) => {
                return match path {
                    Some(path) => self.open_finder_path(&path),
                    // A bare `:file-picker-path` asks the same question the
                    // key does rather than refusing for want of an argument.
                    None => {
                        self.open_prompt(PromptKind::FinderPath);
                        Ok(())
                    }
                };
            }
            (_, InvocationParameters::None) => {}
            _ => anyhow::bail!("invalid parameters for {}", command.metadata().name),
        }

        if let Some(character) = execution.character() {
            let repeated_character_command = matches!(
                command,
                EditorCommand::FindNextChar
                    | EditorCommand::FindPreviousChar
                    | EditorCommand::FindTillNextChar
                    | EditorCommand::FindTillPreviousChar
            );
            let repetitions = if self.macro_replay.is_some() && repeated_character_command {
                1
            } else {
                execution.repetitions()
            };
            if let Some(id) = self.active_terminal() {
                match command {
                    EditorCommand::FindNextChar
                    | EditorCommand::FindPreviousChar
                    | EditorCommand::FindTillNextChar
                    | EditorCommand::FindTillPreviousChar => {
                        let forward = matches!(
                            command,
                            EditorCommand::FindNextChar | EditorCommand::FindTillNextChar
                        );
                        let till = matches!(
                            command,
                            EditorCommand::FindTillNextChar | EditorCommand::FindTillPreviousChar
                        );
                        let (_, rows) = self.pane_cells(self.active_pane);
                        let scroll_offset = self.config.editor.scroll_offset;
                        let extend = self.mode == Mode::Select;
                        let mut found = true;
                        if let Some(session) = self.terminals.get_mut(id) {
                            for _ in 0..repetitions {
                                found &=
                                    session.find_review_character(character, forward, till, extend);
                            }
                            session.focus_review_selection(rows.max(1), scroll_offset);
                        }
                        self.terminals.enforce_memory_budget();
                        if !found {
                            self.action_failed(format!("character not found: {character}"));
                        }
                        self.defer_replayed_character_repetitions(
                            command,
                            character,
                            execution.repetitions().saturating_sub(repetitions),
                        )?;
                        return Ok(());
                    }
                    EditorCommand::ReplaceChar => {
                        self.action_failed(format!(
                            "replace character needs a buffer · {} shows this pane's again",
                            self.binding_label(EditorCommand::LeaveTerminal)
                        ));
                        return Ok(());
                    }
                    _ => {}
                }
            }
            match command {
                EditorCommand::ReplaceChar => self.replace_with_char(character),
                EditorCommand::FindNextChar => {
                    for _ in 0..repetitions {
                        self.find_character(character, true, false);
                    }
                }
                EditorCommand::FindPreviousChar => {
                    for _ in 0..repetitions {
                        self.find_character(character, false, false);
                    }
                }
                EditorCommand::FindTillNextChar => {
                    for _ in 0..repetitions {
                        self.find_character(character, true, true);
                    }
                }
                EditorCommand::FindTillPreviousChar => {
                    for _ in 0..repetitions {
                        self.find_character(character, false, true);
                    }
                }
                EditorCommand::SelectRegister => self.select_register(character),
                EditorCommand::RecordMacro => self.start_macro_recording(character),
                EditorCommand::ReplayMacro => {
                    self.replay_macro(character, execution.repetitions())?;
                }
                _ => unreachable!("validated invocation owns a character-taking command"),
            }
            if repeated_character_command {
                self.defer_replayed_character_repetitions(
                    command,
                    character,
                    execution.repetitions().saturating_sub(repetitions),
                )?;
            }
            return Ok(());
        }

        if let Some(line) = execution.count()
            && matches!(
                command,
                EditorCommand::MoveFileStart | EditorCommand::MoveFileEnd
            )
        {
            if let Some(id) = self.active_terminal() {
                let (_, rows) = self.pane_cells(self.active_pane);
                let scroll_offset = self.config.editor.scroll_offset;
                if let Some(session) = self.terminals.get_mut(id) {
                    session.goto_review_line(line, self.mode == Mode::Select);
                    session.focus_review_selection(rows.max(1), scroll_offset);
                }
                self.terminals.enforce_memory_budget();
            } else {
                self.goto_line(line);
            }
            return Ok(());
        }
        if command == EditorCommand::ReplayDefaultMacro {
            self.replay_macro(DEFAULT_MACRO_REGISTER, execution.repetitions())?;
            return Ok(());
        }
        let repetitions = if self.macro_replay.is_some() {
            1
        } else {
            execution.repetitions()
        };
        for _ in 0..repetitions {
            self.execute_editor_command(command)?;
        }
        let deferred = execution.repetitions().saturating_sub(repetitions);
        if deferred > 0 && !self.should_quit && self.workspace_switch.is_none() {
            let invocation =
                CommandInvocation::editor(command, CommandExecutionContext::default())?;
            self.defer_macro_command(invocation, deferred);
        }
        Ok(())
    }

    fn defer_replayed_character_repetitions(
        &mut self,
        command: EditorCommand,
        character: char,
        repetitions: usize,
    ) -> Result<()> {
        if repetitions == 0 {
            return Ok(());
        }
        let execution =
            CommandExecutionContext::resolved(std::num::NonZeroUsize::MIN, Some(character));
        let invocation = CommandInvocation::editor(command, execution)?;
        self.defer_macro_command(invocation, repetitions);
        Ok(())
    }

    fn execute_colon_invocation(
        &mut self,
        command: ColonCommand,
        parameters: InvocationParameters,
    ) -> Result<()> {
        self.execute_colon_invocation_for_workspace_platform(command, parameters, cfg!(unix))
    }

    pub(super) fn execute_colon_invocation_for_workspace_platform(
        &mut self,
        command: ColonCommand,
        parameters: InvocationParameters,
        platform_supports_persistent_sessions: bool,
    ) -> Result<()> {
        use ColonCommand as Colon;

        match (command, parameters) {
            (Colon::ChangeDirectory, InvocationParameters::Path(path)) => {
                self.change_directory(path)
            }
            (Colon::SessionAttach, InvocationParameters::Path(path)) => {
                if self.reject_unavailable_persistent_session(
                    platform_supports_persistent_sessions,
                    true,
                ) {
                    return Ok(());
                }
                if self.request_workspace_switch(path) {
                    self.should_quit = true;
                }
                Ok(())
            }
            (Colon::SessionList, InvocationParameters::None) => {
                if self.reject_unavailable_persistent_session(
                    platform_supports_persistent_sessions,
                    true,
                ) {
                    return Ok(());
                }
                #[cfg(unix)]
                {
                    self.workspace_previews.clear();
                    self.workspace_preview_target = None;
                    let mut picker = ListPicker::new("Sessions · loading…", Vec::new());
                    picker.primary_action = Some("attach".to_owned());
                    self.list = Some(picker);
                    self.request_workspace_refresh();
                }
                Ok(())
            }
            (Colon::SessionStop, InvocationParameters::OptionalPath(selector)) => {
                if self.reject_unavailable_persistent_session(
                    platform_supports_persistent_sessions,
                    true,
                ) {
                    return Ok(());
                }
                #[cfg(unix)]
                self.stop_session(selector.unwrap_or_else(|| self.project_root.clone()));
                #[cfg(not(unix))]
                let _ = selector;
                Ok(())
            }
            (Colon::SessionRename, InvocationParameters::SessionRename { workspace, name }) => {
                if self.reject_unavailable_persistent_session(
                    platform_supports_persistent_sessions,
                    true,
                ) {
                    return Ok(());
                }
                #[cfg(unix)]
                self.rename_session(workspace, name);
                #[cfg(not(unix))]
                let _ = (workspace, name);
                Ok(())
            }
            (Colon::Format, InvocationParameters::None) => {
                self.lsp_format();
                Ok(())
            }
            (Colon::CloseBuffer, InvocationParameters::None) => {
                self.close_active_buffer(false);
                Ok(())
            }
            (Colon::ForceCloseBuffer, InvocationParameters::None) => {
                self.close_active_buffer(true);
                Ok(())
            }
            (Colon::DiffThis, InvocationParameters::None) => {
                self.diff_this();
                Ok(())
            }
            (Colon::DiffDisk, InvocationParameters::None) => {
                self.diff_disk();
                Ok(())
            }
            (Colon::DiffOff, InvocationParameters::None) => {
                self.diff_off();
                Ok(())
            }
            (Colon::GitCommit, InvocationParameters::None) => {
                self.open_commit_message();
                Ok(())
            }
            (Colon::GitCancel, InvocationParameters::None) => {
                self.cancel_git();
                Ok(())
            }
            (Colon::GitBranches, InvocationParameters::None) => {
                self.open_git_branches();
                Ok(())
            }
            (Colon::GitWorktrees, InvocationParameters::None) => {
                self.open_git_worktrees();
                Ok(())
            }
            (Colon::GitLog, InvocationParameters::None) => {
                self.open_git_log();
                Ok(())
            }
            (Colon::GitSearchCommits, InvocationParameters::None) => {
                self.open_git_commit_search();
                Ok(())
            }
            (Colon::GitStashes, InvocationParameters::None) => {
                self.open_git_stashes();
                Ok(())
            }
            (Colon::GitStashTracked, InvocationParameters::OptionalText(name)) => {
                self.request_stash_create(StashScope::TrackedWorktree, name);
                Ok(())
            }
            (Colon::GitStashAll, InvocationParameters::OptionalText(name)) => {
                self.request_stash_create(StashScope::TrackedWorktreeAndIndex, name);
                Ok(())
            }
            (Colon::GitStashUntracked, InvocationParameters::OptionalText(name)) => {
                self.request_stash_create(StashScope::TrackedAndUntracked, name);
                Ok(())
            }
            (Colon::GitStashApply, InvocationParameters::None) => {
                self.request_selected_stash(false);
                Ok(())
            }
            (Colon::GitStashDrop, InvocationParameters::None) => {
                self.request_selected_stash(true);
                Ok(())
            }
            (Colon::GitStageHunk, InvocationParameters::None) => {
                self.request_partial_hunk(DiffScope::Unstaged, false);
                Ok(())
            }
            (Colon::GitUnstageHunk, InvocationParameters::None) => {
                self.request_partial_hunk(DiffScope::Staged, false);
                Ok(())
            }
            (Colon::GitStageLines, InvocationParameters::None) => {
                self.request_partial_hunk(DiffScope::Unstaged, true);
                Ok(())
            }
            (Colon::GitBlame, InvocationParameters::None) => {
                self.request_git_blame(false);
                Ok(())
            }
            (Colon::GitBlameFile, InvocationParameters::None) => {
                self.request_git_blame(true);
                Ok(())
            }
            (Colon::GitDiscard, InvocationParameters::None) => {
                self.discard_git_changes();
                Ok(())
            }
            (Colon::GitDiff, InvocationParameters::None) => {
                self.open_git_diff();
                Ok(())
            }
            (Colon::GitDiffSideBySide, InvocationParameters::None) => {
                self.open_git_file_comparison();
                Ok(())
            }
            (Colon::GitIndex, InvocationParameters::None) => {
                self.open_git_index();
                Ok(())
            }
            (Colon::GitRefresh, InvocationParameters::None) => {
                self.refresh_git();
                Ok(())
            }
            (Colon::GitStatus, InvocationParameters::None) => {
                self.open_git_status();
                Ok(())
            }
            (Colon::GitStage, InvocationParameters::None) => {
                self.stage_files(true);
                Ok(())
            }
            (Colon::GitUnstage, InvocationParameters::None) => {
                self.stage_files(false);
                Ok(())
            }
            (Colon::Grammar, InvocationParameters::Grammar(grammar)) => {
                match grammar {
                    Some(grammar) => self.select_grammar(grammar),
                    None => self.status(format!("active grammar: {}", self.grammar.kind())),
                }
                Ok(())
            }
            (Colon::LspTrust, InvocationParameters::None) => {
                self.open_lsp_trust();
                Ok(())
            }
            (Colon::LspRestart, InvocationParameters::OptionalText(language)) => {
                if !self.lsp_send(LspCommand::Restart(language)) {
                    self.status("language servers are not running");
                }
                Ok(())
            }
            (Colon::LspStatus, InvocationParameters::None) => {
                if !self.lsp_send(LspCommand::Status) {
                    self.status("language servers are not running");
                }
                Ok(())
            }
            (Colon::LogOpen, InvocationParameters::None) => {
                self.open_log_buffer();
                Ok(())
            }
            (Colon::Notifications, InvocationParameters::None) => {
                self.open_notifications_buffer();
                Ok(())
            }
            (Colon::Path, InvocationParameters::None) => {
                self.open_path_popup();
                Ok(())
            }
            (Colon::ServiceHealth, InvocationParameters::None) => {
                self.open_service_health();
                Ok(())
            }
            (Colon::Open, InvocationParameters::Path(path)) => self.open_file(path),
            (Colon::Detach, InvocationParameters::None) => {
                self.request_detach();
                Ok(())
            }
            (Colon::Quit, InvocationParameters::None) => {
                self.request_view_quit(false);
                Ok(())
            }
            (Colon::ForceQuit, InvocationParameters::None) => {
                self.request_view_quit(true);
                Ok(())
            }
            (Colon::QuitAll, InvocationParameters::None) => {
                self.request_quit(false, ":qa!");
                Ok(())
            }
            (Colon::ForceQuitAll, InvocationParameters::None) => {
                self.request_quit(true, ":qa!");
                Ok(())
            }
            (Colon::QuitHere, InvocationParameters::None) => {
                self.request_quit_here(false);
                Ok(())
            }
            (Colon::ForceQuitHere, InvocationParameters::None) => {
                self.request_quit_here(true);
                Ok(())
            }
            (Colon::Reload, InvocationParameters::None) => self.reload_active(),
            (Colon::ResizeRight, InvocationParameters::PaneResize(delta)) => {
                self.resize_pane_edge(1, 0, delta)
            }
            (Colon::ResizeLeft, InvocationParameters::PaneResize(delta)) => {
                self.resize_pane_edge(-1, 0, delta)
            }
            (Colon::ResizeTop, InvocationParameters::PaneResize(delta)) => {
                self.resize_pane_edge(0, -1, delta)
            }
            (Colon::ResizeBottom, InvocationParameters::PaneResize(delta)) => {
                self.resize_pane_edge(0, 1, delta)
            }
            (Colon::WriteQuit, InvocationParameters::None) => {
                let buffer = self.active().buffer;
                let commit_message = self.buffers[buffer].is_commit_message();
                self.save(None, false)?;
                // Writing a commit message consumes that workflow buffer and
                // returns this pane to its origin. Applying `:q` afterwards
                // would unexpectedly close the origin (or the whole editor),
                // so `:wq` deliberately means the same thing as `:w` there.
                if !commit_message && self.active().buffer == buffer && !self.status_error {
                    self.request_view_quit(false);
                }
                Ok(())
            }
            (Colon::WriteBufferClose, InvocationParameters::None) => {
                let buffer = self.active().buffer;
                let commit_message = self.buffers[buffer].is_commit_message();
                self.save(None, false)?;
                if !commit_message && self.active().buffer == buffer && !self.status_error {
                    self.close_active_buffer(false);
                }
                Ok(())
            }
            _ => anyhow::bail!("invalid parameters for colon command {command:?}"),
        }
    }

    /// Refuses a command that needs a persistent session host.
    ///
    /// `require_persistent_mode` separates the two callers. A `session`
    /// command addresses the host and nothing else, so standalone mode refuses
    /// it outright. The worktree list's `Enter` is a Git view's action that
    /// only its attachment half needs a host for, so it passes `false` and
    /// keeps its own message about what was refused.
    pub(super) fn reject_unavailable_persistent_session(
        &mut self,
        platform_supports_persistent_sessions: bool,
        require_persistent_mode: bool,
    ) -> bool {
        let CommandAvailability::Unavailable(reason) = persistent_session_availability(
            platform_supports_persistent_sessions,
            !require_persistent_mode || self.persistent_session,
        ) else {
            return false;
        };
        self.action_failed(reason);
        true
    }

    pub(super) fn select_grammar(&mut self, kind: crate::command::GrammarKind) {
        let grammar = match ActiveGrammar::new(kind) {
            Ok(grammar) => grammar,
            Err(error) => {
                self.action_failed(error.to_string());
                return;
            }
        };
        self.grammar = grammar;
        match kind {
            crate::command::GrammarKind::Runyte => {
                self.active_mut()
                    .mark_selection_semantics(SelectionSemantics::Runyte);
                self.enter_normal_mode();
            }
            crate::command::GrammarKind::Vim => {
                unreachable!("removed grammar cannot be selected")
            }
        }
        self.status(format!("active grammar: {kind}"));
    }

    /// Test convenience for exercising the typed parser and semantic entry
    /// point without manufacturing terminal input.
    #[cfg(test)]
    pub(super) fn execute_command(&mut self, command: &str) -> Result<CommandOutcome> {
        match parse_colon_command(command) {
            Ok(invocation) => self.execute(invocation),
            Err(error) => {
                self.action_failed(error.to_string());
                Ok(CommandOutcome::UserError(self.status.clone()))
            }
        }
    }

    fn select_next_command(&mut self) {
        let count = self.command_hint_count();
        if count > 0 {
            self.command_selection = (self.command_selection + 1) % count;
        }
    }

    fn select_previous_command(&mut self) {
        let count = self.command_hint_count();
        if count > 0 {
            self.command_selection = (self.command_selection + count - 1) % count;
        }
    }

    /// Accepts the selected finder-path row. The whole prompt is one path, so
    /// nothing has to be quoted against a following argument.
    fn complete_selected_finder_path(&mut self) {
        let Some(hints) = self.finder_path_hints() else {
            return;
        };
        let Some(hint) = hints.get(self.command_selection) else {
            return;
        };
        self.command = hint.value.clone();
        self.command_cursor = self.command.chars().count();
        self.command_selection = 0;
    }

    pub(super) fn complete_selected_command(&mut self) {
        if let Some(hints) = self.matching_path_hints() {
            let Some(hint) = hints.get(self.command_selection) else {
                return;
            };
            let Some((name, argument)) = self.command.split_once(char::is_whitespace) else {
                return;
            };
            let preferred_quote = argument
                .trim_start()
                .chars()
                .next()
                .filter(|character| matches!(character, '\'' | '"'));
            let (argument, cursor_before_closing_quote) =
                quote_path_hint(&hint.value, preferred_quote, hint.is_directory);
            self.command = format!("{name} {argument}");
            self.command_cursor =
                self.command.chars().count() - usize::from(cursor_before_closing_quote);
            self.command_selection = 0;
            return;
        }
        if let Some((name, argument)) = self.command.split_once(char::is_whitespace)
            && resolve_command(name)
                .is_some_and(|spec| spec.id == CommandId::Colon(ColonCommand::Grammar))
        {
            let argument = argument.trim();
            if let Some(grammar) = crate::command::GrammarKind::ALL
                .iter()
                .find(|grammar| grammar.name().starts_with(argument))
            {
                self.command = format!("{name} {grammar}");
                self.command_cursor = self.command.chars().count();
                self.command_selection = 0;
            }
            return;
        }
        if let Some((name, argument)) = self.command.split_once(char::is_whitespace)
            && resolve_command(name)
                .is_some_and(|spec| spec.id == CommandId::Editor(EditorCommand::ShowHelp))
        {
            let argument = argument.trim();
            if let Some(topic) = crate::manual::ManualTopic::ALL
                .iter()
                .find(|topic| topic.slug().starts_with(argument))
            {
                self.command = format!("{name} {}", topic.slug());
                self.command_cursor = self.command.chars().count();
                self.command_selection = 0;
            }
            return;
        }
        let matches = self.matching_commands();
        let Some(matched) = matches.get(self.command_selection).cloned() else {
            return;
        };
        let argument = self
            .command
            .split_once(char::is_whitespace)
            .map(|(_, argument)| argument.trim_start().to_owned());
        // Completing to the spelling the row shows, not the canonical name
        // behind it, so Tab never rewrites what someone deliberately typed.
        self.command = matched.name.to_owned();
        if matched.spec.arguments.accepts_arguments() {
            self.command.push(' ');
            if let Some(argument) = argument {
                self.command.push_str(&argument);
            }
        }
        self.command_cursor = self.command.chars().count();
        self.command_selection = 0;
    }
}

fn is_macro_replay_cancel(input: &InputEvent) -> bool {
    matches!(
        input,
        InputEvent::Key(KeyStroke {
            code: KeyCode::Escape,
            ..
        })
    ) || matches!(input, InputEvent::Key(key) if *key == KeyStroke::ctrl('c'))
}

/// Whether a key in a picker only edits its query.
///
/// Query editing is the one thing a picker does that does not read the list,
/// so it is also the only thing that may run against rows the ranker has
/// already replaced. Caret movement is counted with the rest: publishing a
/// held answer before it costs nothing and keeps the rule one line long.
fn edits_picker_query(key: KeyStroke) -> bool {
    let control = key.modifiers.contains(Modifiers::CONTROL);
    match key.code {
        KeyCode::Backspace | KeyCode::Delete => true,
        KeyCode::Char('w' | 'k') if control => true,
        KeyCode::Char(_) => !key
            .modifiers
            .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER),
        _ => false,
    }
}

fn edit_confirmation_text(input: &mut String, cursor: &mut usize, key: KeyStroke) {
    let mut characters = input.chars().collect::<Vec<_>>();
    *cursor = (*cursor).min(characters.len());
    match (key.code, key.modifiers) {
        (KeyCode::Char(character), modifiers)
            if !modifiers.intersects(Modifiers::CONTROL | Modifiers::ALT) =>
        {
            characters.insert(*cursor, character);
            *cursor += 1;
        }
        (KeyCode::Backspace, _) if *cursor > 0 => {
            *cursor -= 1;
            characters.remove(*cursor);
        }
        (KeyCode::Delete, _) if *cursor < characters.len() => {
            characters.remove(*cursor);
        }
        (KeyCode::Left, _) => *cursor = cursor.saturating_sub(1),
        (KeyCode::Right, _) => *cursor = (*cursor + 1).min(characters.len()),
        (KeyCode::Home, _) => *cursor = 0,
        (KeyCode::End, _) => *cursor = characters.len(),
        _ => return,
    }
    *input = characters.into_iter().collect();
}

fn insert_confirmation_text(input: &mut String, cursor: &mut usize, text: &str) {
    *cursor = (*cursor).min(input.chars().count());
    input.insert_str(char_to_byte(input, *cursor), text);
    *cursor += text.chars().count();
}
