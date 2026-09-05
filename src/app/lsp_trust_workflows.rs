// SPDX-License-Identifier: MPL-2.0

//! Host-owned workspace permission and the shared choice overlay that edits it.

use super::{App, ListAction, ListPicker, PickerItem};
use crate::lsp_trust::TrustStore;
use std::path::PathBuf;

impl App {
    /// Called by production startup before attaching any language services.
    /// Storage is injected so tests never consult or write the user's records.
    pub fn configure_lsp_trust(&mut self, directory: Option<PathBuf>) {
        self.lsp_workspace_allowed = false;
        self.lsp_trust = match TrustStore::new(directory, &self.project_root) {
            Ok(store) => Some(store),
            Err(error) => {
                self.action_warning("LSP permission unavailable", error.to_string());
                None
            }
        };
        let decision = self.lsp_trust.as_ref().map(TrustStore::load).transpose();
        let decision = match decision {
            Ok(decision) => decision.flatten(),
            Err(error) => {
                self.action_warning("LSP permission could not be read", error.to_string());
                None
            }
        };
        self.lsp_workspace_allowed = decision == Some(true) && self.config.lsp.enable;
        if decision.is_none() && self.config.lsp.enable {
            self.open_lsp_trust();
        }
    }

    pub(super) fn open_lsp_trust(&mut self) {
        if !self.config.lsp.enable {
            self.mark_unavailable(
                "LSP is disabled in configuration; enable lsp.enable and restart first",
            );
            return;
        }
        if self.settings_view.is_some() {
            self.cancel_settings_picker();
        }
        let explanation = "Language servers may execute code from this project with your permissions.\n\n\
            Permission covers every configured language server in this exact workspace.\n\n\
            Editing and syntax highlighting remain available with LSP disabled.\n\n\
            You can change this decision later with :lsp-trust.";
        self.list = Some(
            ListPicker::new(
                "Run language servers for this workspace?",
                vec![
                    PickerItem::new("Keep LSP disabled", "Remember this decision", 0)
                        .with_preview(explanation),
                    PickerItem::new(
                        "Allow LSP once",
                        "Until this editor or persistent host stops",
                        1,
                    )
                    .with_preview(explanation),
                    PickerItem::new("Always allow LSP", "Remember for this exact workspace", 2)
                        .with_preview(explanation),
                ],
            )
            .with_column_header(
                format!("Workspace: {}", self.project_root.display()),
                "",
                "",
            )
            .as_choice("apply permission")
            .with_preview("Before you allow LSP"),
        );
        self.list_actions = vec![
            ListAction::LspTrust {
                allowed: false,
                remember: true,
            },
            ListAction::LspTrust {
                allowed: true,
                remember: false,
            },
            ListAction::LspTrust {
                allowed: true,
                remember: true,
            },
        ];
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
                self.action_failed(format!("cannot remember LSP permission: {error}; choose Allow LSP once for a temporary grant"));
                return;
            }
        }
        if !remember
            && let Some(store) = &self.lsp_trust
            && let Err(error) = store.forget()
        {
            self.action_failed(format!("cannot clear remembered LSP permission: {error}"));
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
