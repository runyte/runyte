// SPDX-License-Identifier: MPL-2.0

use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::config::DEFAULT_THEME;

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// A fresh directory holding `config.yaml` and its two fragment directories.
struct Root(PathBuf);

impl Root {
    fn new() -> Self {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "runyte-config-fragments-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn config(&self) -> PathBuf {
        self.0.join("config.yaml")
    }

    fn load(&self) -> Result<Config> {
        Config::load(Some(&self.config())).map(|(config, _)| config)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn keys(config: &Config) -> String {
    serde_yaml::to_string(config.keys.as_ref().unwrap()).unwrap()
}

#[test]
fn directories_beside_the_file_add_to_it_in_name_order() {
    let root = Root::new();
    root.write(
        "config.yaml",
        "theme: mine\nthemes:\n  mine:\n    accent: '#000001'\nkeys:\n  leader: Ctrl-x\n  bind:\n    normal:\n      F5: save\n      F6: yank\n",
    );
    root.write("themes.d/mine.yaml", "accent: '#000002'\n");
    root.write("themes.d/ocean-dark.yaml", "accent: '#000003'\n");
    root.write("themes.d/.swap.yaml", "not: [valid\n");
    root.write("themes.d/notes.txt", "not a theme\n");
    root.write(
        "keybinds.d/10-first.yaml",
        "leader: Ctrl-a\nbind:\n  normal:\n    F6: undo\n    F7: redo\n",
    );
    root.write(
        "keybinds.d/20-second.yaml",
        "bind:\n  normal:\n    F7: null\n  insert:\n    Ctrl-s: save\n",
    );
    root.write("keybinds.d/30-empty.yaml", "");

    let config = root.load().unwrap();

    assert_eq!(config.themes["mine"].accent, "#000002");
    // A file named after a built-in replaces it, as an inline definition does.
    assert_eq!(config.themes["ocean-dark"].accent, "#000003");
    assert!(config.themes.contains_key(DEFAULT_THEME));
    assert_eq!(
        keys(&config),
        "leader: Ctrl-a\nbind:\n  normal:\n    F5: save\n    F6: undo\n    F7: null\n  insert:\n    Ctrl-s: save\n"
    );
}

#[test]
fn the_directories_load_without_a_configuration_file() {
    let root = Root::new();
    root.write("themes.d/mine.yaml", "accent: '#000002'\n");
    root.write("keybinds.d/normal.yaml", "bind:\n  normal:\n    F5: save\n");

    let config = root.load().unwrap();

    assert_eq!(config.themes["mine"].accent, "#000002");
    assert_eq!(keys(&config), "bind:\n  normal:\n    F5: save\n");
}

#[test]
fn a_broken_file_is_named_in_the_error_and_a_broken_key_shape_is_left_to_the_keymap() {
    let root = Root::new();
    root.write("themes.d/mine.yaml", "accent: [unclosed\n");
    let error = format!("{:#}", root.load().unwrap_err());
    let broken = root.0.join("themes.d").join("mine.yaml");
    assert!(error.contains(&broken.display().to_string()), "{error}");

    let root = Root::new();
    root.write("keybinds.d/normal.yaml", "- not\n- a mapping\n");
    assert_eq!(keys(&root.load().unwrap()), "- not\n- a mapping\n");
}

#[test]
fn files_split_themes_and_keys_out_of_the_configuration_file() {
    let mut config: Config = serde_yaml::from_str(
        "themes:\n  mine:\n    accent: '#000001'\n  Mine:\n    accent: '#000002'\n  'not/a file':\n    accent: '#000003'\nkeys:\n  leader: Ctrl-x\n  bind:\n    normal:\n      F5: save\n    insert:\n      Ctrl-s: save\n    visual:\n      F5: save\n",
    )
    .unwrap();
    config.merge_builtin_defaults();

    let files = config.to_files(Path::new("config.yaml")).unwrap();
    let names = files
        .iter()
        .map(|(path, _)| path.as_path())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "config.yaml",
            "themes.d/Mine.yaml",
            "keybinds.d/00-prefixes.yaml",
            "keybinds.d/insert.yaml",
            "keybinds.d/normal.yaml",
            "keybinds.d/replace.yaml",
            "keybinds.d/select.yaml",
        ]
        .map(Path::new)
    );
    let main = &files[0].1;
    assert!(!main.contains("\nkeys:"), "{main}");
    // Only what a file name cannot carry stays inline: a name that is not a
    // portable file name, and one that differs from another only in case.
    assert!(main.contains("\nthemes:\n  mine:\n"), "{main}");
    assert!(main.contains("\n  not/a file:\n"), "{main}");
    assert!(!main.contains("\n  Mine:\n"), "{main}");
    assert_eq!(
        files[1].1,
        serde_yaml::to_string(&config.themes["Mine"]).unwrap()
    );
    // A mode `bind` does not accept stays with the prefixes, where it is
    // still reported.
    assert_eq!(
        files[2].1,
        "leader: Ctrl-x\nbind:\n  visual:\n    F5: save\n"
    );
    assert!(files[4].1.starts_with("bind:\n  normal:\n    F5: save\n"));
}

#[test]
fn written_files_load_back_to_the_same_configuration() {
    let root = Root::new();
    let mut config: Config = serde_yaml::from_str(
        "editor:\n  tab_width: 3\ntheme: mine\nthemes:\n  mine:\n    accent: '#000001'\nkeys:\n  rebind:\n    Space g: Leader G\n  bind:\n    normal:\n      F5: save\n      J: null\n",
    )
    .unwrap();
    config.merge_builtin_defaults();
    let files = config.to_files(Path::new("config.yaml")).unwrap();
    for (relative, text) in &files {
        root.write(&relative.to_string_lossy(), text);
    }

    let loaded = root.load().unwrap();

    assert_eq!(loaded.to_files(Path::new("config.yaml")).unwrap(), files);
    assert_eq!(loaded.editor.tab_width, 3);
    assert_eq!(loaded.keys, config.keys);
}

#[test]
fn explicit_null_theme_roles_survive_writing_inline_and_to_a_theme_file() {
    let root = Root::new();
    // Every role whose omission means a default colour rather than null.
    let nulls = [
        "warning",
        "info",
        "change_added",
        "change_modified",
        "change_removed",
        "diff_added",
        "diff_removed",
        "diff_changed",
    ];
    let mut source = String::from("themes:\n  mine:\n    accent: '#000001'\n");
    for role in nulls {
        source.push_str(&format!("    {role}: null\n"));
    }
    let mut config: Config = serde_yaml::from_str(&source).unwrap();
    config.merge_builtin_defaults();
    let theme = &config.themes["mine"];
    assert!(theme.warning.is_none() && theme.diff_added.is_none());
    assert_ne!(*theme, ThemeDefinition::default());

    let inline: Config = serde_yaml::from_str(&config.to_yaml().unwrap()).unwrap();
    assert_eq!(inline.themes["mine"], *theme);

    for (relative, text) in config.to_files(Path::new("config.yaml")).unwrap() {
        root.write(&relative.to_string_lossy(), &text);
    }
    assert_eq!(root.load().unwrap().themes["mine"], *theme);
}

#[test]
fn default_export_keeps_keys_absent_and_examples_can_be_uncommented() {
    let root = Root::new();
    let original = Config::default();
    crate::settings::write_config(&root.config(), &original, false).unwrap();
    let loaded = root.load().unwrap();
    assert!(
        loaded.keys.is_none(),
        "comment-only files must not activate key compilation"
    );
    let normal = fs::read_to_string(root.0.join("keybinds.d/normal.yaml")).unwrap();
    assert!(normal.contains("#     w: move-word-forward\n"));
    let uncommented = normal
        .lines()
        .skip(1)
        .map(|line| line.strip_prefix("# ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    let keys: Value = serde_yaml::from_str(&uncommented).unwrap();
    for fast in [false, true] {
        let compiled = crate::keymap::configured::compile(&keys, &crate::keymap::keymap_for(fast));
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    }
    assert_eq!(
        loaded.to_files(Path::new("config.yaml")).unwrap(),
        original.to_files(Path::new("config.yaml")).unwrap()
    );
}

#[test]
fn a_large_custom_keymap_exports_without_adding_active_defaults() {
    let root = Root::new();
    let mut source = String::from("keys:\n  bind:\n    normal:\n");
    for n in 0..1400 {
        let letters = format!("{n:04}")
            .bytes()
            .map(|digit| char::from(b'a' + digit - b'0').to_string())
            .collect::<Vec<_>>()
            .join(" ");
        source.push_str(&format!("      F12 {letters}: save\n"));
    }
    let config: Config = serde_yaml::from_str(&source).unwrap();
    crate::settings::write_config(&root.config(), &config, false).unwrap();
    let loaded = root.load().unwrap();
    assert_eq!(loaded.keys, config.keys);
    assert_eq!(
        loaded.keys.as_ref().unwrap()["bind"]["normal"]
            .as_mapping()
            .unwrap()
            .len(),
        1400
    );
    for fast in [false, true] {
        let compiled = crate::keymap::configured::compile(
            loaded.keys.as_ref().unwrap(),
            &crate::keymap::keymap_for(fast),
        );
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    }
    crate::settings::write_config(&root.config(), &loaded, true).unwrap();
    assert_eq!(root.load().unwrap().keys, config.keys);
}

#[test]
fn invalid_keymaps_are_refused_before_any_generated_file_is_written() {
    for source in [
        "keys: {bind: {normal: {F5: no-such-action}}}",
        "keys: {bind: {normal: {F5: save, 'F5 a': yank}}}",
        "keys: {bind: {normal: {Tab: save}}}",
    ] {
        let root = Root::new();
        root.write("config.yaml", "# original config\n");
        root.write("keybinds.d/normal.yaml", "# original keys\n");
        let config: Config = serde_yaml::from_str(source).unwrap();
        let error = crate::settings::write_config(&root.config(), &config, true).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("keys with editor.fast_pane_keys="),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(root.config()).unwrap(),
            "# original config\n"
        );
        assert_eq!(
            fs::read_to_string(root.0.join("keybinds.d/normal.yaml")).unwrap(),
            "# original keys\n"
        );
        assert!(!root.0.join("keybinds.d/insert.yaml").exists());
        assert!(!root.0.join("themes.d").exists());
    }
}

#[test]
fn retained_fragments_participate_in_validation_and_filename_order() {
    let root = Root::new();
    root.write(
        "keybinds.d/zz-custom.yaml",
        "bind: {normal: {F5: no-such-action}}\n",
    );
    let config: Config = serde_yaml::from_str("keys: {bind: {normal: {F5: save}}}").unwrap();
    assert!(crate::settings::write_config(&root.config(), &config, false).is_err());
    assert!(!root.config().exists());
    assert!(!root.0.join("keybinds.d/normal.yaml").exists());
    // The retained later file wins over the generated normal.yaml, as on load.
    root.write("keybinds.d/zz-custom.yaml", "bind: {normal: {F5: yank}}\n");
    crate::settings::write_config(&root.config(), &config, false).unwrap();
    assert_eq!(
        root.load().unwrap().keys.unwrap()["bind"]["normal"]["F5"],
        "yank"
    );
    // A malformed file that is being replaced must not poison the preview.
    root.write("keybinds.d/normal.yaml", "invalid: [\n");
    crate::settings::write_config(&root.config(), &config, true).unwrap();
}

#[test]
fn theme_and_key_directory_limits_are_checked_before_writing() {
    let root = Root::new();
    let mut config = Config::default();
    for n in 0..257 {
        config
            .themes
            .insert(format!("custom-{n:03}"), ThemeDefinition::default());
    }
    let error = crate::settings::write_config(&root.config(), &config, false).unwrap_err();
    assert!(
        error.to_string().contains("more than 256 .yaml files"),
        "{error}"
    );
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
    config.themes.remove("custom-256");
    // An existing file consumes a slot even though it is not being replaced.
    root.write("themes.d/retained.yaml", "accent: '#123456'\n");
    assert!(crate::settings::write_config(&root.config(), &config, true).is_err());
    assert!(!root.config().exists());
    assert_eq!(fs::read_dir(root.0.join("themes.d")).unwrap().count(), 1);
    config.themes.remove("custom-255");
    crate::settings::write_config(&root.config(), &config, true).unwrap();
    assert!(root.load().is_ok());
    // Replacing files does not consume extra slots.
    crate::settings::write_config(&root.config(), &config, true).unwrap();

    let root = Root::new();
    for n in 0..253 {
        root.write(&format!("keybinds.d/extra-{n:03}.yaml"), "# retained\n");
    }
    let error =
        crate::settings::write_config(&root.config(), &Config::default(), true).unwrap_err();
    assert!(
        error.to_string().contains("more than 256 .yaml files"),
        "{error}"
    );
    assert!(!root.config().exists());
    assert_eq!(
        fs::read_dir(root.0.join("keybinds.d")).unwrap().count(),
        253
    );
}

#[test]
fn empty_mode_maps_leave_a_comment_only_template_and_explicit_defaults_stay_active() {
    let root = Root::new();
    let config: Config = serde_yaml::from_str("keys: {bind: {normal: {}, insert: {}}}").unwrap();
    crate::settings::write_config(&root.config(), &config, false).unwrap();
    assert!(root.load().unwrap().keys.is_none());
    let config: Config = serde_yaml::from_str(
        "keys: {bind: {normal: {w: move-word-forward, J: null, F5: [select-all, yank]}}}",
    )
    .unwrap();
    crate::settings::write_config(&root.config(), &config, true).unwrap();
    assert_eq!(root.load().unwrap().keys, config.keys);
    let normal = fs::read_to_string(root.0.join("keybinds.d/normal.yaml")).unwrap();
    assert!(!normal.contains("#     w:"));
    assert!(!normal.contains("#     J:"));
    assert!(normal.contains("\n    w: move-word-forward\n"));
    let edited = normal.replace("#     h: move-left", "    h: move-right");
    assert_ne!(edited, normal, "the example must exist and be editable");
    root.write("keybinds.d/normal.yaml", &edited);
    let keys = root.load().unwrap().keys.unwrap();
    assert_eq!(keys["bind"]["normal"]["h"], "move-right");
    assert!(
        crate::keymap::configured::compile(&keys, &crate::keymap::keymap_for(false))
            .errors
            .is_empty()
    );
}

#[test]
fn over_limit_assignments_refuse_forced_export_without_touching_files() {
    let root = Root::new();
    root.write("config.yaml", "# keep main\n");
    root.write("keybinds.d/normal.yaml", "# keep keys\n");
    let mut source = String::from("keys:\n  bind:\n    normal:\n");
    for n in 0..=crate::keymap::actions::MAX_BINDINGS {
        let letters = format!("{n:05}")
            .bytes()
            .map(|digit| char::from(b'a' + digit - b'0').to_string())
            .collect::<Vec<_>>()
            .join(" ");
        source.push_str(&format!("      F12 {letters}: save\n"));
    }
    let mut config: Config = serde_yaml::from_str(&source).unwrap();
    config
        .themes
        .insert("mine".into(), ThemeDefinition::default());
    let error = crate::settings::write_config(&root.config(), &config, true).unwrap_err();
    assert!(error.to_string().contains("65536 assignments"), "{error}");
    assert_eq!(fs::read_to_string(root.config()).unwrap(), "# keep main\n");
    assert_eq!(
        fs::read_to_string(root.0.join("keybinds.d/normal.yaml")).unwrap(),
        "# keep keys\n"
    );
    assert!(!root.0.join("themes.d").exists());
    assert!(!root.0.join("keybinds.d/insert.yaml").exists());
}
