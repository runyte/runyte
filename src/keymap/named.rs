// SPDX-License-Identifier: MPL-2.0
//! The effective named table shared by execution and discovery.
use super::{
    Binding, BindingScope, KeySequence,
    mode_graph::{BufferTypes, Definitions, EditingRole, ModeGraph, ModeId},
};
use crate::command::Mode;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub(super) struct Tables {
    pub graph: ModeGraph,
    pub definitions: Definitions,
    pub buffer_types: BufferTypes,
    /// Exact tombstones block the same inherited key, not descendants.
    pub removed: HashMap<String, HashSet<KeySequence>>,
    pub plugin_modes: HashSet<String>,
}

pub(super) fn buffer_type(scope: BindingScope) -> &'static str {
    match scope {
        BindingScope::Global | BindingScope::Plugin(_) | BindingScope::SessionManager => "text",
        _ => scope.name(),
    }
}

pub(super) fn required_buffer_types() -> Vec<&'static str> {
    BindingScope::ALL
        .iter()
        .copied()
        .filter(|s| *s != BindingScope::SessionManager)
        .map(buffer_type)
        .collect()
}

/// Validate only role resolution here; sequential mode flow belongs to stage 5.
/// Effective tables ensure overrides and tombstones hide ancestor bindings.
pub(super) fn role_target_errors(map: &super::Keymap) -> Vec<String> {
    use crate::command::{CommandId, EditorCommand, InvocationParameters};
    let Some(tables) = &map.overrides.1 else {
        return Vec::new();
    };
    // Programs can be inherited by many modes and buffer types. Inspect their
    // leaves once, then resolve only entry targets in each reachable context.
    let targets: Vec<Vec<&str>> = map
        .bindings
        .iter()
        .map(|binding| {
            binding
                .program
                .iter()
                .filter_map(|action| {
                    let super::actions::Action::Invocation(invocation) = action else {
                        return None;
                    };
                    if invocation.id() != CommandId::Editor(EditorCommand::ModeEnter) {
                        return None;
                    }
                    match invocation.parameters() {
                        InvocationParameters::OptionalText(Some(target))
                        | InvocationParameters::TemporaryMode(target) => Some(target.as_str()),
                        _ => None,
                    }
                })
                .collect()
        })
        .collect();
    let empty_sequence = KeySequence(Vec::new());
    let mut errors = Vec::new();
    for kind in tables.buffer_types.keys() {
        let mut pending = EditingRole::ALL
            .into_iter()
            .filter_map(|role| tables.graph.role_mode(kind, role))
            .collect::<Vec<_>>();
        let mut seen = HashSet::new();
        while let Some(mode) = pending.pop() {
            if !seen.insert(mode) {
                continue;
            }
            // The empty prefix contains every effective binding, already resolved
            // for execution (including inheritance, overrides and tombstones).
            let effective = map
                .lookup_index
                .get(&super::LookupContext::Named(mode))
                .and_then(|sequences| sequences.get(&empty_sequence));
            for &index in effective.into_iter().flat_map(|entry| &entry.continuations) {
                let binding = &map.bindings[index];
                for &target in &targets[index] {
                    match tables.graph.entry_mode(kind, target) {
                        Some(destination) => pending.push(destination),
                        None => errors.push(format!(
                            "buffer type {kind:?}, mode {:?}, key {:?}: mode-enter role {target:?} is unavailable",
                            tables.graph.get(mode).expect("same graph").name(), binding.sequence.to_string()
                        )),
                    }
                }
            }
        }
    }
    errors
}

impl Tables {
    pub fn mode_for(&self, mode: Mode, scope: BindingScope) -> Option<ModeId> {
        let role = match mode {
            Mode::Normal => EditingRole::Normal,
            Mode::Select => EditingRole::Select,
            Mode::Insert => EditingRole::Insert,
            Mode::Replace => EditingRole::Replace,
            Mode::Command => return None,
            Mode::List => {
                if scope.is_merge_review() {
                    return None;
                }
                return self.graph.id(if scope == BindingScope::SessionManager {
                    "session-manager"
                } else {
                    "list"
                });
            }
        };
        if let BindingScope::Plugin(plugin) = scope
            && let Some(id) = self.graph.id(&format!("plugin-{plugin}-{}", role.name()))
        {
            return Some(id);
        }
        self.graph.role_mode(buffer_type(scope), role)
    }

    /// Empty children share their ancestor's lookup table, but retain their own
    /// identity and input policy. Keep this cache local to one binding revision.
    pub fn all_indices(&self, bindings: &[Binding]) -> Vec<(ModeId, Vec<usize>)> {
        let owners: HashSet<_> = bindings
            .iter()
            .filter_map(|binding| binding.binding_mode.as_deref())
            .collect();
        let mut resolved = HashMap::new();
        self.graph
            .modes()
            .map(|(mode, _)| {
                let owner = self
                    .graph
                    .chain(mode)
                    .find(|&id| {
                        let entry = self.graph.get(id).expect("same graph");
                        owners.contains(entry.name())
                            || self
                                .removed
                                .get(entry.name())
                                .is_some_and(|keys| !keys.is_empty())
                            || entry.parent().is_none()
                    })
                    .expect("chain includes a root");
                let indices = resolved
                    .entry(owner)
                    .or_insert_with(|| self.indices(bindings, owner))
                    .clone();
                (mode, indices)
            })
            .collect()
    }

    pub fn indices(&self, bindings: &[Binding], mode: ModeId) -> Vec<usize> {
        let mut seen = HashSet::new();
        let mut leaves = HashSet::new();
        let mut result = Vec::new();
        for id in self.graph.chain(mode) {
            let name = self.graph.get(id).expect("same graph").name();
            if let Some(removed) = self.removed.get(name) {
                seen.extend(removed.iter());
            }
            let mut own = bindings
                .iter()
                .enumerate()
                .filter(|(_, binding)| binding.binding_mode.as_deref() == Some(name))
                .collect::<Vec<_>>();
            own.sort_by_key(|(_, binding)| binding.role);
            let mut added = Vec::new();
            for (index, binding) in own {
                if seen.insert(&binding.sequence)
                    && !(1..=binding.sequence.len())
                        .any(|len| leaves.contains(&binding.sequence.as_slice()[..len]))
                {
                    result.push(index);
                    added.push(binding.sequence.as_slice());
                }
            }
            leaves.extend(added);
        }
        result
    }
}

impl Tables {
    pub fn contexts(&self, id: ModeId) -> Vec<(Mode, BindingScope)> {
        let mut result = Vec::new();
        for &scope in BindingScope::ALL {
            for mode in [
                Mode::Normal,
                Mode::Select,
                Mode::Insert,
                Mode::Replace,
                Mode::List,
            ] {
                if self
                    .mode_for(mode, scope)
                    .is_some_and(|start| self.graph.chain(start).any(|ancestor| ancestor == id))
                {
                    result.push((mode, scope));
                }
            }
        }
        result
    }
}

pub(super) fn configure_graph(
    base: &super::Keymap,
    modes: Option<&serde_yaml::Value>,
    buffer_types: Option<&serde_yaml::Value>,
    bind: Option<&serde_yaml::Value>,
) -> Result<super::Keymap, String> {
    let Some(tables) = &base.overrides.1 else {
        return Ok(base.clone());
    };
    let mut definitions = tables.definitions.clone();
    let mut starts = tables.buffer_types.clone();
    if let Some(modes) = modes {
        let mapping = modes.as_mapping().ok_or("modes must be a mapping")?;
        if mapping.len() > super::mode_graph::MAX_MODES {
            return Err("too many mode declarations".into());
        }
        for (name, properties) in mapping {
            let name = name.as_str().ok_or("mode names must be strings")?;
            let properties = properties
                .as_mapping()
                .ok_or("mode properties must be a mapping")?;
            let mut merged =
                serde_yaml::to_value(definitions.get(name).cloned().unwrap_or_default())
                    .expect("mode schema");
            merged.as_mapping_mut().unwrap().extend(properties.clone());
            let properties =
                serde_yaml::from_value(merged).map_err(|e| format!("modes.{name}: {e}"))?;
            definitions.insert(name.into(), properties);
        }
    }
    if let Some(buffer_types) = buffer_types {
        let declared: BufferTypes = serde_yaml::from_value(buffer_types.clone())
            .map_err(|e| format!("buffer-types: {e}"))?;
        starts.extend(declared);
    }
    let mut names = tables
        .graph
        .modes()
        .map(|(_, mode)| mode.name())
        .collect::<Vec<_>>();
    if let Some(bind) = bind.and_then(serde_yaml::Value::as_mapping) {
        for name in bind.keys() {
            let name = name.as_str().ok_or("bind mode names must be strings")?;
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    if definitions == tables.definitions
        && starts == tables.buffer_types
        && names.len() == tables.graph.modes().count()
    {
        return Ok(base.clone());
    }
    let graph = ModeGraph::compile(&definitions, &names, &[], &starts, &required_buffer_types())?;
    let mut candidate = base.clone();
    candidate.overrides.1 = Some(std::sync::Arc::new(Tables {
        graph,
        definitions,
        buffer_types: starts,
        removed: tables.removed.clone(),
        plugin_modes: tables.plugin_modes.clone(),
    }));
    candidate.rebuild_lookup_index();
    Ok(candidate)
}

pub(super) fn apply(
    section: Option<&serde_yaml::Value>,
    base: super::Keymap,
    errors: &mut Vec<String>,
    defer_policy: bool,
) -> super::Keymap {
    let mut remaining = section.cloned();
    loop {
        let mut applied = Vec::new();
        let mut diagnostics = Vec::new();
        let result = apply_candidate(
            remaining.as_ref(),
            base.clone(),
            &mut diagnostics,
            defer_policy,
            &mut applied,
        );
        for diagnostic in diagnostics {
            if !errors.contains(&diagnostic) {
                errors.push(diagnostic);
            }
        }
        let (candidate, violations) = match result {
            Ok(candidate) => return candidate,
            Err(failure) => *failure,
        };
        let tables = candidate.overrides.1.as_ref().expect("named graph");
        let mut rejected = false;
        for (owner, source, sequence) in applied {
            let violation = violations.iter().find(|violation| {
                violation.sequences.contains(&sequence)
                    && violation
                        .named_mode
                        .or_else(|| tables.mode_for(violation.mode, violation.scope))
                        .is_some_and(|id| {
                            let effective = tables.indices(&candidate.bindings, id);
                            if let Some(binding) = effective
                                .iter()
                                .map(|&index| &candidate.bindings[index])
                                .find(|binding| binding.sequence == sequence)
                            {
                                return binding.binding_mode.as_deref() == Some(owner.as_str());
                            }
                            tables.graph.chain(id).find_map(|ancestor| {
                                let name = tables.graph.get(ancestor).unwrap().name();
                                tables
                                    .removed
                                    .get(name)
                                    .is_some_and(|removed| removed.contains(&sequence))
                                    .then_some(name)
                            }) == Some(owner.as_str())
                        })
            });
            if let Some(violation) = violation {
                if let Some(entries) = remaining
                    .as_mut()
                    .and_then(|section| section.get_mut(&owner))
                    .and_then(serde_yaml::Value::as_mapping_mut)
                {
                    rejected |= entries
                        .remove(serde_yaml::Value::String(source.clone()))
                        .is_some();
                }
                errors.push(format!("bind.{owner}.{source}: {}", violation.message));
            }
        }
        // Every retry removes at least one authored rule; the assignment cap
        // therefore bounds rollback, including conflicts exposed by a removal.
        if !rejected {
            errors.push(format!("bind rejected: {}", violations[0].message));
            return base;
        }
    }
}

fn apply_candidate(
    section: Option<&serde_yaml::Value>,
    base: super::Keymap,
    errors: &mut Vec<String>,
    defer_policy: bool,
    applied: &mut Vec<(String, String, KeySequence)>,
) -> Result<super::Keymap, Box<(super::Keymap, Vec<super::validate::Violation>)>> {
    use super::{BindingTarget, actions};
    let Some(section) = section else {
        return Ok(base);
    };
    let Some(modes) = section.as_mapping() else {
        errors.push("bind rejected: expected named-mode mappings".into());
        return Ok(base);
    };
    if modes.len() > super::mode_graph::MAX_MODES
        || modes
            .values()
            .filter_map(serde_yaml::Value::as_mapping)
            .map(|m| m.len())
            .sum::<usize>()
            > actions::max_bindings()
    {
        errors.push(format!(
            "bind rejected: at most {} assignments and {} modes",
            actions::max_bindings(),
            super::mode_graph::MAX_MODES
        ));
        return Ok(base);
    }
    let tables = base.overrides.1.as_ref().expect("named graph");
    let mut candidate = base.clone();
    let mut additions = Vec::new();
    for (name, entries) in modes {
        let Some(name) = name.as_str() else {
            continue;
        };
        let Some(id) = tables.graph.id(name) else {
            errors.push(format!("bind.{name}: unknown mode"));
            continue;
        };
        let Some(entries) = entries.as_mapping() else {
            errors.push(format!("bind.{name}: expected key mappings"));
            continue;
        };
        let mut contexts = tables.contexts(id);
        let owners = tables
            .graph
            .chain(id)
            .find_map(|ancestor| {
                let scopes: HashSet<_> = BindingScope::ALL
                    .iter()
                    .copied()
                    .filter(|&scope| {
                        [
                            Mode::Normal,
                            Mode::Select,
                            Mode::Insert,
                            Mode::Replace,
                            Mode::List,
                        ]
                        .into_iter()
                        .any(|mode| tables.mode_for(mode, scope) == Some(ancestor))
                    })
                    .collect();
                (!scopes.is_empty()).then_some(scopes)
            })
            .unwrap_or_default();
        let owner_scope = if owners.len() == 1 {
            *owners.iter().next().unwrap()
        } else {
            BindingScope::Global
        };
        if contexts.is_empty() {
            let mode = tables
                .graph
                .editing_role(buffer_type(owner_scope), id)
                .map_or(Mode::Normal, EditingRole::editing_mode);
            contexts.push((mode, owner_scope));
        }
        let contexts = contexts.as_slice();
        let mut seen = HashSet::new();
        for (source, value) in entries {
            let Some(source) = source.as_str().filter(|s| s.len() <= 512) else {
                errors.push(format!(
                    "bind.{name}: key must be a string of at most 512 bytes"
                ));
                continue;
            };
            if let Some((_, scope)) = contexts
                .iter()
                .find(|(mode, scope)| *mode == Mode::List || scope.is_merge_review())
                && let Err(reason) = actions::validate_list_binding(
                    value,
                    if scope.is_merge_review() || *scope == BindingScope::SessionManager {
                        scope.name()
                    } else {
                        "list"
                    },
                )
            {
                errors.push(format!("bind.{name}.{source}: {reason}"));
                continue;
            }
            // Parse without assuming the entry mode remains active. Completed
            // effective tables supply every starting context to flow analysis.
            let mut parsed = contexts.iter().map(|&(mode, _)| {
                actions::parse_rule_in(mode, source, value, &base, Some(owner_scope))
            });
            let first = parsed.next().expect("at least one context");
            let parsed = if first.is_ok() {
                first
            } else {
                parsed.find(Result::is_ok).unwrap_or(first)
            };
            let rule = match parsed {
                Ok(rule) => rule,
                Err(error) => {
                    errors.push(format!("bind.{name}.{source}: {error}"));
                    continue;
                }
            };
            let rule = &rule;
            if !seen.insert(rule.sequence.clone()) {
                errors.push(format!("bind.{name}.{source}: duplicate normalized key"));
                continue;
            }
            applied.push((name.into(), source.into(), rule.sequence.clone()));
            candidate
                .bindings
                .retain(|b| b.binding_mode.as_deref() != Some(name) || b.sequence != rule.sequence);
            let tables = std::sync::Arc::make_mut(candidate.overrides.1.as_mut().unwrap());
            if rule.program.is_empty() {
                tables
                    .removed
                    .entry(name.into())
                    .or_default()
                    .insert(rule.sequence.clone());
            } else {
                tables
                    .removed
                    .entry(name.into())
                    .or_default()
                    .remove(&rule.sequence);
                let target = rule
                    .program
                    .iter()
                    .next()
                    .map(actions::Action::target)
                    .unwrap_or(BindingTarget::Editor(
                        crate::command::EditorCommand::EnterNormalMode,
                    ));
                let mut binding = Binding::implemented_in(
                    actions::mode_slice(rule.mode),
                    owner_scope,
                    rule.sequence.clone(),
                    target,
                );
                binding.binding_mode = Some(name.into());
                binding.program = rule.program.clone();
                binding.description = rule.description.clone().into();
                binding.configured_actions = true;
                binding.source_paths = vec![vec!["bind".into(), name.into(), source.into()]];
                additions.push(binding);
            }
            // Compatibility provenance for view-based teaching metadata only.
            for &(mode, scope) in contexts {
                candidate
                    .overrides
                    .0
                    .entry((mode, scope))
                    .or_default()
                    .insert(rule.sequence.clone(), !rule.program.is_empty());
            }
        }
    }
    additions
        .sort_by_cached_key(|binding| (binding.binding_mode.clone(), binding.sequence.to_string()));
    candidate.bindings.extend(additions);
    let namespaces = std::mem::take(&mut candidate.namespaces);
    candidate.namespaces = namespaces
        .into_iter()
        .flat_map(|namespace| {
            namespace
                .modes
                .iter()
                .copied()
                .filter_map(|mode| {
                    let reachable =
                        candidate
                            .bindings_for_scope(mode, namespace.scope)
                            .any(|binding| {
                                binding.sequence.len() > namespace.sequence.len()
                                    && binding.sequence.starts_with(&namespace.sequence)
                            });
                    reachable.then(|| {
                        let mut copy = namespace.clone();
                        copy.modes = super::actions::mode_slice(mode);
                        copy
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let violations = super::validate::with_input_policy(
        &candidate.bindings,
        &candidate.namespaces,
        &candidate.context_actions,
        &candidate.overrides,
        (!defer_policy).then_some(candidate.ui()),
    );
    if !violations.is_empty() {
        return Err(Box::new((candidate, violations)));
    }
    candidate.rebuild_lookup_index();
    let flow = if applied.is_empty() {
        Vec::new()
    } else {
        super::flow::failures(&candidate, &super::keymap_for(candidate.fast_pane_keys()))
    }
    .into_iter()
    .filter(|failure| {
        candidate.bindings[failure.binding].configured_actions
            || failure.reason.contains("work bound")
    })
    .map(|failure| failure.violation(&candidate))
    .collect::<Vec<_>>();
    if !flow.is_empty() {
        return Err(Box::new((candidate, flow)));
    }
    Ok(candidate)
}

/// Runtime plugin views get ordinary named children, never a scope overlay.
pub(super) fn plugin_bindings(
    map: &mut super::Keymap,
    bindings: Vec<Binding>,
) -> anyhow::Result<Vec<Binding>> {
    let Some(base) = map.overrides.1.as_ref() else {
        return Ok(bindings);
    };
    let mut tables = (**base).clone();
    let mut definitions = tables.definitions.clone();
    definitions.retain(|name, _| !tables.plugin_modes.contains(name));
    for (_, mode) in tables.graph.modes() {
        if !tables.plugin_modes.contains(mode.name()) {
            definitions.entry(mode.name().to_owned()).or_default();
        }
    }
    let previous_plugin_modes = std::mem::take(&mut tables.plugin_modes);
    let mut result = Vec::new();
    for binding in bindings {
        let mut owners = HashSet::new();
        for &mode in binding.modes {
            if let BindingScope::Plugin(plugin) = binding.scope {
                let parent = tables
                    .mode_for(mode, BindingScope::Global)
                    .and_then(|id| tables.graph.get(id))
                    .ok_or_else(|| anyhow::anyhow!("plugin editing role is unavailable"))?;
                let name = format!("plugin-{plugin}-{}", super::actions::mode_name(mode));
                anyhow::ensure!(
                    tables.graph.id(&name).is_none() || previous_plugin_modes.contains(&name),
                    "plugin mode name {name} is reserved"
                );
                tables.plugin_modes.insert(name.clone());
                definitions.insert(
                    name.clone(),
                    super::mode_graph::Properties {
                        inherits: Some(parent.name().to_owned()),
                        unbound_keys: parent.unbound_keys(),
                    },
                );
                owners.insert(name);
            } else {
                for &scope in BindingScope::ALL {
                    if scope.is_merge_review() || scope == BindingScope::SessionManager {
                        continue;
                    }
                    if let Some(id) = tables.mode_for(mode, scope) {
                        owners.insert(tables.graph.get(id).expect("same graph").name().to_owned());
                    }
                }
            }
        }
        for owner in owners {
            let mut copy = binding.clone();
            copy.binding_mode = Some(owner.into());
            result.push(copy);
        }
    }
    tables.graph = ModeGraph::compile(
        &definitions,
        &[],
        &[],
        &tables.buffer_types,
        &required_buffer_types(),
    )
    .map_err(anyhow::Error::msg)?;
    tables.definitions = definitions;
    // Reject collisions before inheritance suppresses duplicates.
    for binding in &result {
        let id = tables
            .graph
            .id(binding
                .binding_mode
                .as_deref()
                .expect("named plugin binding"))
            .expect("declared plugin mode");
        anyhow::ensure!(
            !tables.indices(&map.bindings, id).iter().any(|&index| {
                let other = &map.bindings[index];
                binding.sequence.starts_with(&other.sequence)
                    || other.sequence.starts_with(&binding.sequence)
            }),
            "plugin binding collision: {}",
            binding.sequence
        );
    }
    for (index, binding) in result.iter().enumerate() {
        anyhow::ensure!(
            !result[..index]
                .iter()
                .any(|other| other.binding_mode == binding.binding_mode
                    && (binding.sequence.starts_with(&other.sequence)
                        || other.sequence.starts_with(&binding.sequence))),
            "plugin binding collision: {}",
            binding.sequence
        );
    }
    map.overrides.1 = Some(std::sync::Arc::new(tables));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{Lookup, configured, default_keymap};
    fn compile(source: &str) -> super::super::configured::CompiledKeymap {
        let config = serde_yaml::from_str(source).unwrap();
        configured::compile_configuration(&config, default_keymap())
    }
    #[test]
    fn missing_roles_are_checked_after_effective_removals_and_replacements() {
        let builtin = default_keymap();
        let normal = builtin
            .named_mode_for(Mode::Normal, BindingScope::Global)
            .unwrap();
        let mut bindings = serde_yaml::Mapping::new();
        for binding in builtin.bindings_for_named_mode(normal) {
            if binding.program.iter().any(|action| {
                matches!(
                    action.target(),
                    crate::keymap::BindingTarget::Editor(
                        crate::command::EditorCommand::ModeEnter
                            | crate::command::EditorCommand::EnterInsertMode
                            | crate::command::EditorCommand::AppendAfter
                            | crate::command::EditorCommand::InsertLineStart
                            | crate::command::EditorCommand::InsertLineEnd
                            | crate::command::EditorCommand::OpenLineAbove
                            | crate::command::EditorCommand::OpenLineBelow
                            | crate::command::EditorCommand::EnterReplaceMode
                    )
                )
            }) {
                bindings.insert(binding.sequence.to_string().into(), serde_yaml::Value::Null);
            }
        }
        bindings.insert("F6".into(), "yank".into());
        let source = format!(
            "buffer-types:\n  text: {{normal: normal}}\nkeys:\n  bind:\n    normal: {}",
            serde_json::to_string(&bindings).unwrap()
        );
        let result = compile(&source);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(
            result
                .keymap
                .named_mode_for(Mode::Insert, BindingScope::Global)
                .is_none()
        );
        // An unrelated valid key survives rollback of a broken role map.
        let rejected = compile(
            "buffer-types:\n  text: {normal: normal}\nkeys:\n  bind:\n    normal: {F6: yank}\n",
        );
        assert!(
            rejected
                .errors
                .iter()
                .any(|e| e.contains("buffer type \"text\"")
                    && e.contains("mode \"normal\"")
                    && e.contains("key")
                    && e.contains("role")),
            "{:?}",
            rejected.errors
        );
        assert!(
            rejected
                .keymap
                .named_mode_for(Mode::Insert, BindingScope::Global)
                .is_some()
        );
        assert!(
            matches!(rejected.keymap.lookup(Mode::Normal, &KeySequence::parse("F6").unwrap()), Lookup::Exact(binding) if binding.target == crate::command::EditorCommand::Yank.into())
        );
    }

    #[test]
    fn role_validation_traverses_conditional_branches_and_named_destinations() {
        use crate::command::{
            CommandExecutionContext, CommandId, CommandInvocation, EditorCommand as C,
            InvocationParameters,
        };
        use crate::keymap::{
            actions::Action,
            program::{Condition, Program},
        };
        let result = compile(
            "modes:\n  extra: {}\nbuffer-types:\n  text: {normal: normal}\nkeys:\n  defaults: false\n  bind:\n    normal: {F6: open-command-palette}\n    extra: {F7: yank}\n",
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let mut map = (*result.keymap).clone();
        let enter = |name: &str| {
            Program::new(vec![Action::Invocation(
                CommandInvocation::from_parts(
                    CommandId::Editor(C::ModeEnter),
                    InvocationParameters::OptionalText(Some(name.into())),
                    CommandExecutionContext::default(),
                )
                .unwrap(),
            )])
            .unwrap()
        };
        let extra = map
            .bindings
            .iter()
            .position(|b| b.binding_mode.as_deref() == Some("extra"))
            .unwrap();
        map.bindings[extra].program = Program::conditional(
            Condition::RecordingMacro,
            Program::single(C::Yank.into()),
            enter("insert"),
        );
        assert!(
            role_target_errors(&map).is_empty(),
            "unreachable mode is not an active role"
        );
        let normal = map
            .bindings
            .iter()
            .position(|b| b.binding_mode.as_deref() == Some("normal"))
            .unwrap();
        map.bindings[normal].program = enter("extra");
        let errors = role_target_errors(&map);
        assert!(
            errors.iter().any(|e| e.contains("text")
                && e.contains("extra")
                && e.contains("F7")
                && e.contains("insert")),
            "{errors:?}"
        );
    }

    #[test]
    fn cached_empty_children_preserve_tombstones_and_ancestor_prefixes() {
        let map = compile(
            "modes:
  ancestor: {inherits: normal}
  parent: {inherits: ancestor}
  child: {inherits: parent}
  empty-child: {inherits: child, unbound-keys: pass-to-buffer}
buffer-types:
  text: {normal: empty-child, select: select, insert: insert, replace: replace}
keys:
  bind:
    ancestor: {'F6 z': move-left}
    parent: {F6: move-right}
    child: {F6: null}
",
        );
        assert!(map.errors.is_empty(), "{:?}", map.errors);
        let tables = map.keymap.overrides.1.as_ref().unwrap();
        for (mode, indices) in tables.all_indices(&map.keymap.bindings) {
            assert_eq!(indices, tables.indices(&map.keymap.bindings, mode));
        }
        // The child's tombstone removes the parent leaf and exposes a longer
        // grandparent sequence. Filtering the parent's effective table loses it.
        assert!(matches!(
            map.keymap
                .lookup(Mode::Normal, &KeySequence::parse("F6 z").unwrap()),
            Lookup::Exact(_)
        ));
        assert!(matches!(
            map.keymap
                .lookup(Mode::Normal, &KeySequence::parse("F6").unwrap()),
            Lookup::Prefix(_)
        ));
        let id = tables.graph.id("empty-child").unwrap();
        assert_eq!(
            tables.graph.get(id).unwrap().unbound_keys(),
            super::super::mode_graph::UnboundKeys::PassToBuffer
        );
    }

    #[test]
    fn cached_default_tables_match_full_inheritance_walks() {
        let map = default_keymap();
        let tables = map.overrides.1.as_ref().unwrap();
        for (mode, indices) in tables.all_indices(&map.bindings) {
            assert_eq!(indices, tables.indices(&map.bindings, mode));
        }
    }

    #[test]
    fn normal_is_text_only_and_common_reaches_terminal() {
        let map = compile("keys:\n  bind:\n    normal: {F6: move-left}\n    common: {F7: save}\n");
        assert!(map.errors.is_empty(), "{:?}", map.errors);
        let key = KeySequence::parse("F6").unwrap();
        assert!(matches!(
            map.keymap
                .lookup_in(Mode::Normal, BindingScope::Global, &key),
            Lookup::Exact(_)
        ));
        assert!(matches!(
            map.keymap
                .lookup_in(Mode::Normal, BindingScope::Terminal, &key),
            Lookup::NoMatch
        ));
        let key = KeySequence::parse("F7").unwrap();
        assert!(matches!(
            map.keymap
                .lookup_in(Mode::Normal, BindingScope::Terminal, &key),
            Lookup::Exact(_)
        ));
    }
    #[test]
    fn custom_start_inherits_and_exact_null_does_not_reveal_parent() {
        let map = compile(
            "modes:\n  custom: {inherits: normal}\nbuffer-types:\n  text: custom\nkeys:\n  bind:\n    custom: {h: null, F6: move-up}\n",
        );
        assert!(map.errors.is_empty(), "{:?}", map.errors);
        assert!(matches!(
            map.keymap
                .lookup(Mode::Normal, &KeySequence::parse("h").unwrap()),
            Lookup::NoMatch
        ));
        assert!(matches!(
            map.keymap
                .lookup(Mode::Normal, &KeySequence::parse("l").unwrap()),
            Lookup::Exact(_)
        ));
        assert!(matches!(
            map.keymap
                .lookup(Mode::Normal, &KeySequence::parse("F6").unwrap()),
            Lookup::Exact(_)
        ));
    }
    #[test]
    fn property_overlay_preserves_parent_and_old_scopes_are_reported() {
        let map = compile(
            "modes:\n  normal: {unbound-keys: pass-to-buffer}\nkeys:\n  scopes: {markdown: {normal: {F6: move-left}}}\n",
        );
        assert_eq!(map.errors.len(), 1, "{:?}", map.errors);
        assert!(map.errors[0].contains("removed in 0.5"));
        assert!(matches!(
            map.keymap
                .lookup(Mode::Normal, &KeySequence::parse("h").unwrap()),
            Lookup::Exact(_)
        ));
        let graph = map.keymap.mode_graph().unwrap();
        assert_eq!(
            graph
                .get(graph.id("normal").unwrap())
                .unwrap()
                .unbound_keys(),
            super::super::mode_graph::UnboundKeys::PassToBuffer
        );
    }
    #[test]
    fn normal_only_mapping_can_replace_defaults_without_unavailable_role_exits() {
        let map = compile(
            "buffer-types:\n  text: {normal: normal}\nkeys:\n  defaults: false\n  bind:\n    normal: {F6: open-command-palette}\n",
        );
        assert!(map.errors.is_empty(), "{:?}", map.errors);
        assert!(matches!(
            map.keymap
                .lookup(Mode::Normal, &KeySequence::parse("F6").unwrap()),
            Lookup::Exact(_)
        ));
        assert!(matches!(
            map.keymap
                .lookup(Mode::Normal, &KeySequence::parse("h").unwrap()),
            Lookup::NoMatch
        ));
        assert!(
            map.keymap
                .named_mode_for(Mode::Insert, BindingScope::Global)
                .is_none()
        );
    }

    #[test]
    fn editor_modes_cannot_enter_ui_owned_list_tables() {
        for source in [
            "buffer-types: {text: {normal: list}}",
            "buffer-types: {text: {normal: session-manager}}",
            "modes: {custom: {inherits: list}}",
            "modes: {custom: {inherits: session-manager}}",
        ] {
            let map = compile(source);
            assert!(!map.errors.is_empty(), "{source}");
            assert!(
                map.errors.iter().any(|error| error.contains("UI-owned")),
                "{:?}",
                map.errors
            );
            assert!(matches!(
                map.keymap
                    .lookup(Mode::Normal, &KeySequence::parse("h").unwrap()),
                Lookup::Exact(_)
            ));
        }
    }
    #[test]
    fn invalid_rule_keeps_independent_rules_in_other_inheritance_chains() {
        let map = compile(
            "keys:\n  bind:\n    normal: {F6: move-left, 'h z': move-right}\n    directory: {'h z': move-right, h: null}\n",
        );
        assert_eq!(map.errors.len(), 1, "{:?}", map.errors);
        assert!(map.errors[0].contains("bind.normal.h z"));
        assert!(
            matches!(map.keymap.lookup(Mode::Normal, &KeySequence::parse("F6").unwrap()), Lookup::Exact(binding) if binding.target == super::super::BindingTarget::Editor(crate::command::EditorCommand::MoveLeft))
        );
        assert!(matches!(
            map.keymap.lookup_in(
                Mode::Normal,
                BindingScope::Directory,
                &KeySequence::parse("h z").unwrap()
            ),
            Lookup::Exact(_)
        ));
        assert!(matches!(
            map.keymap
                .lookup(Mode::Normal, &KeySequence::parse("h").unwrap()),
            Lookup::Exact(_)
        ));
    }
    #[test]
    fn child_conflict_does_not_reject_a_shadowed_ancestor_rule() {
        let map = compile(
            "keys:\n  bind:\n    normal: {F6: move-left}\n    select: {F6: move-up, 'F6 z': move-right}\n",
        );
        assert_eq!(map.errors.len(), 2, "{:?}", map.errors);
        assert!(
            map.errors
                .iter()
                .all(|error| error.starts_with("bind.select."))
        );
        for mode in [Mode::Normal, Mode::Select] {
            assert!(
                matches!(map.keymap.lookup(mode, &KeySequence::parse("F6").unwrap()), Lookup::Exact(binding) if binding.target == super::super::BindingTarget::Editor(crate::command::EditorCommand::MoveLeft))
            );
        }
    }
}
