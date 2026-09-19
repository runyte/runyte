// SPDX-License-Identifier: MPL-2.0

//! Host-owned workspace permission and the shared choice overlay that edits it.

use super::{App, ListAction, ListPicker, PickerItem};
use crate::lsp_trust::TrustStore;
use std::path::PathBuf;

impl App {
    /// Called by production startup before attaching any language services.
    /// Storage is injected so tests never consult or write the user's records.
    pub fn configure_lsp_trust(&mut self, directory: Option<PathBuf>) {
        self.configure_lsp_trust_store(TrustStore::new(directory, &self.project_root));
    }

    pub(super) fn configure_lsp_trust_store(&mut self, store: std::io::Result<TrustStore>) {
        self.lsp_workspace_allowed = false;
        self.lsp_trust_error = None;
        self.lsp_trust = match store {
            Ok(store) => Some(store),
            Err(error) => {
                self.lsp_trust_error = Some(format!("LSP permission storage unavailable: {error}"));
                self.action_warning("LSP permission unavailable", error.to_string());
                None
            }
        };
        let decision = self.lsp_trust.as_ref().map(TrustStore::load).transpose();
        let decision = match decision {
            Ok(decision) => decision.flatten(),
            Err(error) => {
                self.lsp_trust_error = Some(format!("Cannot read LSP permission: {error}"));
                self.action_warning("LSP permission could not be read", error.to_string());
                None
            }
        };
        self.lsp_workspace_allowed = decision == Some(true) && self.config.lsp.enable;
        if decision.is_none() && self.config.lsp.enable {
            self.show_lsp_trust();
        }
    }

    pub(super) fn open_lsp_trust(&mut self) {
        // Explicit reopening retries storage, allowing recovery after the user
        // fixes a transient read/write failure without restarting the editor.
        if let Some(store) = &self.lsp_trust {
            self.lsp_trust_error = store
                .load()
                .err()
                .map(|error| format!("Cannot read LSP permission: {error}"));
        }
        self.show_lsp_trust();
    }

    fn show_lsp_trust(&mut self) {
        if !self.config.lsp.enable {
            self.mark_unavailable(
                "LSP is disabled in configuration; enable lsp.enable and restart first",
            );
            return;
        }
        if self.settings_view.is_some() {
            self.cancel_settings_picker();
        }
        let remember = self.lsp_trust_error.is_none()
            && self
                .lsp_trust
                .as_ref()
                .is_some_and(TrustStore::can_remember);
        if !remember && self.lsp_trust_error.is_none() {
            self.lsp_trust_error = Some("LSP permission storage unavailable".to_owned());
        }
        let mut explanation =
            "Language servers may execute code from this project with your permissions.\n\n\
            Permission covers every configured language server in this exact workspace.\n\n\
            Editing and syntax highlighting remain available with LSP disabled.\n\n\
            You can change this decision later with :lsp-trust."
                .to_owned();
        if let Some(error) = &self.lsp_trust_error {
            explanation = format!(
                "{error}\n\nOnly temporary choices are available. Keeping LSP disabled for now does not change a remembered decision. Allow LSP once must clear any remembered decision first.\n\n{explanation}"
            );
        }
        let mut items = vec![
            PickerItem::new(
                if remember {
                    "Keep LSP disabled"
                } else {
                    "Keep LSP disabled for now"
                },
                if remember {
                    "Remember this decision"
                } else {
                    "Until this editor or persistent host stops"
                },
                0,
            )
            .with_preview(&explanation),
            PickerItem::new(
                "Allow LSP once",
                "Until this editor or persistent host stops",
                1,
            )
            .with_preview(&explanation),
        ];
        self.list_actions = vec![
            ListAction::LspTrust {
                allowed: false,
                remember,
            },
            ListAction::LspTrust {
                allowed: true,
                remember: false,
            },
        ];
        if remember {
            items.push(
                PickerItem::new("Always allow LSP", "Remember for this exact workspace", 2)
                    .with_preview(&explanation),
            );
            self.list_actions.push(ListAction::LspTrust {
                allowed: true,
                remember: true,
            });
        }
        self.list = Some(
            ListPicker::new("Run language servers for this workspace?", items)
                .with_column_header(
                    format!("Workspace: {}", self.project_root.display()),
                    "",
                    "",
                )
                .as_choice("apply permission")
                .with_preview("Before you allow LSP"),
        );
    }

    pub(super) fn choose_lsp_trust(&mut self, allowed: bool, remember: bool) {
        if remember {
            let saved = self
                .lsp_trust
                .as_ref()
                .ok_or_else(|| {
                    std::io::Error::other("private LSP permission storage is unavailable")
                })
                .and_then(|store| store.save(allowed));
            if let Err(error) = saved {
                // A failed durable grant does not enable execution. A refused
                // revocation write still stops this host's current servers.
                if !allowed {
                    self.set_lsp_workspace_allowed(false);
                }
                self.lsp_trust_failed(format!("Cannot remember LSP permission: {error}"));
                return;
            }
        }
        if !remember
            && allowed
            && let Some(store) = &self.lsp_trust
            && let Err(error) = store.forget()
        {
            self.lsp_trust_failed(format!("Cannot clear remembered LSP permission: {error}"));
            return;
        }
        self.set_lsp_workspace_allowed(allowed);
        self.list = None;
        self.list_actions.clear();
        self.status(if allowed {
            "LSP allowed for this workspace"
        } else {
            "LSP disabled for this workspace"
        });
    }

    fn lsp_trust_failed(&mut self, message: String) {
        self.action_failed(message.clone());
        self.lsp_trust_error = Some(message);
        self.show_lsp_trust();
    }

    fn set_lsp_workspace_allowed(&mut self, allowed: bool) {
        self.lsp_workspace_allowed = allowed && self.config.lsp.enable;
        if let Some(handle) = &self.ports.lsp {
            handle.set_allowed(self.lsp_workspace_allowed);
        }
        if self.lsp_workspace_allowed {
            for buffer in 0..self.buffers.len() {
                self.lsp_touch(buffer);
            }
        } else {
            self.lsp_servers.clear();
            self.lsp_documents.clear();
            self.lsp_requests.clear();
            self.pending_lsp_replies.clear();
            self.diagnostics = Default::default();
            self.completion = None;
            self.signature = None;
            self.hover = None;
            self.lsp_actions.clear();
            self.lsp_action_source = None;
        }
    }
}
