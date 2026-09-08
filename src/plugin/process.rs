// SPDX-License-Identifier: MPL-2.0

//! Bounded, nonterminal helper processes owned by an application generation.
use super::application::{Error, ErrorCode};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

mod ring;
pub(crate) mod runtime;
pub(crate) use ring::Ring;

pub const MAX_HANDLES: usize = 4;
pub const PROCESS_CHARGE: usize = 2 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_IO_BYTES: usize = 64 * 1024;
pub const MAX_ARGUMENTS: usize = 64;
pub const MAX_ARGUMENT_LENGTH: usize = 4096;
pub const MAX_ARGUMENT_BYTES: usize = 64 * 1024;

pub(super) fn arguments<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    struct Arguments;
    impl<'de> serde::de::Visitor<'de> for Arguments {
        type Value = Vec<String>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("at most 64 process arguments")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            loop {
                if values.len() == MAX_ARGUMENTS {
                    if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                        return Err(serde::de::Error::custom(
                            "Process argument count exceeds its limit",
                        ));
                    }
                    return Ok(values);
                }
                match sequence.next_element::<String>()? {
                    Some(value) => values.push(value),
                    None => return Ok(values),
                }
            }
        }
    }
    deserializer.deserialize_seq(Arguments)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub label: String,
    pub executable: String,
    #[serde(default, deserialize_with = "arguments")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub capture_stderr: bool,
}
impl Start {
    /// Validates wire bounds only. The worker resolves and checks cwd inside
    /// the workspace before spawning; executable/arguments never form a shell.
    pub fn validate(&self) -> Result<(), Error> {
        validate_launch(
            &self.label,
            &self.executable,
            &self.args,
            self.cwd.as_deref(),
        )
    }
}

pub(super) fn validate_launch(
    label: &str,
    executable: &str,
    args: &[String],
    cwd: Option<&str>,
) -> Result<(), Error> {
    if label.is_empty() || label.len() > 160 || label.chars().any(char::is_control) {
        return Err(invalid("Invalid process label"));
    }
    if executable.is_empty() || executable.len() > 4096 || executable.contains('\0') {
        return Err(invalid("Invalid process executable"));
    }
    if args.len() > MAX_ARGUMENTS
        || args.iter().any(|arg| arg.len() > MAX_ARGUMENT_LENGTH)
        || args
            .iter()
            .try_fold(0usize, |used, arg| used.checked_add(arg.len()))
            .is_none_or(|bytes| bytes > MAX_ARGUMENT_BYTES)
    {
        return Err(limited("Process arguments exceed their limit"));
    }
    if args.iter().any(|arg| arg.contains('\0')) {
        return Err(invalid("Process arguments contain a null byte"));
    }
    if cwd.is_some_and(|path| path.is_empty() || path.len() > 4096 || path.contains('\0')) {
        return Err(invalid("Invalid process working directory"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Running,
    Closing,
    Exited,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Info {
    pub process: String,
    pub label: String,
    pub state: State,
    pub stdout: Bounds,
    pub stderr: Bounds,
    pub capture_stderr: bool,
    pub stdin_closed: bool,
    pub output_truncated: bool,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Read {
    pub process: String,
    pub stream: Stream,
    pub offset: u64,
    pub data: String,
    pub next: u64,
    pub eof: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Write {
    pub process: String,
    pub written: usize,
    pub stdin_closed: bool,
}

pub fn encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn decode(data: &str) -> Result<Vec<u8>, Error> {
    if data.len() > MAX_IO_BYTES.div_ceil(3) * 4 {
        return Err(limited("Process write exceeds its byte limit"));
    }
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| invalid("Invalid process base64 data"))?;
    if bytes.len() > MAX_IO_BYTES {
        return Err(limited("Process write exceeds its byte limit"));
    }
    Ok(bytes)
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidArgument, message)
}
fn limited(message: &str) -> Error {
    Error::new(ErrorCode::LimitExceeded, message)
}

#[cfg(test)]
#[path = "tests/process.rs"]
mod tests;
