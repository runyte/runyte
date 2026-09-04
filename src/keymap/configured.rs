// SPDX-License-Identifier: MPL-2.0

//! Non-fatal compilation of the `keys` section in `config.yaml`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde_yaml::{Mapping, Value};

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
    admit_rebinds(&mut parsed, built_in, &mut errors);

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
        let candidate = resolve(&parsed, &active, leader_active, window_active, built_in);
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
        let violations = validate::validate(
            &candidate.bindings,
            &candidate.namespaces,
            built_in.all_context_actions(),
        );
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
                    &candidate, violation, built_in,
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

    let keymap = Keymap::with_namespaces(resolution.bindings, resolution.namespaces)
        .expect("the configured-keymap validator rejected every duplicate")
        .with_context_actions(built_in.all_context_actions().to_vec())
        .with_spelling_metadata(resolution.leader, resolution.window, resolution.spelling);
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
        if !matches!(key, "leader" | "window" | "rebind") {
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
        keymap::{BindingTarget, Lookup, default_keymap},
    };

    use super::*;

    #[test]
    fn longest_rebind_wins_and_named_prefixes_expand() {
        let value: Value = serde_yaml::from_str(
            "leader: Ctrl-x\nrebind:\n  Space g: Leader G\n  Space g l: Leader G c\n",
        )
        .unwrap();
        let compiled = compile(&value, default_keymap());
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        assert!(matches!(
            compiled.keymap.lookup(Mode::Normal, &KeySequence::parse("Ctrl-x G c").unwrap()),
            Lookup::Exact(binding) if binding.target == BindingTarget::Colon(ColonCommand::GitLog)
        ));
    }

    #[test]
    fn malformed_sections_are_non_fatal() {
        let compiled = compile(&Value::Bool(true), default_keymap());
        assert_eq!(compiled.keymap.leader(), Key::char(' '));
        assert_eq!(compiled.errors.len(), 1);
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
