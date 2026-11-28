// SPDX-License-Identifier: MPL-2.0
//! Explicit classification inventory. No wildcard: new identities require review.
use super::{ActionClassification, SemanticClass, SplitStage};
use crate::command::{ColonCommand, EditorCommand};

pub(super) const fn editor_classification(command: EditorCommand) -> ActionClassification {
    use EditorCommand as C;
    match command {
        C::InsertText
        | C::TerminalUndoPaste
        | C::TerminalPaste
        | C::TerminalClipboardPasteText
        | C::TerminalSearch
        | C::TerminalSearchRegex
        | C::TerminalSearchNext
        | C::TerminalSearchPrevious
        | C::SurroundAdd
        | C::SurroundReplace
        | C::SurroundDelete
        | C::ReplaceWithRegister
        | C::FitSelectionForNormalMode
        | C::OpenCommandPalette
        | C::MoveLeft
        | C::MoveRight
        | C::MoveUp
        | C::MoveDown
        | C::MoveLineStart
        | C::MoveLineEnd
        | C::MoveFirstNonWhitespace
        | C::MoveFileStart
        | C::MoveFileEnd
        | C::MoveWordForward
        | C::MoveWordBackward
        | C::MoveWordEnd
        | C::MoveLongWordForward
        | C::MoveLongWordBackward
        | C::MoveLongWordEnd
        | C::GotoNextParagraph
        | C::GotoPreviousParagraph
        | C::FindNextChar
        | C::FindPreviousChar
        | C::FindTillNextChar
        | C::FindTillPreviousChar
        | C::PageUp
        | C::PageDown
        | C::HalfPageUp
        | C::HalfPageDown
        | C::ReplaceChar
        | C::ToggleCase
        | C::Undo
        | C::Redo
        | C::Yank
        | C::YankLine
        | C::PasteAfter
        | C::Indent
        | C::Unindent
        | C::ToggleComments
        | C::SelectAll
        | C::CollapseSelection
        | C::GotoPreviousSyntaxFunction
        | C::GotoNextSyntaxFunction
        | C::GotoPreviousSyntaxClass
        | C::GotoNextSyntaxClass
        | C::GotoPreviousSyntaxParameter
        | C::GotoNextSyntaxParameter
        | C::DocumentOutline
        | C::ToggleSyntaxFold
        | C::FoldAllSyntax
        | C::UnfoldAllSyntax
        | C::KeepPrimarySelection
        | C::RemovePrimarySelection
        | C::CopySelectionDown
        | C::CopySelectionUp
        | C::CopySelectionDownPadded
        | C::CopySelectionUpPadded
        | C::RotateSelectionForward
        | C::RotateSelectionBackward
        | C::RotateSelectionContentsForward
        | C::RotateSelectionContentsBackward
        | C::KeepMatchingSelections
        | C::RemoveMatchingSelections
        | C::AlignSelections
        | C::TrimSelections
        | C::TrimTrailingWhitespace
        | C::HardWrap
        | C::Reflow
        | C::JoinSelections
        | C::JoinLines
        | C::FormatTable
        | C::SearchExtendNext
        | C::SearchExtendPrevious
        | C::SearchSelectAllMatches
        | C::SearchSetFromSelection
        | C::MediaZoomIn
        | C::MediaZoomOut
        | C::MediaFit
        | C::MediaActualSize
        | C::MediaPanLeft
        | C::MediaPanRight
        | C::MediaCopySelection
        | C::MediaSelectAll
        | C::AlignViewCenter
        | C::AlignViewTop
        | C::AlignViewBottom
        | C::AlignViewMiddle
        | C::ScrollViewDown
        | C::ScrollViewUp
        | C::GotoWindowTop
        | C::GotoWindowCenter
        | C::GotoWindowBottom
        | C::GotoFile
        | C::GotoWord
        | C::ToggleSoftWrap
        | C::ToggleWhitespace
        | C::PreviewDocument
        | C::ToggleFocus
        | C::OpenExplorer
        | C::DirectoryTreeUp
        | C::DirectoryTreeDown
        | C::DirectoryTreeCollapse
        | C::DirectoryTreeSelectParent
        | C::DirectoryTreeExpand
        | C::DirectoryTreeSelectFirstChild
        | C::DirectoryTreeSearch
        | C::DirectoryTreeSearchNext
        | C::DirectoryTreeSearchPrevious
        | C::DirectoryTreeFirst
        | C::DirectoryTreeLast
        | C::DirectoryTreePageUp
        | C::DirectoryTreePageDown
        | C::DirectoryTreeRefresh
        | C::DirectoryTreeLegend
        | C::DirectoryTreeNew
        | C::DirectoryTreeRename
        | C::DirectoryTreeDelete
        | C::DirectoryTreeMove
        | C::OpenWorkingDirectoryExplorer
        | C::OpenFilePicker
        | C::OpenAllFilesPicker
        | C::OpenPathFilePicker
        | C::OpenDirectoryFilePicker
        | C::OpenFuzzyGrep
        | C::OpenDirectoryFuzzyGrep
        | C::RefreshDirectory
        | C::ToggleHiddenFiles
        | C::ToggleDirectoryDetails
        | C::ChooseExplorerOrder
        | C::OpenExplorerSystem
        | C::OpenExplorerFinder
        | C::OpenChangedFile
        | C::StageAllChangedFiles
        | C::CheckoutBranch
        | C::CreateBranch
        | C::DeleteBranch
        | C::OpenGitConflict
        | C::NextConflict
        | C::PreviousConflict
        | C::KeepConflictCurrent
        | C::TakeConflictOther
        | C::InspectConflictSides
        | C::MergeReviewCancel
        | C::MergeReviewBack
        | C::MergeReviewNext
        | C::MergeReviewPrevious
        | C::MergeReviewLeft
        | C::MergeReviewRight
        | C::FetchBranch
        | C::PullBranch
        | C::PushBranch
        | C::OpenWorktree
        | C::CreateWorktree
        | C::CreateNewWorktree
        | C::RemoveWorktree
        | C::OpenWorkspaceSearchResult
        | C::OverrideSettingLanguage
        | C::OverrideSettingPattern
        | C::RemoveSettingOverride
        | C::ActivateSetting
        | C::OpenThemeSettings
        | C::Save
        | C::ForceSave
        | C::MoveWindowLeft
        | C::MoveWindowDown
        | C::MoveWindowUp
        | C::MoveWindowRight
        | C::OnlyWindow
        | C::EqualizeWindows
        | C::OpenTerminalList
        | C::RenameTerminal
        | C::SendToTerminal
        | C::DeleteWordBackward
        | C::DeleteWordForward
        | C::DeleteToLineStart
        | C::DeleteToLineEnd
        | C::DeleteCharBackward
        | C::DeleteCharForward
        | C::InsertNewline
        | C::InsertTab
        | C::InsertLiteralTab
        | C::CommitUndoCheckpoint
        | C::GotoDefinition
        | C::GotoDeclaration
        | C::GotoTypeDefinition
        | C::GotoReferences
        | C::GotoImplementation
        | C::NewBuffer
        | C::OpenNavigator
        | C::PreviousSession
        | C::Session1
        | C::Session2
        | C::Session3
        | C::Session4
        | C::Session5
        | C::Session6
        | C::Session7
        | C::Session8
        | C::Session9
        | C::NextRunningSession
        | C::PreviousRunningSession
        | C::OpenSessionDirectory
        | C::OpenExplorerSession
        | C::OpenExplorerWorkspace
        | C::OpenBufferPicker
        | C::ListAccept
        | C::ListOpenActions
        | C::ListBackspace
        | C::ListNext
        | C::ListPrevious
        | C::ListPageDown
        | C::ListPageUp
        | C::ListFirst
        | C::ListLast
        | C::ListTogglePreview
        | C::ListClearFilter
        | C::ListClose
        | C::OpenSessionDestinations
        | C::ShowDocumentation
        | C::DocumentSymbols
        | C::WorkspaceSymbols
        | C::Diagnostics
        | C::RenameSymbol
        | C::CodeAction
        | C::ClipboardYank
        | C::SelectRegister
        | C::RecordMacro
        | C::RecordDefaultMacro
        | C::StopMacroRecording
        | C::ReplayMacro
        | C::ReplayDefaultMacro
        | C::ListMacros
        | C::ShellPipe
        | C::MatchBracket => ActionClassification {
            semantic: SemanticClass::Atomic,
            pending_split: None,
        },
        C::SelectionUndo | C::SelectionRedo => ActionClassification {
            semantic: SemanticClass::Decision(
                "Restores the selection and mode together from the chosen history entry.",
            ),
            pending_split: None,
        },
        C::ListCloseTarget => ActionClassification {
            semantic: SemanticClass::Decision(
                "Closes the captured target only after its guards or confirmation succeed and reconciles the surviving destination mode.",
            ),
            pending_split: None,
        },
        C::ToggleZen | C::ToggleFullscreen => ActionClassification {
            semantic: SemanticClass::Decision(
                "Restores the previous editor mode when leaving directory-tree focus before changing the window presentation.",
            ),
            pending_split: None,
        },
        C::TerminalEnterInput
        | C::ModeEnter
        | C::AppendAfter
        | C::InsertLineStart
        | C::InsertLineEnd
        | C::OpenLineBelow
        | C::OpenLineAbove => ActionClassification {
            semantic: SemanticClass::ModeAction,
            pending_split: None,
        },
        C::EnterNormalMode | C::EnterInsertMode | C::EnterReplaceMode | C::EnterSelectMode => {
            ActionClassification {
                semantic: SemanticClass::ModeAction,
                pending_split: Some(SplitStage::Stage5),
            }
        }
        C::DeleteSelection | C::DeleteSelectionForChange => ActionClassification {
            semantic: SemanticClass::Decision(
                "Delete and change place the caret and treat whole rows differently.",
            ),
            pending_split: None,
        },
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
        | C::SelectInsideClosestDelimiter => ActionClassification {
            semantic: SemanticClass::Atomic,
            pending_split: None,
        },
        C::ShrinkSyntaxSelection => ActionClassification {
            semantic: SemanticClass::Decision(
                "Restores selection and mode from expansion history that is not visible between program steps.",
            ),
            pending_split: None,
        },
        C::SelectInsideParagraph
        | C::SelectAroundParagraph
        | C::SelectLine
        | C::SelectLineUp
        | C::ExtendLineAbove
        | C::ExtendLineBelow => ActionClassification {
            semantic: SemanticClass::Atomic,
            pending_split: None,
        },
        C::PasteBefore
        | C::SearchNext
        | C::SearchPrevious
        | C::ClipboardPasteAfter
        | C::ClipboardPasteBefore => ActionClassification {
            semantic: SemanticClass::Atomic,
            pending_split: None,
        },
        C::DirectoryPaste => ActionClassification {
            semantic: SemanticClass::Decision(
                "Stages directory copy or move payloads beside entry rows, or pastes a text register using each range's insertion/replacement decision.",
            ),
            pending_split: None,
        },
        C::PasteAfterOrReplace => ActionClassification {
            semantic: SemanticClass::Decision(
                "Chooses insertion or replacement independently for each selection range.",
            ),
            pending_split: None,
        },
        C::CutSelection => ActionClassification {
            semantic: SemanticClass::Decision(
                "Chooses linewise register content or directory move payload and coordinates deletion; independent yank/delete steps cannot retain that ownership.",
            ),
            pending_split: None,
        },
        C::CutSelectionForChange => ActionClassification {
            semantic: SemanticClass::Decision(
                "Chooses operative-span register content or directory move payload and coordinates deletion; independent yank/delete steps cannot retain that ownership.",
            ),
            pending_split: None,
        },
        C::Search => ActionClassification {
            semantic: SemanticClass::Decision(
                "Owns the search prompt and enters Select only when its accepted search succeeds (decision 30).",
            ),
            pending_split: None,
        },
        C::SearchRegex => ActionClassification {
            semantic: SemanticClass::Decision(
                "Owns the regular-expression search prompt and its selection-mode completion (decision 30).",
            ),
            pending_split: None,
        },
        C::MediaShowPage => ActionClassification {
            semantic: SemanticClass::Decision(
                "Chooses and presents a native media page with its corresponding selection and mode.",
            ),
            pending_split: None,
        },
        C::MediaMoveUp | C::MediaMoveDown => ActionClassification {
            semantic: SemanticClass::Decision(
                "Pans zoomed or non-PDF media, but changes PDF pages at fit zoom; the native viewport owns that decision (decision 31).",
            ),
            pending_split: None,
        },
        C::MediaBack => ActionClassification {
            semantic: SemanticClass::Decision(
                "Fits zoomed media, otherwise clears native selection, otherwise leaves preview; authoritative native state chooses the branch (decision 31).",
            ),
            pending_split: None,
        },
        C::ShowAbout
        | C::CopyTerminalOutput
        | C::ToggleMarkdownRender
        | C::GlobalSearch
        | C::GlobalSearchRegex
        | C::JumpBackward
        | C::JumpForward
        | C::JumpBackwardBuffer
        | C::JumpForwardBuffer
        | C::PreviousDestination
        | C::SwapWindow
        | C::ToggleDirectoryTree
        | C::FocusDirectoryTree
        | C::DirectoryTreeOpen
        | C::DirectoryTreeClose
        | C::DirectoryTreeVertical
        | C::DirectoryTreeHorizontal
        | C::DirectoryTreePane1
        | C::DirectoryTreePane2
        | C::DirectoryTreePane3
        | C::DirectoryTreePane4
        | C::DirectoryTreePane5
        | C::DirectoryTreePane6
        | C::DirectoryTreePane7
        | C::DirectoryTreePane8
        | C::DirectoryTreePane9
        | C::OpenDirectoryEntry
        | C::OpenParentDirectory
        | C::MergeBranch
        | C::OpenGitConflicts
        | C::ContinueMerge
        | C::AbortMerge
        | C::ResolveConflictLiteralMarkers
        | C::ResolveConflict
        | C::ReturnToGitConflicts
        | C::KeepConflictFileCurrent
        | C::TakeConflictFileOther
        | C::MergeReviewApprove
        | C::MergeReviewEnter
        | C::NextGitNetworkPage
        | C::PreviousGitNetworkPage
        | C::NextGitLogPage
        | C::PreviousGitLogPage
        | C::OpenGitCommit
        | C::OpenSettings
        | C::SplitVertical
        | C::SplitHorizontal
        | C::ShowHelp
        | C::ShowTutorial
        | C::FocusWindowLeft
        | C::FocusWindowDown
        | C::FocusWindowUp
        | C::FocusWindowRight
        | C::NextWindow
        | C::CloseWindow
        | C::OpenTerminal
        | C::OpenTerminalFileDirectory
        | C::OpenTerminalDirectoryRoot
        | C::OpenTerminalSelectedDirectory
        | C::OpenTerminalSessionDirectory
        | C::ShowTerminal
        | C::LeaveTerminal => ActionClassification {
            semantic: SemanticClass::Decision(
                "Activates or restores a destination and its mode as one guarded workflow.",
            ),
            pending_split: None,
        },
        C::TriggerCompletion => ActionClassification {
            semantic: SemanticClass::Decision(
                "Enters Insert only when a language server is available, then owns the completion request and later text acceptance.",
            ),
            pending_split: None,
        },
        C::ClipboardPaste => ActionClassification {
            semantic: SemanticClass::Decision(
                "Chooses image insertion or text paste from clipboard content.",
            ),
            pending_split: None,
        },
    }
}

pub(super) const fn colon_classification(command: ColonCommand) -> ActionClassification {
    use ColonCommand as C;
    match command {
        C::ChangeDirectory
        | C::InitializeWorkspace
        | C::CloseBuffer
        | C::DiffDisk
        | C::DiffRemote
        | C::DiffOff
        | C::DiffThis
        | C::ForceCloseBuffer
        | C::Format
        | C::GitCancel
        | C::GitDiffSideBySide
        | C::GitDiscard
        | C::GitNetworkChooseRef
        | C::GitSearchCommits
        | C::GitRefresh
        | C::GitStage
        | C::GitStashTracked
        | C::GitStashAll
        | C::GitStashUntracked
        | C::GitStashApply
        | C::GitStashDrop
        | C::GitStageHunk
        | C::GitUnstageHunk
        | C::GitStageLines
        | C::GitUnstage
        | C::ConfigReload
        | C::WriteConfig
        | C::ForceWriteConfig
        | C::LspTrust
        | C::ContextAccess
        | C::LspRestart
        | C::LspStatus
        | C::Open
        | C::Path
        | C::CopyBinding
        | C::Detach
        | C::Quit
        | C::ForceQuit
        | C::QuitAll
        | C::ForceQuitAll
        | C::TerminateQuitAll
        | C::QuitHere
        | C::ForceQuitHere
        | C::Reload
        | C::ServiceHealth
        | C::Pipe
        | C::PipeCancel
        | C::Plugins
        | C::PluginStop
        | C::PluginRestart
        | C::SessionAttach
        | C::SessionList
        | C::SessionStop
        | C::SessionRename
        | C::SessionClean => ActionClassification {
            semantic: SemanticClass::Atomic,
            pending_split: None,
        },
        C::AgentChannel
        | C::LogOpen
        | C::GitCompare
        | C::GitDiff
        | C::GitIndex
        | C::GitBranches
        | C::GitBlame
        | C::GitBlameFile
        | C::GitCommit
        | C::GitLog
        | C::GitNetwork
        | C::GitNetworkHead
        | C::GitNetworkAll
        | C::GitNetworkRef
        | C::GitStatus
        | C::GitStashes
        | C::GitWorktrees
        | C::Notifications
        | C::DescribeKey
        | C::ActionBrowser => ActionClassification {
            semantic: SemanticClass::Decision(
                "Opens or updates a workflow destination with the mode belonging to that destination.",
            ),
            pending_split: None,
        },
        C::Grammar => ActionClassification {
            semantic: SemanticClass::Decision(
                "Changes the editing grammar and reconciles its current mode and selection.",
            ),
            pending_split: None,
        },
        C::Resize => ActionClassification {
            semantic: SemanticClass::Decision("Owns the resize prompt and its mode transition."),
            pending_split: None,
        },
        C::WriteQuit | C::WriteBufferClose => ActionClassification {
            semantic: SemanticClass::Decision(
                "Owns the save continuation; closes only after successful save and preserves commit-message or changed-buffer guards.",
            ),
            pending_split: None,
        },
    }
}
