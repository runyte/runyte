// SPDX-License-Identifier: MPL-2.0

//! Document indentation settings and bounded, precompiled file patterns.

use std::{collections::BTreeMap, path::Path};

use regex::Regex;
use serde::{Deserialize, Deserializer, de::Error};

use crate::config::IndentStyle;

pub const MAX_OVERRIDES: usize = 128;
pub const MAX_PATTERN_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Indentation {
    pub tab_width: usize,
    pub style: IndentStyle,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Values {
    pub tab_width: Option<usize>,
    pub indent: Option<IndentStyle>,
}

impl Values {
    fn apply(&self, result: &mut Indentation) {
        if let Some(width) = self.tab_width {
            result.tab_width = width;
        }
        if let Some(style) = self.indent {
            result.style = style;
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self
            .tab_width
            .is_some_and(|width| !(1..=16).contains(&width))
        {
            return Err("indentation override tab_width must be between 1 and 16".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct FileRule {
    pub pattern: String,
    pub values: Values,
    matcher: Regex,
}

impl FileRule {
    pub fn matches(&self, relative_path: &str) -> bool {
        self.matcher.is_match(relative_path)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Overrides {
    pub languages: BTreeMap<String, Values>,
    #[serde(deserialize_with = "deserialize_files")]
    pub files: Vec<FileRule>,
}

fn deserialize_files<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<FileRule>, D::Error> {
    let mapping = serde_yaml::Mapping::deserialize(deserializer)?;
    if mapping.len() > MAX_OVERRIDES {
        return Err(D::Error::custom(
            "too many indentation file overrides (maximum 128)",
        ));
    }
    mapping
        .into_iter()
        .map(|(pattern, values)| {
            let pattern = pattern
                .as_str()
                .ok_or_else(|| D::Error::custom("file pattern must be a string"))?
                .to_owned();
            let matcher = compile_pattern(&pattern).map_err(D::Error::custom)?;
            let values = serde_yaml::from_value(values).map_err(D::Error::custom)?;
            Ok(FileRule {
                pattern,
                values,
                matcher,
            })
        })
        .collect()
}

/// Patterns use `/`, `*`, `**` and `?`. No shell expansion or filesystem scan.
/// A basename pattern matches at every depth; a slash anchors at workspace root.
pub fn compile_pattern(pattern: &str) -> Result<Regex, String> {
    if pattern.is_empty()
        || pattern.len() > MAX_PATTERN_BYTES
        || pattern
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | '[' | ']' | '{' | '}'))
        || pattern.starts_with('/')
        || pattern
            .split('/')
            .any(|part| matches!(part, ".." | "." | ""))
    {
        return Err("use a relative file pattern (1–256 bytes), with /, *, ** or ?; no .., brackets or backslashes".into());
    }
    let mut expression = if pattern.contains('/') {
        String::from("^")
    } else {
        String::from("^(?:.*/)?")
    };
    let mut chars = pattern.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    expression.push_str("(?:.*/)?");
                } else {
                    expression.push_str(".*");
                }
            }
            '*' => expression.push_str("[^/]*"),
            '?' => expression.push_str("[^/]"),
            c => expression.push_str(&regex::escape(&c.to_string())),
        }
    }
    expression.push('$');
    Regex::new(&expression).map_err(|error| error.to_string())
}

impl Overrides {
    pub fn validate(&self) -> Result<(), String> {
        if self.languages.len() > MAX_OVERRIDES || self.files.len() > MAX_OVERRIDES {
            return Err("too many indentation overrides (maximum 128 per scope)".into());
        }
        for (language, values) in &self.languages {
            if !crate::syntax::is_builtin_language_name(language) {
                return Err(format!("unknown indentation language {language:?}"));
            }
            values.validate()?;
        }
        for rule in &self.files {
            rule.values.validate()?;
        }
        Ok(())
    }

    pub fn set(&mut self, scope: &Scope, values: Values) -> Result<(), String> {
        values.validate()?;
        match scope {
            Scope::Language(name) => {
                self.languages.insert(name.clone(), values);
            }
            Scope::Files(pattern) => {
                if let Some(rule) = self.files.iter_mut().find(|rule| rule.pattern == *pattern) {
                    rule.values = values;
                } else {
                    self.files.push(FileRule {
                        pattern: pattern.clone(),
                        values,
                        matcher: compile_pattern(pattern)?,
                    });
                }
            }
        }
        self.validate()
    }

    pub fn resolve(
        &self,
        mut base: Indentation,
        language: Option<&str>,
        relative_path: Option<&Path>,
    ) -> Indentation {
        if let Some(values) = language.and_then(|name| self.languages.get(name)) {
            values.apply(&mut base);
        }
        if !self.files.is_empty()
            && let Some(path) = relative_path
        {
            // Components keep platform separators out of the pattern contract.
            let path = path
                .components()
                .filter_map(|part| match part {
                    std::path::Component::Normal(value) => Some(value.to_string_lossy()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("/");
            for rule in &self.files {
                if rule.matcher.is_match(&path) {
                    rule.values.apply(&mut base);
                }
            }
        }
        base
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Scope {
    Language(String),
    Files(String),
}

impl Scope {
    pub fn label(&self) -> String {
        match self {
            Self::Language(name) => format!("language: {name}"),
            Self::Files(pattern) => format!("files: {pattern}"),
        }
    }

    pub fn path(&self, field: &str) -> Vec<String> {
        let (kind, name) = match self {
            Self::Language(name) => ("languages", name),
            Self::Files(pattern) => ("files", pattern),
        };
        vec![
            "indentation".into(),
            kind.into(),
            name.clone(),
            field.into(),
        ]
    }

    pub fn values<'a>(&self, overrides: &'a Overrides) -> Option<&'a Values> {
        match self {
            Self::Language(name) => overrides.languages.get(name),
            Self::Files(pattern) => overrides
                .files
                .iter()
                .find(|rule| rule.pattern == *pattern)
                .map(|rule| &rule.values),
        }
    }
}

/// Character offset of the preceding visual stop in an indentation prefix.
/// Stop at the first non-indentation character: ordinary Backspace on a long
/// code line must not copy or scan its entire prefix.
pub(crate) fn backspace_start(prefix: impl Iterator<Item = char>, width: usize) -> Option<usize> {
    let mut column = 0;
    let mut start = None;
    for (index, character) in prefix.enumerate() {
        if !matches!(character, ' ' | '\t') {
            return None;
        }
        if column % width == 0 {
            start = Some(index);
        }
        column += if character == '\t' {
            width - column % width
        } else {
            1
        };
    }
    start
}

#[cfg(test)]
#[path = "indentation/tests.rs"]
mod tests;
