// SPDX-License-Identifier: MPL-2.0

//! Bounded JSON traversal before retaining a tree, also used by YAML settings.
use super::application::{Error, ErrorCode};
use serde::{
    Deserializer,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};

#[derive(Clone, Copy)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_container: usize,
}
struct Budget {
    limits: Limits,
    nodes: usize,
    bytes: usize,
    build: bool,
}
impl Budget {
    fn add<E: de::Error>(&mut self, bytes: usize) -> Result<(), E> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > self.limits.max_bytes {
            return Err(E::custom("JSON byte limit exceeded"));
        }
        Ok(())
    }
    fn string<E: de::Error>(&mut self, value: &str) -> Result<(), E> {
        let bytes = value
            .chars()
            .map(|c| match c {
                '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
                c if (c as u32) < 32 => 6,
                c => c.len_utf8(),
            })
            .sum::<usize>();
        self.add(bytes.saturating_add(2))
    }
}
struct Reject;
impl<'de> DeserializeSeed<'de> for Reject {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, _: D) -> Result<(), D::Error> {
        Err(de::Error::custom("JSON container limit exceeded"))
    }
}
struct Seed<'a> {
    budget: &'a mut Budget,
    depth: usize,
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Option<Value>;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        self.budget.nodes += 1;
        if self.depth > self.budget.limits.max_depth
            || self.budget.nodes > self.budget.limits.max_nodes
        {
            return Err(de::Error::custom("JSON structure limit exceeded"));
        }
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Option<Value>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("bounded JSON")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        self.budget.add(4)?;
        Ok(self.budget.build.then_some(Value::Null))
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        self.visit_unit()
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
        self.budget.add(if v { 4 } else { 5 })?;
        Ok(self.budget.build.then_some(Value::Bool(v)))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        self.number(v.into())
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        self.number(v.into())
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
        self.number(Number::from_f64(v).ok_or_else(|| E::custom("Nonfinite JSON number"))?)
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        self.budget.string(v)?;
        Ok(self.budget.build.then(|| Value::String(v.into())))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
        self.budget.string(&v)?;
        Ok(self.budget.build.then_some(Value::String(v)))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        self.budget.add(2)?;
        let mut values = self.budget.build.then(Vec::new);
        let mut count = 0;
        loop {
            if count == self.budget.limits.max_container {
                if seq.next_element_seed(Reject)?.is_some() {
                    return Err(de::Error::custom("JSON container limit exceeded"));
                }
                break;
            }
            let value = seq.next_element_seed(Seed {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
            let Some(value) = value else { break };
            if count > 0 {
                self.budget.add(1)?;
            }
            count += 1;
            if let Some(values) = &mut values {
                values.push(value.unwrap());
            }
        }
        Ok(values.map(Value::Array))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        self.budget.add(2)?;
        let mut values = self.budget.build.then(Map::new);
        let mut count = 0;
        while let Some(key) = map.next_key::<String>()? {
            if count == self.budget.limits.max_container {
                return Err(de::Error::custom("JSON container limit exceeded"));
            }
            self.budget.string(&key)?;
            self.budget.add(if count == 0 { 1 } else { 2 })?;
            count += 1;
            let value = map.next_value_seed(Seed {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
            if let Some(values) = &mut values {
                values.insert(key, value.unwrap());
            }
        }
        Ok(values.map(Value::Object))
    }
}
impl Seed<'_> {
    fn number<E: de::Error>(self, v: Number) -> Result<Option<Value>, E> {
        self.budget.add(v.to_string().len())?;
        Ok(self.budget.build.then_some(Value::Number(v)))
    }
}

pub fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
    limits: Limits,
) -> Result<Value, D::Error> {
    let mut budget = Budget {
        limits,
        nodes: 0,
        bytes: 0,
        build: true,
    };
    Ok(Seed {
        budget: &mut budget,
        depth: 1,
    }
    .deserialize(deserializer)?
    .unwrap())
}
pub fn validate(raw: &str, limits: Limits) -> Result<(), Error> {
    validate_integer_literals(raw)?;
    let mut budget = Budget {
        limits,
        nodes: 0,
        bytes: 0,
        build: false,
    };
    let mut decoder = serde_json::Deserializer::from_str(raw);
    Seed {
        budget: &mut budget,
        depth: 1,
    }
    .deserialize(&mut decoder)
    .map_err(json_error)?;
    decoder.end().map_err(json_error)
}
pub fn validate_value(value: &Value, limits: Limits) -> Result<(), Error> {
    // Value itself is a borrowing deserializer, so this traverses without cloning.
    let mut budget = Budget {
        limits,
        nodes: 0,
        bytes: 0,
        build: false,
    };
    Seed {
        budget: &mut budget,
        depth: 1,
    }
    .deserialize(value)
    .map(|_| ())
    .map_err(|_| invalid())
}
fn invalid() -> Error {
    Error::new(
        ErrorCode::LimitExceeded,
        "JSON exceeds its byte or structure limits",
    )
}

#[cfg(test)]
mod tests;

fn json_error(error: serde_json::Error) -> Error {
    match error.classify() {
        serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
            Error::new(ErrorCode::InvalidArgument, "Invalid JSON")
        }
        _ => invalid(),
    }
}

/// JSON's f64 fallback must not silently round an integer-shaped identifier.
/// This applies to raw JSON input, not to numeric tokens in another format.
fn validate_integer_literals(raw: &str) -> Result<(), Error> {
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            index += 1;
            while index < bytes.len() {
                match bytes[index] {
                    b'\\' => index = (index + 2).min(bytes.len()),
                    b'"' => {
                        index += 1;
                        break;
                    }
                    _ => index += 1,
                }
            }
        } else if bytes[index] == b'-' || bytes[index].is_ascii_digit() {
            let start = index;
            index += 1;
            while index < bytes.len()
                && matches!(bytes[index], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
            {
                index += 1;
            }
            let token = &raw[start..index];
            if !token.bytes().any(|b| matches!(b, b'.' | b'e' | b'E')) {
                let valid = if token.starts_with('-') {
                    token.parse::<i64>().is_ok()
                } else {
                    token.parse::<u64>().is_ok()
                };
                if !valid {
                    return Err(Error::new(
                        ErrorCode::InvalidArgument,
                        "JSON integer is outside the supported 64-bit range",
                    ));
                }
            }
        } else {
            index += 1;
        }
    }
    Ok(())
}
