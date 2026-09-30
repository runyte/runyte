// SPDX-License-Identifier: MPL-2.0

use super::{App, buffer_language};
use crate::indentation::Indentation;

impl App {
    pub(crate) fn indentation_for(&self, buffer_id: usize) -> Indentation {
        let base = Indentation {
            tab_width: self.config.editor.tab_width.max(1),
            style: self.config.editor.indent,
        };
        let overrides = &self.config.indentation;
        if overrides.languages.is_empty() && overrides.files.is_empty() {
            return base;
        }
        use crate::buffer::GeneratedViewIdentity;
        let shown = &self.buffers[buffer_id];
        let buffer = match shown.generated_view_identity() {
            Some(
                GeneratedViewIdentity::DiskSnapshot { source_buffer, .. }
                | GeneratedViewIdentity::ProviderSnapshot { source_buffer, .. },
            ) => self.buffers.get(*source_buffer).unwrap_or(shown),
            _ => shown,
        };
        let revision_path = match buffer.generated_view_identity() {
            Some(GeneratedViewIdentity::GitRevisionFile {
                repository,
                file,
                side: Some(previous),
                ..
            }) => (if *previous { &file.left } else { &file.right })
                .as_ref()
                .map(|path| repository.join(path)),
            _ => None,
        };
        let path = match buffer.generated_view_identity() {
            Some(GeneratedViewIdentity::GitDiffSide { path, .. }) => Some(path.as_path()),
            _ => revision_path.as_deref().or(buffer.path.as_deref()),
        };
        let language = buffer_language(buffer, &self.registry)
            .or_else(|| {
                path.and_then(|path| {
                    self.registry
                        .language_for_document(Some(path), buffer.text())
                })
            })
            .map(|id| self.registry.language_name(id));
        let path = path.and_then(|path| path.strip_prefix(&self.project_root).ok());
        overrides.resolve(base, language, path)
    }

    pub(super) fn active_indentation(&self) -> Indentation {
        self.indentation_for(self.active().buffer)
    }
}

use super::{
    ListAction, ListPicker, PickerItem, PromptKind, SettingId, SettingPreview, SettingValue,
    SettingsView,
};
use crate::{
    indentation::Scope,
    settings::{override_value, persist_override},
};

#[derive(Clone, Debug)]
pub(super) enum Action {
    Language(SettingId),
    Pattern(SettingId),
    Choose {
        setting: SettingId,
        scope: Scope,
    },
    Value {
        setting: SettingId,
        scope: Scope,
        value: SettingValue,
    },
    Remove {
        setting: SettingId,
        scope: Scope,
    },
}

impl App {
    pub(super) fn setting_context_action_available(
        &self,
        target: crate::keymap::BindingTarget,
    ) -> bool {
        use crate::{command::EditorCommand as C, keymap::BindingTarget};
        let BindingTarget::Editor(
            command @ (C::OverrideSettingLanguage
            | C::OverrideSettingPattern
            | C::RemoveSettingOverride),
        ) = target
        else {
            return true;
        };
        let row = self.cursor_position().row;
        if !matches!(
            self.active_buffer().setting_at(row),
            Some(SettingId::EditorTabWidth | SettingId::EditorIndent)
        ) {
            return false;
        }
        self.active_buffer().setting_override_at(row).is_some()
            == (command == C::RemoveSettingOverride)
    }

    pub(super) fn run_setting_action(&mut self, command: crate::command::EditorCommand) {
        use crate::command::EditorCommand as C;
        if !self.setting_context_action_available(command.into()) {
            self.action_failed("this action is not available for the selected setting row");
            return;
        }
        let row = self.cursor_position().row;
        let Some(setting) = self.active_buffer().setting_at(row) else {
            return;
        };
        let action = match command {
            C::OverrideSettingLanguage => Action::Language(setting),
            C::OverrideSettingPattern => Action::Pattern(setting),
            C::RemoveSettingOverride => Action::Remove {
                setting,
                scope: self
                    .active_buffer()
                    .setting_override_at(row)
                    .unwrap()
                    .clone(),
            },
            _ => return,
        };
        self.activate_indentation_action(action);
    }

    pub(super) fn activate_indentation_action(&mut self, action: Action) {
        match action {
            Action::Language(setting) => {
                let mut languages: Vec<_> = crate::syntax::builtin_language_names().collect();
                languages.sort_unstable();
                let items = languages
                    .iter()
                    .enumerate()
                    .map(|(index, name)| {
                        let scope = Scope::Language((*name).into());
                        let detail =
                            override_value(setting, &scope, &self.persisted_config.indentation)
                                .map_or_else(
                                    || {
                                        format!(
                                            "inherited: {}",
                                            setting.configured_value(&self.persisted_config)
                                        )
                                    },
                                    |value| format!("override: {value}"),
                                );
                        PickerItem::new(*name, detail, index)
                    })
                    .collect();
                self.list_actions = languages
                    .into_iter()
                    .map(|name| {
                        ListAction::Indentation(Action::Choose {
                            setting,
                            scope: Scope::Language(name.into()),
                        })
                    })
                    .collect();
                self.list = Some(ListPicker::new("Choose language", items));
            }
            Action::Pattern(setting) => {
                self.list = None;
                self.list_actions.clear();
                self.open_prompt(PromptKind::IndentationPattern(setting));
                self.status("enter a workspace-relative file pattern");
            }
            Action::Choose { setting, scope } => self.open_override_value(setting, scope),
            Action::Value {
                setting,
                scope,
                value,
            } => {
                self.persist_indentation_override(setting, scope, Some(value));
            }
            Action::Remove { setting, scope } => {
                self.persist_indentation_override(setting, scope, None);
            }
        }
    }

    #[cfg(test)]
    pub(super) fn override_inherited_label(&self, setting: SettingId, scope: &Scope) -> String {
        self.override_inheritance(setting, scope).1
    }

    fn override_inheritance(&self, setting: SettingId, scope: &Scope) -> (SettingValue, String) {
        let default = setting.configured_value(&self.persisted_config);
        if let Scope::Language(_) = scope {
            let label = format!("Inherited: {default} from {}", setting.descriptor().key);
            return (default, label);
        }
        let context = self
            .settings_origin_buffer
            .filter(|id| !self.closed_buffers.contains(id))
            .and_then(|id| self.buffers.get(id));
        if let Some(buffer) = context
            && let Some(path) = buffer
                .path
                .as_deref()
                .and_then(|path| path.strip_prefix(&self.project_root).ok())
        {
            let path = path
                .components()
                .filter_map(|part| match part {
                    std::path::Component::Normal(value) => Some(value.to_string_lossy()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("/");
            let Scope::Files(pattern) = scope else {
                unreachable!()
            };
            if crate::indentation::compile_pattern(pattern)
                .is_ok_and(|matcher| matcher.is_match(&path))
            {
                let mut value = default;
                let mut source = setting.descriptor().key.to_owned();
                if let Some(language) = buffer_language(buffer, &self.registry) {
                    let language_scope =
                        Scope::Language(self.registry.language_name(language).into());
                    if let Some(inherited) =
                        override_value(setting, &language_scope, &self.persisted_config.indentation)
                    {
                        value = inherited;
                        source = language_scope.label();
                    }
                }
                for rule in &self.persisted_config.indentation.files {
                    let candidate = Scope::Files(rule.pattern.clone());
                    if candidate != *scope
                        && rule.matches(&path)
                        && let Some(inherited) =
                            override_value(setting, &candidate, &self.persisted_config.indentation)
                    {
                        value = inherited;
                        source = candidate.label();
                    }
                }
                let label =
                    format!("Inherited for {path}: {value} from {source}; other files may differ");
                return (value, label);
            }
        }
        let label = format!(
            "Inherited: varies by file (language and other matching rules); global default: {default}"
        );
        (default, label)
    }

    pub(super) fn open_override_value(&mut self, setting: SettingId, scope: Scope) {
        self.list = None;
        self.list_actions.clear();
        self.settings_view = None;
        let (inherited_value, inherited) = self.override_inheritance(setting, &scope);
        let value = override_value(setting, &scope, &self.persisted_config.indentation)
            .unwrap_or(inherited_value);
        if setting == SettingId::EditorTabWidth {
            self.open_prompt_with_value(PromptKind::IndentationValue(setting), value.to_string());
            self.indentation_prompt_scope = Some(scope);
            self.indentation_prompt_hint = Some(inherited.clone());
            self.status(inherited);
            return;
        }
        let values = self.setting_values(setting);
        let items = values
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                PickerItem::new(
                    choice.to_string(),
                    if *choice == value {
                        format!("selected · {inherited}")
                    } else {
                        inherited.clone()
                    },
                    index,
                )
            })
            .collect();
        let selected = values
            .iter()
            .position(|choice| *choice == value)
            .unwrap_or(0);
        self.list_actions = values
            .into_iter()
            .map(|value| {
                ListAction::Indentation(Action::Value {
                    setting,
                    scope: scope.clone(),
                    value,
                })
            })
            .collect();
        let mut picker = ListPicker::new(
            format!("{} · {}", setting.descriptor().title, scope.label()),
            items,
        )
        .as_choice("to save");
        picker.selected = selected;
        self.settings_view = Some(SettingsView::Values(Box::new(SettingPreview {
            setting,
            override_scope: Some(scope),
            original_config: self.config.clone(),
            original_theme: self.theme.clone(),
            original_theme_name: self.theme_name.clone(),
            original_grammar: self.grammar.clone(),
            original_mode: self.mode,
        })));
        self.list = Some(picker);
        self.preview_selected_setting_value();
    }

    pub(super) fn persist_indentation_override(
        &mut self,
        setting: SettingId,
        scope: Scope,
        value: Option<SettingValue>,
    ) -> bool {
        let result = self
            .config_path
            .as_ref()
            .ok_or_else(|| "no configuration path was loaded".to_owned())
            .and_then(|path| {
                persist_override(path, setting, &scope, value.as_ref())
                    .map_err(|error| error.to_string())
            });
        let updated = match result {
            Ok(updated) => updated,
            Err(error) => {
                self.rollback_setting_preview();
                let recovery = if value.is_some() {
                    "Enter retries · Esc cancels"
                } else {
                    "Tab → Remove override retries"
                };
                self.action_failed(format!("could not save override: {error} · {recovery}"));
                return false;
            }
        };
        self.config.indentation = updated.indentation.clone();
        self.persisted_config = updated;
        self.sync_keymap();
        self.settings_view = None;
        self.list = None;
        self.list_actions.clear();
        self.refresh_settings_buffers();
        self.status(format!(
            "{} {} override for {}",
            if value.is_some() { "saved" } else { "removed" },
            setting.descriptor().title,
            scope.label()
        ));
        true
    }
}
