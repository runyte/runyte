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

/// Bound the work and storage of configured assignments across all mode files.
pub const MAX_BINDINGS: usize = 65_536;
const MAX_ACTIONS: usize = super::program::MAX_STEPS;

#[cfg(test)]
thread_local! {
    static TEST_MAX_BINDINGS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// The assignment limit in force. Unit tests may lower it on their own thread
/// with [`with_max_bindings`], so the boundary is checked without compiling
/// tens of thousands of assignments; release builds always use
/// [`MAX_BINDINGS`].
pub(super) fn max_bindings() -> usize {
    #[cfg(test)]
    if let Some(limit) = TEST_MAX_BINDINGS.get() {
        return limit;
    }
    MAX_BINDINGS
}

/// Runs `f` with the assignment limit lowered to `limit` on this thread only.
///
/// Each test runs on its own thread, so concurrent tests never see another
/// test's limit. Integration tests and benchmarks link the normally compiled
/// library and always see [`MAX_BINDINGS`].
#[cfg(test)]
pub(crate) fn with_max_bindings<T>(limit: usize, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_MAX_BINDINGS.set(self.0);
        }
    }
    let _restore = Restore(TEST_MAX_BINDINGS.replace(Some(limit)));
    f()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    /// The grammar supplies a count or a subsequent character, as for a default key.
    Editor(EditorCommand),
    Invocation(CommandInvocation),
    /// Unparameterized colon or plugin target, resolved when the key executes.
    Target(BindingTarget),
}

impl Action {
    pub fn can_continue(&self) -> bool {
        let parameters = match self {
            Self::Invocation(invocation) => invocation.parameters(),
            _ => &command::InvocationParameters::None,
        };
        self.target()
            .id()
            .descriptor()
            .is_some_and(|d| d.can_continue(parameters))
    }

    pub fn target(&self) -> BindingTarget {
        match self {
            Self::Target(target) => *target,
            Self::Editor(command) => BindingTarget::Editor(*command),
            Self::Invocation(invocation) => match invocation.id() {
                CommandId::Editor(command) => BindingTarget::Editor(command),
                CommandId::Colon(command) => BindingTarget::Colon(command),
                CommandId::Plugin(id) => BindingTarget::Plugin(id),
            },
        }
    }
}

pub fn editor_modes(command: EditorCommand) -> Vec<Mode> {
    command.action_policy().global_binding_modes.to_vec()
}

/// Unparameterized continuation, shared with discovery.
pub fn can_continue(target: BindingTarget) -> bool {
    Action::Target(target).can_continue()
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

pub(super) fn mode_slice(mode: Mode) -> &'static [Mode] {
    match mode {
        Mode::Normal => &[Mode::Normal],
        Mode::Select => &[Mode::Select],
        Mode::Insert => &[Mode::Insert],
        Mode::Replace => &[Mode::Replace],
        Mode::Command => &[Mode::Command],
        Mode::List => &[Mode::List],
    }
}

pub(super) fn parse_action(
    value: &Value,
    mode: Mode,
    bypass_global_admission: bool,
) -> Result<(Action, String), String> {
    if value
        .as_mapping()
        .is_some_and(|m| m.contains_key("command") || m.contains_key("argument"))
    {
        return Err(super::step::SYNTAX_HINT.into());
    }
    let step: super::step::StepDef = serde_yaml::from_value(value.clone())
        .map_err(|error| format!("{error}; {}", super::step::SYNTAX_HINT))?;
    match step {
        super::step::StepDef::Name(name) => {
            resolve_action(&name, None, mode, bypass_global_admission)
        }
        super::step::StepDef::Action(step) => resolve_action(
            &step.action,
            step.arg.as_deref(),
            mode,
            bypass_global_admission,
        ),
        super::step::StepDef::Conditional(_) => {
            Err("expected one action, not a conditional program".into())
        }
    }
}

pub(super) fn resolve_action(
    name: &str,
    argument: Option<&str>,
    mode: Mode,
    bypass_global_admission: bool,
) -> Result<(Action, String), String> {
    super::step::validate_action(name, argument).map_err(str::to_owned)?;
    let editor = EditorCommand::ALL
        .iter()
        .copied()
        .find(|command| command.metadata().name == name);
    let action = if let Some(command) = editor {
        if !bypass_global_admission && !editor_modes(command).contains(&mode) {
            return Err(format!(
                "{name} is not a configurable {} action",
                mode_name(mode)
            ));
        }
        if let Some(argument) = argument.filter(|_| command.takes_character()) {
            let mut characters = argument.chars();
            let first = characters.next().ok_or("argument requires a character")?;
            let second = command
                .takes_second_character()
                .then(|| characters.next())
                .flatten();
            if characters.next().is_some() || (command.takes_second_character() && second.is_none())
            {
                return Err(if command.takes_second_character() {
                    "argument requires exactly two Unicode scalars (old, new)"
                } else {
                    "argument requires exactly one Unicode scalar"
                }
                .into());
            }
            let mut execution = command::CommandExecutionContext::default().with_character(first);
            if let Some(second) = second {
                execution = execution.with_second_character(second);
            }
            Action::Invocation(
                CommandInvocation::editor(command, execution).map_err(|e| e.to_string())?,
            )
        } else if let Some(argument) = argument.filter(|_| command.supplied_argument().is_some()) {
            command
                .supplied_argument()
                .unwrap()
                .validate(argument)
                .map_err(str::to_owned)?;
            Action::Invocation(
                CommandInvocation::from_parts(
                    CommandId::Editor(command),
                    command::InvocationParameters::OptionalText(Some(argument.into())),
                    command::CommandExecutionContext::default(),
                )
                .map_err(|error| error.to_string())?,
            )
        } else if let Some(argument) = argument {
            let spec = command::COMMANDS
                .iter()
                .find(|spec| spec.id == CommandId::Editor(command))
                .ok_or("this editor action does not take an argument")?;
            Action::Invocation(
                command::parse_named_command(spec.name, Some(argument))
                    .map_err(|error| error.to_string())?,
            )
        } else if command == EditorCommand::InsertText {
            return Err("insert-text requires action/arg".into());
        } else {
            Action::Editor(command)
        }
    } else {
        let invocation =
            command::parse_named_command(name, argument).map_err(|error| error.to_string())?;
        if let CommandId::Editor(command) = invocation.id() {
            if !bypass_global_admission && !editor_modes(command).contains(&mode) {
                return Err(format!(
                    "{name} is not a configurable {} action",
                    mode_name(mode)
                ));
            }
        } else if let CommandId::Colon(command) = invocation.id()
            && !bypass_global_admission
            && !command.action_policy().global_binding_modes.contains(&mode)
        {
            return Err("colon-only commands require normal or select mode".into());
        }
        Action::Invocation(invocation)
    };
    let label = match argument {
        Some(argument) => format!(
            "{name} {}",
            argument
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t")
        ),
        None => name.to_owned(),
    };
    Ok((action, label))
}

#[derive(Clone)]
pub(super) struct Rule {
    pub mode: Mode,
    pub sequence: KeySequence,
    pub source: String,
    pub program: super::program::Program,
    pub description: String,
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
            if terminal.binding_mode.is_some() {
                terminal.binding_mode = Some(
                    if mode == Mode::Insert {
                        "terminal-insert"
                    } else {
                        "terminal-replace"
                    }
                    .into(),
                );
            }
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
    binding.is_plain_action() && has_configurable_target(binding, mode)
}

fn has_configurable_target(binding: &Binding, mode: Mode) -> bool {
    binding.availability == BindingAvailability::Implemented
        && binding.sequence.len() <= 8
        && !is_reserved(mode, &binding.sequence)
        && match binding.target {
            BindingTarget::Plugin(_) => false,
            // What `parse_action` decides for an editor action's bare name,
            // without searching the command inventory for it.
            BindingTarget::Editor(command) => editor_modes(command).contains(&mode),
            BindingTarget::Colon(_) => {
                parse_action(&Value::String(binding.target.name().into()), mode, false).is_ok()
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
            has_configurable_target(binding, mode)
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
pub(super) fn name_scoped_prefixes(
    mut keymap: Keymap,
    namespaces: &[BindingNamespace],
    defer_policy: bool,
) -> Keymap {
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
        && validate::with_input_policy(
            &keymap.bindings,
            &named,
            &keymap.context_actions,
            &keymap.overrides,
            (!defer_policy).then_some(keymap.ui()),
        )
        .is_empty()
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
        keymap
            .bindings_for_scope(mode, BindingScope::Global)
            .any(|binding| {
                binding.availability == BindingAvailability::Implemented
                    && (binding.target == target
                        || binding
                            .program
                            .iter()
                            .any(|action| action.target() == target))
            })
    };
    let mut missing = Vec::new();
    if !reaches(Mode::Normal, EditorCommand::OpenCommandPalette) {
        missing.push("open-command-palette in normal".to_owned());
    }
    for mode in [Mode::Select, Mode::Insert, Mode::Replace] {
        if keymap.mode_graph().is_some()
            && keymap.named_mode_for(mode, BindingScope::Global).is_none()
        {
            continue;
        }
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
    parse_rule_in(mode, source, value, base, None)
}

pub(super) fn parse_rule_in(
    mode: Mode,
    source: &str,
    value: &Value,
    base: &Keymap,
    scope: Option<BindingScope>,
) -> Result<Rule, String> {
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
    if let Some(scope) = scope {
        super::scopes::validate_keys(scope, mode, &sequence, value, base)?;
    }
    if scope.is_none()
        && sequence.as_slice().iter().enumerate().any(|(index, key)| {
            (index == 0
                && matches!(mode, Mode::Normal | Mode::Select)
                && key.modifiers.is_empty()
                && (matches!(key.code, KeyCode::Char('1'..='9'))
                    || (key.code == KeyCode::Tab
                        && base
                            .ui()
                            .lookup(super::ui::Owner::ContextLauncher, *key)
                            .is_some())))
                || (!value.is_null()
                    && index > 0
                    && if matches!(mode, Mode::Normal | Mode::Select) {
                        base.ui()
                            .lookup(super::ui::Owner::GrammarPrefix, *key)
                            .is_some()
                    } else {
                        matches!(key.code, KeyCode::Escape | KeyCode::Backspace)
                    })
        })
    {
        return Err(
            "sequence uses a reserved count, context-action, or prefix-cancellation key".into(),
        );
    }
    let authored_description = if value.get("steps").is_some() {
        let map = value.as_mapping().unwrap();
        if !value.get("steps").is_some_and(Value::is_sequence) {
            return Err("steps must be a nonempty action sequence".into());
        }
        if map
            .keys()
            .any(|key| !matches!(key.as_str(), Some("steps" | "description")))
        {
            return Err("binding metadata is reserved for embedded presets".into());
        }
        value
            .get("description")
            .map(|v| {
                let label = v.as_str().ok_or("description must be a string")?;
                if label.len() > 1024 || label.chars().any(char::is_control) {
                    return Err(
                        "description must be at most 1024 bytes without control characters",
                    );
                }
                Ok(label)
            })
            .transpose()?
    } else {
        None
    };
    let value = value.get("steps").unwrap_or(value);
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
    // Legacy Keymap::new callers have no graph on which to run mode-flow
    // analysis. Preserve their starting-mode contract and reject graph syntax.
    if base.mode_graph().is_none() {
        for value in &values {
            if value.get("action").and_then(Value::as_str) == Some("mode-enter")
                || value.get("if").is_some()
                || value.as_str() == Some("mode-enter")
            {
                return Err(
                    "mode-entry and conditional programs require a named mode graph".into(),
                );
            }
            let scoped = scope.filter(|scope| *scope != BindingScope::Global || mode == Mode::List);
            let (action, _) = parse_action(value, mode, scoped.is_some())?;
            if let Some(scope) = scoped
                && super::scopes::admit(scope, mode, action.target().name()).is_none()
            {
                return Err(format!(
                    "{} is unavailable in {} {}",
                    action.target().name(),
                    scope.name(),
                    mode_name(mode)
                ));
            }
        }
    }
    let steps = values
        .into_iter()
        .map(|value| {
            serde_yaml::from_value::<super::step::StepDef>(value.clone())
                .map_err(|error| format!("{error}; {}", super::step::SYNTAX_HINT))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let program = super::step::compile(&steps, 0, false)?;
    if !program.is_empty() {
        if program.iter().next().is_none() {
            return Err("binding program must contain an action".into());
        }
        program.validate_structure().map_err(str::to_owned)?;
        if let Some(graph) = base.mode_graph() {
            super::presets::validate_entry_targets(&program, graph)?;
        }
    }
    fn labels(steps: &[super::step::StepDef], out: &mut Vec<String>) {
        use super::step::StepDef;
        for step in steps {
            match step {
                StepDef::Name(name) => out.push(name.clone()),
                StepDef::Action(step) => out.push(match &step.arg {
                    Some(arg) => format!("{} {arg}", step.action),
                    None => step.action.clone(),
                }),
                StepDef::Conditional(step) => {
                    labels(&step.then, out);
                    labels(&step.otherwise, out);
                }
            }
        }
    }
    let description = authored_description.map(str::to_owned).unwrap_or_else(|| {
        let mut names = Vec::new();
        labels(&steps, &mut names);
        names.join(" → ")
    });
    Ok(Rule {
        mode,
        sequence,
        source: source.into(),
        program,
        description,
    })
}

pub(super) fn apply(
    section: Option<&Value>,
    base: Keymap,
    errors: &mut Vec<String>,
    defer_policy: bool,
) -> Keymap {
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
            > max_bindings()
    {
        errors.push(format!(
            "bind rejected: at most 4 modes and {} assignments are allowed",
            max_bindings()
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
            if let Some(first) = rule.program.iter().next() {
                let mut binding = Binding::implemented(
                    mode_slice(rule.mode),
                    rule.sequence.clone(),
                    first.target(),
                );
                binding.source_paths = vec![vec![
                    "bind".into(),
                    mode_name(rule.mode).into(),
                    rule.source.clone(),
                ]];
                binding.program = rule.program.clone();
                binding.configured_actions = true;
                binding.description = rule.description.clone().into();
                candidate.bindings.push(binding);
            }
        }
        let snapshot = drop_stale_aliases(&mut candidate.bindings);
        candidate.namespaces = prune_namespaces(candidate.namespaces, &snapshot);
        let violations = validate::with_input_policy(
            &candidate.bindings,
            &candidate.namespaces,
            &candidate.context_actions,
            &candidate.overrides,
            (!defer_policy).then_some(candidate.ui()),
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
            if !binding.configured_actions
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
        "# Bindable actions\n\nUse these names in `keys.bind`. Modes are independent. Sequence position says whether\nan action may be followed by another action. Counts go to the first accepting step in every binding. If none accepts a count,\nthe count is dropped silently and the binding runs. Plugin bindings use `plugins[].bindings`.\n",
    );
    for &category in command::CommandCategory::ALL {
        let commands = EditorCommand::ALL
            .iter()
            .copied()
            .filter(|c| c.descriptor().category == category && !editor_modes(*c).is_empty())
            .collect::<Vec<_>>();
        if commands.is_empty() {
            continue;
        }
        let _ = writeln!(
            out,
            "\n## {} actions\n\n| Action | Modes | Sequence position | Description |\n| --- | --- | --- | --- |",
            category.label()
        );
        for command in commands {
            let modes = editor_modes(command);
            if modes.is_empty() {
                continue;
            }
            let descriptor = command.descriptor();
            let metadata = descriptor.metadata;
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} |",
                metadata.name,
                modes
                    .into_iter()
                    .map(mode_name)
                    .collect::<Vec<_>>()
                    .join(", "),
                crate::command::discovery::display::continuation(descriptor),
                metadata.description
            );
        }
    }
    out.push_str("\n## Colon commands\n\nThese accept the same arguments as the command palette, supplied separately\nas `{ action: name, arg: text }`. They are available in Normal and\nSelect modes. Prompt and background effects must end a sequence; supplying prompt input removes only that prompt. Required arguments must be supplied.\n\n| Command | Usage | Description |\n| --- | --- | --- |\n");
    for spec in command::COMMANDS {
        if matches!(spec.id, CommandId::Colon(_)) {
            let _ = writeln!(
                out,
                "| `{}` | `{}` | {} |",
                spec.name, spec.usage, spec.description
            );
        }
    }
    out.push_str(&super::scopes::reference());
    out.push_str(&crate::command::discovery::descriptor_reference());
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
            format!("{:?}", binding.program)
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
        // The boundary is the same at any limit; compiling the real one in a
        // debug build took most of a minute.
        let limit = 64;
        super::with_max_bindings(limit, || {
            let built_in = crate::keymap::keymap_for(false);
            let errors = configured::compile(&section(limit), &built_in).errors;
            assert!(errors.is_empty(), "{errors:?}");
            let errors = configured::compile(&section(limit + 1), &built_in).errors;
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains(&format!("{limit} assignments"))),
                "{errors:?}"
            );
        });
    }

    #[test]
    fn the_written_keys_section_dispatches_every_key_as_before() {
        let sections = [
            None,
            Some(
                "leader: Ctrl-x\nwindow: Ctrl-a\nrebind:\n  Space g: Leader G\nbind:\n  normal:\n    J: null\n    F6: [select-all, yank]\n    Leader q: { action: pipe, arg: sort }\n  insert:\n    Ctrl-s: save\n",
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

    #[test]
    fn internal_mode_programs_are_removed_but_never_exported_as_bare_actions() {
        use super::{BindingScope, Key, KeySequence, Mode, Value, default_bindings};
        let defaults = default_bindings(None);
        for mode in ["normal", "select"] {
            for key in ["%", ";", "~", "r"] {
                assert!(!defaults[mode].contains_key(Value::String(key.into())));
            }
        }
        let source: Value = serde_yaml::from_str("defaults: false\nbind:\n  normal:\n    ':': open-command-palette\n  select:\n    Esc: enter-normal-mode\n  insert:\n    Esc: enter-normal-mode\n  replace:\n    Esc: enter-normal-mode\n").unwrap();
        let compiled = super::super::configured::compile(&source, &super::super::keymap_for(false));
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        for mode in [Mode::Normal, Mode::Select] {
            for key in [
                Key::char('%'),
                Key::char(';'),
                Key::char('~'),
                Key::char('r'),
            ] {
                assert!(
                    !compiled
                        .keymap
                        .bindings_for_mode(mode)
                        .any(|binding| binding.scope == BindingScope::Global
                            && binding.sequence == KeySequence::from(key))
                );
            }
        }
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
