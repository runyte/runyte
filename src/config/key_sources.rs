// SPDX-License-Identifier: MPL-2.0
//! Locations captured with the accepted config text, never reread during discovery.
use serde_yaml::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Location {
    pub file: PathBuf,
    pub line: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct KeySources(pub BTreeMap<Vec<String>, Location>);

impl KeySources {
    pub(crate) fn merge(&mut self, source: &str, file: &Path, main: bool) {
        let Ok(document) = serde_yaml::from_str::<Value>(source) else {
            return;
        };
        let value = if main {
            document.get("keys").unwrap_or(&Value::Null)
        } else {
            &document
        };
        if value.is_null() {
            return;
        }
        let positions = crate::syntax::yaml_mapping_locations(source);
        fn apply(
            out: &mut KeySources,
            value: &Value,
            path: &mut Vec<String>,
            levels: usize,
            positions: &BTreeMap<Vec<String>, usize>,
            file: &Path,
            main: bool,
        ) {
            let mut lookup = if main {
                vec!["keys".into()]
            } else {
                Vec::new()
            };
            lookup.extend(path.clone());
            let line = (0..=lookup.len())
                .rev()
                .find_map(|n| positions.get(&lookup[..n]).copied());
            if levels == 0 || !value.is_mapping() {
                let removed = out
                    .0
                    .range(path.clone()..)
                    .take_while(|(p, _)| p.starts_with(path))
                    .map(|(p, _)| p.clone())
                    .collect::<Vec<_>>();
                for key in removed {
                    out.0.remove(&key);
                }
            }
            out.0.insert(
                path.clone(),
                Location {
                    file: file.to_owned(),
                    line,
                },
            );
            if levels > 0
                && let Some(mapping) = value.as_mapping()
            {
                for (k, v) in mapping {
                    if let Some(k) = k.as_str() {
                        path.push(k.into());
                        apply(out, v, path, levels - 1, positions, file, main);
                        path.pop();
                    }
                }
            }
        }
        if let Some(mapping) = value.as_mapping() {
            for (k, v) in mapping {
                if let Some(k) = k.as_str() {
                    let levels = match k {
                        "scopes" => 3,
                        "bind" | "ui" => 2,
                        "rebind" | "pointer" => 1,
                        _ => 0,
                    };
                    apply(self, v, &mut vec![k.into()], levels, &positions, file, main);
                }
            }
        } else {
            self.0.clear();
        }
    }
    pub fn location(&self, path: &[String]) -> Option<&Location> {
        self.0.get(path)
    }
}
