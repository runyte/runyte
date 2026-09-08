// SPDX-License-Identifier: MPL-2.0

//! Bounded semantic application projections. Plugin row IDs never become offsets.
use super::application::{Error, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub title: String,
    pub purpose: Purpose,
    pub rows: Vec<Row>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Document,
    List,
    Dashboard,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    pub text: String,
    pub role: Role,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Ordinary,
    Muted,
    Heading,
    Warning,
    Error,
}

impl Model {
    pub fn payload_bytes(&self) -> usize {
        self.title.len()
            + self
                .rows
                .iter()
                .map(|row| row.id.len() + row.text.len() + 1)
                .sum::<usize>()
    }
    pub fn validate(&self) -> Result<(), Error> {
        let fail = |message| Error::new(ErrorCode::InvalidArgument, message);
        if self.title.is_empty()
            || self.title.len() > 160
            || self.title.chars().any(char::is_control)
        {
            return Err(fail("Invalid view title"));
        }
        if self.rows.len() > 10_000 {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "View row limit exceeded",
            ));
        }
        let mut ids = BTreeSet::new();
        let mut bytes = self.title.len();
        for row in &self.rows {
            if row.id.is_empty()
                || row.id.len() > 64
                || row.id.chars().any(char::is_control)
                || !ids.insert(&row.id)
                || row.text.chars().any(char::is_control)
            {
                return Err(fail("Invalid or duplicate view row"));
            }
            bytes += row.id.len() + row.text.len() + 1;
            if bytes > 4 * 1024 * 1024 {
                return Err(Error::new(
                    ErrorCode::LimitExceeded,
                    "View payload limit exceeded",
                ));
            }
        }
        if serde_json::to_vec(self).expect("model serialization").len() > super::MAX_BYTES - 1024 {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "Encoded model exceeds line limit",
            ));
        }
        Ok(())
    }
    pub fn text(&self) -> String {
        let mut text = String::new();
        for row in &self.rows {
            text.push_str(&row.text);
            text.push('\n');
        }
        text
    }
}

pub(crate) struct View {
    pub buffer: usize,
    pub model: Model,
    pub revision: u64,
    pub published: Option<std::time::Instant>,
}
