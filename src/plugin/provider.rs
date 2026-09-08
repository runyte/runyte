// SPDX-License-Identifier: MPL-2.0

//! Transport-neutral, version-bound UTF-8 resource reads and writes.
use super::application::{Error, ErrorCode};
use serde::{Deserialize, Serialize};

pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
// Worst-case JSON escaping still fits the independent 1 MiB encoded frame.
pub const CHUNK_BYTES: usize = 128 * 1024;
pub(crate) const READ_CHARGE: usize = MAX_DOCUMENT_BYTES * 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    Conditional,
    ConfirmedBestEffort,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub name: String,
    pub conditional_write: bool,
    pub atomic_replace: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub key: String,
    pub label: String,
    pub syntax_hint: Option<String>,
    pub version: String,
    pub encoding: String,
    pub bytes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub version: String,
    pub offset: usize,
    pub text: String,
    pub eof: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Response {
    Reconciled {
        metadata: Metadata,
        previous_write: String,
    },
    Stat(Metadata),
    Read(Chunk),
    WriteStarted {
        upload: String,
    },
    WriteChunk {
        offset: usize,
    },
    WriteCommitted {
        version: String,
    },
    WriteRejected {
        error: Error,
    },
    WriteAborted {},
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "method", content = "params")]
pub enum Request {
    #[serde(rename = "resource.reconcile")]
    Reconcile {
        job: String,
        provider: String,
        key: String,
        previous_write: String,
    },
    #[serde(rename = "resource.write.begin")]
    WriteBegin {
        job: String,
        provider: String,
        key: String,
        expected_version: String,
        mode: WriteMode,
        bytes: usize,
        encoding: &'static str,
    },
    #[serde(rename = "resource.write.chunk")]
    WriteChunk {
        job: String,
        upload: String,
        offset: usize,
        text: String,
    },
    #[serde(rename = "resource.write.commit")]
    WriteCommit {
        job: String,
        upload: String,
        expected_version: String,
        mode: WriteMode,
    },
    #[serde(rename = "resource.write.abort")]
    WriteAbort { job: String, upload: Option<String> },
    #[serde(rename = "resource.stat")]
    Stat {
        job: String,
        provider: String,
        key: String,
    },
    #[serde(rename = "resource.read")]
    Read {
        job: String,
        provider: String,
        key: String,
        version: String,
        offset: usize,
        limit: usize,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Finished {
    pub job: String,
    pub buffer: Option<String>,
    pub revision: Option<String>,
    pub error: Option<Error>,
}

pub(crate) fn safe_text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}
pub(crate) fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
}
impl Metadata {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if !safe_text(&self.key, 4096)
            || !safe_text(&self.label, 160)
            || !safe_text(&self.version, 256)
            || self
                .syntax_hint
                .as_ref()
                .is_some_and(|hint| !valid_name(hint))
        {
            return Err(Error::new(
                ErrorCode::InvalidArgument,
                "Invalid resource metadata",
            ));
        }
        if self.encoding != "utf-8" {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Resource encoding must be UTF-8",
            ));
        }
        if self.bytes > MAX_DOCUMENT_BYTES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "Resource exceeds document limit",
            ));
        }
        Ok(())
    }
}
