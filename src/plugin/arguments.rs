// SPDX-License-Identifier: MPL-2.0

use super::application::{Error, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
pub struct Argument {
    pub name: String,
    #[serde(flatten)]
    pub kind: Kind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Kind {
    String,
    Boolean,
    Integer,
    Enum { choices: Vec<String> },
}
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum Scalar {
    String(String),
    Boolean(bool),
    Integer(i64),
}

pub fn validate(schema: &[Argument]) -> Result<(), Error> {
    let mut names = std::collections::BTreeSet::new();
    if schema.len() > 16 {
        return Err(Error::new(
            ErrorCode::LimitExceeded,
            "Too many command arguments",
        ));
    }
    for arg in schema {
        if !super::valid_name(&arg.name) || !names.insert(&arg.name) {
            return Err(Error::new(
                ErrorCode::InvalidArgument,
                "Invalid argument name",
            ));
        }
        if let Kind::Enum { choices } = &arg.kind
            && (choices.is_empty()
                || choices.len() > 32
                || choices.iter().any(|choice| {
                    choice.is_empty() || choice.len() > 160 || choice.chars().any(char::is_control)
                }))
        {
            return Err(Error::new(
                ErrorCode::InvalidArgument,
                "Invalid argument choices",
            ));
        }
    }
    Ok(())
}

pub fn parse(schema: &[Argument], text: &str) -> Result<BTreeMap<String, Scalar>, Error> {
    if text.len() > 8192 {
        return Err(Error::new(
            ErrorCode::LimitExceeded,
            "Command arguments exceed limit",
        ));
    }
    let values = shlex::split(text)
        .ok_or_else(|| Error::new(ErrorCode::InvalidArgument, "Unbalanced argument quote"))?;
    if values.len() != schema.len() {
        return Err(Error::new(
            ErrorCode::InvalidArgument,
            "Incorrect argument count",
        ));
    }
    schema
        .iter()
        .zip(values)
        .map(|(arg, value)| {
            let invalid = || {
                Error::new(
                    ErrorCode::InvalidArgument,
                    "Argument does not match its declared type",
                )
            };
            let value = match &arg.kind {
                Kind::String => Scalar::String(value),
                Kind::Enum { choices } if choices.contains(&value) => Scalar::String(value),
                Kind::Enum { .. } => return Err(invalid()),
                Kind::Boolean => Scalar::Boolean(value.parse().map_err(|_| invalid())?),
                Kind::Integer => Scalar::Integer(value.parse().map_err(|_| invalid())?),
            };
            Ok((arg.name.clone(), value))
        })
        .collect()
}

impl<'de> Deserialize<'de> for Argument {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            name: String,
            #[serde(rename = "type")]
            kind: String,
            choices: Option<Vec<String>>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let kind = match (raw.kind.as_str(), raw.choices) {
            ("string", None) => Kind::String,
            ("boolean", None) => Kind::Boolean,
            ("integer", None) => Kind::Integer,
            ("enum", Some(choices)) => Kind::Enum { choices },
            _ => return Err(serde::de::Error::custom("invalid argument type or choices")),
        };
        Ok(Self {
            name: raw.name,
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_arguments_preserve_quoted_unicode_without_shell_evaluation() {
        let schema: Vec<Argument> = serde_json::from_str(r#"[{"name":"text","type":"string"},{"name":"enabled","type":"boolean"},{"name":"count","type":"integer"},{"name":"sort","type":"enum","choices":["name","date"]}]"#).unwrap();
        validate(&schema).unwrap();
        let values = parse(&schema, "'é $(touch ignored)' true -7 name").unwrap();
        assert_eq!(
            serde_json::to_value(values).unwrap(),
            serde_json::json!({"text":"é $(touch ignored)","enabled":true,"count":-7,"sort":"name"})
        );
        assert!(parse(&schema, "x true 7 missing").is_err());
        assert!(parse(&schema, "x yes 7 name").is_err());
        assert!(parse(&schema, "'unterminated").is_err());
    }
}
