// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::io::Cursor;

fn limits() -> Limits {
    Limits {
        max_bytes: 4096,
        max_depth: 8,
        max_nodes: 64,
        max_container: 8,
    }
}

#[test]
fn array_limit_rejects_before_reading_an_extra_nested_or_malformed_tail() {
    let input = format!("[0,0,[{}BROKEN", "0,".repeat(100_000));
    let mut reader = Cursor::new(input.as_bytes());
    let error = {
        let mut decoder = serde_json::Deserializer::from_reader(&mut reader);
        deserialize(
            &mut decoder,
            Limits {
                max_container: 2,
                ..limits()
            },
        )
        .unwrap_err()
    };
    assert!(
        error.to_string().contains("JSON container limit exceeded"),
        "{error}"
    );
    assert!(
        reader.position() <= 7,
        "read {} bytes before refusing",
        reader.position()
    );
}

#[test]
fn object_limit_rejects_before_reading_an_extra_value() {
    let prefix = r#"{"a":0,"b":0,"c":"#;
    let input = format!("{prefix}[{}BROKEN", "0,".repeat(100_000));
    let mut reader = Cursor::new(input.as_bytes());
    let error = {
        let mut decoder = serde_json::Deserializer::from_reader(&mut reader);
        deserialize(
            &mut decoder,
            Limits {
                max_container: 2,
                ..limits()
            },
        )
        .unwrap_err()
    };
    assert!(
        error.to_string().contains("JSON container limit exceeded"),
        "{error}"
    );
    assert!(
        reader.position() <= prefix.len() as u64,
        "read {} bytes before refusing",
        reader.position()
    );
}

#[test]
fn canonical_byte_budget_counts_escapes_unicode_numbers_and_delimiters() {
    let value = serde_json::json!({
        "text": "\"\\\n\r\t\u{8}\u{c}\0λ雪",
        "values": [null, true, false, -42, 1.25]
    });
    let canonical = serde_json::to_string(&value).unwrap();
    let exact = Limits {
        max_bytes: canonical.len(),
        ..limits()
    };
    validate(&canonical, exact).unwrap();
    validate_value(&value, exact).unwrap();
    let padded = format!(" \n {canonical} \t ");
    validate(&padded, exact).unwrap();
    let mut decoder = serde_json::Deserializer::from_str(&padded);
    assert_eq!(deserialize(&mut decoder, exact).unwrap(), value);
    let short = Limits {
        max_bytes: canonical.len() - 1,
        ..limits()
    };
    assert!(validate(&canonical, short).is_err());
    assert!(validate_value(&value, short).is_err());
    assert!(validate(&format!("{canonical} false"), exact).is_err());
}

#[test]
fn depth_and_node_limits_apply_equally_to_validation_and_materialization() {
    for (input, bound) in [
        (
            "[[0]]",
            Limits {
                max_depth: 3,
                ..limits()
            },
        ),
        (
            r#"{"a":[0,1],"b":true}"#,
            Limits {
                max_nodes: 5,
                ..limits()
            },
        ),
    ] {
        validate(input, bound).unwrap();
        let value: Value = serde_json::from_str(input).unwrap();
        validate_value(&value, bound).unwrap();
        let tighter = if input.starts_with('[') {
            Limits {
                max_depth: bound.max_depth - 1,
                ..bound
            }
        } else {
            Limits {
                max_nodes: bound.max_nodes - 1,
                ..bound
            }
        };
        assert!(validate(input, tighter).is_err());
        assert!(validate_value(&value, tighter).is_err());
        let mut decoder = serde_json::Deserializer::from_str(input);
        assert!(deserialize(&mut decoder, tighter).is_err());
    }
}

#[test]
fn settings_yaml_uses_the_same_depth_bound_and_redacts_rejected_values() {
    let mut value = "credential-canary".to_owned();
    // The settings root and seven arrays place the scalar at depth nine.
    for _ in 0..7 {
        value = format!("[{value}]");
    }
    let yaml = format!("id: bounded\nexecutable: /unused\nsettings:\n  value: {value}\n");
    let error = serde_yaml::from_str::<crate::plugin::PluginConfig>(&yaml)
        .unwrap_err()
        .to_string();
    assert!(!error.contains("credential-canary"), "{error}");
    let yaml = "id: bounded\nexecutable: /unused\nsettings:\n  value: [[[[[[ok]]]]]]\n";
    let config = serde_yaml::from_str::<crate::plugin::PluginConfig>(yaml).unwrap();
    assert!(config.settings.value().is_object());
}
