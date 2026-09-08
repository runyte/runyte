// SPDX-License-Identifier: MPL-2.0

//! Remote document identity is independent of local filesystem paths.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderIdentity {
    pub configured_plugin: String,
    pub provider: String,
    pub key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDocument {
    pub identity: ProviderIdentity,
    /// Validated presentation text; never derived from the opaque resource key.
    pub label: String,
    pub syntax_hint: Option<String>,
    pub version: String,
    pub generation: String,
    pub available: bool,
}

impl Buffer {
    pub fn provider_document(document: ProviderDocument, text: String) -> Self {
        let mut buffer = Self::scratch();
        buffer.kind = BufferKind::Provider(document);
        buffer.text = Text::from_str(&text);
        buffer.longest_line = buffer.text.longest_line_bytes();
        buffer.mark_saved();
        buffer
    }

    pub fn provider(&self) -> Option<&ProviderDocument> {
        match &self.kind {
            BufferKind::Provider(document) => Some(document),
            _ => None,
        }
    }

    pub fn provider_mut(&mut self) -> Option<&mut ProviderDocument> {
        match &mut self.kind {
            BufferKind::Provider(document) => Some(document),
            _ => None,
        }
    }

    pub(crate) fn discard_provider_changes(&mut self) -> Result<()> {
        ensure!(
            self.provider().is_some(),
            "buffer is not a provider document"
        );
        let saved = self
            .saved_text
            .as_ref()
            .context("provider document has no accepted baseline")?
            .clone();
        self.discard_changes_to(&saved.to_string())
    }
}

#[cfg(test)]
#[path = "tests/provider.rs"]
mod tests;
