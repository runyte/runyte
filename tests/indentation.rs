// SPDX-License-Identifier: MPL-2.0

use runyte::{
    config::{Config, IndentStyle},
    indentation::{Indentation, Scope, compile_pattern},
    settings::{SettingId, SettingValue, override_value, persist_override, persist_setting},
};
use std::{fs, path::Path};

#[test]
fn overrides_resolve_each_field_in_language_then_file_order() {
    let config: Config = serde_yaml::from_str("editor:\n  tab_width: 4\nindentation:\n  languages:\n    yaml:\n      tab_width: 2\n  files:\n    '*.yaml':\n      indent: tabs\n    'generated/**':\n      tab_width: 8\n    'generated/special.yaml':\n      tab_width: 4\n").unwrap();
    let base = Indentation {
        tab_width: 4,
        style: IndentStyle::Spaces,
    };
    for (path, expected, style) in [
        ("test.yaml", 2, IndentStyle::Tabs),
        ("nested/test.yaml", 2, IndentStyle::Tabs),
        ("generated/a.yaml", 8, IndentStyle::Tabs),
        ("generated/special.yaml", 4, IndentStyle::Tabs),
    ] {
        assert_eq!(
            config
                .indentation
                .resolve(base, Some("yaml"), Some(Path::new(path))),
            Indentation {
                tab_width: expected,
                style
            }
        );
    }
    assert_eq!(
        config
            .indentation
            .resolve(base, Some("yaml"), None)
            .tab_width,
        2
    );
    assert_eq!(config.indentation.resolve(base, None, None), base);
}

#[test]
fn patterns_have_bounded_explicit_workspace_relative_semantics() {
    for (pattern, yes, no) in [
        ("*.yaml", "nested/a.yaml", "a.yml"),
        ("src/*.py", "src/a.py", "nested/src/a.py"),
        (
            "**/generated/*.yaml",
            "generated/a.yaml",
            "generated/nested/a.yaml",
        ),
        ("a?.py", "ab.py", "abc.py"),
        ("**/a.py", "nested/a.py", "b.py"),
    ] {
        let matcher = compile_pattern(pattern).unwrap();
        assert!(matcher.is_match(yes), "{pattern}");
        assert!(!matcher.is_match(no), "{pattern}");
    }
    for pattern in [
        "",
        "/absolute",
        "../other",
        "a/../b",
        "a\\b",
        "[ab]",
        "{a,b}",
        "a\nb",
        "a//b",
    ] {
        assert!(compile_pattern(pattern).is_err(), "{pattern:?}");
    }
    assert!(compile_pattern(&"a".repeat(257)).is_err());
}

#[test]
fn scoped_writes_preserve_comments_order_crlf_and_unrelated_values() {
    let root = runyte::test_support::TestRuntimeRoot::new("indent").unwrap();
    let path = root.path().join("config.yaml");
    let source = "# settings\r\neditor:\r\n  tab_width: 4 # global\r\nindentation:\r\n  files:\r\n    '*.yaml': # first\r\n      tab_width: 2 # keep\r\n    'special.yaml':\r\n      indent: tabs\r\nunknown: {a: 1}\r\n";
    fs::write(&path, source).unwrap();
    let scope = Scope::Files("*.yaml".into());
    persist_override(
        &path,
        SettingId::EditorTabWidth,
        &scope,
        Some(&SettingValue::Integer(6)),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("tab_width: 2", "tab_width: 6")
    );
    persist_override(&path, SettingId::EditorTabWidth, &scope, None).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("tab_width: 2", "tab_width: null")
    );
    persist_override(
        &path,
        SettingId::EditorIndent,
        &scope,
        Some(&SettingValue::Indent(IndentStyle::Spaces)),
    )
    .unwrap();
    persist_override(
        &path,
        SettingId::EditorTabWidth,
        &Scope::Language("python".into()),
        Some(&SettingValue::Integer(4)),
    )
    .unwrap();
    let loaded =
        persist_setting(&path, SettingId::EditorTabWidth, &SettingValue::Integer(8)).unwrap();
    assert_eq!(
        override_value(
            SettingId::EditorTabWidth,
            &Scope::Language("python".into()),
            &loaded.indentation
        ),
        Some(SettingValue::Integer(4))
    );
    assert_eq!(
        loaded
            .indentation
            .files
            .iter()
            .map(|r| r.pattern.as_str())
            .collect::<Vec<_>>(),
        ["*.yaml", "special.yaml"]
    );
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("unknown: {a: 1}\r\n"));
    assert!(!text.replace("\r\n", "").contains('\n'));
}

#[test]
fn new_overrides_are_explicit_and_quoted_patterns_round_trip() {
    let root = runyte::test_support::TestRuntimeRoot::new("indent").unwrap();
    let path = root.path().join("config.yaml");
    let language = Scope::Language("python".into());
    persist_override(
        &path,
        SettingId::EditorTabWidth,
        &language,
        Some(&SettingValue::Integer(4)),
    )
    .unwrap();
    let scope = Scope::Files("dir/it's: special?.py".into());
    persist_override(
        &path,
        SettingId::EditorIndent,
        &scope,
        Some(&SettingValue::Indent(IndentStyle::Tabs)),
    )
    .unwrap();
    let config = persist_override(
        &path,
        SettingId::EditorTabWidth,
        &scope,
        Some(&SettingValue::Integer(2)),
    )
    .unwrap();
    assert_eq!(
        override_value(SettingId::EditorTabWidth, &scope, &config.indentation),
        Some(SettingValue::Integer(2))
    );
    assert_eq!(
        override_value(SettingId::EditorIndent, &scope, &config.indentation),
        Some(SettingValue::Indent(IndentStyle::Tabs))
    );
}

#[test]
fn invalid_overrides_and_unsafe_yaml_never_modify_configuration() {
    let root = runyte::test_support::TestRuntimeRoot::new("indent").unwrap();
    let path = root.path().join("config.yaml");
    for source in [
        "indentation: {languages: {python: {tab_width: 2}}}\n",
        "indentation:\n  languages:\n    python:\n      tab_width: 2\n      'tab_width': 4\n",
        "editor: &settings\n  tab_width: 4\n",
    ] {
        fs::write(&path, source).unwrap();
        assert!(
            persist_override(
                &path,
                SettingId::EditorTabWidth,
                &Scope::Language("python".into()),
                Some(&SettingValue::Integer(8))
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
    fs::write(&path, "# unchanged\n").unwrap();
    for (scope, value) in [
        (Scope::Language("typo".into()), 2),
        (Scope::Language("python".into()), 0),
        (Scope::Files("../x".into()), 2),
    ] {
        assert!(
            persist_override(
                &path,
                SettingId::EditorTabWidth,
                &scope,
                Some(&SettingValue::Integer(value))
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "# unchanged\n");
    }
}

#[test]
fn filename_patterns_that_look_like_yaml_scalars_stay_strings() {
    let root = runyte::test_support::TestRuntimeRoot::new("indent").unwrap();
    let path = root.path().join("config.yaml");
    for name in ["123", "true", "null", "False", "0xFF"] {
        let scope = Scope::Files(name.into());
        let config = persist_override(
            &path,
            SettingId::EditorTabWidth,
            &scope,
            Some(&SettingValue::Integer(2)),
        )
        .unwrap();
        assert_eq!(
            override_value(SettingId::EditorTabWidth, &scope, &config.indentation),
            Some(SettingValue::Integer(2))
        );
    }
}
