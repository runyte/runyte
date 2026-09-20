// SPDX-License-Identifier: MPL-2.0

//! Optional human-readable command presentation, separate from command identity.
use super::application::{Error, ErrorCode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Presentation {
    pub label: String,
    #[serde(
        default,
        deserialize_with = "authored",
        skip_serializing_if = "Option::is_none"
    )]
    pub group: Option<String>,
    #[serde(default)]
    pub order: u16,
    #[serde(default = "listed")]
    pub listed: bool,
}
fn listed() -> bool {
    true
}
impl Presentation {
    pub fn validate(&self) -> Result<(), Error> {
        let safe = |text: &str, max| {
            !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control)
        };
        if !safe(&self.label, 160) || self.group.as_deref().is_some_and(|s| !safe(s, 64)) {
            return Err(Error::new(
                ErrorCode::InvalidArgument,
                "Invalid command presentation",
            ));
        }
        Ok(())
    }

    pub fn payload_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.label.capacity()
            + self.group.as_ref().map_or(0, String::capacity)
    }
}

pub(crate) fn authored<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}
