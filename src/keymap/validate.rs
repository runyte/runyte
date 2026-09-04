// SPDX-License-Identifier: MPL-2.0

//! Validation shared by built-in and configured keymaps.

use std::collections::{HashMap, HashSet};

use crate::command::Mode;

use super::{Binding, BindingNamespace, BindingScope, ContextAction, KeySequence};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ViolationKind {
    DuplicateEffectiveSequence,
    ExactAndPrefix,
    GlobalScopedShadowing,
    NamespaceDuplicate,
    NamespaceUnreachable,
    NamespaceExecutable,
    AliasBroken,
    ContextActionDuplicate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Violation {
    pub kind: ViolationKind,
    pub mode: Mode,
    pub scope: BindingScope,
    pub sequences: Vec<KeySequence>,
    pub message: String,
}

const MODES: [Mode; 5] = [
    Mode::Normal,
    Mode::Insert,
    Mode::Replace,
    Mode::Select,
    Mode::Command,
];

pub fn validate(
    bindings: &[Binding],
    namespaces: &[BindingNamespace],
    actions: &[ContextAction],
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut views: HashMap<(Mode, BindingScope), Vec<&Binding>> = HashMap::new();
    for mode in MODES {
        let globals = bindings
            .iter()
            .filter(|binding| binding.is_active_in(mode) && binding.scope == BindingScope::Global)
            .collect::<Vec<_>>();
        for &scope in BindingScope::ALL {
            let scoped = if scope == BindingScope::Global {
                Vec::new()
            } else {
                bindings
                    .iter()
                    .filter(|binding| binding.is_active_in(mode) && binding.scope == scope)
                    .collect::<Vec<_>>()
            };
            let scoped_sequences = scoped
                .iter()
                .map(|binding| &binding.sequence)
                .collect::<HashSet<_>>();
            let mut effective = scoped;
            effective.extend(globals.iter().copied().filter(|binding| {
                scope == BindingScope::Global || !scoped_sequences.contains(&binding.sequence)
            }));

            let mut by_sequence: HashMap<&KeySequence, &Binding> = HashMap::new();
            for binding in &effective {
                if let Some(previous) = by_sequence.insert(&binding.sequence, binding) {
                    violations.push(Violation {
                        kind: ViolationKind::DuplicateEffectiveSequence,
                        mode,
                        scope,
                        sequences: vec![previous.sequence.clone(), binding.sequence.clone()],
                        message: format!(
                            "{} reaches both {} and {} in {} {:?}",
                            binding.sequence,
                            previous.target.name(),
                            binding.target.name(),
                            mode.label(),
                            scope
                        ),
                    });
                }
            }

            let mut proper_prefixes = HashMap::new();
            for binding in &effective {
                for length in 1..binding.sequence.len() {
                    proper_prefixes
                        .entry(KeySequence::new(
                            binding.sequence.as_slice()[..length].iter().copied(),
                        ))
                        .or_insert_with(|| binding.sequence.clone());
                }
            }
            for binding in &effective {
                if let Some(longer) = proper_prefixes.get(&binding.sequence) {
                    violations.push(Violation {
                        kind: ViolationKind::ExactAndPrefix,
                        mode,
                        scope,
                        sequences: vec![binding.sequence.clone(), longer.clone()],
                        message: format!(
                            "{} is executable and prefixes {} in {} {:?}",
                            binding.sequence,
                            longer,
                            mode.label(),
                            scope
                        ),
                    });
                }
            }
            if scope != BindingScope::Global {
                let globals_by_sequence = globals
                    .iter()
                    .map(|binding| (&binding.sequence, *binding))
                    .collect::<HashMap<_, _>>();
                for scoped in effective.iter().filter(|binding| binding.scope == scope) {
                    if let Some(global) = globals_by_sequence.get(&scoped.sequence) {
                        violations.push(Violation {
                            kind: ViolationKind::GlobalScopedShadowing,
                            mode,
                            scope,
                            sequences: vec![scoped.sequence.clone(), global.sequence.clone()],
                            message: format!(
                                "{} shadows global command {} in {} {:?}",
                                scoped.sequence,
                                global.target.name(),
                                mode.label(),
                                scope
                            ),
                        });
                    }
                }
            }
            views.insert((mode, scope), effective);
        }
    }

    let mut seen_namespaces = HashSet::new();
    for namespace in namespaces {
        for &mode in namespace.modes {
            if !seen_namespaces.insert((mode, namespace.scope, namespace.sequence.clone())) {
                violations.push(Violation {
                    kind: ViolationKind::NamespaceDuplicate,
                    mode,
                    scope: namespace.scope,
                    sequences: vec![namespace.sequence.clone()],
                    message: format!("duplicate namespace {}", namespace.sequence),
                });
            }
            let effective = &views[&(mode, namespace.scope)];
            let exact = effective
                .iter()
                .any(|binding| binding.sequence == namespace.sequence);
            let descendant = effective.iter().any(|binding| {
                binding.sequence != namespace.sequence
                    && binding.sequence.starts_with(&namespace.sequence)
            });
            if exact {
                violations.push(Violation {
                    kind: ViolationKind::NamespaceExecutable,
                    mode,
                    scope: namespace.scope,
                    sequences: vec![namespace.sequence.clone()],
                    message: format!("namespace {} is executable", namespace.sequence),
                });
            } else if !descendant {
                violations.push(Violation {
                    kind: ViolationKind::NamespaceUnreachable,
                    mode,
                    scope: namespace.scope,
                    sequences: vec![namespace.sequence.clone()],
                    message: format!("namespace {} has no reachable binding", namespace.sequence),
                });
            }
        }
    }

    for binding in bindings {
        let Some(alias) = &binding.alias else {
            continue;
        };
        for &mode in binding.alias_modes.unwrap_or(binding.modes) {
            let reached = views[&(mode, binding.scope)]
                .iter()
                .find(|candidate| candidate.sequence == *alias)
                .map(|candidate| candidate.target);
            if reached != Some(binding.target) {
                violations.push(Violation {
                    kind: ViolationKind::AliasBroken,
                    mode,
                    scope: binding.scope,
                    sequences: vec![binding.sequence.clone(), alias.clone()],
                    message: format!(
                        "{} advertises alias {}, which does not reach {}",
                        binding.sequence,
                        alias,
                        binding.target.name()
                    ),
                });
            }
        }
    }

    let mut seen_actions = HashSet::new();
    for action in actions {
        if !seen_actions.insert((action.scope, action.mnemonic)) {
            violations.push(Violation {
                kind: ViolationKind::ContextActionDuplicate,
                mode: Mode::Normal,
                scope: action.scope,
                sequences: vec![KeySequence::from(action.mnemonic)],
                message: format!("duplicate contextual action {}", action.mnemonic),
            });
        }
    }
    violations
}

pub fn assert_valid(keymap: &super::Keymap) {
    let violations = validate(
        keymap.bindings(),
        keymap.namespaces(),
        keymap.all_context_actions(),
    );
    assert!(
        violations.is_empty(),
        "invalid keymap: {}",
        violations
            .iter()
            .map(|violation| violation.message.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    );
}

#[cfg(test)]
mod tests {
    use crate::{
        command::EditorCommand,
        keymap::{BindingTarget, Key},
    };

    use super::*;

    const NORMAL: &[Mode] = &[Mode::Normal];

    fn binding(sequence: impl Into<KeySequence>, command: EditorCommand) -> Binding {
        Binding::implemented(NORMAL, sequence, command)
    }

    fn kinds(
        bindings: &[Binding],
        namespaces: &[BindingNamespace],
        actions: &[ContextAction],
    ) -> HashSet<ViolationKind> {
        validate(bindings, namespaces, actions)
            .into_iter()
            .map(|violation| violation.kind)
            .collect()
    }

    #[test]
    fn every_structural_violation_class_is_detected() {
        let duplicate = binding(Key::char('a'), EditorCommand::OpenExplorer);
        assert!(
            kinds(&[duplicate.clone(), duplicate], &[], &[])
                .contains(&ViolationKind::DuplicateEffectiveSequence)
        );

        assert!(
            kinds(
                &[
                    binding(Key::char('a'), EditorCommand::OpenExplorer),
                    binding(
                        [Key::char('a'), Key::char('b')],
                        EditorCommand::OpenFilePicker,
                    ),
                ],
                &[],
                &[],
            )
            .contains(&ViolationKind::ExactAndPrefix)
        );

        let mut scoped = binding(Key::char('a'), EditorCommand::OpenExplorer);
        scoped.scope = BindingScope::Help;
        assert!(
            kinds(
                &[
                    scoped,
                    binding(Key::char('a'), EditorCommand::OpenFilePicker),
                ],
                &[],
                &[],
            )
            .contains(&ViolationKind::GlobalScopedShadowing)
        );

        let namespace = BindingNamespace::global(NORMAL, Key::char('a'), "test");
        assert!(
            kinds(
                &[binding(Key::char('a'), EditorCommand::OpenExplorer)],
                &[namespace],
                &[],
            )
            .contains(&ViolationKind::NamespaceExecutable)
        );
        let namespace = BindingNamespace::global(NORMAL, Key::char('a'), "test");
        assert!(
            kinds(&[], std::slice::from_ref(&namespace), &[])
                .contains(&ViolationKind::NamespaceUnreachable)
        );
        assert!(
            kinds(&[], &[namespace.clone(), namespace], &[])
                .contains(&ViolationKind::NamespaceDuplicate)
        );

        let advertised =
            binding(Key::char('a'), EditorCommand::OpenExplorer).with_alias(Key::char('b'));
        let other = binding(Key::char('b'), EditorCommand::OpenFilePicker);
        assert!(kinds(&[advertised, other], &[], &[]).contains(&ViolationKind::AliasBroken));

        let actions = [
            ContextAction::row(
                BindingScope::Help,
                Key::char('a'),
                "open",
                BindingTarget::Editor(EditorCommand::OpenExplorer),
            ),
            ContextAction::row(
                BindingScope::Help,
                Key::char('a'),
                "find",
                BindingTarget::Editor(EditorCommand::OpenFilePicker),
            ),
        ];
        assert!(kinds(&[], &[], &actions).contains(&ViolationKind::ContextActionDuplicate));
    }

    #[test]
    fn both_built_in_variants_satisfy_the_shared_validator() {
        for fast in [false, true] {
            assert_valid(&crate::keymap::keymap_for(fast));
        }
    }
}
