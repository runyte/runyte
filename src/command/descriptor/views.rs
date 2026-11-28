// SPDX-License-Identifier: MPL-2.0
//! Prose for specialized implementations, not a second admission table.
use super::ViewBehavior;
use crate::{command::EditorCommand as C, keymap::BindingScope as S};

macro_rules! views {
    ($($scope:ident: $description:literal),+ $(,)?) => {
        &[$(ViewBehavior { scope: S::$scope, description: $description }),+]
    };
}

pub(super) const fn editor_views(command: C) -> &'static [ViewBehavior] {
    match command {
        C::FitSelectionForNormalMode => {
            views!(Terminal: "Fit the live or captured review selection for Normal mode without entering it")
        }
        C::JumpBackward | C::JumpForward | C::JumpBackwardBuffer | C::JumpForwardBuffer => {
            views!(Terminal: "Navigate retained jump history through the ordinary pane workflow")
        }
        C::ModeEnter => {
            views!(Terminal: "Select captures and focuses review output; Insert returns to live output; Replace is unavailable", Media: "Entering Select initializes selection of PDF text or an image region")
        }
        C::TerminalEnterInput => {
            views!(Terminal: "Return to live output and send subsequent input to the child")
        }
        C::SelectAll => {
            views!(Terminal: "Select all captured terminal output")
        }
        C::CollapseSelection => {
            views!(Terminal: "Collapse the review selection", Media: "Retain the native media selection; this surface has no document caret to collapse")
        }
        C::SelectLine | C::ExtendLineAbove | C::ExtendLineBelow | C::SelectLineUp => {
            views!(Terminal: "Select or extend captured output with whole-line extent, preserving mode")
        }
        C::KeepPrimarySelection => views!(Terminal: "Keep only the primary review selection"),
        C::CopySelectionDown | C::CopySelectionUp => {
            views!(Terminal: "Copy the review cursor to the adjacent captured output line")
        }
        C::MoveLeft | C::MoveRight => {
            views!(Terminal: "Move or extend the cursor in captured output", Media: "Pan horizontally, or extend the media selection in Select mode")
        }
        C::MoveUp | C::MoveDown => {
            views!(Terminal: "Move or extend the cursor in captured output", Media: "Pan vertically, change PDF page at fit, or extend the media selection")
        }
        C::MoveFileStart | C::MoveFileEnd => {
            views!(Terminal: "Move to the captured output boundary", Media: "Select a PDF page by address or move to its first or last page")
        }
        C::MoveLineStart
        | C::MoveLineEnd
        | C::MoveFirstNonWhitespace
        | C::MoveWordForward
        | C::MoveWordBackward
        | C::MoveWordEnd
        | C::MoveLongWordForward
        | C::MoveLongWordBackward
        | C::MoveLongWordEnd
        | C::GotoNextParagraph
        | C::GotoPreviousParagraph => {
            views!(Terminal: "Move or extend the review selection within captured output")
        }
        C::PageUp | C::PageDown => {
            views!(Terminal: "Move the review cursor by a page of captured output", Media: "Move to the previous or next PDF page")
        }
        C::HalfPageUp | C::HalfPageDown => {
            views!(Terminal: "Move the review cursor by half a page of captured output", Media: "Move to the previous or next PDF page", DirectoryTree: "Move the selected tree row by half a page")
        }
        C::GotoWindowTop | C::GotoWindowCenter | C::GotoWindowBottom => {
            views!(Terminal: "Move the review cursor within the visible captured output", DirectoryTree: "Select the tree row at the top, middle or bottom of the view")
        }
        C::GotoFile => {
            views!(Terminal: "Open a path or URL inferred from the review selection and terminal directory")
        }
        C::GotoWord => {
            views!(Terminal: "Label visible captured output words and jump to one", DirectoryTree: "Label visible tree entries and select one")
        }
        C::FindNextChar | C::FindPreviousChar | C::FindTillNextChar | C::FindTillPreviousChar => {
            views!(Terminal: "Find the supplied character within captured output")
        }
        C::ScrollViewUp | C::ScrollViewDown => {
            views!(Terminal: "Capture review and scroll its output", Media: "Pan the image or PDF vertically")
        }
        C::AlignViewCenter | C::AlignViewMiddle => {
            views!(Media: "Center the image or PDF in its pane")
        }
        C::TerminalSearch => {
            views!(Terminal: "Prompt for a case-insensitive search of captured output")
        }
        C::TerminalSearchRegex => {
            views!(Terminal: "Prompt for a regular-expression search of captured output")
        }
        C::DirectoryTreeSearch => views!(DirectoryTree: "Prompt to search visible tree entries"),
        C::DirectoryTreeSearchNext | C::DirectoryTreeSearchPrevious => {
            views!(DirectoryTree: "Select the next or previous matching tree entry")
        }
        C::TerminalSearchNext | C::TerminalSearchPrevious => {
            views!(Terminal: "Select the next or previous terminal review search match")
        }
        C::Yank => {
            views!(Terminal: "Copy the review selection to the selected register and retain its selection and mode", Directory: "Copy selected entries and record a directory copy payload")
        }
        C::YankLine => {
            views!(Terminal: "Copy whole review rows to the selected register, preserving selection and mode", Directory: "Copy selected entry lines and record a directory copy payload")
        }
        C::CutSelection | C::CutSelectionForChange => {
            views!(Directory: "Record selected entries as a move payload without removing files; delete or change the listing selection")
        }
        C::PasteAfter => {
            views!(Directory: "Stage copies or moves from a directory register into the listing")
        }
        C::DirectoryPaste | C::PasteBefore => {
            views!(Directory: "Stage copies or moves from a directory register into the listing")
        }
        C::ReplaceWithRegister => {
            views!(Directory: "Replace listing text from a text register; directory move/copy payloads are refused because entries cannot replace a text selection")
        }
        C::ClipboardYank => {
            views!(Terminal: "Copy the captured review selection to the system clipboard", Media: "Copy selected PDF text or an image region")
        }
        C::TerminalPaste => {
            views!(Terminal: "Send register text to the child without changing mode")
        }
        C::TerminalClipboardPasteText => {
            views!(Terminal: "Send system clipboard text to the child without changing mode")
        }
        C::TerminalUndoPaste => {
            views!(Terminal: "Ask the child to erase the last text Runyte sent, if it has not been submitted")
        }
        C::SelectionUndo | C::SelectionRedo => {
            views!(Terminal: "Restore the previous or next terminal review selection")
        }
        C::MediaBack => {
            views!(Media: "Enter Normal mode, then fit media, clear selection or leave preview", PdfPages: "Collapse a Select-mode page selection, otherwise open the parent explorer")
        }
        C::MediaShowPage => views!(PdfPages: "Display the selected PDF page and enter Normal mode"),
        C::Save | C::ForceSave => {
            views!(Directory: "Review and apply edits to the directory listing as filesystem operations", CommitMessage: "Review staged changes and submit the commit message")
        }
        C::CloseWindow => views!(DirectoryTree: "Close the directory tree and restore pane focus"),
        C::ToggleHiddenFiles => views!(DirectoryTree: "Show or hide hidden tree entries"),
        C::ShowHelp => views!(DirectoryTree: "Open directory-tree help"),
        // Other actions use their canonical description in every admitted view.
        // Admission remains exhaustive in keymap::admission; this list only
        // documents differing implementations and deliberately grants nothing.
        _ => &[],
    }
}
