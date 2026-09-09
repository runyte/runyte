// SPDX-License-Identifier: MPL-2.0

//! Bounded declarative input. Secret values never enter editor text or snapshots.
use super::application::{Error, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SURFACE_CHARGE: usize = 512 * 1024;
pub const MAX_FIELDS: usize = 16;
pub const MAX_VALUE_BYTES: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Value {
    Text(String),
    Boolean(bool),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Text,
    Secret,
    Boolean,
    Choice,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub kind: Kind,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub validate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_message: Option<String>,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub minimum_length: usize,
    #[serde(default = "maximum_length")]
    pub maximum_length: usize,
}
fn is_false(value: &bool) -> bool {
    !value
}
fn maximum_length() -> usize {
    MAX_VALUE_BYTES
}
impl Field {
    pub fn text(id: &str, label: String) -> Self {
        Self {
            id: id.into(),
            label,
            kind: Kind::Text,
            required: true,
            validate: false,
            validation_message: None,
            choices: vec![],
            minimum_length: 0,
            maximum_length: MAX_VALUE_BYTES,
        }
    }
    pub fn initial(&self) -> Value {
        match self.kind {
            Kind::Boolean => Value::Boolean(false),
            Kind::Choice => Value::Text(self.choices.first().cloned().unwrap_or_default()),
            _ => Value::Text(String::new()),
        }
    }
    pub fn accepts(&self, value: &Value) -> bool {
        match (self.kind, value) {
            (Kind::Boolean, Value::Boolean(_)) => true,
            (Kind::Choice, Value::Text(value)) => self.choices.contains(value),
            (Kind::Text | Kind::Secret, Value::Text(value)) => {
                let n = value.chars().count();
                value.len() <= MAX_VALUE_BYTES
                    && !value.chars().any(char::is_control)
                    && n >= self.minimum_length
                    && n <= self.maximum_length
                    && (!self.required || n > 0)
            }
            _ => false,
        }
    }
}
pub fn validate(title: &str, fields: &[Field]) -> Result<(), Error> {
    let safe =
        |s: &str, limit| !s.is_empty() && s.len() <= limit && !s.chars().any(char::is_control);
    let mut ids = BTreeSet::new();
    if !safe(title, 160)
        || fields.is_empty()
        || fields.len() > MAX_FIELDS
        || fields.iter().any(|f| {
            !super::valid_name(&f.id)
                || !ids.insert(&f.id)
                || !safe(&f.label, 160)
                || f.validation_message
                    .as_ref()
                    .is_some_and(|message| !safe(message, 160))
                || f.minimum_length > f.maximum_length
                || f.maximum_length > MAX_VALUE_BYTES
                || (f.kind == Kind::Choice
                    && (f.choices.is_empty()
                        || f.choices.len() > 64
                        || f.choices.iter().any(|s| !safe(s, 160))
                        || f.choices.iter().collect::<BTreeSet<_>>().len() != f.choices.len()))
                || (f.kind != Kind::Choice && !f.choices.is_empty())
        })
    {
        return Err(Error::new(
            ErrorCode::InvalidArgument,
            "Invalid input fields or bounds",
        ));
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize)]
pub struct Submission {
    #[serde(skip)]
    pub(crate) sensitive: bool,
    pub surface: String,
    pub accepted: bool,
    pub values: BTreeMap<String, Value>,
}

/// Values cross only the owning application boundary after physical submit intent.
/// This DTO is never part of editor snapshots, history, or diagnostic feedback.
#[derive(Clone, Debug, Serialize)]
pub struct ValidationRequest {
    pub surface: String,
    pub revision: String,
    pub fields: Vec<String>,
    pub values: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Valid,
    Invalid,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FieldValidation {
    pub field: String,
    pub status: ValidationStatus,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename = "validation")]
pub struct ValidationResult {
    pub surface: String,
    pub revision: String,
    pub fields: Vec<FieldValidation>,
}

impl<'de> Deserialize<'de> for ValidationResult {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Struct tags serialize correctly, but strict struct deserialization
        // treats the tag as an unknown field. A tagged enum consumes it first.
        #[derive(Deserialize)]
        #[serde(tag = "kind", deny_unknown_fields)]
        enum Tagged {
            #[serde(rename = "validation")]
            Validation {
                surface: String,
                revision: String,
                fields: Vec<FieldValidation>,
            },
        }
        let Tagged::Validation {
            surface,
            revision,
            fields,
        } = Tagged::deserialize(deserializer)?;
        Ok(Self {
            surface,
            revision,
            fields,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ValidationCancelled {
    pub request: String,
    pub surface: String,
    pub revision: String,
}

/// A validation callback has no presentation authority and retains no values.
#[derive(Clone)]
pub(crate) struct PendingValidation {
    pub request: String,
    pub surface: String,
    pub revision: String,
    pub fields: Vec<String>,
    pub foreground: u64,
    pub cancelled: bool,
}

#[cfg(test)]
#[path = "tests/interaction_validation.rs"]
mod validation_tests;
