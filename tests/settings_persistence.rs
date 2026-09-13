// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use runyte::settings::{SettingId, SettingValue, persist_setting};

struct ConfigFile(PathBuf);

impl ConfigFile {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "runyte-settings-persistence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.yaml");
        fs::write(&path, source).unwrap();
        Self(path)
    }

    fn text(&self) -> String {
        fs::read_to_string(&self.0).unwrap()
    }
}

impl Drop for ConfigFile {
    fn drop(&mut self) {
        fs::remove_dir_all(self.0.parent().unwrap()).unwrap();
    }
}

#[test]
fn theme_save_preserves_unrelated_flow_collections_byte_for_byte() {
    let collections = [
        "editor: { tab_width: 2 } # compact settings\n",
        "private: {nested: [{name: 'writer''s {room}', text: \"a \\\"}\\\"\"}]}\n",
        "private: { # multiline, including keys that resemble settings\ntheme: light,\nnested: [\n{theme: dark},\n{theme: paper}\n]\n} # end\n",
        "private: [\n{theme: light},\n{theme: dark}\n]\n",
        "private: {text: 'a multiline\n} quoted value', more: true}\n",
    ];
    for collection in collections {
        for newline in ["\n", "\r\n"] {
            for theme_first in [false, true] {
                let collection = collection.replace('\n', newline);
                let original = format!("theme: light # keep{newline}");
                let updated = format!("theme: 'paper' # keep{newline}");
                let (source, expected) = if theme_first {
                    (original + &collection, updated + &collection)
                } else {
                    (collection.clone() + &original, collection + &updated)
                };
                let file = ConfigFile::new(&source);
                let config = persist_setting(
                    &file.0,
                    SettingId::Theme,
                    &SettingValue::Text("paper".into()),
                )
                .unwrap();
                assert_eq!(config.theme.as_deref(), Some("paper"));
                assert_eq!(file.text(), expected);
            }
        }
    }
}

#[test]
fn missing_theme_is_appended_after_a_complete_flow_collection() {
    let source = "private: {\ntheme: light\n}";
    let file = ConfigFile::new(source);
    persist_setting(
        &file.0,
        SettingId::Theme,
        &SettingValue::Text("paper".into()),
    )
    .unwrap();
    assert_eq!(file.text(), format!("{source}\ntheme: 'paper'\n"));
}

#[test]
fn child_setting_is_inserted_after_its_siblings_complete_flow_value() {
    let source = "editor:\n  private: {\nsoft_wrap: false,\nnested: {soft_wrap: false}\n} # keep\n# next section\nprivate: {}\n";
    let file = ConfigFile::new(source);
    let config = persist_setting(
        &file.0,
        SettingId::EditorSoftWrap,
        &SettingValue::Boolean(true),
    )
    .unwrap();
    assert!(config.editor.soft_wrap);
    assert_eq!(
        file.text(),
        source.replace("# next section", "  soft_wrap: true\n# next section")
    );
}

#[test]
fn unsafe_or_invalid_flow_documents_are_never_written() {
    for source in [
        "editor: {tab_width: 2}\n",
        "editor: {\ntab_width: 2\n}\n",
        "editor:\n  tab_width: 2\nprivate: {same: 1, same: 2}\n",
        "editor:\n  tab_width: 2\nprivate: {value: &anchor 1, alias: *anchor}\n",
        "editor:\n  tab_width: 2\nprivate: {broken: [}\n",
        "{editor: {tab_width: 2}}\n",
    ] {
        let file = ConfigFile::new(source);
        assert!(
            persist_setting(
                &file.0,
                SettingId::EditorTabWidth,
                &SettingValue::Integer(8)
            )
            .is_err(),
            "unexpectedly accepted {source}"
        );
        assert_eq!(file.text(), source);
    }
}

#[test]
fn an_unclosed_brace_in_a_plain_theme_name_cannot_consume_other_fields() {
    let source = "themes:\n  'custom{':\n    accent: '#123456'\ntheme: custom{\nprivate: keep\n";
    let file = ConfigFile::new(source);
    assert!(
        persist_setting(
            &file.0,
            SettingId::Theme,
            &SettingValue::Text("paper".into())
        )
        .is_err()
    );
    assert_eq!(file.text(), source);
}
