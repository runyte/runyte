// SPDX-License-Identifier: MPL-2.0

//! Default configuration discovery, independent of editor and service startup.

use std::{ffi::OsString, path::PathBuf};

/// Directory containing Runyte's default per-user configuration file.
/// An explicit `--config` bypasses this discovery in the configuration loader.
pub fn default_config_root() -> Option<PathBuf> {
    config_root_from(|name| std::env::var_os(name))
}

fn config_root_from(mut environment: impl FnMut(&str) -> Option<OsString>) -> Option<PathBuf> {
    let mut nonempty = |name| environment(name).filter(|value| !value.is_empty());
    let root = nonempty("XDG_CONFIG_HOME").map(PathBuf::from);
    #[cfg(windows)]
    let root = root.or_else(|| nonempty("APPDATA").map(PathBuf::from));
    #[cfg(not(windows))]
    let root = root.or_else(|| nonempty("HOME").map(|home| PathBuf::from(home).join(".config")));
    root.map(|root| root.join("runyte"))
}

#[cfg(test)]
#[path = "tests/paths.rs"]
mod tests;
