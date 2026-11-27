// SPDX-License-Identifier: MPL-2.0
use runyte::command::{
    ColonCommand, CommandId, EditorCommand as C,
    descriptor::{ActionEffect, SemanticClass, SplitStage},
};

fn identities() -> impl Iterator<Item = CommandId> {
    C::ALL
        .iter()
        .copied()
        .map(CommandId::Editor)
        .chain(ColonCommand::ALL.iter().copied().map(CommandId::Colon))
}

#[test]
fn every_builtin_has_an_honest_current_classification() {
    for id in identities() {
        let d = id.descriptor().unwrap();
        let class = d.classification;
        if class.pending_split.is_none() && class.semantic == SemanticClass::Atomic {
            assert!(
                !d.policy.effects.contains(&ActionEffect::Mode),
                "{id:?} has an unclassified mode side effect"
            );
        }
        if let SemanticClass::Decision(reason) = class.semantic {
            assert!(!reason.trim().is_empty(), "{id:?}");
        }
        assert!(!class.description().is_empty());
    }
    assert!(CommandId::Plugin(1).descriptor().is_none());
}

#[test]
fn migration_sets_are_exact_and_must_empty_before_their_stages_complete() {
    // Independent reviewed inventories (decisions 34 and 36). Removing an entry
    // requires its actual split, not merely relabelling its descriptor.
    let stage3 = [
        CommandId::Editor(C::DeleteSelection),
        CommandId::Editor(C::ChangeSelection),
        CommandId::Editor(C::FlipSelection),
        CommandId::Editor(C::SplitSelectionAtLineEnds),
        CommandId::Editor(C::SplitSelectionAtLineStarts),
        CommandId::Editor(C::ExpandSyntaxSelection),
        CommandId::Editor(C::ShrinkSyntaxSelection),
        CommandId::Editor(C::SelectSyntaxParent),
        CommandId::Editor(C::SelectSyntaxChild),
        CommandId::Editor(C::SelectPreviousSyntaxSibling),
        CommandId::Editor(C::SelectNextSyntaxSibling),
        CommandId::Editor(C::SelectSyntaxFunction),
        CommandId::Editor(C::SelectInsideSyntaxFunction),
        CommandId::Editor(C::SelectSyntaxClass),
        CommandId::Editor(C::SelectInsideSyntaxClass),
        CommandId::Editor(C::SelectSyntaxParameter),
        CommandId::Editor(C::SelectInsideSyntaxParameter),
        CommandId::Editor(C::SelectAroundParentheses),
        CommandId::Editor(C::SelectInsideParentheses),
        CommandId::Editor(C::SelectAroundSquareBrackets),
        CommandId::Editor(C::SelectInsideSquareBrackets),
        CommandId::Editor(C::SelectAroundBraces),
        CommandId::Editor(C::SelectInsideBraces),
        CommandId::Editor(C::SelectAroundAngleBrackets),
        CommandId::Editor(C::SelectInsideAngleBrackets),
        CommandId::Editor(C::SelectAroundDoubleQuotes),
        CommandId::Editor(C::SelectInsideDoubleQuotes),
        CommandId::Editor(C::SelectAroundSingleQuotes),
        CommandId::Editor(C::SelectInsideSingleQuotes),
        CommandId::Editor(C::SelectAroundBackticks),
        CommandId::Editor(C::SelectInsideBackticks),
        CommandId::Editor(C::SelectInsideWord),
        CommandId::Editor(C::SelectAroundWord),
        CommandId::Editor(C::SelectInsideLongWord),
        CommandId::Editor(C::SelectAroundLongWord),
        CommandId::Editor(C::SelectInsideParagraph),
        CommandId::Editor(C::SelectAroundParagraph),
        CommandId::Editor(C::SelectAroundClosestDelimiter),
        CommandId::Editor(C::SelectInsideClosestDelimiter),
    ];
    let stage4 = [
        CommandId::Editor(C::YankCollapseNormalMode),
        CommandId::Editor(C::YankLineCollapseNormalMode),
        CommandId::Editor(C::PasteAfterOrReplace),
        CommandId::Editor(C::PasteBefore),
        CommandId::Editor(C::ClipboardPasteAfter),
        CommandId::Editor(C::ClipboardPasteBefore),
        CommandId::Editor(C::SearchNext),
        CommandId::Editor(C::SearchPrevious),
        CommandId::Editor(C::ClipboardPaste),
        CommandId::Editor(C::SelectLine),
        CommandId::Editor(C::SelectLineUp),
        CommandId::Editor(C::ExtendLineAbove),
        CommandId::Editor(C::ExtendLineBelow),
    ];
    let stage5 = [
        CommandId::Editor(C::EnterNormalMode),
        CommandId::Editor(C::EnterInsertMode),
        CommandId::Editor(C::EnterReplaceMode),
        CommandId::Editor(C::EnterSelectMode),
    ];
    for (stage, expected) in [
        (SplitStage::Stage3, stage3.as_slice()),
        (SplitStage::Stage4, stage4.as_slice()),
        (SplitStage::Stage5, stage5.as_slice()),
    ] {
        let actual: Vec<_> = identities()
            .filter(|id| id.descriptor().unwrap().classification.pending_split == Some(stage))
            .collect();
        assert_eq!(actual.len(), expected.len(), "{stage:?}");
        for id in expected {
            assert!(actual.contains(id), "{stage:?}: {id:?}");
        }
    }
}

#[test]
fn retained_decisions_and_mode_actions_keep_their_contracts() {
    for c in [
        C::PasteAfterOrReplace,
        C::CutSelection,
        C::CutSelectionForChange,
        C::Search,
        C::SearchRegex,
        C::MediaBack,
        C::MediaMoveUp,
        C::MediaMoveDown,
        C::ClipboardPaste,
    ] {
        assert!(
            matches!(
                c.descriptor().classification.semantic,
                SemanticClass::Decision(_)
            ),
            "{c:?}"
        );
    }
    for c in [ColonCommand::WriteQuit, ColonCommand::WriteBufferClose] {
        assert!(matches!(
            c.descriptor().classification.semantic,
            SemanticClass::Decision(_)
        ));
    }
    for c in [
        C::ModeEnter,
        C::EnterNormalMode,
        C::EnterSelectMode,
        C::EnterInsertMode,
        C::EnterReplaceMode,
        C::AppendAfter,
        C::InsertLineStart,
        C::InsertLineEnd,
        C::OpenLineAbove,
        C::OpenLineBelow,
    ] {
        assert_eq!(
            c.descriptor().classification.semantic,
            SemanticClass::ModeAction
        );
    }
    assert!(
        C::ClipboardPaste
            .action_policy()
            .effects
            .contains(&ActionEffect::Mode)
    );
    let paste = C::PasteAfterOrReplace.descriptor().classification;
    assert_eq!(paste.pending_split, Some(SplitStage::Stage4));
    assert!(paste.description().contains("each selection range"));
}

#[test]
fn plan_pending_inventories_match_current_descriptors() {
    let plan = include_str!("../context/plans/active/PLAN_COMPOSABLE_KEY_BINDINGS.md")
        .replace("\r\n", "\n");
    for (stage, number) in [
        (SplitStage::Stage3, 3),
        (SplitStage::Stage4, 4),
        (SplitStage::Stage5, 5),
    ] {
        let section = plan
            .split_once(&format!("### Pending stage {number} splits\n"))
            .unwrap()
            .1;
        let section = section.split("\n##").next().unwrap();
        let mut documented: Vec<_> = section
            .split('`')
            .enumerate()
            .filter_map(|(i, s)| (i % 2 == 1).then_some(s))
            .collect();
        let mut actual: Vec<_> = identities()
            .filter_map(|id| {
                let d = id.descriptor().unwrap();
                (d.classification.pending_split == Some(stage)).then_some(d.metadata.name)
            })
            .collect();
        documented.sort_unstable();
        actual.sort_unstable();
        assert_eq!(documented, actual, "stage {number} plan inventory drifted");
    }
}
