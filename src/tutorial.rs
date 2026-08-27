// SPDX-License-Identifier: MPL-2.0

//! Interactive, host-owned onboarding state and lesson prose.

use crate::{
    command::{CommandId, Mode},
    selection::Selection,
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
    pub scratch_selection: Selection,
    pub scratch_mode: Mode,
}

pub const LAST_LESSON: u8 = 15;

pub fn render(state: &TutorialState, persistent: bool) -> String {
    let mut text = if state.lesson > LAST_LESSON {
        "Runyte tutorial · complete\n\n".to_owned()
    } else {
        format!("Runyte tutorial · {}/{}\n\n", state.lesson, LAST_LESSON)
    };
    if state.motion_hints.is_none() {
        text.push_str(
            "Runyte has its own selection behavior. Many basic movements support\n\
             familiar Vim-like and Helix-like keybindings. Choose which motion\n\
             keys this tutorial should show. The choice changes the instructions,\n\
             not Runyte's editing model or keymap.\n\n\
             Select an option in the picker and press Enter.\n",
        );
        return text;
    }
    let hints = state.motion_hints.unwrap();
    let body = match state.lesson {
        1 => "MODES\n\nRunyte starts in Normal mode. Press i, type `Hi ` before `hello`,\nthen press Escape. The scratch pane should read `Hi hello`.".to_owned(),
        2 => format!("MOTIONS\n\nMove to the start of the line with {}.\n\nMotion spelling does not change selection behavior: Normal movement\nreplaces the selection; Select movement extends it.", hints.line_start()),
        3 => "SELECTION-FIRST EDITING\n\nThe cursor starts on `b` in `blue`. Press v, then e to extend the\nselection through the word, then d to delete it.".to_owned(),
        4 => "SEARCH CREATES SELECTIONS\n\nPress s, enter `cat`, and press Enter. Runyte selects every match,\nready for one action to affect all of them.".to_owned(),
        5 => "EDIT EVERY MATCH\n\nBoth `cat` matches are selected. Press c, type `fox`, then Escape.\nOne transaction changes every selection.".to_owned(),
        6 => "MULTIPLE CARETS\n\nPress C twice to add carets on the two lines below. Then press i,\ntype `> `, and press Escape.".to_owned(),
        7 => "SPACE COMMANDS\n\nPress Space and pause to read the generated key hints. Continue with\nSpace s c to keep only the primary selection.".to_owned(),
        8 => "PANES WITH CTRL-W\n\nUse Ctrl-w h to focus the tutorial pane on the left. The prefix hint\nlists the same window commands that normal dispatch uses.".to_owned(),
        9 => "PANES WITH CTRL-W\n\nUse Ctrl-w l to return to the scratch pane on the right. Space w\noffers the primary Runyte namespace for the same pane commands.".to_owned(),
        10 => "BUFFER HISTORY\n\nAlt-o walks to the previous position in a different buffer. Press\nAlt-o; this pane should show the tutorial buffer.".to_owned(),
        11 => "BUFFER HISTORY\n\nAlt-i walks forward to the next position in a different buffer. Press\nAlt-i to return this pane to the scratch buffer.".to_owned(),
        12 => format!("JUMP HISTORY\n\nPress {} to jump to the end of this scratch document. File-boundary\nmotions record the position you came from.", hints.file_end()),
        13 => "JUMP HISTORY\n\nPress Ctrl-o to return to the earlier position in this buffer.".to_owned(),
        14 => "JUMP HISTORY\n\nPress Ctrl-i to move forward again. Some legacy terminals cannot\ndistinguish Ctrl-i from Tab; their forward jump has no separate key.".to_owned(),
        15 if state.awaiting_reattach => "PERSISTENT SESSIONS\n\nDetached successfully. Reattach from the shell with:\n\n    runyte --persistent\n\nThe host should retain this lesson, both panes, selections, and the\nscratch text while no TUI is attached.".to_owned(),
        15 if persistent => "PERSISTENT SESSIONS\n\nThis tutorial is owned by the persistent workspace host. Run :detach,\nthen reattach from the shell with `runyte --persistent`. The lesson\ncompletes only after a new interactive TUI attaches to this host.".to_owned(),
        15 => "PERSISTENT SESSIONS\n\nThis is a standalone workspace: its editor state belongs to this TUI\nprocess, and :detach is unavailable. To try the hands-on lesson, exit\nand run:\n\n    runyte --persistent\n    :tutorial sessions\n\nA persistent session retains live editor state while its local host\ncontinues running; it is not crash, reboot, or machine-failure storage.".to_owned(),
        _ => "COMPLETE\n\nYou have practised Runyte's modes, selection-first editing, search,\nmultiple carets, command discovery, panes, buffer history, jump history,\nand persistent-session boundary. Run `:tutorial reset` to start again.".to_owned(),
    };
    text.push_str(&body);
    text.push_str("\n\nThe right pane is disposable tutorial scratch text.\n");
    text
}
