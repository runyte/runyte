// SPDX-License-Identifier: MPL-2.0

//! Integrated terminal-session lifecycle, input, review, and list actions.

// Application-module dependencies:
use super::{
    App, ContentAlignment, EditorCommand, GeneratedViewIdentity, HashSet, InputGrammar, JumpLabels,
    KeyStroke, ListAction, ListPicker, Mode, OsString, Path, PathBuf, PickerItem, PromptKind,
    Range, Register, Result, SearchMode, SentTextUndo, TerminalId, TerminalOutput, TerminalRequest,
    TerminalSession, default_terminal_program, program_label, terminal_preview, terminal_refuses,
};

impl App {
    // -- Terminals ---------------------------------------------------------

    /// The terminal the active pane is showing, if it is showing one.
    pub fn active_terminal(&self) -> Option<TerminalId> {
        self.terminal_of_pane(self.active_pane)
    }

    pub fn terminal_of_pane(&self, pane_id: usize) -> Option<TerminalId> {
        self.panes
            .get(&pane_id)?
            .terminal
            .filter(|id| self.terminals.get(*id).is_some())
    }

    /// The inside of a pane, in cells.
    ///
    /// Geometry is only known once a frame has been prepared, so a terminal
    /// opened before the first one starts at a conventional size and is
    /// resized on the next frame. Guessing is safe here in a way it is not
    /// elsewhere: `TIOCSWINSZ` is the child's only notion of size, and it
    /// arrives before the child has drawn anything.
    pub(super) fn pane_cells(&self, pane_id: usize) -> (usize, usize) {
        match self.areas.get(&pane_id) {
            Some(area) if area.width >= 3 && area.height >= 3 => {
                (usize::from(area.width - 2), usize::from(area.height - 2))
            }
            _ => (80, 24),
        }
    }

    /// Runs a program in the active pane.
    ///
    /// The argument is a command line, split the way a shell would split it.
    /// With nothing given it runs `$SHELL`, which is what "open a terminal"
    /// means everywhere else.
    pub(super) fn open_terminal(&mut self, command: Option<String>) {
        self.open_terminal_at(command, self.working_directory.clone());
    }

    pub(super) fn open_terminal_at(&mut self, command: Option<String>, directory: PathBuf) {
        let request = match self.terminal_request(command, directory) {
            Ok(request) => request,
            Err(error) => {
                self.error(error);
                return;
            }
        };
        let (columns, rows) = self.pane_cells(self.active_pane);
        let label = request.label.clone();
        match self.terminals.open(request, columns, rows) {
            Ok(id) => {
                self.show_terminal(id);
                self.status(format!(
                    "{label} · Ctrl-\\ leaves input, {} returns to the buffer",
                    self.binding_label(EditorCommand::LeaveTerminal)
                ));
            }
            Err(error) => self.error(format!("cannot start {label}: {error}")),
        }
    }

    fn terminal_request(
        &self,
        command: Option<String>,
        directory: PathBuf,
    ) -> std::result::Result<TerminalRequest, String> {
        let Some(command) = command
            .map(|command| command.trim().to_owned())
            .filter(|command| !command.is_empty())
        else {
            let program = default_terminal_program();
            let label = program_label(&program);
            return Ok(TerminalRequest {
                program,
                // A login shell would re-read the person's profile inside an
                // editor that already inherited it. An interactive one is what
                // a terminal pane is for.
                arguments: Vec::new(),
                directory,
                label,
            });
        };
        let Some(words) = shlex::split(&command) else {
            return Err(format!("cannot read {command} as a command line"));
        };
        let Some((program, arguments)) = words.split_first() else {
            return Err("no command to run".to_owned());
        };
        Ok(TerminalRequest {
            program: OsString::from(program),
            arguments: arguments.to_vec(),
            directory,
            label: program_label(&OsString::from(program)),
        })
    }

    pub(super) fn open_terminal_file_directory(&mut self, command: Option<String>) {
        let Some(path) = self.active_buffer().path.as_deref() else {
            self.error("the active view has no file directory");
            return;
        };
        if self.active_buffer().is_directory() {
            self.error("use :terminal-directory-root for a directory view");
            return;
        }
        let Some(directory) = path.parent().map(Path::to_path_buf) else {
            self.error("the active file has no parent directory");
            return;
        };
        self.open_terminal_at(command, directory);
    }

    pub(super) fn open_terminal_directory_root(&mut self, command: Option<String>) {
        if !self.active_buffer().is_directory() {
            self.error("the active view is not a directory buffer");
            return;
        }
        let directory = self
            .active_buffer()
            .path
            .clone()
            .expect("directory buffers have paths");
        self.open_terminal_at(command, directory);
    }

    pub(super) fn open_terminal_selected_directory(&mut self, command: Option<String>) {
        let selected = match self.selected_directory_entry() {
            Ok(Some(path)) => path,
            Ok(None) => {
                self.error("there is no directory entry on this row");
                return;
            }
            Err(error) => {
                self.error(error.to_string());
                return;
            }
        };
        if !selected.is_dir() {
            self.error("the selected entry is not a directory");
            return;
        }
        self.open_terminal_at(command, selected);
    }

    pub(super) fn open_terminal_session_directory(&mut self, target: &str) {
        let id = match self.terminals.resolve(target) {
            Ok(id) => id,
            Err(error) => {
                self.error(error);
                return;
            }
        };
        let directory = self
            .terminals
            .get(id)
            .expect("resolved terminal exists")
            .directory()
            .to_path_buf();
        self.open_terminal_at(None, directory);
    }

    /// Points the active pane at a terminal without disturbing its buffer.
    pub(super) fn show_terminal(&mut self, id: TerminalId) {
        if self.terminals.get(id).is_none() {
            self.error("that terminal is gone");
            return;
        }
        if self.active_terminal() != Some(id) {
            self.push_jump();
        }
        self.move_terminal_to_pane(id, self.active_pane);
        self.last_terminal = Some(id);
        // Showing a terminal changes which content the pane owns; it is not
        // itself an input command. Preserve a review captured before the
        // session was hidden or moved, while a still-live terminal starts in
        // its natural input mode.
        self.settle_terminal_focus(id);
    }

    /// Makes one pane the sole visible owner of a terminal session.
    ///
    /// A PTY has one authoritative cell size. Rendering the same session in
    /// two differently sized panes would resize its emulator and child twice
    /// per frame, destructively retiring or truncating rows. Showing it here
    /// therefore moves the view and reveals every previous pane's underlying
    /// buffer instead of cloning the view.
    pub(super) fn move_terminal_to_pane(&mut self, id: TerminalId, pane_id: usize) -> bool {
        if self.terminals.get(id).is_none() || !self.panes.contains_key(&pane_id) {
            return false;
        }
        for (other_id, pane) in &mut self.panes {
            if *other_id != pane_id && pane.terminal == Some(id) {
                pane.terminal = None;
            }
        }
        let (columns, rows) = self.pane_cells(pane_id);
        self.panes.get_mut(&pane_id).unwrap().terminal = Some(id);
        let session = self.terminals.get_mut(id).unwrap();
        session.resize(columns, rows);
        session.mark_viewed();
        true
    }

    /// Shows the pane's buffer again, leaving the child running.
    pub(super) fn leave_terminal(&mut self) {
        let Some(id) = self.active_terminal() else {
            self.error("this pane is not showing a terminal");
            return;
        };
        self.push_jump();
        if let Some(pane) = self.panes.get_mut(&self.active_pane) {
            pane.terminal = None;
        }
        self.mode = Mode::Normal;
        let name = self
            .terminals
            .get(id)
            .map_or_else(|| id.to_string(), TerminalSession::name);
        self.status(format!(
            "{name} is still running · {} lists it",
            self.binding_label(EditorCommand::OpenTerminalList)
        ));
    }

    pub(super) fn show_terminal_target(&mut self, target: &str) {
        match self.terminals.resolve(target) {
            Ok(id) => self.show_terminal(id),
            Err(error) => self.error(error),
        }
    }

    pub(super) fn rename_active_terminal(&mut self, name: &str) {
        let Some(id) = self.active_terminal() else {
            self.error("this pane is not showing a terminal");
            return;
        };
        let name = name.trim();
        match self
            .terminals
            .get_mut(id)
            .expect("active terminal exists")
            .rename(Some(name.to_owned()))
        {
            Ok(()) => self.status(format!("terminal {id} named {name}")),
            Err(error) => self.error(error),
        }
    }

    pub(super) fn open_terminal_rename_prompt(&mut self) {
        if self.active_terminal().is_none() {
            self.error("this pane is not showing a terminal");
            return;
        }
        self.open_prompt(PromptKind::Command);
        self.command = "terminal-rename ".to_owned();
        self.command_cursor = self.command.chars().count();
    }

    pub(super) fn close_terminal_id(&mut self, id: TerminalId) {
        let name = self
            .terminals
            .get(id)
            .map_or_else(|| id.to_string(), TerminalSession::name);
        self.terminals.close(id);
        for pane in self.panes.values_mut() {
            if pane.terminal == Some(id) {
                pane.terminal = None;
            }
        }
        if self.last_terminal == Some(id) {
            self.last_terminal = None;
        }
        self.mode = Mode::Normal;
        self.status(format!("{name} ended"));
    }

    pub(super) fn open_terminal_list(&mut self) {
        if self.terminals.is_empty() {
            self.status(format!(
                "no terminals · {} starts one",
                self.binding_label(EditorCommand::OpenTerminal)
            ));
            return;
        }
        let mut items = Vec::new();
        let mut actions = Vec::new();
        for session in self.terminals.iter() {
            let mut detail = format!(
                "#{} · running · {}",
                session.id(),
                session.directory().display()
            );
            if session.user_name().is_some()
                && let Some(title) = session.child_title()
            {
                detail.push_str(&format!(" · child {title}"));
            }
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
            items.push(
                PickerItem::new(session.display_name(), detail, actions.len())
                    .with_preview(terminal_preview(session)),
            );
            actions.push(ListAction::Terminal(session.id()));
        }
        self.list_actions = actions;
        self.list = Some(
            ListPicker::new("Terminals", items)
                .with_preview("Output")
                .as_manager("show", "Tab", "actions"),
        );
        self.terminal_action_menu = None;
    }

    /// Freezes the session's output into an ordinary read-only buffer.
    ///
    /// This is what Normal mode over a terminal honestly offers. A live grid
    /// cannot be searched, selected, or yanked with Runyte's own semantics —
    /// the cells are a picture of the child's text, not the text — but a
    /// frozen copy is real text, and everything works on it.
    pub(super) fn copy_terminal_output(&mut self) {
        let Some(id) = self.active_terminal().or(self.last_terminal) else {
            self.error("no terminal to copy output from");
            return;
        };
        let Some(session) = self.terminals.get(id) else {
            self.error("that terminal is gone");
            return;
        };
        let name = session.name();
        let text = session.plain_text();
        let truncated = session.alternate_screen();
        self.open_virtual_page(
            GeneratedViewIdentity::Named(format!("terminal-output:{id}")),
            format!("[{name} output]"),
            &text,
            ContentAlignment::default(),
        );
        if truncated {
            self.status("full-screen program · only the visible screen has text behind it");
        } else {
            self.status(format!("{name} output copied into a buffer"));
        }
    }

    /// Sends the selection — or the whole buffer, when nothing is selected —
    /// to a terminal as one bracketed paste.
    ///
    /// The point of composing in a buffer rather than in the terminal: a
    /// prompt written with multiple cursors is real Runyte text right up until
    /// it is sent, which is the one way modal editing can reach a program that
    /// owns its own input area.
    pub(super) fn send_to_terminal(&mut self) {
        self.send_to_terminal_target(None);
    }

    pub(super) fn send_to_terminal_target(&mut self, target: Option<&str>) {
        let id = if let Some(target) = target {
            match self.terminals.resolve(target) {
                Ok(id) => id,
                Err(error) => {
                    self.error(error);
                    return;
                }
            }
        } else if let Some(id) = self.send_target_terminal() {
            id
        } else {
            self.error(format!(
                "no terminal to send to · {} starts one",
                self.binding_label(EditorCommand::OpenTerminal)
            ));
            return;
        };
        if self.active_terminal() == Some(id) {
            self.error("that is the terminal you are in");
            return;
        }
        let text = if self.active().selection.ranges().iter().all(Range::is_empty) {
            self.active_buffer().text().to_string()
        } else {
            self.selection_text()
        };
        if text.trim().is_empty() {
            self.error("nothing to send");
            return;
        }
        let characters = text.chars().count();
        let Some(session) = self.terminals.get_mut(id) else {
            self.error("that terminal is gone");
            return;
        };
        if !session.live() {
            self.error("that terminal's program has exited");
            return;
        }
        let name = session.name();
        if session.send_text(&text) {
            self.status(format!("sent {characters} characters to {name}"));
        } else {
            self.error("terminal input queue is full or the paste exceeds 1 MiB");
        }
    }

    /// Which terminal a send goes to.
    ///
    /// A terminal on screen is the one being worked with, so it wins over the
    /// one used last; the single terminal in the editor wins over nothing.
    fn send_target_terminal(&self) -> Option<TerminalId> {
        let mut visible = self
            .panes
            .iter()
            .filter(|(pane_id, _)| **pane_id != self.active_pane)
            .filter_map(|(_, pane)| pane.terminal)
            .filter(|id| self.terminals.get(*id).is_some());
        if let Some(id) = visible.next()
            && visible.next().is_none()
        {
            return Some(id);
        }
        self.last_terminal
            .filter(|id| self.terminals.get(*id).is_some())
            .or_else(|| (self.terminals.len() == 1).then(|| self.terminals.ids()[0]))
    }

    /// Applies output a child produced.
    pub fn apply_terminal_output(&mut self, output: TerminalOutput) {
        self.apply_terminal_output_observed(output, true);
    }

    /// Applies child output while distinguishing a visible attached frontend
    /// from a headless persistent host. A pane association alone is not proof
    /// that anyone saw output while its client was detached.
    pub fn apply_terminal_output_observed(&mut self, output: TerminalOutput, observed: bool) {
        let id = output.id();
        let ended = matches!(output, TerminalOutput::Exited { .. });
        self.terminals.apply(output);
        if ended {
            self.finish_terminal(id);
            return;
        }
        let visible = self.panes.values().any(|pane| pane.terminal == Some(id));
        if observed
            && visible
            && let Some(session) = self.terminals.get_mut(id)
        {
            session.mark_viewed();
        }
    }

    /// Retires a child that ended on its own.
    ///
    /// Terminal sessions are live processes, not post-mortem buffers. Ending
    /// one reveals each displaying pane's backing buffer and never changes
    /// the pane layout.
    fn finish_terminal(&mut self, id: TerminalId) {
        let Some(session) = self.terminals.get(id) else {
            return;
        };
        let name = session.name();
        let message = match session.exit_code() {
            Some(Some(code)) if code != 0 => format!("{name} exited with {code}"),
            _ => format!("{name} exited"),
        };
        let was_active = self.active_terminal() == Some(id);
        let manager_open = self
            .list
            .as_ref()
            .is_some_and(|list| list.title == "Terminals");
        let terminal_panes = self
            .panes
            .iter()
            .filter_map(|(pane_id, pane)| (pane.terminal == Some(id)).then_some(*pane_id))
            .collect::<Vec<_>>();

        self.terminals.close(id);
        if self.last_terminal == Some(id) {
            self.last_terminal = None;
        }
        if self
            .terminal_action_menu
            .as_ref()
            .is_some_and(|menu| menu.id == id)
        {
            self.terminal_action_menu = None;
        }
        for pane_id in terminal_panes {
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.terminal = None;
            }
        }

        if was_active {
            if self.mode == Mode::Command {
                self.close_prompt();
            } else {
                self.mode = Mode::Normal;
            }
        }
        if manager_open {
            if self.terminals.is_empty() {
                self.list = None;
                self.list_actions.clear();
            } else {
                self.open_terminal_list();
            }
        }
        self.status(message);
    }

    /// Marks only sessions whose output is present in the frame an attached
    /// observer is about to receive. Hidden sessions retain their activity.
    pub fn mark_visible_terminals_viewed(&mut self) {
        let visible = self
            .panes
            .values()
            .filter_map(|pane| pane.terminal)
            .collect::<HashSet<_>>();
        for id in visible {
            if let Some(session) = self.terminals.get_mut(id) {
                session.mark_viewed();
            }
        }
    }

    /// Routes one keystroke to the child of a terminal in Insert mode.
    pub(super) fn handle_terminal_key(&mut self, id: TerminalId, key: KeyStroke) -> Result<()> {
        let Some(session) = self.terminals.get_mut(id) else {
            self.mode = Mode::Normal;
            return Ok(());
        };
        if !session.live() {
            self.mode = Mode::Normal;
            self.error("this terminal's program has exited");
            return Ok(());
        }
        // A key with no tty encoding — a bare modifier, a media key — is not
        // an error and not an edit. Dropping it is what a terminal does.
        let _ = session.send_key(key);
        Ok(())
    }

    /// What a Normal-mode command does when the pane is showing a terminal.
    ///
    /// Returns whether the terminal consumed it. Everything a terminal has no
    /// answer for is refused rather than allowed to reach the pane's buffer:
    /// the buffer is the document behind the terminal, and editing it from a
    /// view that does not show it would be an invisible change to a file.
    pub(super) fn execute_terminal_command(
        &mut self,
        id: TerminalId,
        command: EditorCommand,
        transient_line_selection: bool,
    ) -> bool {
        use EditorCommand as Command;

        let (_, rows) = self.pane_cells(self.active_pane);
        let page = rows.max(1);
        let scroll_offset = self.config.editor.scroll_offset;
        let Some(session) = self.terminals.get_mut(id) else {
            return false;
        };
        match command {
            Command::EnterNormalMode if self.mode == Mode::Insert => {
                // Terminal Normal is a live, non-input state of its own. The
                // first Ctrl-\ leaves the child keyboard without freezing its
                // output; a second press below captures review. Keeping those
                // transitions separate also gives pane focus a safe mode to
                // enter without accidentally creating a snapshot.
                session.scroll_to_live();
                let _ = session;
                self.mode = Mode::Normal;
                self.grammar.reset();
                self.dismiss_popups();
                self.status("terminal normal · Ctrl-\\ enters review · i returns to input");
                true
            }
            Command::EnterNormalMode => {
                session.begin_review();
                // Unconditionally, the way Escape collapses in a file. A line
                // selection has already handed the mode back by the time this
                // runs, so a Select-mode test here would leave `x` selected.
                session.collapse_review_selection();
                session.focus_review_selection(page, scroll_offset);
                let _ = session;
                self.terminals.enforce_memory_budget();
                self.enter_normal_mode();
                true
            }
            Command::EnterSelectMode => {
                session.begin_review();
                self.mode = if self.mode == Mode::Select {
                    session.collapse_review_selection();
                    Mode::Normal
                } else {
                    Mode::Select
                };
                session.focus_review_selection(page, scroll_offset);
                let _ = session;
                self.terminals.enforce_memory_budget();
                true
            }
            Command::CollapseSelection => {
                session.collapse_review_selection();
                session.focus_review_selection(page, scroll_offset);
                self.mode = Mode::Normal;
                let _ = session;
                self.terminals.enforce_memory_budget();
                true
            }
            Command::KeepPrimarySelection => {
                session.keep_primary_review_selection();
                session.focus_review_selection(page, scroll_offset);
                let _ = session;
                self.terminals.enforce_memory_budget();
                self.status("kept primary selection");
                true
            }
            Command::SelectLine | Command::SelectLineUp => {
                session
                    .select_review_line(command == Command::SelectLine, self.line_select.is_some());
                session.focus_review_selection(page, scroll_offset);
                if self.line_select.is_none() {
                    self.line_select = Some(self.mode);
                    self.mode = Mode::Select;
                }
                let _ = session;
                self.terminals.enforce_memory_budget();
                true
            }
            Command::CopySelectionDown | Command::CopySelectionUp => {
                let copied = session.copy_review_selection(command == Command::CopySelectionDown);
                session.focus_review_selection(page, scroll_offset);
                let _ = session;
                self.terminals.enforce_memory_budget();
                if !copied {
                    self.error("no room for another cursor");
                }
                true
            }
            Command::GotoWord => {
                let targets = session.visible_review_word_targets(page);
                let _ = session;
                self.terminals.enforce_memory_budget();
                match JumpLabels::with_visible_lengths(targets) {
                    Some(labels) => {
                        self.status(format!("jump to word: {} labels", labels.len()));
                        self.jump = Some(labels);
                    }
                    None => self.error("no words on screen to jump to"),
                }
                true
            }
            Command::MoveLeft
            | Command::MoveRight
            | Command::MoveUp
            | Command::MoveDown
            | Command::MoveLineStart
            | Command::MoveLineEnd
            | Command::MoveFirstNonWhitespace
            | Command::MoveFileStart
            | Command::MoveFileEnd
            | Command::MoveWordForward
            | Command::MoveWordBackward
            | Command::MoveWordEnd
            | Command::MoveLongWordForward
            | Command::MoveLongWordBackward
            | Command::MoveLongWordEnd
            | Command::GotoNextParagraph
            | Command::GotoPreviousParagraph
            | Command::PageUp
            | Command::PageDown
            | Command::HalfPageUp
            | Command::HalfPageDown
            | Command::GotoWindowTop
            | Command::GotoWindowCenter
            | Command::GotoWindowBottom => {
                use crate::terminal::ReviewMotion;
                let motion = match command {
                    Command::MoveLeft => ReviewMotion::Left,
                    Command::MoveRight => ReviewMotion::Right,
                    Command::MoveUp => ReviewMotion::Up,
                    Command::MoveDown => ReviewMotion::Down,
                    Command::MoveLineStart => ReviewMotion::LineStart,
                    Command::MoveLineEnd => ReviewMotion::LineEnd,
                    Command::MoveFirstNonWhitespace => ReviewMotion::FirstNonWhitespace,
                    Command::MoveFileStart => ReviewMotion::FileStart,
                    Command::MoveFileEnd => ReviewMotion::FileEnd,
                    Command::MoveWordForward => ReviewMotion::WordForward,
                    Command::MoveWordBackward => ReviewMotion::WordBackward,
                    Command::MoveWordEnd => ReviewMotion::WordEnd,
                    Command::MoveLongWordForward => ReviewMotion::LongWordForward,
                    Command::MoveLongWordBackward => ReviewMotion::LongWordBackward,
                    Command::MoveLongWordEnd => ReviewMotion::LongWordEnd,
                    Command::GotoNextParagraph => ReviewMotion::NextParagraph,
                    Command::GotoPreviousParagraph => ReviewMotion::PreviousParagraph,
                    Command::PageUp => ReviewMotion::PageUp,
                    Command::PageDown => ReviewMotion::PageDown,
                    Command::HalfPageUp => ReviewMotion::HalfPageUp,
                    Command::HalfPageDown => ReviewMotion::HalfPageDown,
                    Command::GotoWindowTop => ReviewMotion::WindowTop,
                    Command::GotoWindowCenter => ReviewMotion::WindowCenter,
                    Command::GotoWindowBottom => ReviewMotion::WindowBottom,
                    _ => unreachable!(),
                };
                session.move_review(motion, self.mode == Mode::Select);
                session.focus_review_selection(page, scroll_offset);
                let _ = session;
                self.terminals.enforce_memory_budget();
                true
            }
            Command::EnterInsertMode
            | Command::AppendAfter
            | Command::InsertLineStart
            | Command::InsertLineEnd
            | Command::OpenLineBelow
            | Command::OpenLineAbove => {
                session.scroll_to_live();
                self.mode = Mode::Insert;
                self.status("terminal input · Ctrl-\\ returns to normal mode");
                true
            }
            Command::ScrollViewUp => {
                session.begin_review();
                session.scroll_back(1);
                let _ = session;
                self.terminals.enforce_memory_budget();
                true
            }
            Command::ScrollViewDown => {
                session.begin_review();
                session.scroll_forward(1);
                let _ = session;
                self.terminals.enforce_memory_budget();
                true
            }
            Command::Search => {
                self.open_prompt(PromptKind::TerminalSearch(SearchMode::Insensitive));
                true
            }
            Command::SearchRegex => {
                self.open_prompt(PromptKind::TerminalSearch(SearchMode::Regex));
                true
            }
            Command::SearchNext | Command::RotateSelectionForward => {
                if !session.step_review_match(true) {
                    self.error("terminal review has no search matches");
                } else {
                    session.focus_review_selection(page, scroll_offset);
                }
                true
            }
            Command::SearchPrevious | Command::RotateSelectionBackward => {
                if !session.step_review_match(false) {
                    self.error("terminal review has no search matches");
                } else {
                    session.focus_review_selection(page, scroll_offset);
                }
                true
            }
            Command::Yank => {
                let mut text = session.review_selection_text();
                if transient_line_selection && !text.ends_with('\n') {
                    text.push('\n');
                }
                let _ = session;
                self.terminals.enforce_memory_budget();
                self.write_selected_register(Register {
                    text,
                    linewise: transient_line_selection,
                    directory: None,
                });
                self.mode = Mode::Normal;
                self.status("terminal review selection yanked");
                true
            }
            Command::ClipboardYank => {
                let mut text = session.review_selection_text();
                if transient_line_selection && !text.ends_with('\n') {
                    text.push('\n');
                }
                let _ = session;
                self.terminals.enforce_memory_budget();
                match self.ports.clipboard().write(&text) {
                    Ok(()) => self.status("terminal review selection yanked to system clipboard"),
                    Err(error) => self.error(error.to_string()),
                }
                true
            }
            Command::PasteAfter | Command::PasteBefore => {
                let _ = session;
                let register = self.read_selected_register();
                if register.text.is_empty() {
                    self.status("Runyte register is empty");
                    return true;
                }
                let sent = self
                    .terminals
                    .get_mut(id)
                    .is_some_and(|session| session.send_text(&register.text));
                if sent {
                    // A register paste is normally followed by input for the
                    // child (most often Enter), so hand the terminal back as
                    // soon as the paste reaches its queue. Buffer paste keeps
                    // its ordinary Normal-mode behavior in `paste_register`.
                    self.mode = Mode::Insert;
                    self.status("pasted Runyte register to terminal");
                } else {
                    self.error("terminal input queue is full or the paste exceeds 1 MiB");
                }
                true
            }
            Command::ClipboardPasteAfter | Command::ClipboardPasteBefore => {
                match self.ports.clipboard().read() {
                    Ok(text) if text.is_empty() => self.status("system clipboard is empty"),
                    Ok(text) => {
                        if let Some(session) = self.terminals.get_mut(id)
                            && !session.send_text(&text)
                        {
                            self.error("terminal input queue is full or the paste exceeds 1 MiB");
                        }
                    }
                    Err(error) => self.error(error.to_string()),
                }
                true
            }
            Command::Undo => {
                let outcome = session.undo_sent_text();
                let _ = session;
                self.terminals.enforce_memory_budget();
                match outcome {
                    SentTextUndo::Erased(characters) => self.status(format!(
                        "asked the child to erase {characters} pasted character{}",
                        if characters == 1 { "" } else { "s" }
                    )),
                    SentTextUndo::NothingSent => {
                        self.status("nothing Runyte sent is still this child's last input")
                    }
                    SentTextUndo::AlreadyRun => {
                        self.error("the paste ended a line and the child has already run it")
                    }
                    SentTextUndo::Refused => self.error("terminal input queue is full"),
                }
                true
            }
            Command::JumpBackward
            | Command::JumpForward
            | Command::JumpBackwardBuffer
            | Command::JumpForwardBuffer => false,
            _ if terminal_refuses(command) => {
                self.error(format!(
                    "{} needs a buffer · {} shows this pane's again",
                    command.metadata().description.to_lowercase(),
                    self.binding_label(EditorCommand::LeaveTerminal)
                ));
                true
            }
            _ => false,
        }
    }

    pub(super) fn search_terminal_review(&mut self, pattern: &str, mode: SearchMode) {
        let Some(id) = self.active_terminal() else {
            self.error("this pane is not showing a terminal");
            return;
        };
        let (_, rows) = self.pane_cells(self.active_pane);
        let scroll_offset = self.config.editor.scroll_offset;
        let session = self.terminals.get_mut(id).expect("active terminal exists");
        let result = session.search_review(pattern, mode == SearchMode::Regex);
        if result.as_ref().is_ok_and(|count| *count > 0) {
            session.focus_review_selection(rows.max(1), scroll_offset);
        }
        self.terminals.enforce_memory_budget();
        match result {
            Ok(0) => self.status("no terminal review matches"),
            Ok(count) => self.status(format!(
                "terminal review · {count} match{} · i returns to live output",
                if count == 1 { "" } else { "es" }
            )),
            Err(error) => self.error(format!("invalid terminal search: {error}")),
        }
    }
}
