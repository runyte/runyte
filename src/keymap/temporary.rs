// SPDX-License-Identifier: MPL-2.0
//! Temporary selection gesture lifecycle shared by execution and flow validation.
use crate::command::EditorCommand as C;

pub(crate) fn is_atomic_selection_action(command: C) -> bool {
    matches!(
        command,
        C::ExpandSyntaxSelection
            | C::FlipSelection
            | C::SplitSelectionAtLineEnds
            | C::SplitSelectionAtLineStarts
            | C::SelectSyntaxParent
            | C::SelectSyntaxChild
            | C::SelectPreviousSyntaxSibling
            | C::SelectNextSyntaxSibling
            | C::SelectSyntaxFunction
            | C::SelectInsideSyntaxFunction
            | C::SelectSyntaxClass
            | C::SelectInsideSyntaxClass
            | C::SelectSyntaxParameter
            | C::SelectInsideSyntaxParameter
            | C::SelectAroundParentheses
            | C::SelectInsideParentheses
            | C::SelectAroundSquareBrackets
            | C::SelectInsideSquareBrackets
            | C::SelectAroundBraces
            | C::SelectInsideBraces
            | C::SelectAroundAngleBrackets
            | C::SelectInsideAngleBrackets
            | C::SelectAroundDoubleQuotes
            | C::SelectInsideDoubleQuotes
            | C::SelectAroundSingleQuotes
            | C::SelectInsideSingleQuotes
            | C::SelectAroundBackticks
            | C::SelectInsideBackticks
            | C::SelectInsideWord
            | C::SelectAroundWord
            | C::SelectInsideLongWord
            | C::SelectAroundLongWord
            | C::SelectAroundClosestDelimiter
            | C::SelectInsideClosestDelimiter
    )
}

pub(crate) fn discards_temporary(command: C) -> bool {
    is_atomic_selection_action(command)
        || matches!(
            command,
            C::SurroundAdd
                | C::SurroundReplace
                | C::SurroundDelete
                | C::SelectAll
                | C::CollapseSelection
                | C::CollapseSelectionStart
                | C::CollapseSelectionHead
                | C::ToggleCase
                | C::ReplaceChar
                | C::FitSelectionForNormalMode
                | C::SearchSetFromSelection
                | C::SearchExtendNext
                | C::SearchExtendPrevious
                | C::SearchSelectAllMatches
                | C::Yank
                | C::YankLine
                | C::PasteAfter
                | C::PasteBefore
                | C::DirectoryPaste
                | C::PasteAfterOrReplace
                | C::ClipboardPaste
                | C::ClipboardPasteAfter
                | C::ClipboardPasteBefore
                | C::SearchNext
                | C::SearchPrevious
                | C::TerminalUndoPaste
                | C::TerminalPaste
                | C::TerminalClipboardPasteText
                | C::TerminalSearch
                | C::TerminalSearchRegex
                | C::TerminalSearchNext
                | C::TerminalSearchPrevious
        )
}

pub(crate) fn continues_temporary(command: C) -> bool {
    matches!(
        command,
        C::SelectLine
            | C::SelectLineUp
            | C::ExtendLineAbove
            | C::ExtendLineBelow
            | C::SelectInsideParagraph
            | C::SelectAroundParagraph
            | C::SelectionUndo
            | C::SelectionRedo
    )
}
