// SPDX-License-Identifier: MPL-2.0

//! Interactive tutorial coordination over ordinary buffers and panes.

use super::*;

impl App {
    pub(super) fn open_tutorial(&mut self, request: Option<&str>) -> Result<()> {
        let request = request.map(str::trim).filter(|value| !value.is_empty());
        if let Some(value) = request
            && !matches!(value, "reset" | "sessions")
        {
            return Err(
                CommandRefusal::routine("tutorial accepts only `reset` or `sessions`").into(),
            );
        }
        if let Some(maximized) = self.maximized {
            return Err(CommandRefusal::routine(format!(
                "leave {} before opening the tutorial",
                maximized.view.label()
            ))
            .into());
        }

        if self.tutorial.is_none() {
            self.create_tutorial_view()?;
        }
        if !self.tutorial_view_is_live() {
            self.tutorial = None;
            self.create_tutorial_view()?;
        } else {
            self.restore_tutorial_view();
        }

        match request {
            Some("reset") => {
                let state = self.tutorial.as_mut().unwrap();
                state.lesson = 0;
                state.motion_hints = None;
                state.awaiting_reattach = false;
                state.explorer_buffer = None;
                state.terminal = None;
                state.last_action = None;
                self.reset_tutorial_scratch("", 0, true);
                self.open_tutorial_motion_picker();
            }
            Some("sessions") => {
                let state = self.tutorial.as_mut().unwrap();
                state.lesson = crate::tutorial::LAST_LESSON;
                state.motion_hints = Some(MotionHints::Both);
                state.awaiting_reattach = false;
                state.explorer_buffer = None;
                state.terminal = None;
                state.last_action = None;
                self.list = None;
                self.list_actions.clear();
                self.reset_tutorial_scratch("persistent tutorial token\n", 0, true);
            }
            None if self.tutorial.as_ref().unwrap().motion_hints.is_none() => {
                self.open_tutorial_motion_picker();
            }
            None => {}
            Some(_) => unreachable!(),
        }
        let exercise = self.tutorial.as_ref().unwrap().exercise_pane;
        self.activate_pane(exercise);
        self.refresh_tutorial_document();
        self.mode = Mode::Normal;
        Ok(())
    }

    fn create_tutorial_view(&mut self) -> Result<()> {
        let instruction_pane = self.active_pane;
        let instruction_buffer = self.open_virtual_page(
            GeneratedViewIdentity::Tutorial,
            crate::tutorial::TUTORIAL_NAME.to_owned(),
            "Runyte tutorial\n",
            ContentAlignment::default(),
        );
        self.split(Axis::Horizontal, None)?;
        let exercise_pane = self.active_pane;
        let mut scratch = Buffer::scratch();
        scratch.discard_changes_to("")?;
        self.buffers.push(scratch);
        self.syntax.push(None);
        let scratch_buffer = self.buffers.len() - 1;
        // The cloned pane starts on the instruction buffer. Record that real
        // position before retargeting so Alt-o/Alt-i can teach buffer history.
        self.push_jump();
        self.active_mut().retarget(scratch_buffer);
        self.tutorial = Some(TutorialState {
            lesson: 0,
            motion_hints: None,
            instruction_buffer,
            scratch_buffer,
            instruction_pane,
            exercise_pane,
            last_action: None,
            awaiting_reattach: false,
            explorer_buffer: None,
            terminal: None,
            scratch_selection: Selection::point(0),
            scratch_mode: Mode::Normal,
        });
        self.open_tutorial_motion_picker();
        self.refresh_tutorial_document();
        Ok(())
    }

    fn tutorial_view_is_live(&self) -> bool {
        self.tutorial.as_ref().is_some_and(|state| {
            self.panes.contains_key(&state.instruction_pane)
                && self.panes.contains_key(&state.exercise_pane)
                && !self.closed_buffers.contains(&state.instruction_buffer)
                && !self.closed_buffers.contains(&state.scratch_buffer)
        })
    }

    fn restore_tutorial_view(&mut self) {
        let Some(state) = self.tutorial.as_ref() else {
            return;
        };
        let instruction_pane = state.instruction_pane;
        let exercise_pane = state.exercise_pane;
        let instruction_buffer = state.instruction_buffer;
        let scratch_buffer = state.scratch_buffer;
        let lesson = state.lesson;
        let scratch_selection = state.scratch_selection.clone();
        let scratch_mode = state.scratch_mode;
        let explorer_buffer = state.explorer_buffer;
        let tutorial_terminal = state.terminal;

        if self.panes[&instruction_pane].buffer != instruction_buffer {
            let pane = self.panes.get_mut(&instruction_pane).unwrap();
            pane.retarget(instruction_buffer);
            pane.replace_selection(Selection::point(0));
        }
        if matches!(lesson, 16 | 17) {
            self.prepare_tutorial_jump_history(lesson);
            return;
        }
        if lesson == 12
            && let Some(explorer) = explorer_buffer
        {
            if self.panes[&exercise_pane].buffer != explorer {
                self.prepare_tutorial_explorer_history(explorer);
            }
            return;
        }
        if lesson == 14
            && let Some(terminal) = tutorial_terminal
            && self.terminals.get(terminal).is_some()
        {
            self.activate_pane(exercise_pane);
            self.show_terminal(terminal);
            return;
        }
        let expected_exercise_buffer = scratch_buffer;
        if self.panes[&exercise_pane].buffer == expected_exercise_buffer {
            return;
        }
        let selection = scratch_selection.transform(|range| {
            Range::new(
                self.buffers[scratch_buffer].clamp_offset(range.anchor, false),
                self.buffers[scratch_buffer].clamp_offset(range.head, false),
            )
        });
        let pane = self.panes.get_mut(&exercise_pane).unwrap();
        pane.retarget(scratch_buffer);
        pane.replace_selection(selection);
        self.mode = scratch_mode;
    }

    fn open_tutorial_motion_picker(&mut self) {
        let items = MotionHints::ALL
            .iter()
            .enumerate()
            .map(|(index, hints)| PickerItem::new(hints.label(), hints.detail(), index))
            .collect();
        self.list_actions = MotionHints::ALL
            .iter()
            .copied()
            .map(ListAction::TutorialMotionHints)
            .collect();
        self.list = Some(
            ListPicker::new("Motion keybindings shown in the tutorial", items).as_choice("to use"),
        );
    }

    pub(super) fn choose_tutorial_motion_hints(&mut self, hints: MotionHints) {
        let Some(state) = self.tutorial.as_mut() else {
            return;
        };
        state.motion_hints = Some(hints);
        state.lesson = 1;
        state.last_action = None;
        self.list = None;
        self.list_actions.clear();
        self.reset_tutorial_scratch("hello\n", 0, true);
        self.refresh_tutorial_document();
        self.status(format!(
            "tutorial will show {} motion keybindings",
            hints.label()
        ));
    }

    pub(super) fn note_tutorial_action(&mut self, id: CommandId, spelling: &str) {
        if let Some(state) = self.tutorial.as_mut() {
            state.last_action = Some((id, spelling.to_owned()));
        }
    }

    pub(super) fn reconcile_tutorial(&mut self) {
        if !self.tutorial_view_is_live() {
            self.tutorial = None;
            return;
        }
        let Some(state) = self.tutorial.as_ref() else {
            return;
        };
        if state.motion_hints.is_none() || state.awaiting_reattach {
            return;
        }
        let lesson = state.lesson;
        let scratch = state.scratch_buffer;
        let instruction_pane = state.instruction_pane;
        let exercise_pane = state.exercise_pane;
        let action = state.last_action.as_ref().map(|(id, _)| *id);
        let active_buffer = self.panes[&self.active_pane].buffer;
        let exercise_buffer = self.panes[&exercise_pane].buffer;
        let head = self.panes[&exercise_pane].head();
        let selections = self.panes[&exercise_pane].selection.len();
        let text = self.buffers[scratch].to_string();
        let tutorial_terminal = state.terminal;
        if exercise_buffer == scratch {
            let selection = self.panes[&exercise_pane].selection.clone();
            let mode = self.mode;
            let state = self.tutorial.as_mut().unwrap();
            state.scratch_selection = selection;
            state.scratch_mode = mode;
        }
        if lesson != 11 && exercise_buffer != scratch {
            return;
        }
        if lesson != 9 && self.active_pane != exercise_pane {
            return;
        }
        if lesson == 14
            && action == Some(CommandId::Editor(EditorCommand::OpenTerminalList))
            && let Some(terminal) = tutorial_terminal
            && let Some(selected) = self
                .list_actions
                .iter()
                .position(|action| matches!(action, ListAction::Terminal(id) if *id == terminal))
            && let Some(list) = self.list.as_mut()
        {
            list.selected = selected;
        }
        let complete = match lesson {
            1 => text == "Hi hello\n" && self.mode == Mode::Normal,
            2 => head == 0 && action == Some(CommandId::Editor(EditorCommand::MoveLineStart)),
            3 => text == "red \n" && self.mode == Mode::Normal,
            4 => {
                text == "north\nsouth\n"
                    && self.mode == Mode::Normal
                    && action == Some(CommandId::Editor(EditorCommand::DeleteSelection))
            }
            5 => text == "cat dog cat\n" && selections == 2 && self.mode == Mode::Select,
            6 => text == "fox dog fox\n" && self.mode == Mode::Normal,
            7 => text == "> one\n> two\n> three\n" && selections == 3 && self.mode == Mode::Normal,
            8 => {
                selections == 1
                    && action == Some(CommandId::Editor(EditorCommand::KeepPrimarySelection))
            }
            9 => {
                self.active_pane == instruction_pane
                    && action == Some(CommandId::Editor(EditorCommand::FocusWindowLeft))
            }
            10 => {
                self.active_pane == exercise_pane
                    && action == Some(CommandId::Editor(EditorCommand::FocusWindowRight))
            }
            11 => {
                exercise_buffer != scratch
                    && active_buffer == exercise_buffer
                    && self.buffers[exercise_buffer].is_directory()
                    && action == Some(CommandId::Editor(EditorCommand::OpenExplorer))
            }
            12 => {
                active_buffer == scratch
                    && action == Some(CommandId::Editor(EditorCommand::JumpBackwardBuffer))
            }
            13 => {
                self.terminal_of_pane(exercise_pane).is_some()
                    && action == Some(CommandId::Editor(EditorCommand::OpenTerminal))
            }
            14 => {
                tutorial_terminal.is_some_and(|terminal| self.terminals.get(terminal).is_none())
                    && self.terminal_of_pane(exercise_pane).is_none()
            }
            15 => {
                head == self.buffers[scratch].clamp_offset(self.buffers[scratch].len_chars(), false)
                    && action == Some(CommandId::Editor(EditorCommand::MoveFileEnd))
            }
            16 => head == 0 && action == Some(CommandId::Editor(EditorCommand::JumpBackward)),
            17 => {
                head == self.buffers[scratch].clamp_offset(self.buffers[scratch].len_chars(), false)
                    && action == Some(CommandId::Editor(EditorCommand::JumpForward))
            }
            _ => false,
        };
        if !complete {
            return;
        }
        if lesson == 13 {
            self.tutorial.as_mut().unwrap().terminal = self.terminal_of_pane(exercise_pane);
        }
        if lesson == 11 {
            self.tutorial.as_mut().unwrap().explorer_buffer = Some(exercise_buffer);
        }
        self.advance_tutorial_lesson(lesson + 1);
    }

    fn advance_tutorial_lesson(&mut self, lesson: u8) {
        let Some(state) = self.tutorial.as_mut() else {
            return;
        };
        state.lesson = lesson;
        state.last_action = None;
        match lesson {
            2 => self.reset_tutorial_scratch("alpha beta\n", 6, true),
            3 => self.reset_tutorial_scratch("red blue\n", 4, true),
            4 => self.reset_tutorial_scratch("north\ncenter\nsouth\n", 6, true),
            5 => self.reset_tutorial_scratch("cat dog cat\n", 0, true),
            7 => self.reset_tutorial_scratch("one\ntwo\nthree\n", 0, true),
            15 => {
                self.list = None;
                self.terminal_action_menu = None;
                self.reset_tutorial_scratch("first\nsecond\nthird\n", 0, true);
            }
            18 => self.reset_tutorial_scratch("persistent tutorial token\n", 0, true),
            _ => {}
        }
        self.refresh_tutorial_document();
    }

    fn prepare_tutorial_explorer_history(&mut self, explorer: usize) {
        let Some(state) = self.tutorial.as_ref() else {
            return;
        };
        let scratch = state.scratch_buffer;
        let exercise = state.exercise_pane;
        let scratch_selection = state.scratch_selection.clone();
        self.activate_pane(exercise);
        let pane = self.panes.get_mut(&exercise).unwrap();
        pane.jumps = JumpList::default();
        pane.retarget(scratch);
        pane.replace_selection(scratch_selection);
        self.push_jump();
        self.panes.get_mut(&exercise).unwrap().retarget(explorer);
    }

    pub(super) fn prepare_tutorial_jump_history(&mut self, lesson: u8) {
        debug_assert!(matches!(lesson, 16 | 17));
        let Some(state) = self.tutorial.as_ref() else {
            return;
        };
        let scratch = state.scratch_buffer;
        let exercise = state.exercise_pane;
        let end = self.buffers[scratch].clamp_offset(self.buffers[scratch].len_chars(), false);
        self.activate_pane(exercise);
        let pane = self.panes.get_mut(&exercise).unwrap();
        pane.jumps = JumpList::default();
        pane.retarget(scratch);
        pane.replace_selection(Selection::point(0));
        self.push_jump();
        self.panes
            .get_mut(&exercise)
            .unwrap()
            .replace_selection(Selection::point(end));
        if lesson == 17 {
            self.jump_in(true, false);
        }
        let selection = self.panes[&exercise].selection.clone();
        let state = self.tutorial.as_mut().unwrap();
        state.scratch_selection = selection;
        state.scratch_mode = Mode::Normal;
    }

    pub(super) fn reset_tutorial_scratch(&mut self, text: &str, head: Offset, clear_jumps: bool) {
        let Some(state) = self.tutorial.as_ref() else {
            return;
        };
        let scratch = state.scratch_buffer;
        let pane = state.exercise_pane;
        let _ = self.buffers[scratch].discard_changes_to(text);
        if let Some(pane) = self.panes.get_mut(&pane) {
            pane.retarget(scratch);
            pane.replace_selection(Selection::point(
                self.buffers[scratch].clamp_offset(head, false),
            ));
            if let Some(state) = self.tutorial.as_mut() {
                state.scratch_selection = pane.selection.clone();
                state.scratch_mode = Mode::Normal;
            }
            pane.scroll_row = 0;
            pane.scroll_wrap = 0;
            pane.scroll_col = 0;
            if clear_jumps {
                pane.jumps = JumpList::default();
            }
        }
        self.mode = Mode::Normal;
        self.search = SearchQuery::default();
        self.search_selection = None;
    }

    pub(super) fn refresh_tutorial_document(&mut self) {
        let Some(state) = self.tutorial.as_ref() else {
            return;
        };
        let buffer = state.instruction_buffer;
        let text = crate::tutorial::render_for(state, self.persistent_session, self.keymap());
        self.buffers[buffer].replace_virtual_text(&text);
        for pane in self.panes.values_mut().filter(|pane| pane.buffer == buffer) {
            pane.replace_selection(Selection::point(0));
            pane.scroll_row = 0;
            pane.scroll_wrap = 0;
            pane.scroll_col = 0;
        }
    }

    pub(super) fn tutorial_requested_detach(&mut self) {
        let Some(state) = self.tutorial.as_mut() else {
            return;
        };
        if state.lesson == crate::tutorial::LAST_LESSON && self.persistent_session {
            state.awaiting_reattach = true;
            self.refresh_tutorial_document();
        }
    }

    pub fn note_frontend_attached(&mut self) {
        let Some(state) = self.tutorial.as_mut() else {
            return;
        };
        if state.lesson == crate::tutorial::LAST_LESSON && state.awaiting_reattach {
            state.awaiting_reattach = false;
            state.lesson = crate::tutorial::LAST_LESSON + 1;
            state.last_action = None;
            self.refresh_tutorial_document();
            self.status("persistent tutorial completed after reattachment");
        }
    }

    #[cfg(test)]
    pub(super) fn tutorial_state(&self) -> Option<&TutorialState> {
        self.tutorial.as_ref()
    }
}
