// SPDX-License-Identifier: MPL-2.0

//! Writing the effective configuration out as a complete YAML file.
//!
//! What is written is the saved configuration, [`App::persisted_config`],
//! rather than the running one. The two differ only for a setting read before
//! the editor existed and changed since: the running editor keeps its launch
//! value, but the file should carry the choice that was made, or writing it
//! back over the loaded file would quietly undo that choice.
//!
//! Custom themes and the `keys` section are written as files of their own
//! beside the configuration file, in `themes.d` and `keybinds.d`. The keys are
//! explicit assignments under `bind`, one file per mode, with unchanged
//! built-in bindings shown as commented examples.

use std::path::PathBuf;

use super::App;
use crate::settings::{SettingError, write_config};

impl App {
    /// Writes the configuration to `path`, or to the per-user configuration
    /// file when no path is given. An existing file is kept unless `replace`.
    pub(super) fn write_configuration(&mut self, path: Option<PathBuf>, replace: bool) {
        let Some(target) = path
            .map(|path| self.resolve_working_path(path))
            .or_else(|| self.default_config_target.clone())
        else {
            self.action_failed(
                "no configuration directory is known; set XDG_CONFIG_HOME or HOME, or give :write-config a path",
            );
            return;
        };
        match write_config(&target, &self.persisted_config, replace) {
            Ok(written) => {
                let beside = written.len().saturating_sub(1);
                self.status(format!(
                    "wrote configuration to {} and {beside} file{} beside it in {} and {}",
                    target.display(),
                    if beside == 1 { "" } else { "s" },
                    crate::config::THEMES_DIRECTORY,
                    crate::config::KEYBINDS_DIRECTORY,
                ))
            }
            Err(SettingError::AlreadyExists(path)) => self.action_warning(
                "Configuration not written",
                format!(
                    "{} already exists · :write-config! replaces it",
                    path.display()
                ),
            ),
            Err(error) => {
                self.error_from("Runyte", "Configuration write failed", error.to_string())
            }
        }
    }
}
