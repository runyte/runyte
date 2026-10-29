// SPDX-License-Identifier: MPL-2.0

//! Non-fatal compilation of the `keys` section in `config.yaml`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde_yaml::{Mapping, Value};

use crate::command::Mode;
use crate::input::{KeyCode, KeyStroke, Modifiers};

use super::{Binding, BindingNamespace, BindingScope, Key, KeySequence, Keymap, validate};

const MAX_REBINDS: usize = 256;
const MAX_SEQUENCE_KEYS: usize = 8;
const MAX_REPORTED: usize = 32;

#[derive(Clone, Debug)]
enum TargetKey {
    Key(KeyStroke),
    Leader,
    Window,
}

#[derive(Clone, Debug)]
struct Rebind {
    source: String,
    target: String,
    from: KeySequence,
    to: Vec<TargetKey>,
}

#[derive(Clone, Debug, Default)]
struct Parsed {
    /// `keys.defaults: false`: the built-in global bindings `bind` could
    /// recreate are left out, so `bind` is the whole of them.
    standalone: bool,
    leader: Option<(String, KeyStroke)>,
    window: Option<(String, KeyStroke)>,
    rebinds: Vec<Rebind>,
}

#[derive(Clone, Debug, Default)]
struct Provenance {
    rebinds: HashSet<String>,
    leader: bool,
    window: bool,
}

impl Provenance {
    fn merge(&mut self, other: &Self) {
        self.rebinds.extend(other.rebinds.iter().cloned());
        self.leader |= other.leader;
        self.window |= other.window;
    }
}

#[derive(Clone, Debug)]
struct Resolution {
    bindings: Vec<Binding>,
    namespaces: Vec<BindingNamespace>,
    spelling: HashMap<KeySequence, KeySequence>,
    provenance: HashMap<KeySequence, Provenance>,
    default_provenance: HashMap<KeySequence, Provenance>,
    leader: KeyStroke,
    window: KeyStroke,
}

#[derive(Clone, Debug)]
pub struct CompiledKeymap {
    pub keymap: Arc<Keymap>,
    pub errors: Vec<String>,
}

/// Compiles a section for one built-in keymap variant. Invalid entries are
/// rejected independently and the returned keymap is always usable.
pub fn compile(section: &Value, built_in: &Keymap) -> CompiledKeymap {
    let mut errors = Vec::new();
    let Some(mut parsed) = parse_section(section, &mut errors) else {
        return CompiledKeymap {
            keymap: Arc::new(built_in.clone()),
            errors,
        };
    };
    // Reported only if the defaults do stay out: a rejected `defaults: false`
    // compiles again with these rebinds applied.
    let ignored_rebinds = if parsed.standalone {
        parsed
            .rebinds
            .drain(..)
            .map(|rule| {
                format!(
                    "rebind {:?} ignored: keys.defaults is false, so there is no built-in binding to move",
                    rule.source
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    admit_rebinds(&mut parsed, built_in, &mut errors);
    // Without the defaults, the prefixes are resolved against what is left of
    // the keymap, so a leader may take a key only a dropped default used.
    let stripped;
    let base = if parsed.standalone {
        stripped = super::actions::without_defaults(built_in.clone());
        &stripped
    } else {
        built_in
    };

    let mut active = parsed
        .rebinds
        .iter()
        .map(|rule| rule.source.clone())
        .collect::<HashSet<_>>();
    let mut leader_active = parsed.leader.is_some();
    let mut window_active = parsed.window.is_some();
    let mut passes = 0;
    let resolution = loop {
        passes += 1;
        let candidate = resolve(&parsed, &active, leader_active, window_active, base);
        if let Some(sequence) = candidate
            .bindings
            .iter()
            .flat_map(|binding| {
                [
                    &binding.sequence,
                    binding.alias.as_ref().unwrap_or(&binding.sequence),
                ]
            })
            .chain(
                candidate
                    .namespaces
                    .iter()
                    .map(|namespace| &namespace.sequence),
            )
            .find(|sequence| sequence.len() > MAX_SEQUENCE_KEYS)
        {
            errors.clear();
            errors.push(format!(
                "keys section rejected: resolved sequence {sequence} exceeds the {MAX_SEQUENCE_KEYS}-key limit"
            ));
            return CompiledKeymap {
                keymap: Arc::new(built_in.clone()),
                errors,
            };
        }
        let mut violations = validate::validate(
            &candidate.bindings,
            &candidate.namespaces,
            built_in.all_context_actions(),
        );
        // A namespace whose defaults are gone may still be continued by
        // `bind`, which prunes it afterwards; until then it is not a conflict.
        if parsed.standalone {
            violations.retain(|violation| {
                violation.kind != validate::ViolationKind::NamespaceUnreachable
            });
        }
        // Nothing need be under the leader or window yet, so a key a special
        // view uses on its own would conflict only once `bind` continued the
        // prefix, and `apply` would then reject each entry, not the prefix.
        if parsed.standalone && violations.is_empty() {
            let executable = |prefix: KeyStroke, modes: &[Mode]| {
                candidate.bindings.iter().find(|binding| {
                    binding.sequence.as_slice() == [prefix]
                        && modes.iter().any(|&mode| binding.is_active_in(mode))
                })
            };
            let modal = [Mode::Normal, Mode::Select];
            let all = [Mode::Normal, Mode::Select, Mode::Insert, Mode::Replace];
            if window_active && let Some(binding) = executable(candidate.window, &all) {
                window_active = false;
                errors.push(format!(
                    "window prefix rejected; the default Ctrl-w prefix was restored: {} is executable in {:?}",
                    binding.sequence, binding.scope
                ));
                continue;
            }
            if leader_active && let Some(binding) = executable(candidate.leader, &modal) {
                leader_active = false;
                errors.push(format!(
                    "leader prefix rejected; the default Space prefix was restored: {} is executable in {:?}",
                    binding.sequence, binding.scope
                ));
                continue;
            }
        }
        if violations.is_empty() {
            break candidate;
        }

        let mut provenance = Provenance::default();
        for violation in &violations {
            for sequence in &violation.sequences {
                if let Some(source) = candidate.provenance.get(sequence) {
                    provenance.merge(source);
                }
            }
            if violation.kind == validate::ViolationKind::NamespaceUnreachable {
                provenance.merge(&unreachable_namespace_provenance(
                    &candidate, violation, base,
                ));
            }
        }
        if let Some(rule) = provenance.rebinds.iter().min().cloned() {
            active.remove(&rule);
            let details = summarize_violations(&violations);
            let target = parsed
                .rebinds
                .iter()
                .find(|candidate| candidate.source == rule)
                .map(|candidate| candidate.target.as_str())
                .unwrap_or_default();
            errors.push(format!("rebind {rule:?} -> {target:?} rejected: {details}"));
        } else if provenance.window && window_active {
            window_active = false;
            errors.push(format!(
                "window prefix rejected; the default Ctrl-w prefix was restored: {}",
                violations[0].message
            ));
        } else if provenance.leader && leader_active {
            leader_active = false;
            errors.push(format!(
                "leader prefix rejected; the default Space prefix was restored: {}",
                violations[0].message
            ));
        } else {
            // This is a built-in invariant failure, not a reader error. Keep
            // the editor usable and make the problem visible.
            errors.push(format!(
                "configured keymap could not be validated: {}",
                violations[0].message
            ));
            return CompiledKeymap {
                keymap: Arc::new(built_in.clone()),
                errors,
            };
        }
        if passes > parsed.rebinds.len() + 2 {
            errors.push("configured keymap exceeded its bounded resolution passes".to_owned());
            return CompiledKeymap {
                keymap: Arc::new(built_in.clone()),
                errors,
            };
        }
    };

    // Help and tutorials name default sequences whose bindings may be gone,
    // so without the defaults their spellings still come from the whole
    // built-in keymap, under the same prefixes.
    let complete = parsed
        .standalone
        .then(|| resolve(&parsed, &active, leader_active, window_active, built_in));
    let spelling = match &complete {
        Some(complete) => complete.spelling.clone(),
        None => resolution.spelling,
    };
    let keymap = Keymap::with_namespaces(resolution.bindings, resolution.namespaces)
        .expect("the configured-keymap validator rejected every duplicate")
        .with_context_actions(built_in.all_context_actions().to_vec())
        .with_fast_pane_keys_enabled(built_in.fast_pane_keys())
        .with_spelling_metadata(resolution.leader, resolution.window, spelling);
    let keymap = super::actions::apply(section.get("bind"), keymap, &mut errors);
    if parsed.standalone
        && let Some(missing) = super::actions::missing_way_out(&keymap)
    {
        // A keymap that cannot be left would lock someone out of the editor
        // they need to repair it, so the built-in bindings stay underneath.
        let mut overlay = section.as_mapping().cloned().unwrap_or_default();
        overlay.remove("defaults");
        let mut compiled = compile(&Value::Mapping(overlay), built_in);
        // What went wrong without the defaults often explains what is
        // missing, as when the entry meant to reach the palette was rejected.
        let mut reported = vec![format!(
            "defaults: false rejected: bind has no {missing}; the built-in bindings were kept"
        )];
        reported.extend(
            errors
                .into_iter()
                .filter(|error| !compiled.errors.contains(error)),
        );
        reported.append(&mut compiled.errors);
        compiled.errors = reported;
        return compiled;
    }
    errors.extend(ignored_rebinds);
    let keymap = match complete {
        Some(complete) => super::actions::name_scoped_prefixes(keymap, &complete.namespaces),
        None => keymap,
    };
    CompiledKeymap {
        keymap: Arc::new(keymap),
        errors,
    }
}

fn parse_section(section: &Value, errors: &mut Vec<String>) -> Option<Parsed> {
    let Value::Mapping(mapping) = section else {
        errors.push("keys section rejected: expected a mapping".to_owned());
        return None;
    };
    for key in mapping.keys() {
        let Some(key) = key.as_str() else {
            errors.push("keys section rejected: member names must be strings".to_owned());
            return None;
        };
        if !matches!(key, "leader" | "window" | "rebind" | "bind" | "defaults") {
            errors.push(format!("keys section rejected: unknown member {key:?}"));
            return None;
        }
    }

    let string = |name: &str| -> Result<Option<String>, String> {
        match mapping.get(Value::String(name.to_owned())) {
            None => Ok(None),
            Some(Value::String(value)) => Ok(Some(value.clone())),
            Some(_) => Err(format!("keys section rejected: {name} must be a string")),
        }
    };
    let leader_source = match string("leader") {
        Ok(value) => value,
        Err(error) => {
            errors.push(error);
            return None;
        }
    };
    let window_source = match string("window") {
        Ok(value) => value,
        Err(error) => {
            errors.push(error);
            return None;
        }
    };

    let rebind_mapping = match mapping.get(Value::String("rebind".to_owned())) {
        None => Mapping::new(),
        Some(Value::Mapping(mapping)) => mapping.clone(),
        Some(_) => {
            errors.push(
                "keys section rejected: rebind must be a string-to-string mapping".to_owned(),
            );
            return None;
        }
    };
    if rebind_mapping.len() > MAX_REBINDS {
        errors.push(format!(
            "keys section rejected: rebind has {} entries; the limit is {MAX_REBINDS}",
            rebind_mapping.len()
        ));
        return None;
    }
    let mut ordered = BTreeMap::new();
    for (from, to) in rebind_mapping {
        let (Value::String(from), Value::String(to)) = (from, to) else {
            errors.push(
                "keys section rejected: rebind must be a string-to-string mapping".to_owned(),
            );
            return None;
        };
        ordered.insert(from, to);
    }
    if let Some((from, to)) = ordered.iter().find(|(from, to)| {
        from.split_whitespace().count() > MAX_SEQUENCE_KEYS
            || to.split_whitespace().count() > MAX_SEQUENCE_KEYS
    }) {
        errors.clear();
        errors.push(format!(
            "keys section rejected: rebind {from:?} -> {to:?} exceeds the {MAX_SEQUENCE_KEYS}-key sequence limit"
        ));
        return None;
    }

    let mut parsed = Parsed::default();
    match mapping.get(Value::String("defaults".to_owned())) {
        None | Some(Value::Bool(true)) => {}
        Some(Value::Bool(false)) => parsed.standalone = true,
        Some(_) => {
            errors.push("keys section rejected: defaults must be true or false".to_owned());
            return None;
        }
    }
    if let Some(source) = leader_source {
        match parse_named_prefix("leader", &source, false) {
            Ok(key) => parsed.leader = Some((source, key)),
            Err(error) => errors.push(error),
        }
    }
    if let Some(source) = window_source {
        match parse_named_prefix("window", &source, true) {
            Ok(key) => parsed.window = Some((source, key)),
            Err(error) => errors.push(error),
        }
    }
    for (from_source, to_source) in ordered {
        match parse_rebind(&from_source, &to_source) {
            Ok(rule) => parsed.rebinds.push(rule),
            Err(error) => errors.push(format!("rebind {from_source:?} rejected: {error}")),
        }
    }
    let mut canonical_sources = HashMap::new();
    parsed.rebinds.retain(|rule| {
        if let Some(previous) = canonical_sources.insert(rule.from.clone(), rule.source.clone()) {
            errors.push(format!(
                "rebind {:?} rejected: it duplicates the default sequence already written as {:?}",
                rule.source, previous
            ));
            false
        } else {
            true
        }
    });
    Some(parsed)
}

fn parse_named_prefix(name: &str, source: &str, window: bool) -> Result<KeyStroke, String> {
    let sequence =
        KeySequence::parse(source).map_err(|error| format!("{name} rejected: {error}"))?;
    if sequence.len() != 1 {
        return Err(format!("{name} rejected: it must be exactly one key"));
    }
    let key = sequence.as_slice()[0];
    if window
        && matches!(
            key,
            KeyStroke {
                code: KeyCode::Char(_),
                modifiers: Modifiers::NONE
            }
        )
    {
        return Err(
            "window rejected: an unmodified character would consume ordinary text input".to_owned(),
        );
    }
    Ok(key)
}

fn parse_rebind(from_source: &str, to_source: &str) -> Result<Rebind, String> {
    if from_source
        .split_whitespace()
        .any(|part| matches!(part, "Leader" | "Window"))
    {
        return Err(
            "the left side uses default spellings; write Space or Ctrl-w instead".to_owned(),
        );
    }
    let from = KeySequence::parse(from_source)?;
    let mut to = Vec::new();
    for token in to_source.split_whitespace() {
        to.push(match token {
            "Leader" => TargetKey::Leader,
            "Window" => TargetKey::Window,
            _ => TargetKey::Key(KeyStroke::parse(token)?),
        });
    }
    if to.is_empty() {
        return Err("the right side cannot be empty".to_owned());
    }
    Ok(Rebind {
        source: from_source.to_owned(),
        target: to_source.to_owned(),
        from,
        to,
    })
}

fn admit_rebinds(parsed: &mut Parsed, built_in: &Keymap, errors: &mut Vec<String>) {
    let aliases = built_in
        .bindings()
        .iter()
        .filter_map(|binding| binding.alias.as_ref())
        .cloned()
        .collect::<HashSet<_>>();
    parsed.rebinds.retain(|rule| {
        let reject = if rule.from == KeySequence::from(Key::char(' ')) {
            Some("use keys.leader to move the bare Space prefix".to_owned())
        } else if rule.from == KeySequence::from(Key::ctrl('w')) {
            Some("use keys.window to move the bare Ctrl-w prefix".to_owned())
        } else {
            let is_alias = aliases.contains(&rule.from);
            let in_scope = matches!(rule.from.as_slice().first(), Some(key) if *key == Key::char(' ') || *key == Key::ctrl('w')) || is_alias;
            let matches = built_in.bindings().iter().any(|binding| {
                binding.sequence == rule.from || binding.sequence.starts_with(&rule.from)
            }) || is_alias;
            if !matches {
                Some("the left side matches no default binding, alias, or prefix".to_owned())
            } else if !in_scope {
                Some("the left side is outside the Space and Ctrl-w namespaces".to_owned())
            } else {
                None
            }
        };
        if let Some(reason) = reject {
            errors.push(format!("rebind {:?} rejected: {reason}", rule.source));
            false
        } else {
            true
        }
    });
}

fn resolve(
    parsed: &Parsed,
    active: &HashSet<String>,
    leader_active: bool,
    window_active: bool,
    built_in: &Keymap,
) -> Resolution {
    let leader = parsed
        .leader
        .as_ref()
        .filter(|_| leader_active)
        .map_or(Key::char(' '), |(_, key)| *key);
    let window = parsed
        .window
        .as_ref()
        .filter(|_| window_active)
        .map_or(Key::ctrl('w'), |(_, key)| *key);
    let rules = parsed
        .rebinds
        .iter()
        .filter(|rule| active.contains(&rule.source))
        .collect::<Vec<_>>();

    let rewrite = |default: &KeySequence| -> (KeySequence, Provenance) {
        let matched = rules
            .iter()
            .filter(|rule| default.starts_with(&rule.from))
            .max_by_key(|rule| rule.from.len());
        let mut keys = Vec::new();
        let mut provenance = Provenance::default();
        if let Some(rule) = matched {
            provenance.rebinds.insert(rule.source.clone());
            for token in &rule.to {
                match token {
                    TargetKey::Key(key) => keys.push(*key),
                    TargetKey::Leader => {
                        keys.push(leader);
                        provenance.leader = leader_active;
                    }
                    TargetKey::Window => {
                        keys.push(window);
                        provenance.window = window_active;
                    }
                }
            }
            keys.extend_from_slice(&default.as_slice()[rule.from.len()..]);
        } else {
            keys.extend_from_slice(default.as_slice());
            if let Some(first) = keys.first_mut() {
                if *first == Key::char(' ') {
                    *first = leader;
                    provenance.leader = leader_active;
                } else if *first == Key::ctrl('w') {
                    *first = window;
                    provenance.window = window_active;
                }
            }
        }
        (KeySequence::new(keys), provenance)
    };

    let mut spelling = HashMap::new();
    let mut provenance_by_sequence: HashMap<KeySequence, Provenance> = HashMap::new();
    let mut provenance_by_default: HashMap<KeySequence, Provenance> = HashMap::new();
    let mut bindings = built_in.bindings().to_vec();
    for binding in &mut bindings {
        let default = binding.sequence.clone();
        let (rewritten, source) = rewrite(&default);
        spelling.insert(default.clone(), rewritten.clone());
        provenance_by_default
            .entry(default.clone())
            .or_default()
            .merge(&source);
        provenance_by_sequence
            .entry(rewritten.clone())
            .or_default()
            .merge(&source);
        binding.sequence = rewritten;
        if let Some(alias) = &mut binding.alias {
            let default = alias.clone();
            let (rewritten, source) = rewrite(&default);
            spelling.insert(default.clone(), rewritten.clone());
            provenance_by_default
                .entry(default.clone())
                .or_default()
                .merge(&source);
            provenance_by_sequence
                .entry(rewritten.clone())
                .or_default()
                .merge(&source);
            *alias = rewritten;
        }
    }
    let mut namespaces = built_in.namespaces().to_vec();
    for namespace in &mut namespaces {
        let default = namespace.sequence.clone();
        let (rewritten, source) = rewrite(&default);
        spelling.insert(default.clone(), rewritten.clone());
        provenance_by_default
            .entry(default.clone())
            .or_default()
            .merge(&source);
        provenance_by_sequence
            .entry(rewritten.clone())
            .or_default()
            .merge(&source);
        namespace.sequence = rewritten;
    }
    Resolution {
        bindings,
        namespaces,
        spelling,
        provenance: provenance_by_sequence,
        default_provenance: provenance_by_default,
        leader,
        window,
    }
}

fn unreachable_namespace_provenance(
    resolution: &Resolution,
    violation: &validate::Violation,
    built_in: &Keymap,
) -> Provenance {
    let Some(effective_namespace) = violation.sequences.first() else {
        return Provenance::default();
    };
    let Some((default_namespace, _)) = built_in
        .namespaces()
        .iter()
        .zip(&resolution.namespaces)
        .find(|(default, effective)| {
            effective.scope == violation.scope
                && effective.modes.contains(&violation.mode)
                && effective.sequence == *effective_namespace
                && default.scope == effective.scope
        })
    else {
        return Provenance::default();
    };

    let mut provenance = Provenance::default();
    for binding in built_in.bindings().iter().filter(|binding| {
        binding.is_active_in(violation.mode)
            && (binding.scope == BindingScope::Global || binding.scope == violation.scope)
            && binding.sequence != default_namespace.sequence
            && binding.sequence.starts_with(&default_namespace.sequence)
    }) {
        if let Some(source) = resolution.default_provenance.get(&binding.sequence) {
            provenance.merge(source);
        }
    }
    provenance
}

pub fn format_errors(errors: &[String]) -> String {
    if errors.len() <= MAX_REPORTED {
        return errors.join("\n");
    }
    let mut rendered = errors[..MAX_REPORTED].join("\n");
    rendered.push_str(&format!("\n… and {} more", errors.len() - MAX_REPORTED));
    rendered
}

pub fn rejected_entry_count<'a>(errors: impl IntoIterator<Item = &'a str>) -> usize {
    errors
        .into_iter()
        .map(|error| {
            if let Some((entry, _)) = error.split_once(" rejected:") {
                entry
            } else if error.starts_with("leader prefix rejected;") {
                "leader"
            } else if error.starts_with("window prefix rejected;") {
                "window"
            } else if error.starts_with("keys section rejected:")
                || error.starts_with("configured keymap")
            {
                "keys section"
            } else {
                error
            }
        })
        .collect::<HashSet<_>>()
        .len()
}

fn summarize_violations(violations: &[validate::Violation]) -> String {
    const DETAIL_LIMIT: usize = 3;
    let mut rendered = violations
        .iter()
        .take(DETAIL_LIMIT)
        .map(|violation| violation.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    if violations.len() > DETAIL_LIMIT {
        rendered.push_str(&format!(
            "; and {} more conflicts",
            violations.len() - DETAIL_LIMIT
        ));
    }
    rendered
}

#[cfg(test)]
mod tests {
    use crate::{
        command::{ColonCommand, Mode},
        keymap::{BindingScope, BindingTarget, Lookup, default_keymap},
    };

    use super::*;

    /// The smallest standalone keymap the editor accepts: a way to the
    /// command palette and a way back to Normal mode.
    const WAY_OUT: &str = "defaults: false\nbind:\n  normal:\n    ':': open-command-palette\n  select:\n    Esc: enter-normal-mode\n  insert:\n    Esc: enter-normal-mode\n  replace:\n    Esc: enter-normal-mode\n";

    /// Whether `keys.defaults: false` took effect without complaint.
    fn standalone(compiled: &CompiledKeymap) -> bool {
        compiled.errors.is_empty() && exact(compiled, Mode::Normal, "w").is_none()
    }

    fn exact(compiled: &CompiledKeymap, mode: Mode, keys: &str) -> Option<BindingTarget> {
        match compiled
            .keymap
            .lookup(mode, &KeySequence::parse(keys).unwrap())
        {
            Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => {
                Some(binding.target)
            }
            Lookup::NoMatch | Lookup::Prefix(_) => None,
        }
    }

    #[test]
    fn without_defaults_only_the_configured_global_keys_remain() {
        let mut value: Value = serde_yaml::from_str(WAY_OUT).unwrap();
        value["bind"]["normal"]["h"] = Value::String("move-down".into());
        let compiled = compile(&value, default_keymap());
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);

        assert_eq!(
            exact(&compiled, Mode::Normal, "h"),
            Some(BindingTarget::Editor(
                crate::command::EditorCommand::MoveDown
            ))
        );
        assert_eq!(exact(&compiled, Mode::Normal, "w"), None);
        assert_eq!(exact(&compiled, Mode::Insert, "Ctrl-w"), None);
        assert!(matches!(
            compiled
                .keymap
                .lookup(Mode::Normal, &KeySequence::parse("Space").unwrap()),
            Lookup::NoMatch
        ));
        // What `bind` could never supply is still there: the reserved
        // context-action key and every scoped binding of a special buffer.
        assert_eq!(
            exact(&compiled, Mode::Normal, "Tab"),
            Some(BindingTarget::Editor(
                crate::command::EditorCommand::CodeAction
            ))
        );
        for binding in default_keymap()
            .bindings()
            .iter()
            .filter(|binding| binding.scope != BindingScope::Global)
        {
            assert!(
                compiled.keymap.bindings().contains(binding),
                "{:?} {} lost",
                binding.scope,
                binding.sequence
            );
        }
    }

    #[test]
    fn without_defaults_a_keymap_that_cannot_be_left_keeps_the_defaults() {
        let value: Value =
            serde_yaml::from_str("defaults: false\nbind:\n  normal:\n    x: move-left\n").unwrap();
        let compiled = compile(&value, default_keymap());

        // `Ctrl-\\` still leaves the other modes, so only the palette is missing.
        assert_eq!(
            compiled.errors,
            [
                "defaults: false rejected: bind has no open-command-palette in normal; the built-in bindings were kept"
            ]
        );
        assert_eq!(
            exact(&compiled, Mode::Normal, "x"),
            Some(BindingTarget::Editor(
                crate::command::EditorCommand::MoveLeft
            ))
        );
        assert!(exact(&compiled, Mode::Normal, "w").is_some());

        // Taking the terminal escape away as well leaves Insert with no exit.
        let mut value: Value = serde_yaml::from_str(WAY_OUT).unwrap();
        value["bind"]["insert"] =
            serde_yaml::from_str("{ 'Ctrl-\\': null, Ctrl-4: null }").unwrap();
        let compiled = compile(&value, default_keymap());
        assert_eq!(
            compiled.errors,
            [
                "defaults: false rejected: bind has no enter-normal-mode in insert; the built-in bindings were kept"
            ]
        );
    }

    #[test]
    fn without_defaults_prefix_conflicts_are_reported_as_they_are() {
        // A leader and window on one key is the window's fault, as with the
        // defaults, rather than a reason to drop all of `bind`.
        let mut value: Value = serde_yaml::from_str(WAY_OUT).unwrap();
        value["leader"] = Value::String("Ctrl-a".into());
        value["window"] = Value::String("Ctrl-a".into());
        value["bind"]["normal"]["Leader h"] = Value::String("move-down".into());
        let compiled = compile(&value, default_keymap());
        assert_eq!(compiled.errors.len(), 1, "{:?}", compiled.errors);
        assert!(
            compiled.errors[0].starts_with("window prefix rejected"),
            "{:?}",
            compiled.errors
        );
        assert_eq!(exact(&compiled, Mode::Normal, "w"), None);
        assert_eq!(
            exact(&compiled, Mode::Normal, "Ctrl-a h"),
            Some(BindingTarget::Editor(
                crate::command::EditorCommand::MoveDown
            ))
        );

        // So is a leader a special view uses on its own, as Media uses `-`.
        let mut value: Value = serde_yaml::from_str(WAY_OUT).unwrap();
        value["leader"] = Value::String("-".into());
        value["bind"]["normal"]["Leader f"] = Value::String("move-down".into());
        let compiled = compile(&value, default_keymap());
        assert_eq!(compiled.errors.len(), 1, "{:?}", compiled.errors);
        assert!(
            compiled.errors[0].starts_with("leader prefix rejected"),
            "{:?}",
            compiled.errors
        );
        assert_eq!(
            exact(&compiled, Mode::Normal, "Space f"),
            Some(BindingTarget::Editor(
                crate::command::EditorCommand::MoveDown
            ))
        );

        // When the rejected entry was the way out, the rejection says why.
        let mut value: Value = serde_yaml::from_str(WAY_OUT).unwrap();
        value["leader"] = Value::String("x".into());
        value["bind"]["normal"] = serde_yaml::from_str("{ x: open-command-palette }").unwrap();
        let compiled = compile(&value, default_keymap());
        assert!(
            compiled.errors[0].starts_with(
                "defaults: false rejected: bind has no open-command-palette in normal"
            ),
            "{:?}",
            compiled.errors
        );
        assert!(
            compiled.errors[1..]
                .iter()
                .any(|error| error.starts_with("bind.normal \"x\" rejected:")),
            "{:?}",
            compiled.errors
        );
        assert!(exact(&compiled, Mode::Normal, "w").is_some());
    }

    #[test]
    fn without_defaults_the_user_guide_example_is_accepted() {
        // A Windows checkout may convert the guide to CRLF.
        let guide = crate::manual::USER_GUIDE.replace("\r\n", "\n");
        let section = &guide[guide.find("### Without the built-in keys").unwrap()..];
        let start = section.find("```yaml\n").unwrap() + "```yaml\n".len();
        let example = &section[start..start + section[start..].find("```").unwrap()];
        let config: Value = serde_yaml::from_str(example).unwrap();
        let compiled = compile(&config["keys"], default_keymap());
        assert!(standalone(&compiled), "{:?}", compiled.errors);
        for (keys, command) in [
            ("Enter", crate::command::EditorCommand::InsertNewline),
            (
                "Backspace",
                crate::command::EditorCommand::DeleteCharBackward,
            ),
        ] {
            assert_eq!(
                exact(&compiled, Mode::Insert, keys),
                Some(BindingTarget::Editor(command))
            );
        }
    }

    #[test]
    fn without_defaults_the_terminal_escape_stays() {
        let compiled = compile(&serde_yaml::from_str(WAY_OUT).unwrap(), default_keymap());
        assert!(standalone(&compiled), "{:?}", compiled.errors);
        // Escape belongs to the program inside a terminal; `Ctrl-\\` in both
        // its spellings is the only way out of its input.
        for mode in [Mode::Normal, Mode::Select, Mode::Insert, Mode::Replace] {
            for keys in ["Ctrl-\\", "Ctrl-4"] {
                assert_eq!(
                    exact(&compiled, mode, keys),
                    Some(BindingTarget::Editor(
                        crate::command::EditorCommand::EnterNormalMode
                    )),
                    "{mode:?} {keys}"
                );
            }
        }
        assert_eq!(
            compiled.keymap.lookup_in(
                Mode::Insert,
                BindingScope::Terminal,
                &KeySequence::parse("Ctrl-\\").unwrap()
            ),
            compiled
                .keymap
                .lookup(Mode::Insert, &KeySequence::parse("Ctrl-\\").unwrap())
        );
    }

    #[test]
    fn without_defaults_terminals_keep_their_insert_keys() {
        let mut value: Value = serde_yaml::from_str(WAY_OUT).unwrap();
        value["window"] = Value::String("Ctrl-a".into());
        let compiled = compile(&value, default_keymap());
        assert!(standalone(&compiled), "{:?}", compiled.errors);
        let keymap = &compiled.keymap;
        // Nothing `bind` assigns reaches a terminal's input, so what the
        // terminal answered to through the global Insert keys stays there.
        assert!(keymap.terminal_clipboard_key(Key::ctrl('C')));
        let terminal = |keys: &str| match keymap.lookup_in(
            Mode::Insert,
            BindingScope::Terminal,
            &KeySequence::parse(keys).unwrap(),
        ) {
            Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => {
                Some(binding.target)
            }
            Lookup::NoMatch | Lookup::Prefix(_) => None,
        };
        assert_eq!(
            terminal("Ctrl-a h"),
            Some(BindingTarget::Editor(
                crate::command::EditorCommand::FocusWindowLeft
            ))
        );
        assert_eq!(exact(&compiled, Mode::Insert, "Ctrl-a h"), None);
        assert_eq!(exact(&compiled, Mode::Insert, "Backspace"), None);
        // Keys a terminal sends to its program get no copy, so a window
        // prefix may still take one only a dropped Insert default used.
        assert_eq!(terminal("Ctrl-s"), None);
        let mut value: Value = serde_yaml::from_str(WAY_OUT).unwrap();
        value["window"] = Value::String("Ctrl-s".into());
        value["bind"]["insert"]["Window l"] = Value::String("focus-window-right".into());
        let compiled = compile(&value, default_keymap());
        assert!(standalone(&compiled), "{:?}", compiled.errors);
        assert_eq!(
            exact(&compiled, Mode::Insert, "Ctrl-s l"),
            Some(BindingTarget::Editor(
                crate::command::EditorCommand::FocusWindowRight
            ))
        );
    }

    #[test]
    fn without_defaults_prefixes_take_keys_only_dropped_defaults_used() {
        let mut value: Value = serde_yaml::from_str(WAY_OUT).unwrap();
        value["leader"] = Value::String("x".into());
        value["window"] = Value::String("Ctrl-a".into());
        value["bind"]["normal"]["Leader h"] = Value::String("move-down".into());
        value["bind"]["insert"]["Window l"] = Value::String("focus-window-right".into());
        let compiled = compile(&value, default_keymap());
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        assert!(standalone(&compiled), "{:?}", compiled.errors);

        assert_eq!(
            exact(&compiled, Mode::Normal, "x h"),
            Some(BindingTarget::Editor(
                crate::command::EditorCommand::MoveDown
            ))
        );
        assert_eq!(exact(&compiled, Mode::Normal, "Space h"), None);
        assert_eq!(
            exact(&compiled, Mode::Insert, "Ctrl-a l"),
            Some(BindingTarget::Editor(
                crate::command::EditorCommand::FocusWindowRight
            ))
        );
        // The prefixes `bind` continues keep their names for key hints; the
        // ones nothing global continues any more are gone from the keyboard.
        let named = |mode, keys| {
            let sequence = KeySequence::parse(keys).unwrap();
            compiled.keymap.namespaces().iter().any(|namespace| {
                namespace.scope == BindingScope::Global
                    && namespace.modes.contains(&mode)
                    && namespace.sequence == sequence
            })
        };
        assert!(named(Mode::Normal, "x"));
        assert!(named(Mode::Insert, "Ctrl-a"));
        assert!(!named(Mode::Normal, "x g"));
        assert!(!named(Mode::Normal, "g"));

        // A built-in namespace key is free too: its name goes with its keys.
        for leader in ["g", "z"] {
            let mut value: Value = serde_yaml::from_str(WAY_OUT).unwrap();
            value["leader"] = Value::String(leader.into());
            value["bind"]["normal"]["Leader h"] = Value::String("move-down".into());
            let compiled = compile(&value, default_keymap());
            assert!(standalone(&compiled), "{leader}: {:?}", compiled.errors);
            assert_eq!(
                exact(&compiled, Mode::Normal, &format!("{leader} h")),
                Some(BindingTarget::Editor(
                    crate::command::EditorCommand::MoveDown
                ))
            );
            let sequence = KeySequence::parse(leader).unwrap();
            let names = compiled
                .keymap
                .namespaces()
                .iter()
                .filter(|namespace| {
                    namespace.modes.contains(&Mode::Normal) && namespace.sequence == sequence
                })
                .map(|namespace| namespace.description)
                .collect::<Vec<_>>();
            assert_eq!(names, ["Application commands"], "{leader}");
        }
    }

    #[test]
    fn without_defaults_help_still_spells_every_built_in_prefix() {
        let mut value: Value = serde_yaml::from_str(WAY_OUT).unwrap();
        value["window"] = Value::String("Ctrl-a".into());
        let compiled = compile(&value, default_keymap());
        assert!(standalone(&compiled), "{:?}", compiled.errors);
        // Every default prefix keeps its configured spelling, whether or not
        // anything is left under it; a marker for one that is gone resolves
        // to a placeholder rather than failing.
        for namespace in default_keymap().namespaces() {
            let mut expected = namespace.sequence.as_slice().to_vec();
            if expected[0] == Key::ctrl('w') {
                expected[0] = Key::ctrl('a');
            }
            assert_eq!(
                compiled.keymap.spelling_for_default(&namespace.sequence),
                Some(&KeySequence::new(expected)),
                "{}",
                namespace.sequence
            );
            let marker = format!("{{prefix:{}}}", namespace.sequence);
            assert!(
                crate::key_spelling::resolve(&marker, &compiled.keymap).is_ok(),
                "{marker}"
            );
        }
        // The manual and its tutorials expect every marker to resolve.
        crate::manual::render_document_for(&compiled.keymap);

        // A terminal's own `Ctrl-w` keys outlive the global pane moves, so
        // its prefix is still named there, under the configured spelling.
        let spelled = crate::key_spelling::resolve("{prefix:Ctrl-w}", &compiled.keymap)
            .unwrap()
            .text;
        assert_eq!(spelled, "Ctrl-a");
        assert!(compiled.keymap.namespaces().iter().any(|namespace| {
            namespace.scope == BindingScope::Terminal
                && namespace.is_active_in(Mode::Insert)
                && namespace.sequence == KeySequence::parse("Ctrl-a").unwrap()
        }));
    }

    #[test]
    fn without_defaults_a_sequence_of_actions_is_a_way_out() {
        let mut value: Value = serde_yaml::from_str(WAY_OUT).unwrap();
        value["bind"]["insert"] = serde_yaml::from_str(
            "{ Esc: [move-left, enter-normal-mode], 'Ctrl-\\': null, Ctrl-4: null }",
        )
        .unwrap();
        value["bind"]["normal"] =
            serde_yaml::from_str("{ ':': [select-all, open-command-palette] }").unwrap();
        let compiled = compile(&value, default_keymap());
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        assert!(standalone(&compiled), "{:?}", compiled.errors);
    }

    #[test]
    fn without_defaults_fast_pane_keys_stay_while_configured_on() {
        let value: Value = serde_yaml::from_str(WAY_OUT).unwrap();
        let fast = compile(&value, &crate::keymap::keymap_for(true));
        assert!(standalone(&fast), "{:?}", fast.errors);
        for mode in [Mode::Normal, Mode::Insert] {
            assert_eq!(
                exact(&fast, mode, "Ctrl-h"),
                Some(BindingTarget::Editor(
                    crate::command::EditorCommand::FocusWindowLeft
                )),
                "{mode:?}"
            );
        }
        // Off, the keys are ordinary defaults and go with the rest.
        let plain = compile(&value, &crate::keymap::keymap_for(false));
        assert_eq!(exact(&plain, Mode::Insert, "Ctrl-h"), None);
    }

    #[test]
    fn without_defaults_unimplemented_bindings_go_too() {
        let compiled = compile(&serde_yaml::from_str(WAY_OUT).unwrap(), default_keymap());
        assert!(standalone(&compiled), "{:?}", compiled.errors);
        let remaining = compiled
            .keymap
            .bindings()
            .iter()
            .filter(|binding| {
                binding.scope == BindingScope::Global
                    && binding.availability != crate::keymap::BindingAvailability::Implemented
                    && binding.modes.iter().any(|mode| {
                        matches!(
                            mode,
                            Mode::Normal | Mode::Select | Mode::Insert | Mode::Replace
                        )
                    })
            })
            .map(|binding| binding.sequence.to_string())
            .collect::<Vec<_>>();
        assert!(remaining.is_empty(), "{remaining:?}");
    }

    #[test]
    fn without_defaults_rebind_is_ignored_and_defaults_must_be_a_boolean() {
        let value: Value =
            serde_yaml::from_str(&format!("{WAY_OUT}rebind:\n  Space g: Leader G\n")).unwrap();
        let compiled = compile(&value, default_keymap());
        assert_eq!(
            compiled.errors,
            [
                "rebind \"Space g\" ignored: keys.defaults is false, so there is no built-in binding to move"
            ]
        );

        let value: Value = serde_yaml::from_str("defaults: maybe\n").unwrap();
        let compiled = compile(&value, default_keymap());
        assert_eq!(
            compiled.errors,
            ["keys section rejected: defaults must be true or false"]
        );
        assert!(exact(&compiled, Mode::Normal, "w").is_some());
    }

    #[test]
    fn terminal_copy_admission_follows_the_effective_registry() {
        let base = default_keymap();
        assert!(base.terminal_clipboard_key(Key::ctrl('C')));
        assert_eq!(
            base.terminal_clipboard_key(Key::new(
                crate::input::KeyCode::Char('c'),
                crate::input::Modifiers::SUPER
            )),
            cfg!(target_os = "macos")
        );
        assert!(!base.terminal_clipboard_key(Key::ctrl('c')));
        assert!(!base.terminal_clipboard_key(Key::ctrl('v')));
        for sequence in ["F11", "F11 c"] {
            let mut bindings = base.bindings().to_vec();
            for binding in &mut bindings {
                if binding.sequence == KeySequence::from(Key::ctrl('C')) {
                    binding.sequence = KeySequence::parse(sequence).unwrap();
                }
            }
            let changed = crate::keymap::Keymap::new(bindings).unwrap();
            assert!(!changed.terminal_clipboard_key(Key::ctrl('C')));
            assert!(
                changed.terminal_clipboard_key(Key::plain(crate::input::KeyCode::Function(11)))
            );
        }
    }

    #[test]
    fn longest_rebind_wins_and_named_prefixes_expand() {
        let value: Value = serde_yaml::from_str(
            "leader: Ctrl-x\nrebind:\n  Space g: Leader G\n  Space g l: Leader G q\n",
        )
        .unwrap();
        let compiled = compile(&value, default_keymap());
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        assert!(matches!(
            compiled.keymap.lookup(Mode::Normal, &KeySequence::parse("Ctrl-x G q").unwrap()),
            Lookup::Exact(binding) if binding.target == BindingTarget::Colon(ColonCommand::GitLog)
        ));
    }

    #[test]
    fn malformed_sections_are_non_fatal() {
        let compiled = compile(&Value::Bool(true), default_keymap());
        assert_eq!(compiled.keymap.leader(), Key::char(' '));
        assert_eq!(compiled.errors.len(), 1);
    }

    #[test]
    fn grammar_reserved_remaps_restore_reachable_defaults() {
        use crate::{
            command::EditorCommand,
            input::InputEvent,
            input_grammar::{EditorIntent, GrammarContext, InputGrammar, RunyteGrammar},
        };

        for source in [
            "leader: '1'",
            "leader: Tab",
            "rebind: {'Space e': '2 e'}",
            "rebind: {'Space e': 'Tab e'}",
            "rebind: {'Space e': 'F12 Esc e'}",
            "rebind: {'Space e': 'F12 Ctrl-Backspace e'}",
        ] {
            let compiled = configured(source);
            assert!(!compiled.errors.is_empty(), "accepted {source}");
            assert_eq!(compiled.keymap.leader(), Key::char(' '));
            validate::assert_valid(&compiled.keymap);
            for mode in [Mode::Normal, Mode::Select] {
                let mut grammar = RunyteGrammar::default();
                let context = crate::input_grammar::GrammarContext::new(
                    mode,
                    BindingScope::Global,
                    &compiled.keymap,
                );
                grammar
                    .translate(InputEvent::Key(Key::char(' ')), context)
                    .unwrap();
                let output = grammar
                    .translate(InputEvent::Key(Key::char('e')), context)
                    .unwrap();
                assert!(
                    matches!(output.intents.as_slice(), [EditorIntent::Command(command)]
                    if command.id() == crate::command::CommandId::Editor(EditorCommand::OpenExplorer)),
                    "{source} did not restore explorer dispatch"
                );
            }
        }

        let compiled =
            configured("leader: Ctrl-x\nrebind: {'Space e': 'F12 Esc e', 'Space f': F10}");
        assert_eq!(compiled.keymap.leader(), Key::ctrl('x'));
        assert!(matches!(
            compiled.keymap.lookup(Mode::Normal, &KeySequence::parse("F10").unwrap()),
            Lookup::Exact(binding) if binding.target == BindingTarget::Editor(EditorCommand::OpenFilePicker)
        ));

        // Grammar-owned reservations do not apply to a literal Tab in Insert
        // mode, or to the directory tree's direct numbered pane commands.
        let compiled = configured("bind: {insert: {Tab: insert-newline}}");
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        validate::assert_valid(&compiled.keymap);
        let mut grammar = RunyteGrammar::default();
        let output = grammar
            .translate(
                InputEvent::Key(Key::plain(KeyCode::Tab)),
                GrammarContext::new(
                    crate::command::Mode::Insert,
                    BindingScope::Global,
                    &compiled.keymap,
                ),
            )
            .unwrap();
        assert!(
            matches!(output.intents.as_slice(), [EditorIntent::Command(command)]
            if command.id() == crate::command::CommandId::Editor(EditorCommand::InsertNewline))
        );
    }

    #[test]
    fn hint_scroll_keys_cannot_replace_prefix_continuations() {
        for source in [
            "rebind: {'Space e': 'F12 Ctrl-n'}",
            "rebind: {'Ctrl-w h': 'Ctrl-w Ctrl-p'}",
            "bind: {normal: {'F12 Ctrl-n': move-left}}",
            "bind: {insert: {'Ctrl-w Ctrl-p': move-left}}",
        ] {
            let compiled = configured(source);
            assert!(!compiled.errors.is_empty(), "accepted {source}");
            validate::assert_valid(&compiled.keymap);
        }

        // Insert-mode arbitrary prefixes bypass key discovery, so their
        // Ctrl-n/p strokes remain available to the grammar.
        for mode in [Mode::Insert, Mode::Replace] {
            let mode_name = if mode == Mode::Insert {
                "insert"
            } else {
                "replace"
            };
            let compiled = configured(&format!(
                "bind: {{{mode_name}: {{'F12 Ctrl-n': move-left}}}}"
            ));
            assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
            let mut app = crate::app::App::new(crate::config::Config::default(), None).unwrap();
            app.set_keymap(compiled.keymap);
            app.mode = mode;
            app.handle_input(crate::input::InputEvent::Text("ab".into()))
                .unwrap();
            let mut hints = crate::key_hints::KeyHintState::default();
            for key in KeySequence::parse("F12 Ctrl-n").unwrap().as_slice() {
                if let Some(mode) = app.key_hint_mode_for_key(*key) {
                    assert_eq!(
                        hints.observe_in(*key, mode, app.key_binding_scope(), app.keymap()),
                        crate::key_hints::HintEventResult::Forward,
                    );
                } else {
                    hints.clear();
                }
                app.handle_key(*key).unwrap();
            }
            assert_eq!(app.cursor_position().col, 1);
        }
    }

    fn configured(source: &str) -> CompiledKeymap {
        let value: Value = serde_yaml::from_str(source).unwrap();
        compile(&value, default_keymap())
    }

    #[test]
    fn leader_and_literal_space_have_distinct_meanings() {
        let compiled = configured("leader: Ctrl-x\nrebind:\n  Space e: Space\n");
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        assert_eq!(compiled.keymap.leader(), Key::ctrl('x'));
        assert!(matches!(
            compiled.keymap.lookup(Mode::Normal, &KeySequence::parse("Space").unwrap()),
            Lookup::Exact(binding) if binding.target == BindingTarget::Editor(crate::command::EditorCommand::OpenExplorer)
        ));

        let rejected = configured("rebind:\n  Space e: Space\n");
        assert!(
            rejected
                .errors
                .iter()
                .any(|error| error.contains("Space e"))
        );
        assert!(matches!(
            rejected
                .keymap
                .lookup(Mode::Normal, &KeySequence::parse("Space e").unwrap()),
            Lookup::Exact(_)
        ));
    }

    #[test]
    fn advertised_alias_moves_with_its_registry_advertisement() {
        let compiled = configured("rebind:\n  \",\": F12\n");
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        let binding = compiled
            .keymap
            .bindings()
            .iter()
            .find(|binding| binding.sequence == KeySequence::parse("Space s c").unwrap())
            .unwrap();
        assert_eq!(binding.alias, Some(KeySequence::parse("F12").unwrap()));
        assert!(matches!(
            compiled.keymap.lookup(Mode::Normal, &KeySequence::parse("F12").unwrap()),
            Lookup::Exact(found) if found.target == binding.target
        ));
    }

    #[test]
    fn invalid_window_and_conflicting_rules_are_rolled_back() {
        let invalid = configured("window: a\n");
        assert_eq!(invalid.keymap.window_prefix(), Key::ctrl('w'));
        assert!(invalid.errors[0].contains("ordinary text"));

        let collision = configured("leader: q\n");
        assert_eq!(collision.keymap.leader(), Key::char(' '));
        assert!(
            collision
                .errors
                .iter()
                .any(|error| error.contains("leader prefix rejected"))
        );

        let rebind = configured("rebind:\n  Space e: Space f\n");
        assert!(rebind.errors.iter().any(|error| error.contains("Space e")));
        assert!(matches!(
            rebind
                .keymap
                .lookup(Mode::Normal, &KeySequence::parse("Space e").unwrap()),
            Lookup::Exact(_)
        ));

        let rebind_with_leader = configured("leader: Ctrl-a\nrebind:\n  Space e: Leader f\n");
        assert_eq!(rebind_with_leader.keymap.leader(), Key::ctrl('a'));
        assert!(
            rebind_with_leader
                .errors
                .iter()
                .any(|error| error.starts_with("rebind "))
        );

        let window_collision = configured("window: Ctrl-x\n");
        assert_eq!(window_collision.keymap.window_prefix(), Key::ctrl('w'));
        assert!(
            window_collision
                .errors
                .iter()
                .any(|error| error.contains("window prefix rejected"))
        );
    }

    #[test]
    fn structural_and_sequence_bounds_reject_without_panicking() {
        for source in [
            "unknown: true\n",
            "rebind: nope\n",
            "leader: Space e\n",
            "leader: NotAKey\n",
            "rebind:\n  Space e: Shift-g\n",
            "rebind:\n  Leader g: Space G\n",
            "rebind:\n  Space a b c d e f g h i: Space e\n",
        ] {
            let compiled = configured(source);
            assert!(!compiled.errors.is_empty(), "{source}");
            assert_eq!(compiled.keymap.leader(), Key::char(' '));
        }

        let bound = configured("leader: Ctrl-x\nrebind:\n  Space a b c d e f g h i: Space e\n");
        assert_eq!(bound.errors.len(), 1);
        assert!(bound.errors[0].starts_with("keys section rejected:"));
        assert_eq!(bound.keymap.leader(), Key::char(' '));

        let resolved_bound = configured("leader: Ctrl-x\nrebind:\n  Space g: F13 a b c d e f g\n");
        assert_eq!(resolved_bound.errors.len(), 1);
        assert!(resolved_bound.errors[0].contains("resolved sequence"));
        assert_eq!(resolved_bound.keymap.leader(), Key::char(' '));
    }

    #[test]
    fn canonical_duplicate_sources_reject_the_later_source() {
        let compiled = configured("rebind:\n  \"Space  g\": Leader G\n  Space g: Leader H\n");
        assert_eq!(
            compiled
                .errors
                .iter()
                .filter(|error| error.contains("duplicates the default sequence"))
                .count(),
            1
        );
    }

    #[test]
    fn modified_character_window_prefixes_are_accepted() {
        let compiled = configured("window: Ctrl-a\n");
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        assert_eq!(compiled.keymap.window_prefix(), Key::ctrl('a'));
    }

    #[test]
    fn rules_that_empty_a_namespace_are_attributed_and_rolled_back() {
        let namespace = KeySequence::parse("Space o").unwrap();
        let descendants = default_keymap()
            .bindings()
            .iter()
            .filter(|binding| {
                binding.sequence != namespace && binding.sequence.starts_with(&namespace)
            })
            .map(|binding| binding.sequence.to_string())
            .collect::<HashSet<_>>();
        assert!(!descendants.is_empty());
        let mut source = String::from("rebind:\n");
        for (index, descendant) in descendants.iter().enumerate() {
            source.push_str(&format!("  {descendant}: F13 {index}\n"));
        }
        let compiled = configured(&source);
        assert!(
            compiled
                .errors
                .iter()
                .any(|error| error.starts_with("rebind ")),
            "{:?}",
            compiled.errors
        );
        assert!(
            !compiled
                .errors
                .iter()
                .any(|error| { error.starts_with("configured keymap could not be validated") })
        );
        assert!(compiled.keymap.namespaces().iter().any(|candidate| {
            candidate.sequence == namespace
                && compiled.keymap.bindings().iter().any(|binding| {
                    binding.sequence != namespace && binding.sequence.starts_with(&namespace)
                })
        }));
    }

    #[test]
    fn the_rebind_count_bound_rejects_257_entries() {
        let rebind = (0..257)
            .map(|index| {
                (
                    Value::String(format!("Space invalid-{index}")),
                    Value::String("F12".to_owned()),
                )
            })
            .collect::<Mapping>();
        let section = Value::Mapping(Mapping::from_iter([(
            Value::String("rebind".to_owned()),
            Value::Mapping(rebind),
        )]));
        let compiled = compile(&section, default_keymap());
        assert_eq!(compiled.errors.len(), 1);
        assert!(compiled.errors[0].contains("257 entries"));
    }

    #[test]
    fn named_prefix_diagnostics_count_as_one_entry_across_variants() {
        let normal = crate::keymap::keymap_for(false);
        let fast = crate::keymap::keymap_for(true);
        let section: Value = serde_yaml::from_str("window: Ctrl-j\n").unwrap();
        let normal = compile(&section, &normal);
        let fast = compile(&section, &fast);
        assert_eq!(
            rejected_entry_count(normal.errors.iter().chain(&fast.errors).map(String::as_str)),
            1
        );
    }
}
