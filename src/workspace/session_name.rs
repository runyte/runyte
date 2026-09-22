// SPDX-License-Identifier: MPL-2.0

//! Shared logical session-name spelling; names are not filesystem components.

use anyhow::{Result, ensure};

pub(crate) const MAX_HOST_NAME_BYTES: usize = 64;

pub(super) fn validate_host_name(name: &str) -> Result<()> {
    ensure!(!name.is_empty(), "session name cannot be empty");
    ensure!(
        name == name.trim(),
        "session name cannot start or end with whitespace"
    );
    ensure!(
        name.len() <= MAX_HOST_NAME_BYTES,
        "session name cannot exceed {MAX_HOST_NAME_BYTES} UTF-8 bytes"
    );
    ensure!(
        !name.chars().any(char::is_control),
        "session name cannot contain control characters"
    );
    Ok(())
}

/// Normalizes a person-supplied session name without changing any other
/// identity characters. Spaces at the edges are discarded; spaces that carry
/// meaning between words become hyphens. Persisted historical names remain
/// readable even if they predate this input rule.
pub(super) fn normalize_session_name(name: &str) -> String {
    name.trim_matches(' ').replace(' ', "-")
}
