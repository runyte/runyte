// SPDX-License-Identifier: MPL-2.0

//! The settings registry as a whole: every identity it lists, the field each
//! one writes, and what it refuses.
//!
//! These are exhaustive over `SettingId::ALL` rather than a sample, because a
//! setting wired to the wrong configuration field is silent — it reads back
//! whatever the field it should have written still holds, and only the setting
//! nobody happened to test is wrong.

use runyte::{
    command::GrammarKind,
    config::{Config, WorkspaceMode},
    settings::{SettingId, SettingRegistry, SettingType, SettingValue},
};

/// A value of the setting's own type, different from `current` wherever the
/// type admits a second value at all.
fn other_value(setting: SettingId, current: &SettingValue, config: &Config) -> SettingValue {
    match (setting.descriptor().value_type, current) {
        (SettingType::Boolean, SettingValue::Boolean(value)) => SettingValue::Boolean(!value),
        (SettingType::Integer { minimum, maximum }, SettingValue::Integer(value)) => {
            assert!(
                minimum < maximum,
                "{} has one legal value",
                setting.descriptor().key
            );
            SettingValue::Integer(if *value == minimum { maximum } else { minimum })
        }
        (SettingType::Grammar, SettingValue::Grammar(value)) => SettingValue::Grammar(
            GrammarKind::ALL
                .iter()
                .copied()
                .find(|candidate| candidate != value)
                .unwrap_or(*value),
        ),
        (SettingType::WorkspaceMode, SettingValue::WorkspaceMode(value)) => {
            SettingValue::WorkspaceMode(
                WorkspaceMode::ALL
                    .iter()
                    .copied()
                    .find(|candidate| candidate != value)
                    .expect("a second workspace mode"),
            )
        }
        (SettingType::Theme, SettingValue::Text(value)) => SettingValue::Text(
            setting
                .allowed_values(config)
                .into_iter()
                .find(|candidate| candidate != value)
                .expect("a second usable theme"),
        ),
        (value_type, value) => {
            panic!(
                "{value:?} is not {value_type} for {}",
                setting.descriptor().key
            )
        }
    }
}

/// A value of a type the setting does not accept.
fn wrong_typed_value(setting: SettingId) -> SettingValue {
    match setting.descriptor().value_type {
        SettingType::Boolean => SettingValue::Integer(0),
        SettingType::Grammar
        | SettingType::Integer { .. }
        | SettingType::Theme
        | SettingType::WorkspaceMode
        | SettingType::Text => SettingValue::Boolean(true),
    }
}

#[test]
fn every_setting_writes_and_reads_back_the_field_its_descriptor_names() {
    let mut config = Config::default();
    for setting in SettingId::ALL.iter().copied() {
        let key = setting.descriptor().key;
        let current = setting.configured_value(&config);
        let replacement = other_value(setting, &current, &config);

        setting
            .apply(&replacement, &mut config)
            .unwrap_or_else(|error| panic!("{key}: {error}"));

        assert_eq!(
            setting.configured_value(&config),
            replacement,
            "{key} did not read back what applying it wrote"
        );
    }

    // Applying every setting in turn must have left each earlier one standing:
    // two identities sharing a field would only show up once the second has
    // been written.
    let mut written = Config::default();
    let intended = SettingId::ALL
        .iter()
        .copied()
        .map(|setting| {
            let value = other_value(setting, &setting.configured_value(&written), &written);
            setting.apply(&value, &mut written).unwrap();
            (setting, value)
        })
        .collect::<Vec<_>>();
    for (setting, value) in intended {
        assert_eq!(
            setting.configured_value(&written),
            value,
            "{} was overwritten by a later setting",
            setting.descriptor().key
        );
    }
}

#[test]
fn a_value_of_the_wrong_type_is_refused_and_names_what_the_setting_expects() {
    let mut config = Config::default();
    for setting in SettingId::ALL.iter().copied() {
        let descriptor = setting.descriptor();
        let unchanged = setting.configured_value(&config);
        let error = setting
            .apply(&wrong_typed_value(setting), &mut config)
            .expect_err(descriptor.key);

        let message = error.to_string();
        assert!(message.contains(descriptor.key), "{message}");
        assert!(
            message.contains(&descriptor.value_type.to_string()),
            "{message} does not say it expected {}",
            descriptor.value_type
        );
        assert_eq!(
            setting.configured_value(&config),
            unchanged,
            "{} changed despite being refused",
            descriptor.key
        );
    }
}

#[test]
fn an_integer_outside_its_range_and_a_theme_that_cannot_be_resolved_are_refused() {
    let mut config = Config::default();
    let SettingType::Integer { minimum, maximum } =
        SettingId::EditorTabWidth.descriptor().value_type
    else {
        panic!("the tab width is an integer setting");
    };

    for out_of_range in [minimum.saturating_sub(1), maximum + 1] {
        let error = SettingId::EditorTabWidth
            .apply(&SettingValue::Integer(out_of_range), &mut config)
            .expect_err("out of range");
        assert!(
            error
                .to_string()
                .contains(&format!("from {minimum} through {maximum}")),
            "{error}"
        );
    }

    let error = SettingId::Theme
        .apply(&SettingValue::Text("no-such-theme".to_owned()), &mut config)
        .expect_err("unknown theme");
    assert!(error.to_string().contains("no-such-theme"), "{error}");
    assert_eq!(
        SettingId::Theme.configured_value(&config),
        SettingId::Theme.configured_value(&Config::default()),
        "a refused theme still changed the configuration"
    );
    assert_eq!(
        SettingId::EditorTabWidth.configured_value(&config),
        SettingId::EditorTabWidth.configured_value(&Config::default()),
        "a refused tab width still changed the configuration"
    );
}

#[test]
fn only_the_enumerated_setting_types_offer_values_to_choose_from() {
    let config = Config::default();
    for setting in SettingId::ALL.iter().copied() {
        let allowed = setting.allowed_values(&config);
        let key = setting.descriptor().key;
        match setting.descriptor().value_type {
            SettingType::Boolean => assert_eq!(allowed, ["true", "false"], "{key}"),
            SettingType::Integer { .. } | SettingType::Text => {
                assert!(allowed.is_empty(), "{key} offered {allowed:?}");
            }
            SettingType::Grammar => {
                let names = GrammarKind::ALL
                    .iter()
                    .map(|kind| kind.name().to_owned())
                    .collect::<Vec<_>>();
                assert_eq!(allowed, names, "{key}");
            }
            SettingType::WorkspaceMode => {
                let modes = WorkspaceMode::ALL
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                assert_eq!(allowed, modes, "{key}");
            }
            SettingType::Theme => {
                assert!(!allowed.is_empty(), "{key} offered no theme");
                for name in &allowed {
                    config
                        .resolve_theme(name)
                        .unwrap_or_else(|error| panic!("{key}: {name}: {error}"));
                }
            }
        }
    }
}

#[test]
fn the_registry_finds_every_listed_setting_by_key_and_nothing_else() {
    for setting in SettingId::ALL.iter().copied() {
        let key = setting.descriptor().key;
        assert_eq!(
            SettingRegistry::find(key).map(|descriptor| descriptor.id),
            Some(setting)
        );
    }
    assert!(SettingRegistry::find("editor").is_none());
    assert!(SettingRegistry::find("").is_none());
}
