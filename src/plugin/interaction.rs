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
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub minimum_length: usize,
    #[serde(default = "maximum_length")]
    pub maximum_length: usize,
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
    pub surface: String,
    pub accepted: bool,
    pub values: BTreeMap<String, Value>,
}
