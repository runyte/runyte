// SPDX-License-Identifier: MPL-2.0

//! Re-reading the loaded configuration file into a running editor.
//!
//! The native settings page already applies most settings the moment they are
//! saved, because it holds both halves of the change: the value being written
//! and the running editor to apply it to. An edit made in another editor has
//! only the file, so this module supplies the missing half. It re-reads the
//! same path that was loaded at startup — including an explicit `--config`
//! path — and brings the running editor into line with it.
//!
//! Two rules shape everything here. A replacement that cannot be parsed,
//! validated, or resolved to a theme leaves the working configuration exactly
//! as it was: a reload can fail, but it cannot break a running editor. And a
//! setting that was read before this editor existed is reported as needing a
//! restart rather than silently adopted, so nothing claims to be in effect
//! that is not. The file's value is still recorded as the saved one, which is
//! what makes the settings page show it as saved while the effective column
//! keeps naming what the editor is actually doing.
//!
//! Services are the host's half of the same operation. Language servers are
//! reconfigured from here because the editor owns that handle directly;
//! configured plugins are left to [`crate::workspace::host`], which owns their
//! processes and the work in flight that a restart would interrupt.

use super::App;
use crate::{
    config::Config,
    lsp::LspCommand,
    notification::{NotificationDraft, NotificationSeverity},
};

/// A setting whose value was read before there was an editor to apply it to.
///
/// Adopting one of these would make the settings page, the manual, and the
/// running editor disagree: the value would read as effective while the thing
/// it configures kept doing what it was told at startup. A reload therefore
/// keeps the running value and names the key, and the file's value still
/// reaches [`App::persisted_config`] so it is offered as the saved one.
struct StartupBound {
    key: &'static str,
    /// Whether the loaded file asks for something other than what is running.
    differs: fn(&Config, &Config) -> bool,
    /// Puts the running value back into the configuration about to be adopted.
    restore: fn(&mut Config, &Config),
}

const STARTUP_BOUND: &[StartupBound] = &[
    StartupBound {
        key: "editor.mouse",
        differs: |loaded, running| loaded.editor.mouse != running.editor.mouse,
        restore: |loaded, running| loaded.editor.mouse = running.editor.mouse,
    },
    StartupBound {
        key: "lsp.enable",
        differs: |loaded, running| loaded.lsp.enable != running.lsp.enable,
        restore: |loaded, running| loaded.lsp.enable = running.lsp.enable,
    },
    StartupBound {
        key: "workspace.mode",
        differs: |loaded, running| loaded.workspace.mode != running.workspace.mode,
        restore: |loaded, running| loaded.workspace.mode = running.workspace.mode,
    },
    StartupBound {
        key: "workspace.state",
        differs: |loaded, running| loaded.workspace.state != running.workspace.state,
        restore: |loaded, running| loaded.workspace.state.clone_from(&running.workspace.state),
    },
    StartupBound {
        key: "workspace.state_anchor",
        differs: |loaded, running| loaded.workspace.state_anchor != running.workspace.state_anchor,
        restore: |loaded, running| loaded.workspace.state_anchor = running.workspace.state_anchor,
    },
];

impl App {
    /// Re-reads the loaded configuration file and applies what can be applied.
    ///
    /// A refusal reports why through the interaction line and changes nothing.
    pub(super) fn reload_configuration(&mut self) {
        let Some(path) = self.config_path.clone() else {
            self.action_failed("no configuration file is loaded; nothing to reload");
            return;
        };
        let loaded = match Config::reload(&path) {
            Ok(Some(loaded)) => loaded,
            Ok(None) if self.config_file_read => {
                self.error_from(
                    "Runyte",
                    "Configuration reload failed",
                    format!(
                        "config {} no longer exists · the running configuration is unchanged",
                        path.display()
                    ),
                );
                return;
            }
            Ok(None) => {
                // Nothing was ever read from this path, so the built-in
                // defaults already in effect are what it would reload to.
                self.status(format!("no configuration file at {}", path.display()));
                return;
            }
            Err(error) => {
                self.error_from(
                    "Runyte",
                    "Configuration reload failed",
                    format!("{error:#} · the running configuration is unchanged"),
                );
                return;
            }
        };

        let mut effective = loaded.clone();
        let mut restart_required = Vec::new();
        for bound in STARTUP_BOUND {
            if (bound.differs)(&effective, &self.config) {
                restart_required.push(bound.key);
                (bound.restore)(&mut effective, &self.config);
            }
        }

        // Resolved before anything is replaced. A name that cannot be built
        // keeps the theme already on screen rather than falling back to the
        // built-in default the way a fresh start does: a typo is not a reason
        // to repaint a running editor, and the rest of the file is still
        // perfectly good. The failure is reported and the unresolvable name
        // stays visible as the saved value.
        let requested = effective
            .theme
            .clone()
            .unwrap_or_else(|| crate::config::DEFAULT_THEME.to_owned());
        let resolved = effective.resolve_theme(&requested);
        let theme_error = resolved
            .as_ref()
            .err()
            .map(|error| format!("{requested}: {error:#}"));
        let theme = resolved.ok().map(|theme| (requested, theme));
        let keys = super::compile_configured_keymaps(effective.keys.as_ref());

        let plugins_changed = effective.plugins != self.config.plugins;
        let grammar_changed = effective.editor.grammar != self.grammar.kind();
        // The three settings `ListingView::from_config` reads. Re-reading an
        // explorer costs it its remembered view and jump history, so a reload
        // that changed none of them must leave every listing alone.
        let explorers_changed = effective.editor.show_hidden_files
            != self.config.editor.show_hidden_files
            || effective.editor.explorer_sort != self.config.editor.explorer_sort
            || effective.editor.explorer_details != self.config.editor.explorer_details;
        let history_limit_changed =
            effective.notifications.history_limit != self.config.notifications.history_limit;

        self.persisted_config = loaded;
        self.config = effective;
        self.config_file_read = true;
        // Dispatch, help, and key hints all read `self.keymap`, so replacing
        // the compiled maps and re-selecting through `sync_keymap` is what
        // keeps them describing the same bindings after a `keys` change.
        //
        // The compiled maps also carry the keys of every plugin running right
        // now, which a freshly compiled section knows nothing about, so they
        // are laid back over it. A section that collides with one of them is
        // refused on its own, exactly as a malformed entry is: the rest of the
        // file still applies, and dispatch keeps a keymap the live plugins
        // were validated against, which is what every later rebuild of theirs
        // assumes.
        let previous_keymaps = self.configured_keymaps.take();
        self.configured_keymaps = keys.keymaps;
        self.sync_keymap();
        let key_collision = match self.plugin_keymaps(&self.plugins.commands) {
            Ok(maps) => {
                self.install_plugin_keymaps(maps);
                None
            }
            Err(error) => {
                self.configured_keymaps = previous_keymaps;
                self.sync_keymap();
                Some(format!("{error:#}"))
            }
        };
        if let Some((name, theme)) = theme {
            self.replace_theme(name, theme);
        }
        if grammar_changed {
            self.select_grammar(self.config.editor.grammar);
        }
        if history_limit_changed {
            self.notifications
                .set_limit(self.config.notifications.history_limit);
            self.refresh_notification_buffers();
        }
        let listing_error = explorers_changed
            .then(|| self.refresh_listings(None).err())
            .flatten();
        self.refresh_settings_buffers();

        // Sent on every reload rather than only when the definitions look
        // changed. The manager compares them itself and disturbs only what
        // differs, so an unconditional send costs one message and removes the
        // question of what a dropped one leaves behind: running the command
        // again simply sends it again. It goes out whatever the workspace
        // permission is, because a denied manager holds the definitions a
        // later approval would start servers from.
        let lsp_refused = self
            .ports
            .send_lsp(LspCommand::Reconfigure(Box::new(self.config.lsp.clone())))
            == Some(false);
        if plugins_changed {
            self.plugins.configuration_reload = true;
        }

        if !keys.errors.is_empty() {
            self.push_notification(NotificationDraft::new(
                NotificationSeverity::Error,
                "Configuration",
                "Key bindings",
                crate::keymap::configured::format_errors(&keys.errors),
            ));
        }
        if let Some(error) = &key_collision {
            self.push_notification(NotificationDraft::new(
                NotificationSeverity::Error,
                "Configuration",
                "Key bindings",
                format!(
                    "{error} · the keys section was not applied; the bindings in use are unchanged"
                ),
            ));
        }
        if let Some(error) = &theme_error {
            self.push_notification(NotificationDraft::new(
                NotificationSeverity::Warning,
                "Runyte",
                "Theme unavailable",
                format!("{error} · keeping {}", self.theme_name),
            ));
        }
        if lsp_refused {
            self.push_notification(NotificationDraft::new(
                NotificationSeverity::Warning,
                "LSP",
                "Language servers not reconfigured",
                "the language server manager did not accept the new definitions; the running servers keep the ones they were started with, and :config-reload retries",
            ));
        }
        if let Some(error) = &listing_error {
            self.push_notification(NotificationDraft::new(
                NotificationSeverity::Warning,
                "Runyte",
                "Explorer refresh failed",
                error.to_string(),
            ));
        }
        if !restart_required.is_empty() {
            self.info_from(
                "Runyte",
                "Restart required",
                format!(
                    "{} {} read at startup; this session keeps the value it launched with, and the saved value applies on the next launch",
                    restart_required.join(", "),
                    if restart_required.len() == 1 { "is" } else { "are" },
                ),
            );
        }

        let mut parts = Vec::new();
        if key_collision.is_some() {
            parts.push("keys section refused; bindings unchanged".to_owned());
        }
        if lsp_refused {
            parts.push("language servers not reconfigured".to_owned());
        }
        if listing_error.is_some() {
            parts.push("explorer refresh failed".to_owned());
        }
        if theme_error.is_some() {
            parts.push(format!("theme unavailable; keeping {}", self.theme_name));
        }
        if keys.rejected != 0 {
            parts.push(format!("{} key binding entries rejected", keys.rejected));
        }
        if !restart_required.is_empty() {
            parts.push(format!(
                "restart required for {}",
                restart_required.join(", ")
            ));
        }
        let summary = format!("reloaded config: {}", path.display());
        if parts.is_empty() {
            self.status(summary);
        } else {
            self.status(format!("{summary} · {}", parts.join(" · ")));
        }
    }
}
