// SPDX-License-Identifier: MPL-2.0

//! Explicit-submit validation: metadata-only pending ownership, no typing RPCs.

use super::WorkspaceHost;
use crate::plugin::{
    self, application as api,
    interaction::{PendingValidation, ValidationCancelled, ValidationRequest},
};
use anyhow::{Result, ensure};
use std::collections::BTreeSet;

const MAX_RETIRED: usize = 16;

impl WorkspaceHost {
    pub(super) fn sync_plugin_validation(&mut self) {
        // There can be one active input surface, but a dismissed owner's callback
        // may still be finishing. Its bounded slot remains charged until settlement.
        let owners: Vec<_> = self.app.plugins.instances.keys().copied().collect();
        for owner in owners {
            if self.sync_validation_owner(owner).is_err() {
                self.stop_plugin(owner, "validation consumer is too slow");
            }
        }
    }

    fn sync_validation_owner(&mut self, owner: usize) -> Result<()> {
        let state = &self.app.plugins.instances[&owner].application;
        if let Some(pending) = &state.validation {
            if !pending.cancelled
                && !self
                    .app
                    .plugin_validation_current(owner, &pending.surface, &pending.revision)
            {
                let pending = pending.clone();
                self.app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application
                    .validation
                    .as_mut()
                    .unwrap()
                    .cancelled = true;
                self.send_validation_cancellation(owner, &pending)?;
            }
            return Ok(());
        }
        if state.requests.len() + state.provider_requests >= api::MAX_REQUESTS
            || self
                .app
                .plugins
                .input
                .as_ref()
                .is_none_or(|surface| surface.owner != owner)
        {
            return Ok(());
        }
        let Some(intent) = self.app.peek_plugin_validation() else {
            return Ok(());
        };
        if state.retired_validations.len() == MAX_RETIRED {
            // Retain known late IDs instead of silently evicting them or growing
            // forever. A late response restores admission; the form can retry.
            if self.app.plugin_validation_pending(&intent) {
                self.app.plugin_validation_failed(
                    owner,
                    &intent.surface,
                    &intent.revision,
                    intent.foreground,
                );
            }
            return Ok(());
        }
        if !self.app.plugin_validation_pending(&intent) {
            return Ok(());
        }
        let mut context = self.app.plugins.input.as_ref().unwrap().context.clone();
        context.foreground_allowed = false;
        context.foreground = intent.foreground;
        context.action = None;
        self.app.plugins.next_invocation += 1;
        let request = format!("h:{}", self.app.plugins.next_invocation);
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        state.requests.insert(request.clone(), context);
        state.validation = Some(PendingValidation {
            request: request.clone(),
            surface: intent.surface.clone(),
            revision: intent.revision.clone(),
            fields: intent.fields.clone(),
            foreground: intent.foreground,
            cancelled: false,
        });
        self.application_send(
            owner,
            api::HostMessage::ValidationRequest {
                id: request.clone(),
                method: "ui.validate",
                params: ValidationRequest {
                    surface: intent.surface,
                    revision: intent.revision,
                    fields: intent.fields,
                    values: intent.values,
                },
            },
        )?;
        self.plugin_send(
            owner,
            plugin::HostMessage::Deadline {
                token: request,
                after_ms: Some(10_000),
            },
        )
    }

    pub(super) fn validation_response(
        &mut self,
        owner: usize,
        request: &str,
        outcome: &api::CommandResponse,
    ) -> Result<bool> {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        if state.retired_validations.remove(request) {
            // A deadline already settled editor ownership. A known late reply has
            // no UI effect, irrespective of the now-obsolete result shape.
            return Ok(true);
        }
        if !state
            .validation
            .as_ref()
            .is_some_and(|pending| pending.request == request)
        {
            return Ok(false);
        }
        let pending = state.validation.as_ref().unwrap();
        if let api::CommandResponse::Validation { result } = outcome {
            ensure!(
                result.surface == pending.surface && result.revision == pending.revision,
                "validation response identity mismatch"
            );
            ensure!(
                result.fields.len() == pending.fields.len(),
                "validation response field count mismatch"
            );
            let fields: BTreeSet<_> = result.fields.iter().map(|field| &field.field).collect();
            ensure!(
                fields.len() == result.fields.len()
                    && pending.fields.iter().all(|field| fields.contains(field)),
                "validation response fields mismatch"
            );
        } else {
            ensure!(
                matches!(outcome, api::CommandResponse::Failure { .. }),
                "validation callback requires a validation result"
            );
        }
        let pending = state.validation.take().unwrap();
        state.requests.remove(request);
        self.plugin_send(
            owner,
            plugin::HostMessage::Deadline {
                token: request.into(),
                after_ms: None,
            },
        )?;
        if let api::CommandResponse::Validation { result } = outcome {
            self.app.apply_plugin_validation(
                owner,
                &pending.surface,
                &pending.revision,
                pending.foreground,
                &result.fields,
            );
        } else {
            // Error details may contain submitted secrets or server credentials.
            self.app.plugin_validation_failed(
                owner,
                &pending.surface,
                &pending.revision,
                pending.foreground,
            );
        }
        Ok(true)
    }

    fn send_validation_cancellation(
        &mut self,
        owner: usize,
        pending: &PendingValidation,
    ) -> Result<()> {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        state.sequence += 1;
        let sequence = format!("e:{}", state.sequence);
        self.application_send(
            owner,
            api::HostMessage::Event {
                sequence,
                event: "ui.validation_cancelled",
                data: api::EventData::ValidationCancelled(ValidationCancelled {
                    request: pending.request.clone(),
                    surface: pending.surface.clone(),
                    revision: pending.revision.clone(),
                }),
            },
        )
    }

    pub(super) fn validation_deadline(&mut self, owner: usize, request: &str) -> Result<bool> {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        if state.retired_validations.contains(request) {
            return Ok(true);
        }
        if !state
            .validation
            .as_ref()
            .is_some_and(|pending| pending.request == request)
        {
            return Ok(false);
        }
        let pending = state.validation.take().unwrap();
        state.requests.remove(request);
        state.retired_validations.insert(request.to_owned());
        self.app.plugin_validation_failed(
            owner,
            &pending.surface,
            &pending.revision,
            pending.foreground,
        );
        if !pending.cancelled {
            self.send_validation_cancellation(owner, &pending)?;
        }
        Ok(true)
    }
}
