// SPDX-License-Identifier: MPL-2.0

//! Immutable configured settings and a deliberately small registration schema.
use super::{
    application::{Error, ErrorCode},
    json,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};
use std::{collections::BTreeSet, sync::Arc};

pub const MAX_BYTES: usize = 64 * 1024;
pub const LIMITS: json::Limits = json::Limits {
    max_bytes: MAX_BYTES,
    max_depth: 8,
    max_nodes: 4096,
    max_container: 4096,
};

#[derive(Clone)]
pub struct Settings(Arc<SettingsInner>);
struct SettingsInner {
    value: Value,
    encoded: Arc<RawValue>,
}
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Settings(<private>)")
    }
}
impl Default for Settings {
    fn default() -> Self {
        Self::from_value(Value::Object(Default::default())).expect("empty settings are valid")
    }
}
impl Settings {
    pub fn value(&self) -> &Value {
        &self.0.value
    }
    /// Outbound reads retain canonical JSON, never a second parsed node tree.
    pub fn encoded(&self) -> Arc<RawValue> {
        self.0.encoded.clone()
    }
    pub fn from_value(value: Value) -> Result<Self, Error> {
        json::validate_value(&value, LIMITS)?;
        if !value.is_object() {
            return Err(invalid("Settings must be an object"));
        }
        let encoded = serde_json::value::to_raw_value(&value)
            .map_err(|_| invalid("Invalid plugin settings"))?;
        Ok(Self(Arc::new(SettingsInner {
            value,
            encoded: Arc::from(encoded),
        })))
    }
}
impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = json::deserialize(deserializer, LIMITS)
            .map_err(|_| serde::de::Error::custom("Invalid plugin settings"))?;
        Self::from_value(value)
            .map_err(|_| serde::de::Error::custom("Invalid or oversized plugin settings"))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    pub fields: Vec<Field>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Field {
    String {
        name: String,
        #[serde(default, skip_serializing_if = "is_false")]
        required: bool,
        #[serde(default, rename = "enum", skip_serializing_if = "Option::is_none")]
        choices: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<usize>,
    },
    Integer {
        name: String,
        #[serde(default, skip_serializing_if = "is_false")]
        required: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    Boolean {
        name: String,
        #[serde(default, skip_serializing_if = "is_false")]
        required: bool,
    },
}
fn is_false(value: &bool) -> bool {
    !*value
}
fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidArgument, message)
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
impl Field {
    fn name(&self) -> &str {
        match self {
            Self::String { name, .. } | Self::Integer { name, .. } | Self::Boolean { name, .. } => {
                name
            }
        }
    }
    fn required(&self) -> bool {
        match self {
            Self::String { required, .. }
            | Self::Integer { required, .. }
            | Self::Boolean { required, .. } => *required,
        }
    }
    fn accepts(&self, value: &Value) -> bool {
        match self {
            Self::String {
                choices,
                max_length,
                ..
            } => value.as_str().is_some_and(|text| {
                max_length.is_none_or(|max| text.chars().count() <= max)
                    && choices
                        .as_ref()
                        .is_none_or(|choices| choices.iter().any(|choice| choice == text))
            }),
            Self::Integer { min, max, .. } => value
                .as_i64()
                .is_some_and(|n| min.is_none_or(|min| n >= min) && max.is_none_or(|max| n <= max)),
            Self::Boolean { .. } => value.is_boolean(),
        }
    }
}
impl Schema {
    /// All diagnostics are static. Configured values and unknown keys may be secrets.
    pub fn validate(&self, settings: &Settings) -> Result<(), Error> {
        if self.fields.len() > 64 {
            return Err(invalid("Too many settings schema fields"));
        }
        json::validate_value(
            &serde_json::to_value(self).map_err(|_| invalid("Invalid settings schema"))?,
            LIMITS,
        )?;
        let mut names = BTreeSet::new();
        for field in &self.fields {
            if !valid_name(field.name()) || !names.insert(field.name()) {
                return Err(invalid("Invalid or duplicate settings schema field"));
            }
            match field {
                Field::String {
                    choices,
                    max_length,
                    ..
                } => {
                    if max_length.is_some_and(|max| max > MAX_BYTES) {
                        return Err(invalid("Invalid settings string length limit"));
                    }
                    if let Some(choices) = choices
                        && (choices.is_empty()
                            || choices.len() > 64
                            || choices.iter().any(|choice| {
                                choice.len() > 4096
                                    || max_length.is_some_and(|max| choice.chars().count() > max)
                            })
                            || choices.iter().collect::<BTreeSet<_>>().len() != choices.len())
                    {
                        return Err(invalid("Invalid settings string enum"));
                    }
                }
                Field::Integer { min, max, .. }
                    if min.zip(*max).is_some_and(|(min, max)| min > max) =>
                {
                    return Err(invalid("Invalid settings integer range"));
                }
                _ => {}
            }
        }
        let values = settings
            .value()
            .as_object()
            .ok_or_else(|| invalid("Settings must be an object"))?;
        if values.keys().any(|key| !names.contains(key.as_str())) {
            return Err(invalid("Unknown configured settings field"));
        }
        for field in &self.fields {
            match values.get(field.name()) {
                Some(value) if !field.accepts(value) => {
                    return Err(invalid("Configured setting does not match its schema"));
                }
                None if field.required() => {
                    return Err(invalid("Required configured setting is missing"));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/settings.rs"]
mod tests;
