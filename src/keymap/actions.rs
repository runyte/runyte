// SPDX-License-Identifier: MPL-2.0

//! Bounded semantic action bindings layered over the resolved keymap.

use super::{
    Binding, BindingAvailability, BindingScope, BindingTarget, Key, KeySequence, Keymap, validate,
};
use crate::command::{self, CommandId, CommandInvocation, EditorCommand, Mode};
use crate::input::KeyCode;
use serde_yaml::Value;
use std::collections::{BTreeMap, HashSet};

const MAX_BINDINGS: usize = 256;
const MAX_ACTIONS: usize = 16;
const MAX_ARGUMENT_BYTES: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    /// The grammar supplies a count or a subsequent character, as for a default key.
    Editor(EditorCommand),
    Invocation(CommandInvocation),
}

impl Action {
    pub fn target(&self) -> BindingTarget {
        match self {
            Self::Editor(command) => BindingTarget::Editor(*command),
            Self::Invocation(invocation) => match invocation.id() {
                CommandId::Editor(command) => BindingTarget::Editor(command),
                CommandId::Colon(command) => BindingTarget::Colon(command),
                CommandId::Plugin(_) => unreachable!("configured built-in actions exclude plugins"),
            },
        }
    }
}

pub fn editor_modes(command: EditorCommand) -> Vec<Mode> {
    if command::INTERNAL_EDITOR_COMMANDS.contains(&command)
        || command::GRAMMAR_ONLY_EDITOR_COMMANDS.contains(&command)
        || command == EditorCommand::ShellPipe
    {
        return Vec::new();
    }
    let map = super::default_keymap();
    let target = BindingTarget::Editor(command);
    [Mode::Normal, Mode::Select, Mode::Insert, Mode::Replace]
        .into_iter()
        .filter(|mode| {
            map.bindings().iter().any(|binding| {
                binding.scope == BindingScope::Global
                    && binding.target == target
                    && binding.is_active_in(*mode)
                    && binding.availability == BindingAvailability::Implemented
            }) || (matches!(mode, Mode::Normal | Mode::Select)
                && matches!(
                    command,
                    EditorCommand::ExtendLineAbove
                        | EditorCommand::ExtendLineBelow
                        | EditorCommand::SelectLineUp
                ))
        })
        .collect()
}

/// Commands whose completed synchronous effect can feed the next action.
/// Everything else is available only as the final action. Extend deliberately.
pub fn can_continue(target: BindingTarget) -> bool {
    use EditorCommand as C;
    matches!(
        target,
        BindingTarget::Editor(
            C::MoveLeft
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
                | C::PageUp
                | C::PageDown
                | C::HalfPageUp
                | C::HalfPageDown
                | C::GotoWindowTop
                | C::GotoWindowCenter
                | C::GotoWindowBottom
                | C::SelectLine
                | C::SelectLineUp
                | C::ExtendLineAbove
                | C::ExtendLineBelow
                | C::SelectAll
                | C::CollapseSelection
                | C::FlipSelection
                | C::KeepPrimarySelection
                | C::RemovePrimarySelection
                | C::CopySelectionDown
                | C::CopySelectionUp
                | C::CopySelectionDownPadded
                | C::CopySelectionUpPadded
                | C::RotateSelectionForward
                | C::RotateSelectionBackward
                | C::AlignSelections
                | C::TrimSelections
                | C::SplitSelectionAtLineEnds
                | C::SplitSelectionAtLineStarts
                | C::Yank
                | C::YankLine
                | C::DeleteSelection
                | C::Indent
                | C::Unindent
                | C::ToggleCase
                | C::ToggleComments
                | C::PasteAfter
                | C::PasteBefore
                | C::Undo
                | C::Redo
                | C::SelectionUndo
                | C::SelectionRedo
                | C::DeleteWordBackward
                | C::DeleteWordForward
                | C::DeleteToLineStart
                | C::DeleteToLineEnd
                | C::DeleteCharBackward
                | C::DeleteCharForward
                | C::InsertNewline
                | C::InsertTab
                | C::InsertLiteralTab
        )
    )
}

pub fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "normal",
        Mode::Select => "select",
        Mode::Insert => "insert",
        Mode::Replace => "replace",
        Mode::Command => "command",
        Mode::List => "list",
    }
}

fn mode_slice(mode: Mode) -> &'static [Mode] {
    match mode {
        Mode::Normal => &[Mode::Normal],
        Mode::Select => &[Mode::Select],
        Mode::Insert => &[Mode::Insert],
        Mode::Replace => &[Mode::Replace],
        Mode::Command => &[Mode::Command],
        Mode::List => &[Mode::List],
    }
}

fn parse_action(value: &Value, mode: Mode) -> Result<(Action, String), String> {
    let (name, argument) = match value {
        Value::String(name) => (name.as_str(), None),
        Value::Mapping(mapping)
            if mapping.len() <= 2
                && mapping
                    .keys()
                    .all(|key| matches!(key.as_str(), Some("command" | "argument"))) =>
        {
            let name = mapping
                .get("command")
                .and_then(Value::as_str)
                .ok_or("command must be a string")?;
            let argument = mapping
                .get("argument")
                .map(|value| value.as_str().ok_or("argument must be a string"))
                .transpose()?;
            (name, argument)
        }
        _ => return Err("expected an action name or { command, argument }".into()),
    };
    if name.len() > 128
        || argument.is_some_and(|argument| {
            argument.len() > MAX_ARGUMENT_BYTES || argument.chars().any(char::is_control)
        })
    {
        return Err(
            "action name or argument exceeds its bound or contains control characters".into(),
        );
    }
    let editor = EditorCommand::ALL
        .iter()
        .copied()
        .find(|command| command.metadata().name == name);
    let action = if let Some(command) = editor {
        if !editor_modes(command).contains(&mode) {
            return Err(format!(
                "{name} is not a configurable {} action",
                mode_name(mode)
            ));
        }
        if let Some(argument) = argument {
            let spec = command::COMMANDS
                .iter()
                .find(|spec| spec.id == CommandId::Editor(command))
                .ok_or("this editor action does not take an argument")?;
            Action::Invocation(
                command::parse_named_command(spec.name, Some(argument))
                    .map_err(|error| error.to_string())?,
            )
        } else {
            Action::Editor(command)
        }
    } else {
        let invocation =
            command::parse_named_command(name, argument).map_err(|error| error.to_string())?;
        if let CommandId::Editor(command) = invocation.id() {
            if !editor_modes(command).contains(&mode) {
                return Err(format!(
                    "{name} is not a configurable {} action",
                    mode_name(mode)
                ));
            }
        } else if !matches!(mode, Mode::Normal | Mode::Select) {
            return Err("colon-only commands require normal or select mode".into());
        }
        Action::Invocation(invocation)
    };
    let label = match argument {
        Some(argument) => format!("{name} {argument}"),
        None => name.to_owned(),
    };
    Ok((action, label))
}

#[derive(Clone)]
struct Rule {
    mode: Mode,
    sequence: KeySequence,
    source: String,
    actions: Vec<Action>,
    description: String,
}

fn parse_rule(mode: Mode, source: &str, value: &Value, base: &Keymap) -> Result<Rule, String> {
    let tokens: Vec<_> = source.split_whitespace().collect();
    if tokens.is_empty() || tokens.len() > 8 {
        return Err("expected 1 through 8 keys".into());
    }
    let sequence = KeySequence::new(
        tokens
            .iter()
            .map(|token| match *token {
                "Leader" => Ok(base.leader()),
                "Window" => Ok(base.window_prefix()),
                _ => Key::parse(token),
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    if sequence.as_slice().iter().enumerate().any(|(index, key)| {
        (index > 0 && matches!(key.code, KeyCode::Escape | KeyCode::Backspace))
            || (index == 0
                && matches!(mode, Mode::Normal | Mode::Select)
                && key.modifiers.is_empty()
                && matches!(key.code, KeyCode::Char('1'..='9') | KeyCode::Tab))
    }) {
        return Err(
            "sequence uses a reserved count, context-action, or prefix-cancellation key".into(),
        );
    }
    let values: Vec<&Value> = match value {
        Value::Null => Vec::new(),
        Value::Sequence(values) if !values.is_empty() && values.len() <= MAX_ACTIONS => {
            values.iter().collect()
        }
        Value::Sequence(_) => {
            return Err("expected 1 through 16 actions; use null to unbind".into());
        }
        _ => vec![value],
    };
    let parsed = values
        .into_iter()
        .map(|value| parse_action(value, mode))
        .collect::<Result<Vec<_>, _>>()?;
    for (action, _) in parsed.iter().take(parsed.len().saturating_sub(1)) {
        if !can_continue(action.target()) {
            return Err(format!(
                "{} must be the last action",
                action.target().name()
            ));
        }
    }
    let description = parsed
        .iter()
        .map(|(_, label)| label.as_str())
        .collect::<Vec<_>>()
        .join(" → ");
    Ok(Rule {
        mode,
        sequence,
        source: source.into(),
        actions: parsed.into_iter().map(|(action, _)| action).collect(),
        description,
    })
}

pub(super) fn apply(section: Option<&Value>, base: Keymap, errors: &mut Vec<String>) -> Keymap {
    let Some(section) = section else {
        return base;
    };
    let Some(modes) = section.as_mapping() else {
        errors.push("bind rejected: expected mode mappings".into());
        return base;
    };
    if modes.len() > 4
        || modes
            .values()
            .filter_map(Value::as_mapping)
            .map(|mapping| mapping.len())
            .sum::<usize>()
            > MAX_BINDINGS
    {
        errors.push("bind rejected: at most 4 modes and 256 assignments are allowed".into());
        return base;
    }
    let mut rules = Vec::new();
    let mut seen = HashSet::new();
    for (name, mapping) in modes {
        let mode = match name.as_str() {
            Some("normal") => Mode::Normal,
            Some("select") => Mode::Select,
            Some("insert") => Mode::Insert,
            Some("replace") => Mode::Replace,
            _ => {
                errors.push(format!("bind rejected: unknown mode {name:?}"));
                continue;
            }
        };
        let Some(mapping) = mapping.as_mapping() else {
            errors.push(format!(
                "bind.{} rejected: expected a mapping",
                mode_name(mode)
            ));
            continue;
        };
        let ordered: BTreeMap<_, _> = mapping
            .iter()
            .filter_map(|(key, value)| match key.as_str() {
                Some(source) => Some((source, value)),
                None => {
                    errors.push("bind rejected: keys must be strings".into());
                    None
                }
            })
            .collect();
        for (source, value) in ordered {
            match parse_rule(mode, source, value, &base) {
                Ok(rule) if seen.insert((mode, rule.sequence.clone())) => rules.push(rule),
                Ok(_) => errors.push(format!(
                    "bind.{} {source:?} rejected: duplicate normalized key",
                    mode_name(mode)
                )),
                Err(reason) => errors.push(format!(
                    "bind.{} {source:?} rejected: {reason}",
                    mode_name(mode)
                )),
            }
        }
    }
    let mut active = vec![true; rules.len()];
    loop {
        let mut candidate = base.clone();
        for (rule, enabled) in rules.iter().zip(&active) {
            if !enabled {
                continue;
            }
            candidate.bindings = candidate
                .bindings
                .into_iter()
                .flat_map(|binding| {
                    if binding.scope == BindingScope::Global
                        && binding.sequence == rule.sequence
                        && binding.is_active_in(rule.mode)
                    {
                        let mut retained = binding
                            .modes
                            .iter()
                            .filter(|&&mode| mode != rule.mode)
                            .map(|&mode| {
                                let mut copy = binding.clone();
                                copy.modes = mode_slice(mode);
                                copy
                            })
                            .collect::<Vec<_>>();
                        if matches!(rule.mode, Mode::Insert | Mode::Replace)
                            && !base.bindings.iter().any(|original| {
                                original.scope == BindingScope::Terminal
                                    && original.is_active_in(rule.mode)
                                    && original.sequence == rule.sequence
                            })
                        {
                            let mut terminal = binding.clone();
                            terminal.scope = BindingScope::Terminal;
                            terminal.modes = mode_slice(rule.mode);
                            terminal.alias = None;
                            terminal.alias_modes = None;
                            retained.push(terminal);
                        }
                        retained
                    } else {
                        vec![binding]
                    }
                })
                .collect();
            if let Some(first) = rule.actions.first() {
                let mut binding = Binding::implemented(
                    mode_slice(rule.mode),
                    rule.sequence.clone(),
                    first.target(),
                );
                binding.actions = rule.actions.clone();
                binding.description = rule.description.clone().into();
                candidate.bindings.push(binding);
            }
        }
        // Remove stale alias advertisements, including ones whose destination
        // is now a sequence starting with the old command.
        let snapshot = candidate.bindings.clone();
        for binding in &mut candidate.bindings {
            if let Some(alias) = &binding.alias {
                let valid = binding
                    .alias_modes
                    .unwrap_or(binding.modes)
                    .iter()
                    .all(|mode| {
                        snapshot.iter().any(|other| {
                            other.sequence == *alias
                                && other.visible_in(*mode, binding.scope)
                                && other.target == binding.target
                                && other.is_plain_action()
                                && (other.scope == binding.scope
                                    || other.scope == BindingScope::Global)
                        })
                    });
                if !valid {
                    binding.alias = None;
                    binding.alias_modes = None;
                }
            }
        }
        candidate.namespaces = candidate
            .namespaces
            .into_iter()
            .flat_map(|namespace| {
                namespace
                    .modes
                    .iter()
                    .filter(|&&mode| {
                        snapshot.iter().any(|binding| {
                            binding.visible_in(mode, namespace.scope)
                                && (binding.scope == namespace.scope
                                    || binding.scope == BindingScope::Global)
                                && binding.sequence != namespace.sequence
                                && binding.sequence.starts_with(&namespace.sequence)
                        })
                    })
                    .map(|&mode| {
                        let mut copy = namespace.clone();
                        copy.modes = mode_slice(mode);
                        copy
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let violations = validate::validate(
            &candidate.bindings,
            &candidate.namespaces,
            &candidate.context_actions,
        );
        if violations.is_empty() {
            candidate.rebuild_lookup_index();
            return candidate;
        }
        let mut rejected = false;
        for (index, rule) in rules.iter().enumerate() {
            if !active[index] {
                continue;
            }
            if let Some(violation) = violations.iter().find(|violation| {
                violation.mode == rule.mode && violation.sequences.contains(&rule.sequence)
            }) {
                active[index] = false;
                rejected = true;
                errors.push(format!(
                    "bind.{} {:?} rejected: {}",
                    mode_name(rule.mode),
                    rule.source,
                    violation.message
                ));
            }
        }
        if !rejected {
            errors.push("bind rejected: unable to validate effective keymap".into());
            return base;
        }
    }
}

/// Generated from the same mode admission and continuation policy as compilation.
pub fn reference() -> String {
    use std::fmt::Write;
    let mut out = String::from(
        "# Bindable actions\n\nUse these names in `keys.bind`. Modes are independent. Actions marked `last`\nmust end a sequence; `continue` actions may precede another action. Counts\non multi-action bindings are rejected. Plugin bindings use `plugins[].bindings`.\n\n| Action | Modes | Sequence position | Description |\n| --- | --- | --- | --- |\n",
    );
    for &command in EditorCommand::ALL {
        let modes = editor_modes(command);
        if modes.is_empty() {
            continue;
        }
        let metadata = command.metadata();
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} |",
            metadata.name,
            modes
                .into_iter()
                .map(mode_name)
                .collect::<Vec<_>>()
                .join(", "),
            if can_continue(BindingTarget::Editor(command)) {
                "continue"
            } else {
                "last"
            },
            metadata.description
        );
    }
    out.push_str("\n## Colon commands\n\nThese accept the same arguments as the command palette, supplied separately\nas `{ command: name, argument: text }`. They are available in Normal and\nSelect modes and must end a sequence. Required arguments must be supplied.\n\n| Command | Usage | Description |\n| --- | --- | --- |\n");
    for spec in command::COMMANDS {
        if matches!(spec.id, CommandId::Colon(_)) {
            let _ = writeln!(
                out,
                "| `{}` | `{}` | {} |",
                spec.name, spec.usage, spec.description
            );
        }
    }
    out
}
