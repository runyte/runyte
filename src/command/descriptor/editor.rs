// SPDX-License-Identifier: MPL-2.0
//! Exhaustive editor action policies. Adding an identity requires classifying
//! it here; no default category guesses or scans of the built-in keys occur.
use super::{ActionEffect::*, ActionPolicy, BufferMutation, CountBehavior, OperandArity};
use crate::command::{EditorCommand as C, Mode};
const ALL: &[Mode] = &[Mode::Normal, Mode::Select, Mode::Insert, Mode::Replace];
const NORMAL_SELECT: &[Mode] = &[Mode::Normal, Mode::Select];
const INSERT_REPLACE: &[Mode] = &[Mode::Insert, Mode::Replace];
const NONE: &[Mode] = &[];

pub(super) const fn editor_policy(command: C) -> ActionPolicy {
    match command {
        C::FitSelectionForNormalMode => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[Selection, View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::ModeEnter => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::EnterNormalMode => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[Mode, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenCommandPalette => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Prompt],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::MoveLeft | C::MoveRight | C::MoveUp | C::MoveDown | C::PageUp | C::PageDown => {
            ActionPolicy {
                global_binding_modes: ALL,
                effects: &[Selection, View],
                count: CountBehavior::Repeat,
                operands: OperandArity::Zero,
                requires_writable_buffer: false,
                buffer_mutation: BufferMutation::Unchanged,
            }
        }
        C::MoveLineStart | C::MoveLineEnd => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::MoveFirstNonWhitespace
        | C::KeepPrimarySelection
        | C::RemovePrimarySelection
        | C::CopySelectionDown
        | C::CopySelectionUp
        | C::RotateSelectionForward
        | C::RotateSelectionBackward
        | C::TrimSelections
        | C::GotoWindowTop
        | C::GotoWindowCenter
        | C::GotoWindowBottom => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::MoveFileStart | C::MoveFileEnd => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection, View],
            count: CountBehavior::Interpret,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::MoveWordForward
        | C::MoveWordBackward
        | C::MoveWordEnd
        | C::MoveLongWordForward
        | C::MoveLongWordBackward
        | C::MoveLongWordEnd
        | C::GotoNextParagraph
        | C::GotoPreviousParagraph => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection],
            count: CountBehavior::Repeat,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::SelectionUndo | C::SelectionRedo => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection, Mode],
            count: CountBehavior::Repeat,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::FindNextChar | C::FindPreviousChar | C::FindTillNextChar | C::FindTillPreviousChar => {
            ActionPolicy {
                global_binding_modes: NORMAL_SELECT,
                effects: &[Operand, Selection],
                count: CountBehavior::Reject,
                operands: OperandArity::One,
                requires_writable_buffer: false,
                buffer_mutation: BufferMutation::Unchanged,
            }
        }
        C::HalfPageUp | C::HalfPageDown => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection, View],
            count: CountBehavior::Repeat,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::EnterInsertMode
        | C::EnterReplaceMode
        | C::AppendAfter
        | C::InsertLineStart
        | C::InsertLineEnd => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Mode, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenLineBelow | C::OpenLineAbove => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text, Mode, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::ReplaceChar => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Operand, Text],
            count: CountBehavior::Reject,
            operands: OperandArity::One,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::SurroundAdd | C::SurroundDelete => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Operand, Text, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::One,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::SurroundReplace => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Operand, Text, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Two,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::ToggleCase => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::ReplaceWithRegister | C::DeleteSelection => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::Undo => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text, TerminalInput],
            count: CountBehavior::Repeat,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::Redo => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text],
            count: CountBehavior::Repeat,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::Yank | C::YankLine => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Register],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::YankCollapseNormalMode | C::YankLineCollapseNormalMode => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Register, Selection, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::PasteAfter => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text, Selection, TerminalInput],
            count: CountBehavior::Interpret,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::PasteAfterOrReplace | C::PasteBefore => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text, Selection, Mode, TerminalInput],
            count: CountBehavior::Interpret,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::Indent | C::Unindent => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::ToggleComments => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[Text],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::CutSelection => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Register, Text, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::CutSelectionForChange => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Register, Text, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::ChangeSelection => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::EnterSelectMode => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Mode, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::SelectLine | C::ExtendLineAbove | C::ExtendLineBelow | C::SelectLineUp => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection, Mode],
            count: CountBehavior::Repeat,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::SelectAll | C::CollapseSelection => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::FlipSelection | C::SplitSelectionAtLineEnds | C::SplitSelectionAtLineStarts => {
            ActionPolicy {
                global_binding_modes: NORMAL_SELECT,
                effects: &[Selection, Mode],
                count: CountBehavior::Reject,
                operands: OperandArity::Zero,
                requires_writable_buffer: false,
                buffer_mutation: BufferMutation::Unchanged,
            }
        }
        C::ExpandSyntaxSelection
        | C::ShrinkSyntaxSelection
        | C::SelectSyntaxParent
        | C::SelectSyntaxChild
        | C::SelectPreviousSyntaxSibling
        | C::SelectNextSyntaxSibling => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection, Mode],
            count: CountBehavior::Repeat,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::SelectSyntaxFunction
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
        | C::SelectInsideParagraph
        | C::SelectAroundParagraph
        | C::SelectAroundClosestDelimiter
        | C::SelectInsideClosestDelimiter => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::GotoPreviousSyntaxFunction
        | C::GotoNextSyntaxFunction
        | C::GotoPreviousSyntaxClass
        | C::GotoNextSyntaxClass
        | C::GotoPreviousSyntaxParameter
        | C::GotoNextSyntaxParameter => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection],
            count: CountBehavior::Repeat,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::DocumentOutline
        | C::ToggleSyntaxFold
        | C::FoldAllSyntax
        | C::UnfoldAllSyntax
        | C::AlignViewCenter
        | C::AlignViewTop
        | C::AlignViewBottom
        | C::AlignViewMiddle
        | C::OpenTerminalList
        | C::NewBuffer
        | C::OpenBufferPicker
        | C::Diagnostics
        | C::ListMacros => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::ToggleZen | C::ToggleFullscreen => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::CopyTerminalOutput => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::CopySelectionDownPadded | C::CopySelectionUpPadded | C::AlignSelections => {
            ActionPolicy {
                global_binding_modes: NORMAL_SELECT,
                effects: &[Text, Selection],
                count: CountBehavior::Reject,
                operands: OperandArity::Zero,
                requires_writable_buffer: false,
                buffer_mutation: BufferMutation::MayMutate,
            }
        }
        C::RotateSelectionContentsForward | C::RotateSelectionContentsBackward => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::KeepMatchingSelections | C::RemoveMatchingSelections => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Prompt, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::TrimTrailingWhitespace => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::HardWrap | C::Reflow | C::JoinLines | C::FormatTable => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::JoinSelections => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Prompt, Text],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::Search | C::SearchRegex => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Prompt, SearchState, Selection, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::SearchNext | C::SearchPrevious => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection, View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::SearchExtendNext | C::SearchExtendPrevious => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection, View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::SearchSelectAllMatches => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection, View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::SearchSetFromSelection => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[SearchState],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::MediaShowPage | C::MediaBack => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View, Mode, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::MediaZoomIn
        | C::MediaZoomOut
        | C::MediaFit
        | C::MediaActualSize
        | C::MediaPanLeft
        | C::MediaPanRight
        | C::DirectoryTreeLegend
        | C::MergeReviewCancel
        | C::MergeReviewBack
        | C::ListBackspace
        | C::ListTogglePreview
        | C::ListClearFilter
        | C::ListClose => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::ToggleMarkdownRender | C::ShowAbout => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::MediaMoveUp | C::MediaMoveDown => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View, Selection],
            count: CountBehavior::Repeat,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::MediaCopySelection => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Register],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::MediaSelectAll
        | C::DirectoryTreeUp
        | C::DirectoryTreeDown
        | C::DirectoryTreeFirst
        | C::DirectoryTreeLast
        | C::DirectoryTreePageUp
        | C::DirectoryTreePageDown
        | C::MergeReviewNext
        | C::MergeReviewPrevious
        | C::MergeReviewLeft
        | C::MergeReviewRight
        | C::ListNext
        | C::ListPrevious
        | C::ListPageDown
        | C::ListPageUp
        | C::ListFirst
        | C::ListLast => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::ScrollViewDown | C::ScrollViewUp => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[View],
            count: CountBehavior::Repeat,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::GotoFile => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[File, View, Prompt, Background],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::GotoWord => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            // Jump labels own input outside the resumable character-operand grammar.
            effects: &[Prompt, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::ToggleSoftWrap | C::ToggleWhitespace => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[View, Configuration],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::PreviewDocument => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Background, View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::ToggleFocus | C::ToggleHiddenFiles | C::ToggleDirectoryDetails => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View, Configuration],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenExplorer | C::OpenWorkingDirectoryExplorer => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[File, View, Prompt],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::ToggleDirectoryTree | C::FocusDirectoryTree => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Pane, View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::DirectoryTreeCollapse
        | C::DirectoryTreeSelectParent
        | C::DirectoryTreeExpand
        | C::DirectoryTreeSelectFirstChild => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::DirectoryTreeOpen
        | C::DirectoryTreePane1
        | C::DirectoryTreePane2
        | C::DirectoryTreePane3
        | C::DirectoryTreePane4
        | C::DirectoryTreePane5
        | C::DirectoryTreePane6
        | C::DirectoryTreePane7
        | C::DirectoryTreePane8
        | C::DirectoryTreePane9 => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[File, View, Mode, Prompt, Background],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::DirectoryTreeClose => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Pane, View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::DirectoryTreeRefresh => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[File, View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::DirectoryTreeVertical | C::DirectoryTreeHorizontal => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[File, View, Mode, Prompt, Background, Pane],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::DirectoryTreeNew
        | C::DirectoryTreeRename
        | C::DirectoryTreeDelete
        | C::DirectoryTreeMove => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Prompt, File],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenFilePicker | C::OpenAllFilesPicker => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[View, Background],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenPathFilePicker => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Prompt, View, Background],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenDirectoryFilePicker | C::OpenFuzzyGrep | C::OpenDirectoryFuzzyGrep => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View, Background],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenDirectoryEntry => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[File, View, Mode, Selection, Prompt, Background],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenParentDirectory => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[File, View, Mode, Selection, Prompt],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::RefreshDirectory => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[File, View, Prompt],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::ChooseExplorerOrder => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Prompt, View, Configuration],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenExplorerSystem
        | C::OpenExplorerFinder
        | C::OpenSessionDirectory
        | C::OpenExplorerSession
        | C::OpenExplorerWorkspace
        | C::OpenSessionDestinations => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[File, View, Background],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenChangedFile | C::CheckoutBranch | C::PullBranch => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Background, File, View, Prompt],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::StageAllChangedFiles
        | C::OpenGitConflict
        | C::InspectConflictSides
        | C::FetchBranch
        | C::PushBranch
        | C::OpenWorktree => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Background, File, View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::CreateBranch
        | C::DeleteBranch
        | C::CreateWorktree
        | C::CreateNewWorktree
        | C::RemoveWorktree => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Prompt, Background, File],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::MergeBranch | C::ContinueMerge => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Background, File, View, Prompt, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenGitConflicts => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Background, File, View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::AbortMerge
        | C::ResolveConflictLiteralMarkers
        | C::ResolveConflict
        | C::KeepConflictFileCurrent
        | C::TakeConflictFileOther => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Prompt, Background, File, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::NextConflict | C::PreviousConflict => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Selection, View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::KeepConflictCurrent | C::TakeConflictOther => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Text],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::ReturnToGitConflicts
        | C::NextGitNetworkPage
        | C::NextGitLogPage
        | C::PreviousGitLogPage
        | C::OpenGitCommit => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Background, File, View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::MergeReviewApprove | C::MergeReviewEnter => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View, Background, File, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::PreviousGitNetworkPage | C::ShowTerminal => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenWorkspaceSearchResult => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View, File, Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OverrideSettingLanguage | C::OverrideSettingPattern | C::ActivateSetting => {
            ActionPolicy {
                global_binding_modes: NONE,
                effects: &[Prompt, Configuration],
                count: CountBehavior::Reject,
                operands: OperandArity::Zero,
                requires_writable_buffer: false,
                buffer_mutation: BufferMutation::Unchanged,
            }
        }
        C::RemoveSettingOverride => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Configuration, View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenSettings | C::ShowHelp | C::LeaveTerminal => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenThemeSettings => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Configuration, View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::SplitVertical | C::SplitHorizontal => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Pane, Mode, File, Prompt, Background],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::Save => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[File, Text, Prompt, Background],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::ForceSave => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[File, Text, Prompt, Background],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::ShowTutorial => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View, Pane, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::FocusWindowLeft
        | C::FocusWindowDown
        | C::FocusWindowUp
        | C::FocusWindowRight
        | C::NextWindow => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[Pane, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::SwapWindow => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[Pane, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::MoveWindowLeft | C::MoveWindowDown | C::MoveWindowUp | C::MoveWindowRight => {
            ActionPolicy {
                global_binding_modes: ALL,
                effects: &[Pane],
                count: CountBehavior::Reject,
                operands: OperandArity::Zero,
                requires_writable_buffer: false,
                buffer_mutation: BufferMutation::Unchanged,
            }
        }
        C::CloseWindow => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Pane, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OnlyWindow | C::EqualizeWindows => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Pane],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenTerminal => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Background, View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenTerminalFileDirectory
        | C::OpenTerminalDirectoryRoot
        | C::OpenTerminalSelectedDirectory
        | C::OpenTerminalSessionDirectory => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Background, View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::RenameTerminal => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Prompt, Configuration],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::SendToTerminal => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[TerminalInput],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::DeleteWordBackward
        | C::DeleteWordForward
        | C::DeleteToLineStart
        | C::DeleteToLineEnd
        | C::DeleteCharBackward
        | C::DeleteCharForward
        | C::InsertNewline
        | C::InsertTab
        | C::InsertLiteralTab => ActionPolicy {
            global_binding_modes: INSERT_REPLACE,
            effects: &[Text],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::CommitUndoCheckpoint => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[History],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::GotoDefinition
        | C::GotoDeclaration
        | C::GotoTypeDefinition
        | C::GotoReferences
        | C::GotoImplementation
        | C::ShowDocumentation
        | C::DocumentSymbols => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Background, View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::OpenNavigator => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::PreviousDestination => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::PreviousSession | C::NextRunningSession | C::PreviousRunningSession => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[File, View, Background],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::Session1
        | C::Session2
        | C::Session3
        | C::Session4
        | C::Session5
        | C::Session6
        | C::Session7
        | C::Session8
        | C::Session9 => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[File, View, Background],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::ListAccept | C::ListOpenActions => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View, Prompt],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::ListCloseTarget => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[View, Prompt, Background, Text, File, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::WorkspaceSymbols => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Prompt, Background, View],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::GlobalSearchRegex | C::GlobalSearch => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Prompt, Background, View, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::TriggerCompletion => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[Background, Prompt, Text, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::JumpBackward | C::JumpForward | C::JumpBackwardBuffer | C::JumpForwardBuffer => {
            ActionPolicy {
                global_binding_modes: NORMAL_SELECT,
                effects: &[View, Selection, Mode],
                count: CountBehavior::Reject,
                operands: OperandArity::Zero,
                requires_writable_buffer: false,
                buffer_mutation: BufferMutation::Unchanged,
            }
        }
        C::RenameSymbol | C::CodeAction => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Background, Prompt, Text],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::ClipboardPasteAfter | C::ClipboardPasteBefore => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Text, Selection, Mode, TerminalInput],
            count: CountBehavior::Interpret,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::ClipboardYank => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[Register],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::ClipboardPaste => ActionPolicy {
            global_binding_modes: ALL,
            effects: &[Text, TerminalInput, Mode],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: true,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::SelectRegister => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Operand, Register],
            count: CountBehavior::Reject,
            operands: OperandArity::One,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::RecordMacro => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Operand, Macro],
            count: CountBehavior::Reject,
            operands: OperandArity::One,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::RecordDefaultMacro => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Macro],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::StopMacroRecording => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Macro],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
        C::ReplayMacro => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Operand, Macro, Text, Background],
            count: CountBehavior::Repeat,
            operands: OperandArity::One,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::ReplayDefaultMacro => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Macro, Text, Background],
            count: CountBehavior::Repeat,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::ShellPipe => ActionPolicy {
            global_binding_modes: NONE,
            effects: &[Prompt, Background, Text],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::MayMutate,
        },
        C::MatchBracket => ActionPolicy {
            global_binding_modes: NORMAL_SELECT,
            effects: &[Selection],
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            requires_writable_buffer: false,
            buffer_mutation: BufferMutation::Unchanged,
        },
    }
}
