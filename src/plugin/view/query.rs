// SPDX-License-Identifier: MPL-2.0
use super::{Error, ErrorCode};
use serde::{Deserialize, Serialize};

pub const MAX_QUERY_BYTES: usize = 1024;
pub const QUERY_CHARGE: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub revision: String,
    pub text: String,
    pub pending: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct QueryState {
    pub revision: u64,
    pub text: String,
    pub pending: bool,
}

pub(crate) fn check_query(query: Option<&QueryState>, expected: Option<&str>) -> Result<(), Error> {
    let matches = match (query, expected) {
        (None, None) => true,
        (Some(query), Some(expected)) => expected == format!("qv:{}", query.revision),
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(Error::new(ErrorCode::Stale, "View query changed"))
    }
}

impl QueryState {
    pub fn set(
        previous: Option<&Self>,
        expected: Option<&str>,
        text: String,
    ) -> Result<Self, Error> {
        check_query(previous, expected)?;
        if text.len() > MAX_QUERY_BYTES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "View query exceeds limit",
            ));
        }
        if text.chars().any(char::is_control) {
            return Err(Error::new(
                ErrorCode::InvalidArgument,
                "View query must be a single line",
            ));
        }
        let revision = previous
            .map_or(0, |query| query.revision)
            .checked_add(1)
            .ok_or_else(|| Error::new(ErrorCode::Internal, "View query revision exhausted"))?;
        // Query metadata has a fixed reservation; do not retain excess capacity
        // inherited from a caller's decoded or reused input allocation.
        let text = text.into_boxed_str().into_string();
        Ok(Self {
            revision,
            text,
            pending: true,
        })
    }
    pub fn wire(&self) -> Query {
        Query {
            revision: format!("qv:{}", self.revision),
            text: self.text.clone(),
            pending: self.pending,
        }
    }
}
