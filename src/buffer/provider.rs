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
    pub baseline_epoch: u64,
    pub uncertain: Option<ProviderUncertain>,
}

/// Captured bytes whose remote commit outcome has not been established.
#[derive(Clone, Debug)]
pub struct ProviderUncertain {
    pub text: Text,
    pub version: String,
}

impl PartialEq for ProviderUncertain {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version && self.text.same_content(&other.text)
    }
}
impl Eq for ProviderUncertain {}

/// Immutable remote save input, independent of later editing and undo.
#[derive(Clone, Debug)]
pub struct ProviderSave {
    pub identity: ProviderIdentity,
    pub generation: String,
    pub epoch: u64,
    pub version: String,
    pub text: Text,
}

#[derive(Debug)]
pub struct ProviderConflict;
impl std::fmt::Display for ProviderConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Remote resource differs from the accepted or uncertain saved content")
    }
}
impl std::error::Error for ProviderConflict {}

impl ProviderDocument {
    fn matches_save(&self, save: &ProviderSave) -> bool {
        self.identity == save.identity
            && self.generation == save.generation
            && self.baseline_epoch == save.epoch
            && self.version == save.version
    }
}

impl Buffer {
    pub fn provider_document(document: ProviderDocument, text: String) -> Self {
        let mut buffer = Self::scratch();
        buffer.kind = BufferKind::Provider(document);
        buffer.text = Text::from_str(&text);
        buffer.longest_line = buffer.text.longest_line_bytes();
        buffer.mark_saved();
        if buffer
            .provider()
            .is_some_and(|document| document.uncertain.is_some())
        {
            buffer.mark_write_uncertain();
        }
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
        self.text = Text::from_str(&saved.to_string());
        self.longest_line = self.text.longest_line_bytes();
        self.undo.clear();
        self.redo.clear();
        self.undo_group = None;
        self.write_uncertain |= self
            .provider()
            .is_some_and(|document| document.uncertain.is_some());
        self.update_dirty();
        Ok(())
    }

    pub fn prepare_provider_save(&self) -> Result<ProviderSave> {
        let document = self
            .provider()
            .context("buffer is not a provider document")?;
        ensure!(document.available, "Resource provider is unavailable");
        ensure!(
            document.uncertain.is_none() && !self.write_uncertain,
            "Reconcile the unknown remote write before saving again"
        );
        ensure!(
            self.text.len_bytes() <= 8 * 1024 * 1024,
            "Provider document exceeds save limit"
        );
        ensure!(
            !self.text.rope().chunks().any(|chunk| chunk.contains('\0')),
            "Provider document contains binary NUL data"
        );
        Ok(ProviderSave {
            identity: document.identity.clone(),
            generation: document.generation.clone(),
            epoch: document.baseline_epoch,
            version: document.version.clone(),
            text: self.text.clone(),
        })
    }

    pub fn accept_provider_save(&mut self, save: ProviderSave, new_version: String) -> bool {
        let Some(document) = self.provider_mut().filter(|document| {
            document.available && document.uncertain.is_none() && document.matches_save(&save)
        }) else {
            return false;
        };
        document.version = new_version;
        document.baseline_epoch = document.baseline_epoch.wrapping_add(1);
        self.saved_text = Some(save.text);
        self.write_uncertain = false;
        self.update_dirty();
        true
    }

    pub fn mark_provider_uncertain(&mut self, save: ProviderSave) -> bool {
        let Some(document) = self
            .provider_mut()
            .filter(|document| document.uncertain.is_none() && document.matches_save(&save))
        else {
            return false;
        };
        document.uncertain = Some(ProviderUncertain {
            text: save.text,
            version: save.version,
        });
        self.mark_write_uncertain();
        true
    }

    /// Rebind only to bytes matching a known remote baseline. Live edits and
    /// history remain untouched; a changed baseline may change dirty state.
    pub fn reconcile_provider(
        &mut self,
        identity: &ProviderIdentity,
        generation: String,
        version: String,
        remote: &str,
    ) -> Result<()> {
        ensure!(
            remote.len() <= 8 * 1024 * 1024,
            "Provider document exceeds reconciliation limit"
        );
        let document = self
            .provider()
            .context("buffer is not a provider document")?;
        ensure!(&document.identity == identity, ProviderConflict);
        // Rebinding an uncertain write reuses its reservation: the captured
        // rope and bounded remote string coexist without a third text copy.
        let matches_remote = |text: &Text| {
            text.len_bytes() == remote.len() && text.rope().chars().eq(remote.chars())
        };
        let committed = document
            .uncertain
            .as_ref()
            .filter(|unknown| matches_remote(&unknown.text))
            .map(|unknown| unknown.text.clone());
        ensure!(
            committed.is_some() || self.saved_text.as_ref().is_some_and(matches_remote),
            ProviderConflict
        );
        if let Some(committed) = committed {
            self.saved_text = Some(committed);
        }
        let document = self.provider_mut().expect("validated provider identity");
        document.generation = generation;
        document.version = version;
        document.available = true;
        document.uncertain = None;
        document.baseline_epoch = document.baseline_epoch.wrapping_add(1);
        self.write_uncertain = false;
        self.update_dirty();
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/provider.rs"]
mod tests;
