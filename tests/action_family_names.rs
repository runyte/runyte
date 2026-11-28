// SPDX-License-Identifier: MPL-2.0

use runyte::{
    command::{self, CommandId, CommandParseError, EditorCommand as C},
    keymap::{KeySequence, Lookup, configured, default_keymap},
};
use std::collections::HashSet;

fn migration_table(document: &str, heading: &str) -> Vec<(String, String)> {
    document
        .split_once(heading)
        .expect("migration heading")
        .1
        .lines()
        .skip_while(|line| !line.starts_with("| Old action |"))
        .skip(2)
        .take_while(|line| line.starts_with("| `"))
        .map(|line| {
            let cells: Vec<_> = line.split('`').collect();
            (cells[1].to_owned(), cells[3].to_owned())
        })
        .collect()
}

#[test]
fn family_migrations_are_unique_documented_and_refused_in_every_binding_form() {
    let guide = migration_table(
        include_str!("../docs/user-guide/custom-key-bindings.md"),
        "### Action name migration",
    );
    let plan = migration_table(
        include_str!("../context/plans/active/PLAN_COMPOSABLE_KEY_BINDINGS.md"),
        "### Decision 47",
    );
    assert_eq!(guide, plan);
    assert_eq!(guide.len(), 66);
    let names: HashSet<_> = C::ALL.iter().map(|c| c.metadata().name).collect();
    assert_eq!(names.len(), C::ALL.len());
    let mut old_names = HashSet::new();
    let mut new_names = HashSet::new();
    let mut yaml = String::from("bind:\n  normal:\n");
    let mut cases = Vec::new();
    for (old, new) in guide {
        assert!(old_names.insert(old.clone()));
        assert!(new_names.insert(new.clone()));
        assert!(!names.contains(old.as_str()));
        assert!(names.contains(new.as_str()), "missing replacement {new}");
        assert!(command::resolve_command(&old).is_none());
        let replacement = format!("use {new}");
        assert_eq!(
            command::removed_action_replacement(&old),
            Some(replacement.as_str())
        );
        for argument in [None, Some("value")] {
            assert!(matches!(
                command::parse_named_command(&old, argument),
                Err(CommandParseError::Removed { replacement: advice, .. }) if advice == replacement
            ));
        }
        for binding in [
            old.clone(),
            format!("[move-right, {old}]"),
            format!("{{action: {old}}}"),
            format!("{{action: {old}, arg: value}}"),
        ] {
            let index = cases.len();
            let key = format!(
                "F6 {} {}",
                char::from(b'a' + (index / 26) as u8),
                char::from(b'a' + (index % 26) as u8)
            );
            yaml.push_str(&format!("    '{key}': {binding}\n"));
            cases.push((key, old.clone(), replacement.clone()));
        }
    }
    // Validate all forms in one compilation; invalid programs must install no
    // partial binding, including the valid move-right prefix in each sequence.
    let value = serde_yaml::from_str(&yaml).unwrap();
    let result = configured::compile(&value, default_keymap());
    assert_eq!(result.errors.len(), cases.len());
    for (key, old, replacement) in cases {
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains(&key) && e.contains(&old) && e.contains(&replacement)),
            "{key}: {:?}",
            result.errors
        );
        assert!(matches!(
            result
                .keymap
                .lookup(command::Mode::Normal, &KeySequence::parse(&key).unwrap()),
            Lookup::NoMatch
        ));
    }
}

#[test]
fn shared_colon_migrations_preserve_identity_and_existing_terminal_spellings() {
    for (new, expected, argument) in [
        ("directory-open-system", C::OpenExplorerSystem, None),
        ("directory-open-finder", C::OpenExplorerFinder, None),
        ("directory-open-workspace", C::OpenExplorerWorkspace, None),
        ("git-network-next-page", C::NextGitNetworkPage, None),
        ("git-network-previous-page", C::PreviousGitNetworkPage, None),
        ("session-open-directory", C::OpenSessionDirectory, None),
        ("session-open-explorer", C::OpenExplorerSession, None),
        ("outline", C::DocumentOutline, None),
        ("syntax-document-outline", C::DocumentOutline, None),
        ("terminal", C::OpenTerminal, None),
        (
            "terminal-file-directory",
            C::OpenTerminalFileDirectory,
            None,
        ),
        ("terminal-show", C::ShowTerminal, Some("1")),
        ("terminal-rename", C::RenameTerminal, Some("build")),
        ("terminal-send", C::SendToTerminal, Some("1")),
    ] {
        let invocation = command::parse_named_command(new, argument).unwrap();
        assert_eq!(invocation.id(), CommandId::Editor(expected));
    }
    for command in [
        C::FitSelectionForNormalMode,
        C::ReplaceWithRegister,
        C::MatchBracket,
        C::SelectSyntaxFunction,
        C::GotoNextSyntaxFunction,
    ] {
        assert!(command::removed_action_replacement(command.metadata().name).is_none());
    }
}
