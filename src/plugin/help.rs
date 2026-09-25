// SPDX-License-Identifier: MPL-2.0

//! Plugin-authored contextual help topics, negotiated through `view-help`.
//!
//! A topic is workflow prose only. Action labels, availability and key
//! spellings are generated from the live command and keymap registries when
//! help opens, so a topic never carries a second action or key table.
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;

pub const MAX_TOPICS: usize = 16;
pub const MAX_PARAGRAPHS: usize = 16;
pub const MAX_TITLE_BYTES: usize = 64;
pub const MAX_PARAGRAPH_BYTES: usize = 2048;
/// Total prose retained for one registration, across every topic.
pub const MAX_TOTAL_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Topic {
    pub id: String,
    pub title: String,
    #[serde(deserialize_with = "paragraphs")]
    pub paragraphs: Vec<String>,
}

impl Topic {
    pub fn payload_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.id.capacity()
            + self.title.capacity()
            + self.paragraphs.capacity() * std::mem::size_of::<String>()
            + self.paragraphs.iter().map(String::capacity).sum::<usize>()
    }
}

/// A registration's retained help: its topics and the application name their
/// titles are shown under. Absent for a registration without topics, which
/// is charged nothing for help.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registered {
    pub application: String,
    pub topics: std::collections::BTreeMap<String, Topic>,
}

impl Registered {
    pub fn new(application: String, topics: Vec<Topic>) -> Option<Self> {
        (!topics.is_empty()).then(|| Self {
            application,
            topics: topics
                .into_iter()
                .map(|topic| (topic.id.clone(), topic))
                .collect(),
        })
    }

    pub fn payload_bytes(&self) -> usize {
        self.application.capacity()
            + self
                .topics
                .iter()
                .map(|(id, topic)| id.capacity() + topic.payload_bytes())
                .sum::<usize>()
    }
}

fn safe(text: &str, bytes: usize) -> bool {
    !text.is_empty() && text.len() <= bytes && !text.chars().any(char::is_control)
}

/// Checks a complete registration's topics; the first failure rejects them all.
pub fn validate(topics: &[Topic]) -> Result<(), String> {
    if topics.len() > MAX_TOPICS {
        return Err("Too many help topics".into());
    }
    let mut ids = BTreeSet::new();
    let mut total = 0usize;
    for topic in topics {
        if !super::valid_name(&topic.id) || !ids.insert(topic.id.as_str()) {
            return Err("Invalid or duplicate help topic id".into());
        }
        if !safe(&topic.title, MAX_TITLE_BYTES) {
            return Err("Invalid help topic title".into());
        }
        if topic.paragraphs.is_empty()
            || topic.paragraphs.len() > MAX_PARAGRAPHS
            || topic
                .paragraphs
                .iter()
                .any(|paragraph| !safe(paragraph, MAX_PARAGRAPH_BYTES))
        {
            return Err("Invalid help topic paragraphs".into());
        }
        total += topic.title.len() + topic.paragraphs.iter().map(String::len).sum::<usize>();
        if total > MAX_TOTAL_BYTES {
            return Err("Help topics exceed 64 KiB".into());
        }
    }
    Ok(())
}

/// Streams at most `MAX_PARAGRAPHS` entries plus one, so an oversized list is
/// refused without retaining the remainder.
fn paragraphs<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    bounded(deserializer, MAX_PARAGRAPHS)
}

pub(crate) fn topics<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<Topic>>, D::Error> {
    bounded(deserializer, MAX_TOPICS).map(Some)
}

fn bounded<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
    limit: usize,
) -> Result<Vec<T>, D::Error> {
    use serde::de::{Error, IgnoredAny, SeqAccess, Visitor};
    struct Bounded<T>(usize, std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Bounded<T> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a bounded help array")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while let Some(value) = sequence.next_element::<T>()? {
                values.push(value);
                if values.len() == self.0 {
                    if sequence.next_element::<IgnoredAny>()?.is_some() {
                        return Err(A::Error::custom("Help array limit exceeded"));
                    }
                    break;
                }
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Bounded(limit, std::marker::PhantomData))
}

#[cfg(test)]
#[path = "tests/help_topics.rs"]
mod tests;
