// SPDX-License-Identifier: MPL-2.0

//! Bounded semantic action bindings layered over the resolved keymap.

use super::{
    Binding, BindingAvailability, BindingNamespace, BindingScope, BindingTarget, Key, KeySequence,
    Keymap, validate,
};
use crate::command::{self, CommandId, CommandInvocation, EditorCommand, Mode};
use crate::input::KeyCode;
use serde_yaml::{Mapping, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

/// Bound the work and storage of configured assignments across all mode files.
pub const MAX_BINDINGS: usize = 65_536;
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
    // Scanned once: configuration compiles ask this for every admitted rule
    // and, without the defaults, for every built-in binding in every mode.
    static MODES: LazyLock<HashMap<EditorCommand, Vec<Mode>>> = LazyLock::new(|| {
        EditorCommand::ALL
            .iter()
            .map(|&command| (command, scan_editor_modes(command)))
            .collect()
    });
    MODES.get(&command).cloned().unwrap_or_default()
}

fn scan_editor_modes(command: EditorCommand) -> Vec<Mode> {
    if command::INTERNAL_EDITOR_COMMANDS.contains(&command)
        || command::GRAMMAR_ONLY_EDITOR_COMMANDS.contains(&command)
        || command == EditorCommand::ShellPipe
    {
        return Vec::new();
    }
    let map = super::default_keymap();
    let target = BindingTarget::Editor(command);
    BIND_MODES
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
                        | EditorCommand::GotoNextParagraph
                        | EditorCommand::GotoPreviousParagraph
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

/// Keeps each namespace only in the modes where some binding still continues
/// it, so a prefix whose keys were all removed stops being advertised.
fn prune_namespaces(
    namespaces: Vec<BindingNamespace>,
    bindings: &[Binding],
) -> Vec<BindingNamespace> {
    namespaces
        .into_iter()
        .flat_map(|namespace| {
            namespace
                .modes
                .iter()
                .filter(|&&mode| {
                    bindings.iter().any(|binding| {
                        binding.visible_in(mode, namespace.scope)
                            && (binding.scope == BindingScope::Global
                                || binding.scope == namespace.scope)
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
        .collect()
}

/// Removes stale alias advertisements, including ones whose destination is
/// now a sequence starting with the old command, and returns the bindings as
/// they were before.
fn drop_stale_aliases(bindings: &mut [Binding]) -> Vec<Binding> {
    let snapshot = bindings.to_vec();
    for binding in bindings {
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
                            && (other.scope == binding.scope || other.scope == BindingScope::Global)
                    })
                });
            if !valid {
                binding.alias = None;
                binding.alias_modes = None;
            }
        }
    }
    snapshot
}

/// Insert and Replace keys a terminal already answers to with its own
/// bindings. Anything else replaced in those modes keeps its built-in meaning
/// inside terminals, through [`terminal_copies`], since a binding `bind`
/// assigns is not visible there.
fn terminal_keys(bindings: &[Binding]) -> HashSet<(Mode, KeySequence)> {
    bindings
        .iter()
        .filter(|binding| binding.scope == BindingScope::Terminal)
        .flat_map(|binding| {
            binding
                .modes
                .iter()
                .map(move |&mode| (mode, binding.sequence.clone()))
        })
        .collect()
}

/// Terminal-scoped copies of a global `binding` removed in `modes`, for the
/// Insert and Replace ones a terminal has no binding of its own for.
fn terminal_copies(
    binding: &Binding,
    modes: &[Mode],
    terminal_keys: &HashSet<(Mode, KeySequence)>,
) -> Vec<Binding> {
    modes
        .iter()
        .filter(|&&mode| {
            matches!(mode, Mode::Insert | Mode::Replace)
                && !terminal_keys.contains(&(mode, binding.sequence.clone()))
        })
        .map(|&mode| {
            let mut terminal = binding.clone();
            terminal.scope = BindingScope::Terminal;
            terminal.modes = mode_slice(mode);
            terminal.alias = None;
            terminal.alias_modes = None;
            terminal
        })
        .collect()
}

/// Whether `keys.bind` can recreate `binding` in `mode`: an implemented
/// built-in action, on a key the grammar does not keep for itself.
fn is_expressible(binding: &Binding, mode: Mode) -> bool {
    binding.availability == BindingAvailability::Implemented
        && binding.sequence.len() <= 8
        && !is_reserved(mode, &binding.sequence)
        && match binding.target {
            BindingTarget::Plugin(_) => false,
            // What `parse_action` decides for an editor action's bare name,
            // without searching the command inventory for it.
            BindingTarget::Editor(command) => editor_modes(command).contains(&mode),
            BindingTarget::Colon(_) => {
                parse_action(&Value::String(binding.target.name().into()), mode).is_ok()
            }
        }
}

/// Whether `keys.defaults: false` leaves `binding` out in `mode`. That is
/// every built-in global binding `bind` could put on the same key, and the
/// unimplemented ones that only answer with a message, since a keyboard of
/// someone's own should not advertise Runyte's plans. Kept are the keys
/// nothing else could supply, and the two a person chose elsewhere or could
/// not get back without: the terminal escape `Ctrl-\`, which `Esc` cannot be
/// inside a terminal, and the fast pane keys `editor.fast_pane_keys` turned on.
fn is_default(binding: &Binding, mode: Mode, fast_pane_keys: bool) -> bool {
    let single = |test: fn(Key) -> bool| matches!(binding.sequence.as_slice(), [key] if test(*key));
    let terminal_escape = binding.target == BindingTarget::Editor(EditorCommand::EnterNormalMode)
        && single(super::is_terminal_normal_key);
    binding.scope == BindingScope::Global
        && binding.is_active_in(mode)
        && !(fast_pane_keys && single(super::is_fast_pane_key))
        && !terminal_escape
        && if binding.availability == BindingAvailability::Implemented {
            is_expressible(binding, mode)
        } else {
            !matches!(binding.target, BindingTarget::Plugin(_))
                && !is_reserved(mode, &binding.sequence)
        }
}

/// `keymap` without its defaults, for `keys.defaults: false`; see
/// [`is_default`]. Inside terminals the removed Insert and Replace keys that
/// reach the keymap there keep their meaning, as they do when `bind` replaces
/// them with the defaults underneath, since nothing `bind` assigns is visible
/// there. Of the namespaces only the leader and window prefixes are
/// kept whole, since they are what `Leader` and `Window` mean in `bind`, which
/// may continue them; [`apply`] prunes them if it does not. Any other built-in
/// namespace described keys that are gone, and keeping one would also claim
/// its key from a leader configured onto it.
pub(super) fn without_defaults(mut keymap: Keymap) -> Keymap {
    let fast_pane_keys = keymap.fast_pane_keys();
    let terminal_keys = terminal_keys(&keymap.bindings);
    keymap.bindings = keymap
        .bindings
        .into_iter()
        .flat_map(|binding| {
            let dropped = BIND_MODES
                .into_iter()
                .filter(|&mode| is_default(&binding, mode, fast_pane_keys))
                .collect::<Vec<_>>();
            if dropped.is_empty() {
                return vec![binding];
            }
            let mut retained = binding
                .modes
                .iter()
                .filter(|mode| !dropped.contains(mode))
                .map(|&mode| {
                    let mut copy = binding.clone();
                    copy.modes = mode_slice(mode);
                    copy
                })
                .collect::<Vec<_>>();
            if reaches_terminal_input(&binding) {
                retained.extend(terminal_copies(&binding, &dropped, &terminal_keys));
            }
            retained
        })
        .collect();
    drop_stale_aliases(&mut keymap.bindings);
    let prefixes = [
        KeySequence::from(Key::char(' ')),
        KeySequence::from(Key::ctrl('w')),
    ];
    keymap.namespaces = std::mem::take(&mut keymap.namespaces)
        .into_iter()
        .flat_map(|namespace| {
            if prefixes.contains(&namespace.sequence) {
                vec![namespace]
            } else {
                prune_namespaces(vec![namespace], &keymap.bindings)
            }
        })
        .collect();
    keymap.rebuild_lookup_index();
    keymap
}

/// Without the defaults, a built-in namespace can lose every global binding
/// under it while a special view's own keys still continue it, as the
/// terminal's `Ctrl-w v` does once the global pane moves are gone. Such a
/// namespace from `namespaces` is named again for those views alone, so help
/// and key hints still describe the prefix their keys begin.
pub(super) fn name_scoped_prefixes(mut keymap: Keymap, namespaces: &[BindingNamespace]) -> Keymap {
    let mut named = keymap.namespaces.clone();
    for namespace in namespaces {
        for &mode in namespace.modes {
            if named
                .iter()
                .any(|other| other.sequence == namespace.sequence && other.is_active_in(mode))
            {
                continue;
            }
            let mut scopes = Vec::new();
            for binding in &keymap.bindings {
                if binding.scope != BindingScope::Global
                    && binding.is_active_in(mode)
                    && binding.sequence != namespace.sequence
                    && binding.sequence.starts_with(&namespace.sequence)
                    && !scopes.contains(&binding.scope)
                {
                    scopes.push(binding.scope);
                }
            }
            for scope in scopes {
                let mut copy = namespace.clone();
                copy.scope = scope;
                copy.modes = mode_slice(mode);
                named.push(copy);
            }
        }
    }
    if named.len() > keymap.namespaces.len()
        && validate::validate(&keymap.bindings, &named, &keymap.context_actions).is_empty()
    {
        keymap.namespaces = named;
        keymap.rebuild_lookup_index();
    }
    keymap
}

/// Whether a terminal in Insert mode hands `binding`'s first key to the
/// keymap rather than to its program, as `App::handle_key_stroke` decides:
/// the window prefix, the copy key and persistent-session navigation. The
/// terminal escape and the fast pane keys are never removed at all. A copy of
/// anything else could never run, yet would still claim its key from a
/// window prefix configured onto it. Sequences are the default spellings.
fn reaches_terminal_input(binding: &Binding) -> bool {
    use EditorCommand as C;
    binding.sequence.as_slice().first() == Some(&Key::ctrl('w'))
        || matches!(
            binding.target,
            BindingTarget::Editor(
                C::ClipboardYank
                    | C::NextRunningSession
                    | C::PreviousRunningSession
                    | C::PreviousSession
            )
        )
}

/// What a keymap without the defaults lacks to be left again: the command
/// palette in Normal mode, from which every command and the configuration can
/// be reached, and a way back to Normal mode from the other three. A sequence
/// of actions counts when one of them is the command.
pub(super) fn missing_way_out(keymap: &Keymap) -> Option<String> {
    let reaches = |mode, command| {
        let target = BindingTarget::Editor(command);
        keymap.bindings_for_mode(mode).any(|binding| {
            binding.availability == BindingAvailability::Implemented
                && (binding.target == target
                    || binding
                        .actions
                        .iter()
                        .any(|action| action.target() == target))
        })
    };
    let mut missing = Vec::new();
    if !reaches(Mode::Normal, EditorCommand::OpenCommandPalette) {
        missing.push("open-command-palette in normal".to_owned());
    }
    for mode in [Mode::Select, Mode::Insert, Mode::Replace] {
        if !reaches(mode, EditorCommand::EnterNormalMode) {
            missing.push(format!("enter-normal-mode in {}", mode_name(mode)));
        }
    }
    (!missing.is_empty()).then(|| missing.join(", "))
}

/// Whether `sequence` uses a key the grammar keeps for itself in `mode`.
fn is_reserved(mode: Mode, sequence: &KeySequence) -> bool {
    sequence.as_slice().iter().enumerate().any(|(index, key)| {
        (index > 0 && matches!(key.code, KeyCode::Escape | KeyCode::Backspace))
            || (index == 0
                && matches!(mode, Mode::Normal | Mode::Select)
                && key.modifiers.is_empty()
                && matches!(key.code, KeyCode::Char('1'..='9') | KeyCode::Tab))
    })
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
    if is_reserved(mode, &sequence) {
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
        errors.push(format!(
            "bind rejected: at most 4 modes and {MAX_BINDINGS} assignments are allowed"
        ));
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
    let terminal_keys = terminal_keys(&base.bindings);
    let mut active = vec![true; rules.len()];
    loop {
        let mut candidate = base.clone();
        // One pass over the bindings for all rules: a written configuration
        // carries the whole built-in keymap, and a pass per rule would cost
        // their product at every startup.
        let mut replaced = HashMap::<&KeySequence, Vec<Mode>>::new();
        for (rule, _) in rules.iter().zip(&active).filter(|(_, enabled)| **enabled) {
            replaced.entry(&rule.sequence).or_default().push(rule.mode);
        }
        candidate.bindings = candidate
            .bindings
            .into_iter()
            .flat_map(|binding| {
                let modes = match replaced.get(&binding.sequence) {
                    Some(modes) if binding.scope == BindingScope::Global => modes
                        .iter()
                        .copied()
                        .filter(|&mode| binding.is_active_in(mode))
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                };
                if modes.is_empty() {
                    return vec![binding];
                }
                let mut retained = binding
                    .modes
                    .iter()
                    .filter(|mode| !modes.contains(mode))
                    .map(|&mode| {
                        let mut copy = binding.clone();
                        copy.modes = mode_slice(mode);
                        copy
                    })
                    .collect::<Vec<_>>();
                retained.extend(terminal_copies(&binding, &modes, &terminal_keys));
                retained
            })
            .collect();
        for (rule, _) in rules.iter().zip(&active).filter(|(_, enabled)| **enabled) {
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
        let snapshot = drop_stale_aliases(&mut candidate.bindings);
        candidate.namespaces = prune_namespaces(candidate.namespaces, &snapshot);
        let violations = validate::validate(
            &candidate.bindings,
            &candidate.namespaces,
            &candidate.context_actions,
        );
        if violations.is_empty() {
            candidate.rebuild_lookup_index();
            return candidate;
        }
        // Keep the first diagnostic for each rule without rescanning every
        // violation for every assignment. Large sets of conflicting prefixes
        // must be cheap to reject as well as valid maps being cheap to admit.
        let mut violation_by_rule = HashMap::new();
        for violation in &violations {
            for sequence in &violation.sequences {
                violation_by_rule
                    .entry((violation.mode, sequence))
                    .or_insert(violation);
            }
        }
        let mut rejected = false;
        for (index, rule) in rules.iter().enumerate() {
            if !active[index] {
                continue;
            }
            if let Some(violation) = violation_by_rule.get(&(rule.mode, &rule.sequence)) {
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

/// The modes `keys.bind` accepts, in the order they are written.
const BIND_MODES: [Mode; 4] = [Mode::Normal, Mode::Select, Mode::Insert, Mode::Replace];

/// Built-in assignments for commented export examples, after prefix remapping.
/// Explicit assignments are omitted, including null removals. Compile only the
/// prefix/remap section: the examples never need to materialize custom rules.
/// Fast pane keys and sequences `bind` cannot express stay implicit.
pub(crate) fn default_bindings(section: Option<&Value>) -> BTreeMap<&'static str, Mapping> {
    let mut section = section
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    let mut bind = BTreeMap::new();
    // `keys.defaults: false` leaves no built-in binding underneath to suggest.
    // Export refuses a section with key errors, so it was not rejected here.
    if section.get("defaults") == Some(&Value::Bool(false)) {
        for mode in BIND_MODES {
            bind.insert(mode_name(mode), Mapping::new());
        }
        return bind;
    }
    let own_bind = section.remove("bind");
    let keymap =
        super::configured::compile(&Value::Mapping(section), &super::keymap_for(false)).keymap;

    for mode in BIND_MODES {
        let own = own_bind
            .as_ref()
            .and_then(|bind| bind.get(mode_name(mode)))
            .and_then(Value::as_mapping);
        let claimed = own
            .into_iter()
            .flatten()
            .filter_map(|(source, _)| sequence_for(source.as_str()?, &keymap))
            .collect::<HashSet<_>>();
        let mut entries = BTreeMap::new();
        for binding in keymap.bindings_for_mode(mode) {
            let sequence = &binding.sequence;
            if binding.actions.is_empty()
                && !claimed.contains(sequence)
                && !matches!(sequence.as_slice(), [key] if super::is_fast_pane_key(*key))
                && is_expressible(binding, mode)
            {
                entries
                    .entry(sequence.to_string())
                    .or_insert_with(|| Value::String(binding.target.name().into()));
            }
        }
        let written = entries
            .into_iter()
            .map(|(sequence, action)| (Value::String(sequence), action))
            .collect::<Mapping>();
        bind.insert(mode_name(mode), written);
    }
    bind
}

/// The sequence a `bind` key names, as [`parse_rule`] reads it.
fn sequence_for(source: &str, keymap: &Keymap) -> Option<KeySequence> {
    source
        .split_whitespace()
        .map(|token| match token {
            "Leader" => Ok(keymap.leader()),
            "Window" => Ok(keymap.window_prefix()),
            _ => Key::parse(token),
        })
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .map(KeySequence::new)
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

#[cfg(test)]
mod tests {
    use super::{BIND_MODES, reference};
    use crate::keymap::{Binding, BindingScope, KeySequence, Keymap, Lookup, configured};
    use std::collections::BTreeSet;

    /// What `binding` does when its key is pressed, ignoring whether that
    /// came from the built-in table or from `bind`.
    fn effect(binding: &Binding) -> String {
        if binding.is_plain_action() {
            format!("{:?}", binding.target)
        } else {
            format!("{:?}", binding.actions)
        }
    }

    fn effects(bindings: &[&Binding]) -> BTreeSet<String> {
        bindings
            .iter()
            .map(|binding| format!("{} {}", binding.sequence, effect(binding)))
            .collect()
    }

    /// Every lookup a keystroke could make, in every mode and scope.
    fn dispatch(keymap: &Keymap) -> Vec<String> {
        let mut prefixes = std::collections::HashSet::new();
        for binding in keymap.bindings() {
            for length in 1..=binding.sequence.len() {
                prefixes.insert(KeySequence::new(
                    binding.sequence.as_slice()[..length].iter().copied(),
                ));
            }
        }
        let mut out = Vec::new();
        for &scope in BindingScope::ALL {
            for mode in BIND_MODES {
                for sequence in &prefixes {
                    let found = match keymap.lookup_in(mode, scope, sequence) {
                        Lookup::NoMatch => continue,
                        Lookup::Exact(binding) => effect(binding),
                        Lookup::Prefix(continuations) => format!("{:?}", effects(&continuations)),
                        Lookup::ExactAndPrefix {
                            exact,
                            continuations,
                        } => format!("{} {:?}", effect(exact), effects(&continuations)),
                    };
                    out.push(format!("{scope:?} {mode:?} {sequence}: {found}"));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn bind_admits_up_to_its_assignment_limit() {
        // Five letters after F12 per entry: distinct, equally long, and clear
        // of the count keys.
        let section = |count: usize| {
            let mut source = String::from("bind:\n  normal:\n");
            for n in 0..count {
                let letters = format!("{n:05}")
                    .bytes()
                    .map(|digit| char::from(b'a' + digit - b'0').to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                source.push_str(&format!("    F12 {letters}: save\n"));
            }
            serde_yaml::from_str::<serde_yaml::Value>(&source).unwrap()
        };
        let built_in = crate::keymap::keymap_for(false);
        let errors = configured::compile(&section(super::MAX_BINDINGS), &built_in).errors;
        assert!(errors.is_empty(), "{errors:?}");
        let errors = configured::compile(&section(super::MAX_BINDINGS + 1), &built_in).errors;
        assert!(
            errors
                .iter()
                .any(|error| error.contains("65536 assignments")),
            "{errors:?}"
        );
    }

    #[test]
    fn the_written_keys_section_dispatches_every_key_as_before() {
        let sections = [
            None,
            Some(
                "leader: Ctrl-x\nwindow: Ctrl-a\nrebind:\n  Space g: Leader G\nbind:\n  normal:\n    J: null\n    F6: [select-all, yank]\n    Leader q: { command: pipe, argument: sort }\n  insert:\n    Ctrl-s: save\n",
            ),
            Some(
                "defaults: false\nleader: Ctrl-x\nbind:\n  normal:\n    ':': open-command-palette\n    Leader h: move-down\n  select:\n    Esc: enter-normal-mode\n  insert:\n    Esc: enter-normal-mode\n  replace:\n    Esc: enter-normal-mode\n",
            ),
        ];
        for source in sections {
            let section = source.map(|source| serde_yaml::from_str(source).unwrap());
            let config = crate::config::Config {
                keys: section.clone(),
                ..Default::default()
            };
            let files = config
                .to_files(std::path::Path::new("config.yaml"))
                .unwrap();
            let text = files
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            // Without the defaults there is nothing built in to suggest: the
            // written section is the configured one.
            let standalone = source.is_some_and(|source| source.starts_with("defaults: false"));
            assert_eq!(
                text.contains("#     w: move-word-forward\n"),
                !standalone,
                "{text}"
            );
            assert_eq!(text.contains("# Built-in defaults"), !standalone, "{text}");
            let mut written = crate::config::Config::default();
            for (path, text) in &files {
                if path.starts_with(crate::config::KEYBINDS_DIRECTORY) {
                    written.merge_keys_source(text).unwrap();
                }
            }
            if standalone {
                assert_eq!(written.keys, section, "{text}");
            }
            let empty = serde_yaml::Value::Mapping(Default::default());
            for fast in [false, true] {
                let built_in = crate::keymap::keymap_for(fast);
                let before = configured::compile(section.as_ref().unwrap_or(&empty), &built_in);
                let after = configured::compile(written.keys.as_ref().unwrap_or(&empty), &built_in);
                assert_eq!(after.errors, before.errors, "{text}");
                assert_eq!(dispatch(&after.keymap), dispatch(&before.keymap), "{text}");
            }
        }
    }

    /// Collects the backquoted first cell of every table row in `text`.
    fn first_cells(text: &str) -> BTreeSet<String> {
        text.lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .filter_map(|rest| rest.split_once('`'))
            .map(|(name, _)| name.to_owned())
            .collect()
    }

    /// The user guide groups the `:help key-actions` inventory by topic, so it
    /// cannot be generated verbatim. It must still name exactly the actions and
    /// colon commands the registry offers, no more and no fewer.
    #[test]
    fn user_guide_action_reference_matches_the_registry() {
        let guide = crate::manual::USER_GUIDE;
        let section = guide
            .split_once("### Bindable action reference")
            .expect("user guide has a bindable action reference")
            .1
            .split_once("\n## ")
            .expect("the action reference ends at the next top-level section")
            .0;
        let documented = first_cells(section);
        let registered = first_cells(&reference());
        let missing = registered.difference(&documented).collect::<Vec<_>>();
        let stale = documented.difference(&registered).collect::<Vec<_>>();
        assert!(
            missing.is_empty() && stale.is_empty(),
            "docs/user-guide/ action reference is missing {missing:?} and lists unknown {stale:?}"
        );
    }
}
