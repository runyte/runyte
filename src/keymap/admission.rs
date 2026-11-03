// SPDX-License-Identifier: MPL-2.0
//! Which editor commands act in a view whose pane does not show its buffer
//! as text.
//!
//! A media surface, the directory tree and a terminal each sit in front of a
//! buffer. A command written for text — a word motion, a register, an edit —
//! would reach that hidden buffer and do something nobody can see, so each of
//! these views decides what it answers to. Dispatch reads the rule here to
//! refuse the rest, and help reads it to leave the rest out, so `Space ?`
//! lists exactly the keys that do something in the view it describes.
//!
//! Colon-command and plugin bindings are not editor commands and are always
//! admitted: they run the same in every view.

use super::{BindingScope, BindingTarget};
use crate::command::{CommandCategory, EditorCommand as C};

/// Whether a binding to `target` does anything in `scope`.
pub fn admits(scope: BindingScope, target: BindingTarget) -> bool {
    match target {
        BindingTarget::Editor(command) => admits_command(scope, command),
        BindingTarget::Colon(_) | BindingTarget::Plugin(_) => true,
    }
}

/// Whether `command` does anything in `scope`. Every scope not listed here
/// shows its buffer as text, where every command has its ordinary meaning.
pub fn admits_command(scope: BindingScope, command: C) -> bool {
    match scope {
        BindingScope::Media => {
            !(addresses_text(command) && !MEDIA.contains(&command))
                && !MEDIA_REFUSED_VIEWS.contains(&command)
        }
        BindingScope::DirectoryTree => TREE.contains(&command) || TREE_PASSES.contains(&command),
        BindingScope::Terminal => !(addresses_text(command) && !TERMINAL.contains(&command)),
        _ => true,
    }
}

/// Commands that act on a buffer's text, its selections or its registers,
/// rather than on panes, files or the application.
///
/// Decided by category rather than by name so a command added later is
/// refused by default in these views: the failure that matters is an editing
/// command reaching the document hidden behind them, and silence is the worst
/// way for that to happen. Project-wide search is deliberately not included —
/// it looks past this pane, and its results replace the pane's content anyway.
fn addresses_text(command: C) -> bool {
    matches!(
        command,
        C::Search
            | C::SearchRegex
            | C::SearchForward
            | C::SearchBackward
            | C::SearchNext
            | C::SearchPrevious
            | C::SearchSelection
    ) || matches!(
        command.category(),
        CommandCategory::Editing
            | CommandCategory::Movement
            | CommandCategory::Selection
            | CommandCategory::Syntax
            | CommandCategory::Language
            | CommandCategory::Register
            | CommandCategory::Clipboard
    )
}

/// Text commands an image or PDF answers to. The media handler owns most of
/// them, including the system-clipboard yank, which copies the media
/// selection; page and file-boundary motions move along the PDF's page rows,
/// which is how a count before `g g` reaches a page. The jump list and macros
/// work across buffers rather than inside this one.
const MEDIA: &[C] = &[
    C::EnterSelectMode,
    C::ClipboardYank,
    C::MoveLeft,
    C::MoveRight,
    C::MoveUp,
    C::MoveDown,
    C::PageUp,
    C::PageDown,
    C::HalfPageUp,
    C::HalfPageDown,
    C::MoveFileStart,
    C::MoveFileEnd,
    C::JumpBackward,
    C::JumpForward,
    C::JumpBackwardBuffer,
    C::JumpForwardBuffer,
    C::RecordMacro,
    C::RecordDefaultMacro,
    C::StopMacroRecording,
    C::ReplayMacro,
    C::ReplayDefaultMacro,
    C::ListMacros,
];

/// View commands that would only change how the hidden text is laid out.
const MEDIA_REFUSED_VIEWS: &[C] = &[
    C::AlignViewTop,
    C::AlignViewBottom,
    C::ToggleSoftWrap,
    C::ToggleWhitespace,
    C::PreviewDocument,
];

/// What the focused directory tree handles itself.
const TREE: &[C] = &[
    C::ToggleDirectoryTree,
    C::FocusDirectoryTree,
    C::DirectoryTreeUp,
    C::DirectoryTreeDown,
    C::DirectoryTreeLeft,
    C::DirectoryTreeRight,
    C::DirectoryTreeFirst,
    C::DirectoryTreeLast,
    C::DirectoryTreePageUp,
    C::DirectoryTreePageDown,
    C::DirectoryTreeOpen,
    C::DirectoryTreeClose,
    C::DirectoryTreeRefresh,
    C::DirectoryTreeLegend,
    C::DirectoryTreeVertical,
    C::DirectoryTreeHorizontal,
    C::DirectoryTreePane1,
    C::DirectoryTreePane2,
    C::DirectoryTreePane3,
    C::DirectoryTreePane4,
    C::DirectoryTreePane5,
    C::DirectoryTreePane6,
    C::DirectoryTreePane7,
    C::DirectoryTreePane8,
    C::DirectoryTreePane9,
    C::DirectoryTreeNew,
    C::DirectoryTreeRename,
    C::DirectoryTreeDelete,
    C::DirectoryTreeMove,
    C::CloseWindow,
    C::GotoWord,
    C::HalfPageUp,
    C::HalfPageDown,
    C::GotoWindowTop,
    C::GotoWindowCenter,
    C::GotoWindowBottom,
    C::ToggleHiddenFiles,
    C::SearchRegex,
    C::SearchNext,
    C::SearchPrevious,
    C::ShowHelp,
];

/// Commands the focused tree hands on to ordinary dispatch. Everything else is
/// refused with a message: the pane behind the tree is not what the person is
/// looking at.
pub(crate) const TREE_PASSES: &[C] = &[
    C::ToggleFullscreen,
    C::ToggleZen,
    C::OpenCommandPalette,
    C::FocusWindowLeft,
    C::FocusWindowRight,
    C::FocusWindowUp,
    C::FocusWindowDown,
    C::NextWindow,
];

/// Text commands terminal review answers to, on its captured snapshot. The
/// character-taking ones are resolved before the terminal handler sees them;
/// insert-mode entries send input to the child.
const TERMINAL: &[C] = &[
    C::AppendAfter,
    C::ClipboardPasteAfter,
    C::ClipboardPasteBefore,
    C::ClipboardYank,
    C::CollapseSelection,
    C::CopySelectionDown,
    C::CopySelectionUp,
    C::EnterInsertMode,
    C::EnterReplaceMode,
    C::EnterSelectMode,
    C::ExtendLineAbove,
    C::ExtendLineBelow,
    C::FindNextChar,
    C::FindPreviousChar,
    C::FindTillNextChar,
    C::FindTillPreviousChar,
    C::GotoFile,
    C::GotoNextParagraph,
    C::GotoPreviousParagraph,
    C::GotoWindowBottom,
    C::GotoWindowCenter,
    C::GotoWindowTop,
    C::GotoWord,
    C::HalfPageDown,
    C::HalfPageUp,
    C::InsertLineEnd,
    C::InsertLineStart,
    C::JumpBackward,
    C::JumpBackwardBuffer,
    C::JumpForward,
    C::JumpForwardBuffer,
    C::KeepPrimarySelection,
    C::MoveDown,
    C::MoveFileEnd,
    C::MoveFileStart,
    C::MoveFirstNonWhitespace,
    C::MoveLeft,
    C::MoveLineEnd,
    C::MoveLineStart,
    C::MoveLongWordBackward,
    C::MoveLongWordEnd,
    C::MoveLongWordForward,
    C::MoveRight,
    C::MoveUp,
    C::MoveWordBackward,
    C::MoveWordEnd,
    C::MoveWordForward,
    C::OpenLineAbove,
    C::OpenLineBelow,
    C::PageDown,
    C::PageUp,
    C::PasteAfter,
    C::PasteBefore,
    C::RecordMacro,
    C::ReplayMacro,
    C::RotateSelectionBackward,
    C::RotateSelectionForward,
    C::ScrollViewDown,
    C::ScrollViewUp,
    C::Search,
    C::SearchNext,
    C::SearchPrevious,
    C::SearchRegex,
    C::SelectAll,
    C::SelectLine,
    C::SelectLineUp,
    C::SelectRegister,
    C::Undo,
    C::Yank,
];
