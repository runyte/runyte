// SPDX-License-Identifier: MPL-2.0

//! Interactive, host-owned onboarding state and lesson prose.

use crate::{
    command::{CommandId, Mode},
    keymap::{Keymap, default_keymap},
    selection::Selection,
    terminal::TerminalId,
};

pub const TUTORIAL_NAME: &str = "[tutorial]";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionHints {
    HelixLike,
    VimLike,
    Both,
}

impl MotionHints {
    pub const ALL: &'static [Self] = &[Self::HelixLike, Self::VimLike, Self::Both];

    pub const fn label(self) -> &'static str {
        match self {
            Self::HelixLike => "Helix-like",
            Self::VimLike => "Vim-like",
            Self::Both => "Both",
        }
    }

    pub const fn detail(self) -> &'static str {
        match self {
            Self::HelixLike => "show Helix-like motion spellings",
            Self::VimLike => "show Vim-like motion spellings",
            Self::Both => "show both motion spellings side by side",
        }
    }

    pub const fn line_start(self) -> &'static str {
        match self {
            Self::HelixLike => "gh",
            Self::VimLike => "0",
            Self::Both => "gh (Helix-like) or 0 (Vim-like)",
        }
    }

    pub const fn file_end(self) -> &'static str {
        match self {
            Self::HelixLike => "ge",
            Self::VimLike => "G",
            Self::Both => "ge (Helix-like) or G (Vim-like)",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TutorialState {
    pub lesson: u8,
    pub motion_hints: Option<MotionHints>,
    pub instruction_buffer: usize,
    pub scratch_buffer: usize,
    pub instruction_pane: usize,
    pub exercise_pane: usize,
    pub last_action: Option<(CommandId, String)>,
    pub awaiting_reattach: bool,
    pub explorer_buffer: Option<usize>,
    pub terminal: Option<TerminalId>,
    pub scratch_selection: Selection,
    pub scratch_mode: Mode,
}

pub const LAST_LESSON: u8 = 18;

pub fn render(state: &TutorialState, persistent: bool) -> String {
    render_for(state, persistent, default_keymap())
}

pub fn render_for(state: &TutorialState, persistent: bool, keymap: &Keymap) -> String {
    let template = render_template(state, persistent);
    crate::key_spelling::resolve(&template, keymap)
        .expect("tutorial key markers must resolve against every built-in keymap")
        .text
}

fn render_template(state: &TutorialState, persistent: bool) -> String {
    let mut text = if state.lesson > LAST_LESSON {
        "Runyte tutorial · complete\n\n".to_owned()
    } else {
        format!("Runyte tutorial · {}/{}\n\n", state.lesson, LAST_LESSON)
    };
    if state.motion_hints.is_none() {
        text.push_str(
            "CHOOSE MOTION SPELLINGS\n\n\
             Runyte is a selection-first modal editor with its own behavior.\n\
             Many basic movements have both Helix-like and Vim-like bindings.\n\
             This tutorial can display either family, or show both side by side.\n\
             The choice changes only the motion spellings in these instructions.\n\
             It does not change Runyte's editing model, keymap, or configuration.\n\
             Other commands keep their ordinary Runyte spellings in every choice.\n\
             Use Up and Down to inspect the three options in the picker.\n\
             The right pane will become disposable scratch text after the choice.\n\
             Press Enter to choose the highlighted option and begin lesson 1.\n",
        );
        return text;
    }
    let hints = state.motion_hints.unwrap();
    let body = match state.lesson {
        1 => "MODES\n\nRunyte starts in Normal mode, where keys move or act on text.\nThe block cursor in the right pane is on the `h` in `hello`.\nInsert mode sends ordinary text into the buffer before that cursor.\nPress i once to enter Insert mode and watch the mode label change.\nType `Hi `, including the trailing space, before the existing word.\nPress Escape when the insertion is complete to return to Normal mode.\nThe scratch pane should then contain exactly `Hi hello`.\nInsert sessions form undo checkpoints; Escape finishes this one.\nThe lesson advances only after the text and mode both match.".to_owned(),
        2 => format!("MOTIONS\n\nThe cursor begins on the `b` in the right pane's `alpha beta`.\nNormal-mode motion replaces the current selection with a new caret.\nSelect-mode motion instead extends from the selection's anchor.\nThat behavior is fixed even when a motion has several key spellings.\nFor this tutorial you chose the spelling shown on the next line.\nPress {} to move to the start of the line.\nThe caret should land on the `a` in `alpha`, at column one.\nNo text changes during a motion, and the editor stays in Normal mode.\nThe lesson checks both the command and the position before advancing.", hints.line_start()),
        3 => "SELECTION-FIRST EDITING\n\nRunyte lets you choose text first and then choose what happens to it.\nThe caret starts on the `b` in `blue` in the right scratch pane.\nPress v to enter Select mode and establish the selection anchor.\nPress e to extend the selection through the end of the word `blue`.\nThe highlighted word is now the input to the next editing command.\nPress d to delete exactly that selection and return to Normal mode.\nThe remaining line should read `red `, including its trailing space.\nThe same delete command works on one selection or many selections.\nThis explicit selection is characterwise rather than linewise.".to_owned(),
        4 => "WHOLE-LINE SELECTION WITH x AND X\n\nRunyte also has a fast, transient way to select complete lines.\nThe caret starts on `center`, the middle line of the scratch pane.\nPress x once to snap the current selection to the whole middle line.\nPress x again to extend the selected edge down through `south`.\nPress X once to walk that same edge upward and remove `south` again.\nThe line `center` should now be the only complete selected line.\nPress d to delete it, including its terminating newline.\nThe scratch text should become `north` followed directly by `south`.\nRepeated x walks downward; X mirrors it by walking upward.".to_owned(),
        5 => "SEARCH CREATES SELECTIONS\n\nSearch is one of the quickest ways to make several selections at once.\nThe scratch line contains two occurrences of `cat` around one `dog`.\nPress s to open Runyte's case-insensitive literal-search prompt.\nType `cat`, then press Enter to accept the search query.\nBoth non-overlapping matches should become highlighted selections.\nRunyte enters Select mode because the result is selected text.\nNothing has been edited yet; search only established the targets.\nThe status line reports how many selections are currently active.\nThe next lesson will apply one edit to both matches together.".to_owned(),
        6 => "EDIT EVERY MATCH\n\nBoth `cat` matches from the previous search remain selected.\nEditing commands operate on every current selection in one operation.\nPress c to remove both selected words and enter Insert mode.\nType `fox`; the same inserted text appears at both active carets.\nPress Escape to finish the change and return to Normal mode.\nThe scratch line should now read exactly `fox dog fox`.\nThis batch edit is one transaction and therefore one undo checkpoint.\nSelections are ordinary editor state, not a separate multi-cursor tool.\nSearch plus change is a compact pattern for repeated edits.".to_owned(),
        7 => "MULTIPLE CARETS\n\nMultiple carets can also be built vertically without running a search.\nThe first caret begins on the first of three short scratch lines.\nPress C once to copy that caret to the nearest valid line below.\nPress C a second time to add a third caret on the final line.\nThere should now be one caret at column one on each of the three lines.\nPress i to enter Insert mode at all three carets simultaneously.\nType `> `, including the space, then press Escape to finish.\nEvery line should now begin with the same two-character prefix.\nThe status line should report three selections after the edit.".to_owned(),
        8 => "APPLICATION COMMANDS\n\n{prefix:Space} begins Runyte's main command namespaces in Normal and Select mode.\nPress {prefix:Space} once and pause instead of immediately typing another key.\nA generated hint overlay lists the valid continuation keys and meanings.\nThese rows come from the same keymap registry used for real dispatch.\nThe `s` row opens commands that operate on the current selections.\nContinue by pressing s, then c: the complete sequence is {binding:Space s c}.\nThat command keeps only the primary selection from the current three.\nThe scratch text itself should not change during this lesson.\nPrefix hints are available for other families such as g and {prefix:Ctrl-w}.".to_owned(),
        9 => "WINDOW COMMANDS\n\nA pane is one view of one buffer; this tutorial currently has two panes.\nThe instructions are in the left pane and the exercise is on the right.\n{prefix:Ctrl-w} begins the compatibility spelling for pane and window commands.\nPress {prefix:Ctrl-w} and pause to see the available continuations in a hint.\nThen press h to focus the pane immediately to the left.\nThe active border and status information should move to the instructions.\nNo buffer is closed or copied when focus moves between panes.\nThe equivalent primary Runyte commands live below the {prefix:Space w} namespace.\nThe lesson advances only after the left tutorial pane receives focus.".to_owned(),
        10 => "WINDOW COMMANDS\n\nBoth tutorial panes remain visible after the previous focus change.\nThe left pane is active now, while the disposable scratch is still right.\nPress {prefix:Ctrl-w} and pause if you want to inspect the window hints again.\nThen press l to focus the pane immediately to the right.\nThe exercise pane should regain its active border and status information.\nThe buffers did not move; only the active pane changed between the two.\n{prefix:Space w} offers the primary Runyte namespace for the same pane operations.\nSplits can show different buffers or two views of the same buffer.\nThe next lessons change what the right pane is showing.".to_owned(),
        11 => "BUFFER TYPES: OPEN THE EXPLORER\n\nA buffer owns text, selections, and editing history; a pane displays it.\nThe right pane currently shows an editable, pathless scratch buffer.\nThe left instructions are a read-only generated special buffer.\nFiles are ordinary path-backed buffers, while explorers are editable special buffers.\nSpecial buffers still support normal movement, selection, search, and help.\nPress {binding:Space e} to open an explorer for the active buffer's directory.\nThe right pane should change to a listing whose title begins `[explorer]`.\nThe explorer's rows are editable, but filesystem changes require a reviewed write.\nChanging buffers retargets this pane without changing the two-pane layout.".to_owned(),
        12 => "BUFFER HISTORY: GO BACK\n\nThe right pane now shows an explorer instead of the tutorial scratch text.\nOpening it recorded the previous scratch position in this pane's history.\nAlt-o jumps to the previous position in a different buffer or terminal surface.\nPress Alt-o once to go back from the explorer to the scratch buffer.\nThe three edited scratch lines should reappear with their selections preserved.\nThe explorer remains a buffer even when no pane is currently displaying it.\nAlt-i normally walks forward through the same cross-buffer history.\nCtrl-o and Ctrl-i use the finer history that also includes same-buffer jumps.\nThe lesson advances when this pane is showing the tutorial scratch again.".to_owned(),
        13 => "TERMINALS ARE PANE CONTENT\n\nAn integrated terminal session is not a buffer and owns no editor transaction log.\nA pane can show a live terminal instead of showing its current buffer.\nThe buffer remains attached underneath so the pane can return to it later.\nPress {binding:Space t n} to start your default shell in the right pane.\nRunyte enters Terminal Insert mode, where ordinary keys go to the child process.\nEscape is also sent to that process; it does not leave terminal input.\nCtrl-\\ is the deliberate key that returns control to Terminal Normal mode.\nTerminals have their own manager and do not appear in {binding:Space b b}.\nThe next lesson closes the exact terminal session created here.".to_owned(),
        14 => "CLOSE THE TUTORIAL TERMINAL\n\nThe right pane is showing the live shell created by the previous lesson.\nFirst press Ctrl-\\ once to leave Terminal Insert for Terminal Normal mode.\nNow press {binding:Space t t} to open the manager of live terminal sessions.\nPress Tab on the selected tutorial terminal to open its action menu.\nPress Down twice so the `Close` action, third in the menu, is selected.\nPress Enter to end and forget this terminal session explicitly.\nBecause the terminal is still visible, no second confirmation is required.\nThe right pane then reveals its underlying tutorial scratch buffer again.\nClosing a buffer with {binding:Space b c} is different and is refused on terminals.".to_owned(),
        15 => format!("JUMP HISTORY\n\nThe remaining lessons return to movement inside the scratch buffer.\nThe right pane now contains three lines: `first`, `second`, and `third`.\nThe caret begins at the first character of the first line.\nFile-boundary motions record the position they leave in jump history.\nFor your selected motion spelling, the end-of-file command is shown below.\nPress {} to jump to the end of this scratch document.\nThe caret should land at the final valid position after the last line.\nNo buffer switch occurs; this is movement within the same scratch buffer.\nThe next two lessons walk backward and forward through that jump.", hints.file_end()),
        16 => "JUMP HISTORY: BACKWARD\n\nThe previous file-end motion recorded where the caret started.\nCtrl-o walks backward through the pane's detailed jump history.\nUnlike Alt-o, it may land at another position in the same buffer.\nPress Ctrl-o once to return to the earlier position in this scratch text.\nThe caret should move back to the first character of the first line.\nThe three lines themselves should remain unchanged by the jump.\nSearch results and structural navigation can also contribute useful jumps.\nRunyte keeps jump history per pane, matching the view where it was made.\nThe lesson checks the command and exact destination before advancing.".to_owned(),
        17 => "JUMP HISTORY: FORWARD\n\nAfter a backward jump, the same history has a forward direction.\nPress Ctrl-i once to revisit the end-of-file position you just left.\nThe caret should return to the final valid position in the scratch buffer.\nThis does not create another edit, buffer, pane, or terminal session.\nSome legacy terminals cannot distinguish Ctrl-i from the Tab key.\nOn those terminals the forward jump has no separate reachable key.\nRunyte requests keyboard disambiguation where the terminal supports it.\nAlt-i is different: it skips positions until it reaches another buffer.\nThe persistent-session boundary is the final guided lesson.".to_owned(),
        18 if state.awaiting_reattach => "PERSISTENT SESSIONS\n\nThe interactive client detached successfully from the workspace host.\nThe host still owns this lesson, both panes, selections, and scratch text.\nIt also retains live terminal sessions while no TUI client is attached.\nNo buffer or pane was closed when the client detached.\nReturn to the shell from which you launched the persistent workspace.\nReattach to this workspace with the following command:\n\n    runyte --persistent\n\nA new interactive attachment completes the lesson and opens Next steps.\nThis is not crash, reboot, or machine-failure storage.".to_owned(),
        18 if persistent => "PERSISTENT SESSIONS\n\nThis tutorial is owned by the persistent workspace host, not by this TUI.\nThe host retains open buffers, unsaved edits, panes, selections, and terminals.\nIts state remains available while an interactive client is detached.\nRun :detach now to leave the client without stopping the workspace host.\nThen reattach from a shell with `runyte --persistent`.\nThe lesson completes only after a new interactive TUI attaches to this host.\nDetaching is distinct from closing a buffer, pane, or terminal session.\nPersistent state is not crash, reboot, or machine-failure storage.\nAfter reattachment, the tutorial ends with links for further exploration.".to_owned(),
        18 => "PERSISTENT SESSIONS\n\nThis is a standalone workspace, so its editor state belongs to this TUI process.\nThe :detach command is unavailable because there is no separate workspace host.\nA persistent session can retain buffers, panes, selections, and live terminals.\nTo try the hands-on detach lesson, exit and launch `runyte --persistent`.\nThen run `:tutorial sessions` to open this lesson directly in that workspace.\nPersistent sessions are not crash, reboot, or machine-failure storage.\n\nNEXT STEPS\nRun :help to read the complete Runyte manual and its topic index.\nPress {binding:Space ?} in each view for contextual behavior and its exact active keys.".to_owned(),
        _ => "NEXT STEPS\n\nYou have practised modes, motions, characterwise and whole-line selections.\nYou used search and multiple carets to edit several targets together.\nYou explored generated, scratch, and editable explorer buffer types.\nYou moved among panes and buffers and opened and closed a terminal session.\nYou also walked jump history and crossed the persistent-session boundary.\nRun :help to open the complete Runyte manual and its topic index.\nUse :help <topic> when you already know the subject you want to revisit.\nPress {binding:Space ?} in each view for its contextual behavior and exact active keys.\nRun `:tutorial reset` whenever you want to practise these lessons again.".to_owned(),
    };
    text.push_str(&body);
    text.push_str(
        "\n\nExercises happen in the right pane. It may temporarily show the tutorial\n\
         scratch buffer, an explorer, or a terminal during the guided steps.\n",
    );
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(lesson: u8) -> TutorialState {
        TutorialState {
            lesson,
            motion_hints: Some(MotionHints::Both),
            instruction_buffer: 0,
            scratch_buffer: 1,
            instruction_pane: 0,
            exercise_pane: 1,
            last_action: None,
            awaiting_reattach: false,
            explorer_buffer: None,
            terminal: None,
            scratch_selection: Selection::default(),
            scratch_mode: Mode::Normal,
        }
    }

    #[test]
    fn authored_key_markers_are_complete_and_resolve_in_both_variants() {
        let mut chooser = state(0);
        chooser.motion_hints = None;
        crate::key_spelling::assert_authored_template(&render_template(&chooser, false));
        for lesson in 1..=LAST_LESSON + 1 {
            let state = state(lesson);
            crate::key_spelling::assert_authored_template(&render_template(&state, false));
            crate::key_spelling::assert_authored_template(&render_template(&state, true));
            if lesson == 18 {
                let mut awaiting = state.clone();
                awaiting.awaiting_reattach = true;
                crate::key_spelling::assert_authored_template(&render_template(&awaiting, true));
            }
        }
    }
}
