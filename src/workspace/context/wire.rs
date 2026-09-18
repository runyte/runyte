// SPDX-License-Identifier: MPL-2.0

//! Strict external context profile of the stable application protocol.
//! Parsing admits syntax only; the host checks the live grant and resource owner.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::plugin::{
    application::{Error, ErrorCode},
    editor, json,
};

pub const FEATURE: &str = "runyte.context.v1";
pub const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_READERS: usize = 8;
pub const MAX_REQUESTS: usize = 16;
pub const MAX_LIST_ITEMS: usize = 256;
pub const MAX_SNAPSHOTS: usize = 2;
pub const MAX_TERMINAL_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_RETAINED_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_HOST_RETAINED_BYTES: usize = 64 * 1024 * 1024;
pub const SNAPSHOT_IDLE_SECONDS: u64 = 30;
pub const MAX_PROPOSALS: usize = 4;
pub const MAX_HOST_PROPOSALS: usize = 16;
pub const PROPOSAL_EXPIRY_SECONDS: u64 = 120;
pub const MAX_IDENTIFIER_BYTES: usize = 256;
pub const MAX_REQUEST_ID_BYTES: usize = 128;
pub const MAX_DEDUPLICATED_REQUESTS: usize = 1024;
/// An append's optional suffix guard is compared in one host turn, so it only
/// needs to identify the end of the text a writer composed against.
pub const MAX_EXPECTED_TAIL_BYTES: usize = 4096;
/// Enough of the inserted text to show escaping or formatting mistakes.
pub const APPEND_PREVIEW_CHARS: usize = 256;
pub const CAPABILITIES: &[&str] = &[
    "terminal_read",
    "editor_context_read",
    "buffer_edit",
    "terminal_propose",
];

/// The context feature adds its resource bounds to the stable hello envelope.
/// Unsupported application resources advertise zero authority and capacity.
pub fn limits() -> serde_json::Value {
    let mut value = serde_json::to_value(crate::plugin::application::Limits::default())
        .expect("application limits serialize");
    for (key, limit) in value.as_object_mut().expect("limits object") {
        if key != "resources" {
            *limit = serde_json::json!(0);
        }
    }
    if let Some(resources) = value["resources"].as_object_mut() {
        for limit in resources.values_mut() {
            *limit = serde_json::json!(0);
        }
    }
    value["line_bytes"] = serde_json::json!(MAX_FRAME_BYTES);
    value["requests"] = serde_json::json!(MAX_REQUESTS);
    value["control_queue_messages"] = serde_json::json!(MAX_REQUESTS);
    value["control_queue_bytes"] = serde_json::json!(MAX_FRAME_BYTES);
    value["control_deadline_seconds"] = serde_json::json!(10);
    value["context"] = serde_json::json!({
        "readers":MAX_READERS,"handles":256,"list_items":MAX_LIST_ITEMS,
        "read_rows":crate::terminal::read::MAX_ROWS,"read_bytes":crate::terminal::read::MAX_BYTES,
        "read_cells":crate::terminal::read::MAX_CELLS,"snapshots":MAX_SNAPSHOTS,
        "terminal_snapshot_bytes":MAX_TERMINAL_SNAPSHOT_BYTES,"retained_bytes":MAX_RETAINED_BYTES,
        "host_retained_bytes":MAX_HOST_RETAINED_BYTES,"snapshot_idle_seconds":SNAPSHOT_IDLE_SECONDS,
        "proposals":MAX_PROPOSALS,"host_proposals":MAX_HOST_PROPOSALS,
        "proposal_text_bytes":crate::terminal::proposal::MAX_TEXT_BYTES,"proposal_expiry_seconds":PROPOSAL_EXPIRY_SECONDS,
        "edit_changes":editor::MAX_CHANGES,"edit_replacement_bytes":editor::MAX_REPLACEMENT_BYTES,
        "request_ids":MAX_DEDUPLICATED_REQUESTS,"append_tail_bytes":MAX_EXPECTED_TAIL_BYTES,
        "append_preview_chars":APPEND_PREVIEW_CHARS
    });
    value
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    TerminalRead,
    EditorContextRead,
    BufferEdit,
    TerminalPropose,
}

impl Scope {
    pub fn capability(self) -> &'static str {
        match self {
            Self::TerminalRead => "terminal_read",
            Self::EditorContextRead => "editor_context_read",
            Self::BufferEdit => "buffer_edit",
            Self::TerminalPropose => "terminal_propose",
        }
    }
    pub fn from_capability(value: &str) -> Option<Self> {
        match value {
            "terminal_read" => Some(Self::TerminalRead),
            "editor_context_read" => Some(Self::EditorContextRead),
            "buffer_edit" => Some(Self::BufferEdit),
            "terminal_propose" => Some(Self::TerminalPropose),
            _ => None,
        }
    }
}

pub fn validate_scopes(scopes: &BTreeSet<Scope>) -> Result<(), Error> {
    if (scopes.contains(&Scope::BufferEdit) && !scopes.contains(&Scope::EditorContextRead))
        || (scopes.contains(&Scope::TerminalPropose) && !scopes.contains(&Scope::TerminalRead))
    {
        return Err(Error::new(
            ErrorCode::CapabilityDenied,
            "Write scopes require the corresponding read scope",
        ));
    }
    Ok(())
}

/// Intentionally has no Debug implementation: credentials must not enter logs.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authentication {
    #[serde(rename = "type")]
    kind: AuthenticationType,
    pub credential: String,
}
#[derive(Deserialize)]
enum AuthenticationType {
    #[serde(rename = "authenticate")]
    Authenticate,
}

pub fn parse_authentication(raw: &str) -> Result<Authentication, Error> {
    preflight(raw)?;
    let value: Authentication = decode(raw)?;
    let _ = &value.kind;
    if value.credential.len() != 64
        || !value
            .credential
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(invalid("Invalid context credential"));
    }
    Ok(value)
}

/// Registration preserves the public runyte-1 field names and release range.
/// Context readers register no commands or settings schema.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    #[serde(rename = "type")]
    pub kind: RegistrationType,
    pub version: String,
    pub runyte: String,
    pub name: String,
    pub commands: Vec<crate::plugin::application::Registration>,
    #[serde(deserialize_with = "unique_names")]
    pub required_capabilities: BTreeSet<String>,
    #[serde(deserialize_with = "unique_names")]
    pub optional_capabilities: BTreeSet<String>,
    #[serde(deserialize_with = "unique_names")]
    pub required_features: BTreeSet<String>,
    #[serde(deserialize_with = "unique_names")]
    pub optional_features: BTreeSet<String>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum RegistrationType {
    #[serde(rename = "register")]
    Register,
}

pub fn parse_registration(raw: &str) -> Result<Registration, Error> {
    preflight(raw)?;
    let value: Registration = decode(raw)?;
    if value.version != crate::plugin::application::VERSION
        || !value.required_features.contains(FEATURE)
    {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "The context profile requires runyte-1 and runyte.context.v1",
        ));
    }
    crate::plugin::compatibility::negotiate_features(
        &value.required_features,
        &value.optional_features,
        &[FEATURE],
    )
    .map_err(|_| Error::new(ErrorCode::Unsupported, "Unsupported context features"))?;
    crate::plugin::compatibility::ReleaseRange::parse(&value.runyte)
        .map_err(|_| invalid("Invalid Runyte release range"))?;
    if value.name.is_empty()
        || value.name.len() > 128
        || value.name.chars().any(char::is_control)
        || !value.commands.is_empty()
    {
        return Err(invalid("Invalid context registration name or commands"));
    }
    if !value
        .required_capabilities
        .is_disjoint(&value.optional_capabilities)
        || value
            .required_capabilities
            .union(&value.optional_capabilities)
            .any(|name| Scope::from_capability(name).is_none())
    {
        return Err(Error::new(
            ErrorCode::CapabilityDenied,
            "Unsupported context capability",
        ));
    }
    Ok(value)
}

fn unique_names<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<BTreeSet<String>, D::Error> {
    let names = Vec::<String>::deserialize(decoder)?;
    let mut result = BTreeSet::new();
    for name in names {
        if name.is_empty() || name.len() > 64 || result.len() == 32 || !result.insert(name) {
            return Err(serde::de::Error::custom(
                "Invalid or duplicate negotiation name",
            ));
        }
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    Screen,
    Tail,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum Request {
    #[serde(rename = "buffer.list")]
    BufferList { offset: usize, limit: usize },
    #[serde(rename = "buffer.read")]
    BufferRead {
        buffer: String,
        expected_revision: String,
        from: usize,
        to: usize,
    },
    #[serde(rename = "buffer.edit")]
    BufferEdit {
        buffer: String,
        expected_revision: String,
        changes: Vec<editor::Change>,
    },
    /// Appends at the buffer end current when the host applies it. There is
    /// no expected revision: appends serialize on the host loop, so no writer
    /// is lost. `expected_tail` guards against composing against stale text.
    #[serde(rename = "buffer.append")]
    BufferAppend {
        buffer: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_tail: Option<String>,
    },
    #[serde(rename = "buffer.snapshot.open")]
    SnapshotOpen {
        buffer: String,
        expected_revision: String,
    },
    #[serde(rename = "buffer.snapshot.read")]
    SnapshotRead {
        snapshot: String,
        from: usize,
        to: usize,
    },
    #[serde(rename = "buffer.snapshot.close")]
    SnapshotClose { snapshot: String },
    #[serde(rename = "selection.get")]
    SelectionGet { pane: String },
    #[serde(rename = "pane.context.list")]
    PaneContextList(Empty),
    #[serde(rename = "pane.viewport.read")]
    PaneViewportRead {
        pane: String,
        expected_revision: Option<String>,
        max_rows: usize,
        max_bytes: usize,
        max_cells: usize,
    },
    #[serde(rename = "terminal.list")]
    TerminalList { offset: usize, limit: usize },
    #[serde(rename = "terminal.read")]
    TerminalRead {
        terminal: String,
        region: Region,
        expected_revision: Option<String>,
        max_rows: usize,
        max_bytes: usize,
        max_cells: usize,
    },
    #[serde(rename = "terminal.snapshot.open")]
    TerminalSnapshotOpen {
        terminal: String,
        region: Region,
        expected_revision: Option<String>,
        max_rows: usize,
        max_bytes: usize,
        max_cells: usize,
    },
    #[serde(rename = "terminal.snapshot.read")]
    TerminalSnapshotRead {
        snapshot: String,
        offset: usize,
        limit: usize,
    },
    #[serde(rename = "terminal.snapshot.close")]
    TerminalSnapshotClose { snapshot: String },
    #[serde(rename = "terminal.input.propose")]
    TerminalInputPropose {
        terminal: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    #[serde(rename = "terminal.input.status")]
    TerminalInputStatus { proposal: String },
    #[serde(rename = "terminal.input.cancel")]
    TerminalInputCancel { proposal: String },
}

impl Request {
    pub fn required_scope(&self) -> Scope {
        match self {
            Self::BufferEdit { .. } | Self::BufferAppend { .. } => Scope::BufferEdit,
            Self::TerminalInputPropose { .. }
            | Self::TerminalInputStatus { .. }
            | Self::TerminalInputCancel { .. } => Scope::TerminalPropose,
            Self::TerminalList { .. }
            | Self::TerminalRead { .. }
            | Self::TerminalSnapshotOpen { .. }
            | Self::TerminalSnapshotRead { .. }
            | Self::TerminalSnapshotClose { .. } => Scope::TerminalRead,
            Self::BufferList { .. }
            | Self::BufferRead { .. }
            | Self::SnapshotOpen { .. }
            | Self::SnapshotRead { .. }
            | Self::SnapshotClose { .. }
            | Self::SelectionGet { .. }
            | Self::PaneContextList(_)
            | Self::PaneViewportRead { .. } => Scope::EditorContextRead,
        }
    }

    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::BufferList { offset, limit } | Self::TerminalList { offset, limit } => {
                page(*offset, *limit, MAX_LIST_ITEMS)?;
            }
            Self::BufferRead {
                buffer,
                expected_revision,
                from,
                to,
            } => {
                identifier(buffer)?;
                identifier(expected_revision)?;
                range(*from, *to)?;
            }
            Self::BufferEdit {
                buffer,
                expected_revision,
                changes,
            } => {
                identifier(buffer)?;
                identifier(expected_revision)?;
                if changes.is_empty()
                    || changes.len() > editor::MAX_CHANGES
                    || changes.iter().map(|c| c.text.len()).sum::<usize>()
                        > editor::MAX_REPLACEMENT_BYTES
                {
                    return Err(Error::new(
                        ErrorCode::LimitExceeded,
                        "Buffer edit exceeds limits",
                    ));
                }
                for change in changes {
                    if change.from > change.to {
                        return Err(invalid("Reversed edit range"));
                    }
                }
            }
            Self::BufferAppend {
                buffer,
                text,
                expected_tail,
            } => {
                identifier(buffer)?;
                if text.is_empty() || expected_tail.as_ref().is_some_and(String::is_empty) {
                    return Err(invalid("Append text and expected tail must not be empty"));
                }
                if text.len() > editor::MAX_REPLACEMENT_BYTES
                    || expected_tail
                        .as_ref()
                        .is_some_and(|tail| tail.len() > MAX_EXPECTED_TAIL_BYTES)
                {
                    return Err(Error::new(
                        ErrorCode::LimitExceeded,
                        "Buffer append exceeds limits",
                    ));
                }
            }
            Self::SnapshotOpen {
                buffer,
                expected_revision,
            } => {
                identifier(buffer)?;
                identifier(expected_revision)?;
            }
            Self::SnapshotRead { snapshot, from, to } => {
                identifier(snapshot)?;
                range(*from, *to)?;
            }
            Self::SnapshotClose { snapshot } | Self::TerminalSnapshotClose { snapshot } => {
                identifier(snapshot)?
            }
            Self::SelectionGet { pane } => identifier(pane)?,
            Self::PaneContextList(_) => {}
            Self::PaneViewportRead {
                pane,
                expected_revision,
                max_rows,
                max_bytes,
                max_cells,
            } => {
                identifier(pane)?;
                optional_revision(expected_revision)?;
                read_limits(*max_rows, *max_bytes, *max_cells)?;
            }
            Self::TerminalRead {
                terminal,
                expected_revision,
                max_rows,
                max_bytes,
                max_cells,
                ..
            }
            | Self::TerminalSnapshotOpen {
                terminal,
                expected_revision,
                max_rows,
                max_bytes,
                max_cells,
                ..
            } => {
                identifier(terminal)?;
                optional_revision(expected_revision)?;
                read_limits(*max_rows, *max_bytes, *max_cells)?;
            }
            Self::TerminalSnapshotRead {
                snapshot,
                offset,
                limit,
            } => {
                identifier(snapshot)?;
                page(*offset, *limit, crate::terminal::read::MAX_ROWS)?;
            }
            Self::TerminalInputPropose {
                terminal,
                text,
                reason,
            } => {
                identifier(terminal)?;
                if reason.as_ref().is_some_and(|reason| {
                    reason.len() > 1024
                        || reason.chars().count() > 256
                        || reason
                            .chars()
                            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
                }) {
                    return Err(invalid("Invalid terminal proposal reason"));
                }
                crate::terminal::proposal::Text::new(text).map_err(|_| {
                    invalid("Terminal proposal must be bounded single-line text without controls")
                })?;
            }
            Self::TerminalInputStatus { proposal } | Self::TerminalInputCancel { proposal } => {
                identifier(proposal)?
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RequestEnvelope {
    #[serde(rename = "type")]
    pub kind: RequestType,
    pub id: String,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequest {
    #[serde(rename = "type")]
    kind: RequestType,
    id: String,
    method: String,
    params: Box<serde_json::value::RawValue>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum RequestType {
    #[serde(rename = "request")]
    Request,
}

pub fn parse_request(raw: &str) -> Result<RequestEnvelope, Error> {
    preflight(raw)?;
    let value: RawRequest = decode(raw)?;
    let _ = &value.kind;
    if value.id.len() > MAX_REQUEST_ID_BYTES {
        return Err(invalid("Request ID exceeds limit"));
    }
    identifier(&value.id)?;
    // Preserve original parameter bytes so duplicate members remain visible to
    // serde's strict typed decoder instead of disappearing inside Value maps.
    let request: Request = decode(&format!(
        "{{\"method\":{},\"params\":{}}}",
        serde_json::to_string(&value.method).unwrap(),
        value.params.get()
    ))?;
    request.validate()?;
    Ok(RequestEnvelope {
        kind: RequestType::Request,
        id: value.id,
        request,
    })
}

fn preflight(raw: &str) -> Result<(), Error> {
    if raw.len() > MAX_FRAME_BYTES {
        return Err(Error::new(
            ErrorCode::LimitExceeded,
            "Context frame exceeds limit",
        ));
    }
    json::validate(
        raw,
        json::Limits {
            max_bytes: MAX_FRAME_BYTES,
            max_depth: 12,
            max_nodes: 16_384,
            max_container: 2048,
        },
    )
}
fn decode<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, Error> {
    serde_json::from_str(raw).map_err(|_| invalid("Invalid context message"))
}
fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidArgument, message)
}
fn identifier(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b':' | b'-' | b'_' | b'.'))
    {
        return Err(invalid("Invalid context identifier"));
    }
    Ok(())
}
fn optional_revision(value: &Option<String>) -> Result<(), Error> {
    value.as_deref().map(identifier).transpose().map(|_| ())
}
fn page(offset: usize, limit: usize, maximum: usize) -> Result<(), Error> {
    if limit == 0 || limit > maximum || offset.checked_add(limit).is_none() {
        return Err(invalid("Invalid page limits"));
    }
    Ok(())
}
fn range(from: usize, to: usize) -> Result<(), Error> {
    if from > to || to - from > editor::MAX_CHUNK_BYTES {
        return Err(invalid("Invalid buffer range"));
    }
    Ok(())
}
fn read_limits(rows: usize, bytes: usize, cells: usize) -> Result<(), Error> {
    if !(1..=crate::terminal::read::MAX_ROWS).contains(&rows)
        || !(1..=crate::terminal::read::MAX_BYTES).contains(&bytes)
        || !(1..=crate::terminal::read::MAX_CELLS).contains(&cells)
    {
        return Err(invalid("Invalid context read limits"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/contract.rs"]
mod tests;
