// SPDX-License-Identifier: MPL-2.0

//! The directories beside a configuration file that add to it.
//!
//! `themes.d` holds one theme per file, named after the file. `keybinds.d`
//! holds fragments of the `keys` section. Both are read in file-name order
//! after the configuration file itself, so a later file wins over an earlier
//! one and over what the configuration file says inline.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde_yaml::{Mapping, Value};

use super::{Config, ThemeDefinition, custom_themes};

/// One theme per file, `<name>.yaml`.
pub const THEMES_DIRECTORY: &str = "themes.d";
/// Fragments of the `keys` section.
pub const KEYBINDS_DIRECTORY: &str = "keybinds.d";
/// The `keybinds.d` file `:write-config` gives everything other than `bind`.
pub const PREFIXES_FILE: &str = "00-prefixes.yaml";
/// A directory with more files than this is refused rather than read.
const MAX_FILES: usize = 256;
/// The modes written to a `keybinds.d` file of their own.
const BIND_MODES: [&str; 4] = ["normal", "select", "insert", "replace"];

impl Config {
    /// Adds the themes and key fragments found beside `path`, the
    /// configuration file, whether or not that file exists.
    pub(crate) fn merge_fragments(&mut self, path: &Path) -> Result<()> {
        self.merge_prospective_fragments(path, &[])
    }

    /// Read the destination as it would exist after replacing the generated
    /// files. Retained fragments participate in the same ordering and limits
    /// as an ordinary load, without creating files during validation.
    pub(crate) fn merge_prospective_fragments(
        &mut self,
        path: &Path,
        generated: &[(PathBuf, String)],
    ) -> Result<()> {
        let directory = path.parent().unwrap_or(Path::new("."));
        for (file, source) in prospective_files(directory, THEMES_DIRECTORY, generated)? {
            let source = source.cloned().map(Ok).unwrap_or_else(|| read(&file))?;
            let name = file
                .file_stem()
                .and_then(|stem| stem.to_str())
                .with_context(|| format!("theme file {} is not named in UTF-8", file.display()))?
                .to_owned();
            self.merge_theme_source(name, &source)
                .with_context(|| format!("invalid theme in {}", file.display()))?;
        }
        for (file, source) in prospective_files(directory, KEYBINDS_DIRECTORY, generated)? {
            let source = source.cloned().map(Ok).unwrap_or_else(|| read(&file))?;
            self.merge_keys_source(&source)
                .with_context(|| format!("invalid YAML in {}", file.display()))?;
        }
        Ok(())
    }

    pub(crate) fn merge_theme_source(&mut self, name: String, source: &str) -> Result<()> {
        let theme: ThemeDefinition = serde_yaml::from_str(source)?;
        self.themes.insert(name, theme);
        Ok(())
    }

    /// An empty file adds nothing. Anything else is merged into `keys`, where
    /// a malformed shape is reported by the keymap compiler like any other
    /// `keys` mistake rather than refusing the whole configuration.
    pub(crate) fn merge_keys_source(&mut self, source: &str) -> Result<()> {
        let fragment: Value = serde_yaml::from_str(source)?;
        if fragment.is_null() {
            return Ok(());
        }
        match &mut self.keys {
            Some(keys) => merge_keys(keys, fragment),
            None => self.keys = Some(fragment),
        }
        Ok(())
    }

    /// The configuration as the files `:write-config` writes: the
    /// configuration file itself as `file_name`, then each custom theme under
    /// `themes.d` and the `keys` section split by mode under `keybinds.d`.
    /// Paths are relative to the directory holding the configuration file.
    ///
    /// A theme whose name could not be a portable file name, or would collide
    /// with another on a case-insensitive filesystem, stays inline.
    pub fn to_files(&self, file_name: &Path) -> Result<Vec<(PathBuf, String)>> {
        let mut main = self.clone();
        let keys = main.keys.take();
        let mut taken = HashSet::new();
        let mut themes = Vec::new();
        for (name, theme) in custom_themes(&self.themes) {
            if is_portable_file_name(name) && taken.insert(name.to_ascii_lowercase()) {
                themes.push((
                    Path::new(THEMES_DIRECTORY).join(format!("{name}.yaml")),
                    serde_yaml::to_string(theme).context("failed to serialize a theme")?,
                ));
                main.themes.remove(name);
            }
        }
        let mut files = vec![(file_name.to_path_buf(), main.to_yaml()?)];
        files.extend(themes);
        let examples = crate::keymap::actions::default_bindings(keys.as_ref());
        let mut fragments = keys
            .map(split_keys)
            .unwrap_or_default()
            .into_iter()
            .map(|(name, fragment)| {
                Ok((
                    name,
                    serde_yaml::to_string(&fragment).context("failed to serialize keys")?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        for mode in BIND_MODES {
            let text = fragments.entry(format!("{mode}.yaml")).or_default();
            let entries = examples.get(mode).filter(|entries| !entries.is_empty());
            // A comment-only file remains empty YAML: exporting defaults alone
            // must not create a keys section or trigger configured compilation.
            // The file is written even with nothing in it, so that one left by
            // an earlier export cannot go on supplying keys.
            match (text.is_empty(), entries.is_some()) {
                (true, true) => {
                    text.push_str("# Built-in defaults. Uncomment bind, the mode, and entries to customize.\n# bind:\n");
                    text.push_str(&format!("#   {mode}:\n"));
                }
                (false, true) => text.push_str(
                    "\n# Built-in defaults. Uncomment individual entries to customize.\n",
                ),
                (true, false) => text.push_str(&format!(
                    "# No {mode} bindings, and keys.defaults is false, so no built-in ones.\n"
                )),
                (false, false) => {}
            }
            if let Some(entries) = entries {
                for line in serde_yaml::to_string(entries)?.lines() {
                    text.push_str("#     ");
                    text.push_str(line);
                    text.push('\n');
                }
            }
        }
        files.extend(
            fragments
                .into_iter()
                .map(|(name, text)| (Path::new(KEYBINDS_DIRECTORY).join(name), text)),
        );
        Ok(files)
    }
}

/// Merges a later `keys` fragment into an earlier one. `bind` merges per
/// mode and per key, `rebind` per key, and anything else is replaced whole,
/// so a later file can override one binding without restating the others.
fn merge_keys(into: &mut Value, fragment: Value) {
    let (Value::Mapping(into), Value::Mapping(fragment)) = (&mut *into, &fragment) else {
        *into = fragment;
        return;
    };
    for (key, value) in fragment.clone() {
        let levels = match key.as_str() {
            Some("bind") => 2,
            Some("rebind") => 1,
            _ => 0,
        };
        match into.get_mut(&key) {
            Some(existing) => merge_levels(existing, value, levels),
            None => {
                into.insert(key, value);
            }
        }
    }
}

fn merge_levels(into: &mut Value, value: Value, levels: usize) {
    match (into, value) {
        (Value::Mapping(into), Value::Mapping(value)) if levels > 0 => {
            for (key, value) in value {
                match into.get_mut(&key) {
                    Some(existing) => merge_levels(existing, value, levels - 1),
                    None => {
                        into.insert(key, value);
                    }
                }
            }
        }
        (into, value) => *into = value,
    }
}

/// `keys` as `keybinds.d` files: each bindable mode in a file of its own, and
/// everything else, including a shape the compiler will refuse, in
/// [`PREFIXES_FILE`] so it is still reported when the files are read.
fn split_keys(keys: Value) -> Vec<(String, Value)> {
    let Value::Mapping(mut prefixes) = keys else {
        return vec![(PREFIXES_FILE.to_owned(), keys)];
    };
    let mut files = Vec::new();
    if let Some(Value::Mapping(bind)) = prefixes.get_mut("bind") {
        for mode in BIND_MODES {
            if let Some(entries) = bind.remove(mode) {
                if entries.as_mapping().is_some_and(Mapping::is_empty) {
                    continue;
                }
                let mut modes = Mapping::new();
                modes.insert(mode.into(), entries);
                let mut fragment = Mapping::new();
                fragment.insert("bind".into(), Value::Mapping(modes));
                files.push((format!("{mode}.yaml"), Value::Mapping(fragment)));
            }
        }
        if bind.is_empty() {
            prefixes.remove("bind");
        }
    }
    if !prefixes.is_empty() {
        files.insert(0, (PREFIXES_FILE.to_owned(), Value::Mapping(prefixes)));
    }
    files
}

fn is_portable_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The `.yaml` files directly in `directory`, in name order. A missing
/// directory has none; hidden files, such as an editor's swap files, are
/// skipped.
fn yaml_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to list {}", directory.display()));
        }
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to list {}", directory.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || !name.ends_with(".yaml") {
            continue;
        }
        files.push(entry.path());
        if files.len() > MAX_FILES {
            bail!(
                "{} has more than {MAX_FILES} .yaml files",
                directory.display()
            );
        }
    }
    files.sort();
    Ok(files)
}

fn read(path: &Path) -> Result<String> {
    Config::read_source(path).with_context(|| format!("failed to read config {}", path.display()))
}

fn prospective_files<'a>(
    directory: &Path,
    kind: &str,
    generated: &'a [(PathBuf, String)],
) -> Result<BTreeMap<PathBuf, Option<&'a String>>> {
    let folder = directory.join(kind);
    let mut files = yaml_files(&folder)?
        .into_iter()
        .map(|path| (path, None))
        .collect::<BTreeMap<_, _>>();
    for (relative, source) in generated {
        if relative.parent() == Some(Path::new(kind)) {
            let target = directory.join(relative);
            files.insert(target, Some(source));
        }
    }
    if files.len() > MAX_FILES {
        bail!("{} has more than {MAX_FILES} .yaml files", folder.display());
    }
    Ok(files)
}

#[cfg(test)]
#[path = "tests/fragments.rs"]
mod tests;
