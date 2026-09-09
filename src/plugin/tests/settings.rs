// SPDX-License-Identifier: MPL-2.0

use super::*;
use serde_json::json;

fn schema(value: Value) -> Schema {
    serde_json::from_value(value).unwrap()
}

#[test]
fn settings_are_objects_with_bounded_nested_values_and_redacted_diagnostics() {
    let default: super::super::PluginConfig =
        serde_yaml::from_str("id: sample\nexecutable: /missing\n").unwrap();
    assert_eq!(default.settings.value(), &json!({}));
    let config: super::super::PluginConfig = serde_yaml::from_str(
        "id: sample\nexecutable: /missing\nsettings:\n  nested: [true, 12, null, {text: 'λ'}]\n",
    )
    .unwrap();
    assert_eq!(
        config.settings.value(),
        &json!({"nested":[true,12,null,{"text":"λ"}]})
    );
    assert!(!format!("{:?}", config.settings).contains("nested"));
    for value in [
        json!(null),
        json!([]),
        json!("credential-marker"),
        json!({"credential-marker":"x".repeat(MAX_BYTES)}),
    ] {
        let err = Settings::from_value(value).unwrap_err();
        assert!(!err.message.contains("credential-marker"));
    }
    let mut value = json!(true);
    for _ in 0..10 {
        value = json!({"nested":value});
    }
    assert!(Settings::from_value(value).is_err());
    assert!(Settings::from_value(json!({"values":vec![false;4096]})).is_err());
    let huge = format!(
        "id: sample\nexecutable: /missing\nsettings:\n  secret: '{}'\n",
        "credential-marker".repeat(5000)
    );
    let error = serde_yaml::from_str::<super::super::PluginConfig>(&huge)
        .unwrap_err()
        .to_string();
    assert!(!error.contains("credential-marker"), "{error}");
}

#[test]
fn settings_schema_has_exact_types_no_defaults_and_unicode_scalar_lengths() {
    let spec = schema(json!({"fields":[
        {"name":"mode","type":"string","required":true,"enum":["λ","雪"],"max_length":1},
        {"name":"count","type":"integer","min":-2,"max":4},
        {"name":"enabled","type":"boolean"}
    ]}));
    let settings = Settings::from_value(json!({"mode":"雪"})).unwrap();
    spec.validate(&settings).unwrap();
    assert_eq!(settings.value(), &json!({"mode":"雪"}));
    for value in [
        json!({}),
        json!({"mode":"unknown-secret"}),
        json!({"mode":"雪","count":2.0}),
        json!({"mode":"雪","count":true}),
        json!({"mode":"雪","count":5}),
        json!({"mode":"雪","enabled":"true"}),
        json!({"mode":"雪","credential-marker":false}),
    ] {
        let err = spec
            .validate(&Settings::from_value(value).unwrap())
            .unwrap_err();
        assert!(!err.message.contains("credential-marker"));
        assert!(!err.message.contains("unknown-secret"));
    }
    spec.validate(&Settings::from_value(json!({"mode":"λ","count":-2,"enabled":true})).unwrap())
        .unwrap();
}

#[test]
fn settings_schema_rejects_unsupported_keywords_duplicate_names_and_limits() {
    for field in [
        json!({"name":"x","type":"string","pattern":".*"}),
        json!({"name":"x","type":"boolean","default":true}),
        json!({"name":"x","type":"integer","enum":[1]}),
        json!({"name":"x","type":"object"}),
    ] {
        assert!(serde_json::from_value::<Schema>(json!({"fields":[field]})).is_err());
    }
    for fields in [
        vec![json!({"name":"x","type":"string"}); 65],
        vec![json!({"name":"x","type":"string"}); 2],
        vec![json!({"name":"bad.name","type":"boolean"})],
        vec![json!({"name":"x","type":"integer","min":3,"max":2})],
        vec![json!({"name":"x","type":"string","enum":[]})],
        vec![json!({"name":"x","type":"string","enum":["a","a"]})],
        vec![json!({"name":"x","type":"string","enum":["aa"],"max_length":1})],
        vec![json!({"name":"x","type":"string","max_length":65537})],
    ] {
        assert!(
            schema(json!({"fields":fields}))
                .validate(&Settings::default())
                .is_err()
        );
    }
}

#[test]
fn settings_accept_exact_byte_node_and_depth_boundaries() {
    Settings::from_value(json!({"x":"x".repeat(MAX_BYTES-8)})).unwrap();
    assert!(Settings::from_value(json!({"x":"x".repeat(MAX_BYTES-7)})).is_err());
    Settings::from_value(json!({"x":"\n".repeat((MAX_BYTES-8)/2)})).unwrap();
    assert!(Settings::from_value(json!({"x":"\n".repeat((MAX_BYTES-8)/2+1)})).is_err());
    Settings::from_value(json!({"x":vec![0;4094]})).unwrap();
    assert!(Settings::from_value(json!({"x":vec![0;4095]})).is_err());
    let mut value = json!(false);
    for _ in 0..7 {
        value = json!({"x":value});
    }
    Settings::from_value(value.clone()).unwrap();
    assert!(Settings::from_value(json!({"x":value})).is_err());
}

#[test]
fn settings_optional_null_constraints_are_equivalent_to_omission() {
    let spec = schema(json!({"fields":[
        {"name":"text","type":"string","enum":null,"max_length":null},
        {"name":"count","type":"integer","min":null,"max":null}
    ]}));
    spec.validate(&Settings::from_value(json!({"text":"λ","count":-100})).unwrap())
        .unwrap();
    assert_eq!(
        serde_json::to_value(spec).unwrap(),
        json!({"fields":[{"name":"text","type":"string"},{"name":"count","type":"integer"}]})
    );
    assert!(
        serde_json::from_value::<Schema>(
            json!({"fields":[{"name":"x","type":"boolean","required":null}]})
        )
        .is_err()
    );
}

#[test]
fn settings_clones_and_outbound_reads_share_their_bounded_backing_storage() {
    let settings = Settings::from_value(
        json!({"rows":(0..1024).map(|n| json!({"id":n})).collect::<Vec<_>>()}),
    )
    .unwrap();
    let clone = settings.clone();
    assert!(std::ptr::eq(settings.value(), clone.value()));
    let first = settings.encoded();
    let second = clone.encoded();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(
        &serde_json::from_str::<Value>(first.get()).unwrap(),
        settings.value()
    );
    assert_eq!(
        serde_json::to_value(super::super::application::ResultValue::Settings { settings: second })
            .unwrap(),
        json!({"settings":settings.value()})
    );
}
