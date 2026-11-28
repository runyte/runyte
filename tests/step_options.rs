// SPDX-License-Identifier: MPL-2.0
use runyte::{
    command::{Mode, discovery},
    config::Config,
    keymap::{BindingScope, KeySequence, Lookup, configured, default_keymap},
};

#[test]
fn invalid_step_options_roll_back_only_the_bad_binding() {
    for step in [
        "{action: move-right, count: 0}",
        "{action: move-right, count: -1}",
        "{action: move-right, count: 1000000}",
        "{action: move-right, count: '2'}",
        "{action: move-right, count: true}",
        "{action: move-right, count: null}",
        "{action: yank, count: 2}",
        "{action: move-right, register: a}",
        "{action: yank, register: 1}",
        "{action: yank, register: ''}",
        "{action: yank, register: ab}",
        "{action: yank, register: null}",
        "{action: yank, extend: true}",
        "{action: move-right, extend: 'true'}",
        "{action: move-right, extend: null}",
        "{if: {mode: normal}, then: [yank], else: [], count: 1}",
    ] {
        let config: Config = serde_yaml::from_str(&format!(
            "keys: {{bind: {{normal: {{F6: {step}, F7: yank}}}}}}"
        ))
        .unwrap();
        let compiled = configured::compile_configuration(&config, default_keymap());
        assert_eq!(compiled.errors.len(), 1, "{step}: {:?}", compiled.errors);
        assert!(matches!(
            compiled
                .keymap
                .lookup(Mode::Normal, &KeySequence::parse("F7").unwrap()),
            Lookup::Exact(_)
        ));
        assert!(matches!(
            compiled
                .keymap
                .lookup(Mode::Normal, &KeySequence::parse("F6").unwrap()),
            Lookup::NoMatch
        ));
    }
}

#[test]
fn discovery_and_copy_preserve_all_step_options() {
    let section = serde_yaml::from_str("bind: {normal: {F6: [{action: move-right, count: 3, extend: true}, {action: yank, register: λ}, {action: paste-after, count: none, register: none}, {action: move-left, count: inherit, extend: false}, {action: yank, register: inherit}]}}").unwrap();
    let compiled = configured::compile(&section, default_keymap());
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    let key = KeySequence::parse("F6").unwrap();
    let Lookup::Exact(binding) = compiled.keymap.lookup(Mode::Normal, &key) else {
        panic!()
    };
    let description = discovery::binding_steps(binding, BindingScope::Global);
    for field in [
        "count: 3",
        "count: none",
        "count: inherit",
        "register: λ",
        "register: none",
        "register: inherit",
        "extend: true",
        "extend: false",
    ] {
        assert!(description.contains(field), "{field}: {description}");
    }
    let yaml =
        discovery::binding_yaml(binding, Mode::Normal, Some(&section), &compiled.keymap).unwrap();
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    let copied = configured::compile(&value["keys"], default_keymap());
    assert!(copied.errors.is_empty(), "{:?}", copied.errors);
    let Lookup::Exact(copy) = copied.keymap.lookup(Mode::Normal, &key) else {
        panic!()
    };
    assert_eq!(binding.program, copy.program);
}

#[test]
fn mode_flow_uses_fixed_none_and_consumed_count_paths() {
    for (steps, valid) in [
        (
            "[{action: mode-enter, arg: select, temporary: true}, {action: move-file-end, count: 1}, {if: {mode: normal}, then: [insert-newline], else: [yank]}]",
            true,
        ),
        (
            "[{action: mode-enter, arg: select, temporary: true}, {action: move-file-end, count: none}, {if: {mode: normal}, then: [insert-newline], else: [yank]}]",
            false,
        ),
        (
            "[{action: mode-enter, arg: select, temporary: true}, move-right, move-file-end, {if: {mode: normal}, then: [insert-newline], else: [yank]}]",
            false,
        ),
    ] {
        let config: Config =
            serde_yaml::from_str(&format!("keys:\n  bind:\n    normal:\n      F6: {steps}\n"))
                .unwrap();
        let compiled = configured::compile_configuration(&config, default_keymap());
        assert_eq!(
            compiled.errors.is_empty(),
            valid,
            "{steps}: {:?}",
            compiled.errors
        );
    }
}

#[test]
fn inherited_count_presence_stays_correlated_across_mode_conditions() {
    let config: Config = serde_yaml::from_str("keys:\n  bind:\n    normal:\n      F6:\n        - {action: mode-enter, arg: select, temporary: true}\n        - {action: move-file-end, count: inherit}\n        - if: {mode: normal}\n          then:\n            - {action: mode-enter, arg: select, temporary: true}\n            - {action: move-file-end, count: inherit}\n            - {if: {mode: select}, then: [insert-newline], else: []}\n          else: []\n").unwrap();
    let compiled = configured::compile_configuration(&config, default_keymap());
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
}

#[test]
fn tree_motion_options_are_rejected_in_the_reachable_view() {
    for steps in [
        "{action: goto-window-bottom, extend: true}",
        "{action: half-page-down, extend: false}",
    ] {
        let config: Config = serde_yaml::from_str(&format!(
            "keys:\n  bind:\n    directory-tree:\n      F6: {steps}\n"
        ))
        .unwrap();
        let compiled = configured::compile_configuration(&config, default_keymap());
        assert!(
            compiled.errors.iter().any(|error| error.contains("extend")),
            "{:?}",
            compiled.errors
        );
    }
}
