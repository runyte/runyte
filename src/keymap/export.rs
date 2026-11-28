// SPDX-License-Identifier: MPL-2.0
//! Deterministic public configuration for the effective named keymap.
use super::{Binding, BindingAvailability, BindingTarget, Keymap};
use serde_yaml::{Mapping, Value};
use std::collections::BTreeMap;

const HEADER: &str = "# Effective Runyte keymap; load on top of the same preset, platform and fast-pane setting.\n# Planned/unsupported bindings and preset teaching metadata are omitted.\n# Unchanged UI/list rows stay implicit until decision 54's post-0.5 list executor; structural UI ownership remains external.\n# Plugin registrations and bindings are excluded; retain plugins[].bindings separately.\n";

fn value(binding: &Binding) -> Result<Value, String> {
    let program = super::program_yaml(&binding.program)?;
    let mut record = Mapping::new();
    record.insert(
        "steps".into(),
        match program {
            Value::Sequence(_) => program,
            step => Value::Sequence(vec![step]),
        },
    );
    record.insert("description".into(), binding.description.to_string().into());
    Ok(record.into())
}

fn list_owner(map: &Keymap, owner: &str) -> bool {
    let tables = map.overrides.1.as_ref().expect("named map");
    tables.graph.id(owner).is_some_and(|id| {
        tables
            .contexts(id)
            .iter()
            .any(|(mode, scope)| *mode == crate::command::Mode::List || scope.is_merge_review())
    })
}

fn implicit_reason(binding: &Binding, preset: &[Binding], map: &Keymap) -> Option<String> {
    if !preset.iter().any(|b| {
        b.binding_mode == binding.binding_mode
            && b.sequence == binding.sequence
            && b.program == binding.program
            && b.description == binding.description
    }) {
        return None;
    }
    let owner = binding.binding_mode.as_deref()?;
    if list_owner(map, owner) {
        return Some("single-action UI owner (decision 54)".into());
    }
    let tables = map.overrides.1.as_ref()?;
    let id = tables.graph.id(owner)?;
    let value = value(binding).ok()?;
    tables.contexts(id).into_iter().find_map(|(mode, scope)| {
        super::actions::parse_rule_in(
            mode,
            &binding.sequence.to_string(),
            &value,
            map,
            Some(scope),
        )
        .err()
    })
}

/// Render a fragment against the same built-in preset. No compiler or runtime
/// state is changed; keys hidden by inheritance remain in their owning table.
pub fn yaml(map: &Keymap) -> Result<String, String> {
    let tables = map
        .overrides
        .1
        .as_ref()
        .ok_or("keymap has no named mode graph")?;
    let preset = super::configured::export_base(map);
    let mut modes = tables.definitions.clone();
    modes.retain(|name, _| !tables.plugin_modes.contains(name));
    let mut bind = BTreeMap::<String, BTreeMap<String, Value>>::new();
    for (_, mode) in tables.graph.modes() {
        if !tables.plugin_modes.contains(mode.name()) {
            modes.entry(mode.name().into()).or_default();
            bind.entry(mode.name().into()).or_default();
        }
    }
    // A moved or removed preset assignment needs an exact tombstone, including
    // aliases and platform-only keys. Inheritance must not resurrect it.
    for binding in &preset {
        let Some(owner) = binding.binding_mode.as_deref() else {
            continue;
        };
        if !map
            .bindings
            .iter()
            .any(|b| b.binding_mode.as_deref() == Some(owner) && b.sequence == binding.sequence)
        {
            bind.entry(owner.into())
                .or_default()
                .insert(binding.sequence.to_string(), Value::Null);
        }
    }
    for (owner, sequences) in &tables.removed {
        if tables.plugin_modes.contains(owner) {
            continue;
        }
        for sequence in sequences {
            bind.entry(owner.clone())
                .or_default()
                .insert(sequence.to_string(), Value::Null);
        }
    }
    for binding in &map.bindings {
        let Some(owner) = binding.binding_mode.as_deref() else {
            continue;
        };
        if tables.plugin_modes.contains(owner)
            || matches!(binding.target, BindingTarget::Plugin(_))
            || binding.availability != BindingAvailability::Implemented
        {
            continue;
        }
        let list = list_owner(map, owner);
        if implicit_reason(binding, &preset, map).is_some() {
            continue;
        }
        let serialized = if list {
            super::program_yaml(&binding.program)?
        } else {
            value(binding)?
        };
        bind.entry(owner.into())
            .or_default()
            .insert(binding.sequence.to_string(), serialized);
    }
    let count: usize = bind.values().map(BTreeMap::len).sum();
    let limit = super::actions::max_bindings();
    if count > limit {
        return Err(format!(
            "effective keymap needs {count} assignments; export limit is {limit}; no file written"
        ));
    }
    let mut keys = Mapping::new();
    keys.insert("leader".into(), map.leader.to_string().into());
    keys.insert("window".into(), map.window.to_string().into());
    keys.insert(
        "bind".into(),
        serde_yaml::to_value(bind).map_err(|e| e.to_string())?,
    );
    keys.insert("ui".into(), map.ui.export());
    keys.insert("pointer".into(), map.pointer.export());
    let mut document = Mapping::new();
    document.insert(
        "modes".into(),
        serde_yaml::to_value(modes).map_err(|e| e.to_string())?,
    );
    document.insert(
        "buffer-types".into(),
        serde_yaml::to_value(&tables.buffer_types).map_err(|e| e.to_string())?,
    );
    document.insert("keys".into(), keys.into());
    Ok(format!(
        "{HEADER}{}",
        serde_yaml::to_string(&document).map_err(|e| e.to_string())?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn executable_program(
        program: &super::super::program::Program,
    ) -> super::super::program::Program {
        use super::super::{
            actions::Action,
            program::{Program, Step},
        };
        use crate::command::{CommandId, CommandInvocation, EditorCommand, HelpInvocation};
        let action = |action: &Action| match action {
            Action::Invocation(invocation) => Action::Invocation(invocation.clone()),
            _ if action.target().id() == CommandId::Editor(EditorCommand::ShowHelp) => {
                Action::Invocation(CommandInvocation::help(HelpInvocation::ActiveView))
            }
            _ => action
                .target()
                .invocation()
                .map(Action::Invocation)
                .unwrap_or_else(|_| Action::Target(action.target())),
        };
        Program::from_steps(
            program
                .steps()
                .iter()
                .map(|step| match step {
                    Step::Action(a) => Step::Action(action(a)),
                    Step::ActionWithOptions { action: a, options } => Step::ActionWithOptions {
                        action: action(a),
                        options: *options,
                    },
                    Step::If {
                        condition,
                        then,
                        otherwise,
                    } => Step::If {
                        condition: condition.clone(),
                        then: Box::new(executable_program(then)),
                        otherwise: Box::new(executable_program(otherwise)),
                    },
                })
                .collect(),
        )
        .unwrap()
    }
    fn executable(
        map: &Keymap,
    ) -> BTreeMap<String, BTreeMap<String, (super::super::program::Program, String)>> {
        let tables = map.overrides.1.as_ref().unwrap();
        tables
            .graph
            .modes()
            .map(|(id, mode)| {
                let bindings = tables
                    .indices(&map.bindings, id)
                    .into_iter()
                    .map(|i| &map.bindings[i])
                    .filter(|b| b.availability == BindingAvailability::Implemented)
                    .map(|b| {
                        (
                            b.sequence.to_string(),
                            (executable_program(&b.program), b.description.to_string()),
                        )
                    })
                    .collect();
                (mode.name().into(), bindings)
            })
            .collect()
    }
    fn round_trip(source: &str, fast: bool) {
        let preset = super::super::keymap_for(fast);
        let config: crate::config::Config = serde_yaml::from_str(source).unwrap();
        let original = super::super::configured::compile_configuration(&config, &preset);
        assert!(original.errors.is_empty(), "{:?}", original.errors);
        let first = yaml(&original.keymap).unwrap();
        let config: crate::config::Config = serde_yaml::from_str(&first).unwrap();
        let loaded = super::super::configured::compile_configuration(&config, &preset);
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let before = executable(&original.keymap);
        let after = executable(&loaded.keymap);
        assert_eq!(
            before.keys().collect::<Vec<_>>(),
            after.keys().collect::<Vec<_>>()
        );
        for (mode, entries) in &before {
            for (key, program) in entries {
                assert_eq!(Some(program), after[mode].get(key), "{mode} {key}");
            }
            for key in after[mode].keys() {
                assert!(entries.contains_key(key), "extra binding {mode} {key}");
            }
        }
        assert!(
            original
                .keymap
                .mode_graph()
                .unwrap()
                .same_structure(loaded.keymap.mode_graph().unwrap())
        );
        assert_eq!(
            original.keymap.pointer.masks(),
            loaded.keymap.pointer.masks()
        );
        assert_eq!(first, yaml(&loaded.keymap).unwrap());
    }
    #[test]
    fn preset_round_trip() {
        for fast in [false, true] {
            round_trip("{}", fast);
        }
    }
    #[test]
    fn implicit_preset_rows_are_an_explicit_inventory() {
        let map = super::super::default_keymap();
        let preset = super::super::configured::export_base(map);
        let mut rows = map
            .bindings
            .iter()
            .filter_map(|binding| {
                implicit_reason(binding, &preset, map).map(|reason| {
                    format!(
                        "{} | {} | {reason}",
                        binding.binding_mode.as_deref().unwrap(),
                        binding.sequence
                    )
                })
            })
            .collect::<Vec<_>>();
        rows.sort();
        assert_eq!(
            rows,
            include_str!("../../tests/fixtures/keymap/export-implicit.txt")
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn long_generated_descriptions_are_canonical_and_bounded() {
        let argument = format!("\"{}\"", "a".repeat(1100));
        let config = serde_yaml::to_string(
            &serde_yaml::from_str::<Value>(&format!(
                "keys: {{bind: {{normal: {{F2: {{action: open, arg: '{argument}'}}}}}}}}"
            ))
            .unwrap(),
        )
        .unwrap();
        round_trip(&config, false);
    }

    #[test]
    fn oversized_fragment_is_refused_before_writing() {
        let preset = super::super::default_keymap();
        super::super::actions::with_max_bindings(1, || {
            assert!(
                yaml(preset)
                    .unwrap_err()
                    .contains("export limit is 1; no file written")
            );
        });
    }

    #[test]
    fn plugins_remain_external_and_can_be_registered_after_loading() {
        let preset = super::super::default_keymap();
        let plugin = || {
            Binding::implemented_in(
                super::super::MODAL,
                super::super::BindingScope::Plugin(7),
                super::super::KeySequence::parse("F12").unwrap(),
                BindingTarget::Plugin(1),
            )
        };
        let map = preset.with_plugin_bindings(vec![plugin()]).unwrap();
        let first = yaml(&map).unwrap();
        assert!(!first.contains("plugin-7"));
        let config = serde_yaml::from_str(&first).unwrap();
        let loaded = super::super::configured::compile_configuration(&config, preset);
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let registered = loaded.keymap.with_plugin_bindings(vec![plugin()]).unwrap();
        assert_eq!(executable(&map), executable(&registered));
        assert_eq!(first, yaml(&registered).unwrap());
    }

    #[test]
    fn standalone_and_remapped_bindings_round_trip() {
        round_trip(
            r#"
keys:
  defaults: false
  bind:
    normal:
      ':': open-command-palette
      F5: move-right
    select:
      Esc: [fit-selection-for-normal-mode, {action: mode-enter, arg: normal}]
    insert:
      Esc: [fit-selection-for-normal-mode, {action: mode-enter, arg: normal}]
    replace:
      Esc: [fit-selection-for-normal-mode, {action: mode-enter, arg: normal}]
"#,
            false,
        );
    }

    #[test]
    fn configured_programs_and_owner_overrides_round_trip() {
        round_trip(
            r#"
modes:
  review: { inherits: normal }
keys:
  leader: Ctrl-x
  window: Ctrl-z
  rebind: { 'Space f': 'F9' }
  bind:
    normal:
      F2: move-right
      F3:
        description: 'A supplied program'
        steps:
          - { action: mode-enter, arg: select, temporary: true }
          - { action: move-right, count: 3, extend: true }
          - { action: yank, register: a }
      F4:
        - if: recording-macro
          then: [{action: move-right, count: inherit}]
          else: [{action: move-left, count: none, extend: false}]
      F5: { action: find-next-char, arg: x }
      F6: { action: open, arg: 'some file.txt' }
      F7: { action: help, arg: key-actions }
      Alt-F7: help
      F8: { action: mode-enter, arg: review, temporary: true }
      Alt-F9: { action: session-rename, arg: '"old session" new name'  }
      Alt-F10: { action: grammar, arg: runyte }
      Alt-F11: { action: write-config }
      Ctrl-Super-F12: move-left
      x: null
    review:
      F2: move-left
      F3: null
    insert:
      F2: { action: insert-text, arg: "hello\nworld" }
    list:
      F2: list-next
      Down: list-previous
      Up: null
  ui:
    context-launcher: {open-actions: []}
    command-prompt: {cancel: [F12]}
  pointer:
    pane-move: [Alt-RightDrag]
"#,
            false,
        );
    }
}
