// SPDX-License-Identifier: MPL-2.0
use runyte::{
    config::Config,
    keymap::{configured, default_keymap},
};

#[test]
fn supplied_arguments_validate_shapes_and_keep_valid_neighbors() {
    for (action, argument) in [
        ("find-next-char", "''"),
        ("replace-char", "ab"),
        ("surround-replace", "a"),
        ("surround-replace", "abc"),
        ("search", "''"),
        ("search", "42"),
        ("search", "null"),
        ("yank", "x"),
    ] {
        let config: Config = serde_yaml::from_str(&format!("keys:\n  bind:\n    normal:\n      F6: {{action: {action}, arg: {argument}}}\n      F7: yank\n")).unwrap();
        let compiled = configured::compile_configuration(&config, default_keymap());
        assert_eq!(
            compiled.errors.len(),
            1,
            "{action} {argument}: {:?}",
            compiled.errors
        );
    }
}

#[test]
fn shared_mode_guide_examples_compile_together() {
    let config: Config = serde_yaml::from_str("keys:\n  bind:\n    common:\n      Ctrl-s: save\n    modal-keys:\n      F6: yank\n    insert-keys:\n      Alt-Left: move-left\n").unwrap();
    let compiled = configured::compile_configuration(&config, default_keymap());
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
}

#[test]
fn prompt_ownership_and_background_work_remain_last_only() {
    for step in [
        "search",
        "{action: search-workspace, arg: abc}",
        "{action: macro-replay, arg: a}",
    ] {
        let config: Config = serde_yaml::from_str(&format!(
            "keys: {{bind: {{normal: {{F6: [{step}, yank]}}}}}}"
        ))
        .unwrap();
        let compiled = configured::compile_configuration(&config, default_keymap());
        assert!(!compiled.errors.is_empty(), "{step}");
    }
}
