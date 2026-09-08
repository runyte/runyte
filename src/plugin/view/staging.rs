// SPDX-License-Identifier: MPL-2.0
use super::{Error, ErrorCode, MAX_CHUNK_BYTES, MAX_MODEL_BYTES, invalid, limited};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    Model,
    Patch,
}

pub(crate) struct Stage {
    pub view: String,
    pub expected_revision: String,
    pub kind: StageKind,
    pub declared: usize,
    text: String,
}
impl Stage {
    pub fn new(
        view: String,
        expected_revision: String,
        kind: StageKind,
        declared: usize,
    ) -> Result<Self, Error> {
        if declared == 0 || declared > MAX_MODEL_BYTES {
            return Err(limited("View stage byte limit exceeded"));
        }
        Ok(Self {
            view,
            expected_revision,
            kind,
            declared,
            text: String::with_capacity(declared),
        })
    }
    pub fn append(&mut self, offset: usize, text: &str) -> Result<usize, Error> {
        if offset != self.text.len() {
            return Err(Error::new(ErrorCode::Stale, "View stage offset changed"));
        }
        if text.is_empty()
            || text.len() > MAX_CHUNK_BYTES
            || self.text.len().saturating_add(text.len()) > self.declared
        {
            return Err(limited("View stage chunk exceeds its byte limit"));
        }
        self.text.push_str(text);
        Ok(self.text.len())
    }
    pub fn is_complete(&self) -> bool {
        self.text.len() == self.declared
    }
    pub fn into_text(self) -> Result<String, Error> {
        if self.text.len() != self.declared {
            return Err(invalid("View stage is incomplete"));
        }
        Ok(self.text)
    }
}

pub(crate) struct ReadSnapshot {
    pub revision: String,
    pub encoded: Arc<str>,
}
impl ReadSnapshot {
    pub fn chunk(&self, offset: usize, limit: usize) -> Result<(String, Option<usize>), Error> {
        if limit == 0
            || limit > MAX_CHUNK_BYTES
            || offset > self.encoded.len()
            || !self.encoded.is_char_boundary(offset)
        {
            return Err(invalid("Invalid view snapshot range"));
        }
        let mut end = offset.saturating_add(limit).min(self.encoded.len());
        while !self.encoded.is_char_boundary(end) {
            end -= 1;
        }
        if end == offset && offset != self.encoded.len() {
            return Err(invalid(
                "View snapshot chunk cannot contain the next scalar",
            ));
        }
        Ok((
            self.encoded[offset..end].to_owned(),
            (end < self.encoded.len()).then_some(end),
        ))
    }
}
