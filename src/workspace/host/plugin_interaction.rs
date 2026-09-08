// SPDX-License-Identifier: MPL-2.0
use super::WorkspaceHost;
use crate::{
    app::plugin_interaction::Surface,
    plugin::{
        self, application as api,
        interaction::{self, Field, Kind},
    },
};

impl WorkspaceHost {
    pub(super) fn application_input_request(
        &mut self,
        owner: usize,
        request: api::Request,
    ) -> Result<api::ResultValue, api::Error> {
        let fail = api::Error::new;
        let state = &self.app.plugins.instances[&owner].application;
        if !state.capabilities.contains("interaction") {
            return Err(fail(
                api::ErrorCode::CapabilityDenied,
                "Interaction capability was not granted",
            ));
        }
        if let api::Request::UiDismiss { surface } = request {
            self.app.cancel_plugin_input(owner, Some(&surface));
            return Ok(api::ResultValue::Empty(api::Empty {}));
        }
        let confirmation = matches!(request, api::Request::UiConfirm { .. });
        let is_picker = matches!(request, api::Request::UiPick { .. });
        let (invocation, title, fields) = match request {
            api::Request::UiForm {
                invocation,
                title,
                fields,
            } => (invocation, title, fields),
            api::Request::UiPrompt {
                invocation,
                title,
                field,
            } => (invocation, title, vec![field]),
            api::Request::UiPick {
                invocation,
                title,
                choices,
            } => {
                let mut field = Field::text("choice", "Choose an option".into());
                field.kind = Kind::Choice;
                field.choices = choices;
                (invocation, title, vec![field])
            }
            api::Request::UiConfirm {
                invocation,
                title,
                message,
            } => {
                let mut field = Field::text("confirmed", message);
                field.kind = Kind::Boolean;
                (invocation, title, vec![field])
            }
            _ => unreachable!(),
        };
        interaction::validate(&title, &fields)?;
        let context = state
            .requests
            .get(&invocation)
            .ok_or_else(|| fail(api::ErrorCode::ContextChanged, "Invoking command finished"))?
            .clone();
        self.app.plugin_foreground(&context)?;
        if self.app.plugin_has_input_surface() {
            return Err(fail(
                api::ErrorCode::Busy,
                "An input surface or macro owns input",
            ));
        }
        if !state.input_surfaces.is_empty() {
            return Err(fail(
                api::ErrorCode::Busy,
                "Previous input result is still pending",
            ));
        }
        self.reserve_application_payload(owner, interaction::SURFACE_CHARGE)?;
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        state.next_handle += 1;
        let handle = format!("u:{}:{}", state.generation, state.next_handle);
        state.input_surfaces.insert(handle.clone());
        state.retained_payload += interaction::SURFACE_CHARGE;
        let picker = is_picker.then(|| {
            crate::picker::ListPicker::fuzzy(
                title.clone(),
                fields[0]
                    .choices
                    .iter()
                    .enumerate()
                    .map(|(i, label)| crate::picker::PickerItem::new(label, "", i))
                    .collect(),
            )
        });
        self.app.plugins.input = Some(Surface {
            picker,
            confirmation,
            owner,
            handle: handle.clone(),
            context,
            title,
            values: fields.iter().map(Field::initial).collect(),
            fields,
            selected: 0,
            cursor: 0,
            error: false,
        });
        self.app.plugins.presentation_dirty = true;
        Ok(api::ResultValue::Surface { surface: handle })
    }
    pub(super) fn sync_plugin_inputs(&mut self) {
        self.app.sync_plugin_input();
        for (owner, context, params) in std::mem::take(&mut self.app.plugins.input_finished) {
            let Some(state) = self
                .app
                .plugins
                .instances
                .get_mut(&owner)
                .map(|i| &mut i.application)
            else {
                continue;
            };
            if !state.input_surfaces.remove(&params.surface) {
                continue;
            }
            state.retained_payload -= interaction::SURFACE_CHARGE;
            if state.requests.len() >= api::MAX_REQUESTS {
                self.stop_plugin(owner, "input result consumer is too slow");
                continue;
            }
            self.app.plugins.next_invocation += 1;
            let id = format!("h:{}", self.app.plugins.next_invocation);
            state.requests.insert(id.clone(), context);
            if self
                .application_send(
                    owner,
                    api::HostMessage::InputRequest {
                        id: id.clone(),
                        method: "ui.submit",
                        params,
                    },
                )
                .is_err()
                || self
                    .plugin_send(
                        owner,
                        plugin::HostMessage::Deadline {
                            token: id,
                            after_ms: Some(10_000),
                        },
                    )
                    .is_err()
            {
                self.stop_plugin(owner, "input result consumer is too slow");
            }
        }
    }
}
