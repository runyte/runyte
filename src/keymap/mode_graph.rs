// SPDX-License-Identifier: MPL-2.0
//! Named binding modes, independent of the editor's temporary editing-state enum.
//!
//! A compiled graph has one parent per mode. Buffer types select a starting
//! identity; neither names nor buffer types add a second lookup dimension.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Bounds apply to the effective graph, including the selected preset.
pub const MAX_MODES: usize = 256;
pub const MAX_NAME_BYTES: usize = 64;
pub const MAX_INHERITANCE_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ModeId {
    graph: u64,
    index: usize,
}

static NEXT_GRAPH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum UnboundKeys {
    PassToBuffer,
    Overwrite,
    #[default]
    Report,
}

/// Only the stage 4 properties. Editing properties belong to stage 7.
#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Properties {
    #[serde(default)]
    pub inherits: Option<String>,
    #[serde(default)]
    pub unbound_keys: UnboundKeys,
}

pub type Definitions = BTreeMap<String, Properties>;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum EditingRole {
    Normal,
    Select,
    Insert,
    Replace,
}
impl EditingRole {
    pub const ALL: [Self; 4] = [Self::Normal, Self::Select, Self::Insert, Self::Replace];
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Select => "select",
            Self::Insert => "insert",
            Self::Replace => "replace",
        }
    }
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == name)
    }
    pub fn editing_mode(self) -> crate::command::Mode {
        use crate::command::Mode;
        match self {
            Self::Normal => Mode::Normal,
            Self::Select => Mode::Select,
            Self::Insert => Mode::Insert,
            Self::Replace => Mode::Replace,
        }
    }
}

/// Scalar shorthand supplies shared non-normal roles when those modes exist.
/// An explicit mapping supplies exactly its roles; omissions are unavailable.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(untagged)]
pub enum RoleNames {
    Normal(String),
    Explicit(BTreeMap<EditingRole, String>),
}
pub type BufferTypes = BTreeMap<String, RoleNames>;

#[derive(Clone, Debug)]
pub struct BindingMode {
    name: String,
    parent: Option<ModeId>,
    unbound_keys: UnboundKeys,
}

impl BindingMode {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn parent(&self) -> Option<ModeId> {
        self.parent
    }
    pub fn unbound_keys(&self) -> UnboundKeys {
        self.unbound_keys
    }
}

#[derive(Clone, Debug, Default)]
pub struct ModeGraph {
    generation: u64,
    modes: Vec<BindingMode>,
    names: HashMap<String, ModeId>,
    roles: BTreeMap<String, BTreeMap<EditingRole, ModeId>>,
}

pub(crate) fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return Err(format!(
            "invalid mode name {name:?}: expected 1 through {MAX_NAME_BYTES} ASCII letters, digits, '-' or '_'"
        ));
    }
    Ok(())
}

impl ModeGraph {
    /// Compile only after preset and user declarations have been layered.
    /// A binding table implicitly declares its mode, including an empty table.
    /// Entry targets alone do not make a misspelt mode valid (decision 22).
    pub fn compile(
        declarations: &Definitions,
        binding_modes: &[&str],
        entered_modes: &[&str],
        buffer_types: &BufferTypes,
        required_buffer_types: &[&str],
    ) -> Result<Self, String> {
        if declarations.len() > MAX_MODES || binding_modes.len() > MAX_MODES {
            return Err(format!("mode graph exceeds {MAX_MODES} modes"));
        }
        let mut definitions = declarations.clone();
        for &name in binding_modes {
            valid_name(name)?;
            definitions.entry(name.to_owned()).or_default();
            if definitions.len() > MAX_MODES {
                return Err(format!("mode graph exceeds {MAX_MODES} modes"));
            }
        }
        let mut graph = Self {
            generation: NEXT_GRAPH
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |n| n.checked_add(1),
                )
                .expect("mode graph identity exhausted"),
            ..Self::default()
        };
        for (name, properties) in &definitions {
            valid_name(name)?;
            let id = ModeId {
                graph: graph.generation,
                index: graph.modes.len(),
            };
            graph.names.insert(name.clone(), id);
            graph.modes.push(BindingMode {
                name: name.clone(),
                parent: None,
                unbound_keys: properties.unbound_keys,
            });
        }
        for (name, properties) in &definitions {
            let id = graph.names[name];
            if let Some(parent) = &properties.inherits {
                let parent_id = graph.id(parent).ok_or_else(|| {
                    format!("modes.{name}.inherits names missing mode {parent:?}")
                })?;
                graph.modes[id.index].parent = Some(parent_id);
            }
        }
        // Bounded traversal also detects self-parenting and longer cycles.
        for start in 0..graph.modes.len() {
            let mut seen = Vec::new();
            let mut next = Some(ModeId {
                graph: graph.generation,
                index: start,
            });
            while let Some(id) = next {
                if seen.contains(&id) {
                    return Err(format!(
                        "mode inheritance cycle from {:?} through {:?}",
                        graph.modes[start].name, graph.modes[id.index].name
                    ));
                }
                if seen.len() == MAX_INHERITANCE_DEPTH {
                    return Err(format!(
                        "mode {:?} exceeds inheritance depth {MAX_INHERITANCE_DEPTH}",
                        graph.modes[start].name
                    ));
                }
                seen.push(id);
                next = graph.modes[id.index].parent;
            }
        }
        for (id, mode) in graph.modes() {
            if !matches!(mode.name(), "list" | "session-manager")
                && graph.chain(id).any(|ancestor| {
                    matches!(
                        graph.get(ancestor).unwrap().name(),
                        "list" | "session-manager"
                    )
                })
            {
                return Err(format!(
                    "editor mode {:?} cannot inherit UI-owned list modes",
                    mode.name()
                ));
            }
        }
        for &name in entered_modes {
            if graph.id(name).is_none() {
                return Err(format!(
                    "entered mode {name:?} has neither a declaration nor bindings"
                ));
            }
        }
        for &name in required_buffer_types {
            if !buffer_types.contains_key(name) {
                return Err(format!("buffer-types.{name} requires a starting mode"));
            }
        }
        for (kind, names) in buffer_types {
            if !required_buffer_types.contains(&kind.as_str()) {
                return Err(format!("unknown buffer type {kind:?}"));
            }
            let names = match names {
                RoleNames::Normal(name) => {
                    let mut roles = BTreeMap::from([(EditingRole::Normal, name.clone())]);
                    for role in [
                        EditingRole::Select,
                        EditingRole::Insert,
                        EditingRole::Replace,
                    ] {
                        if graph.id(role.name()).is_some() {
                            roles.insert(role, role.name().to_owned());
                        }
                    }
                    roles
                }
                RoleNames::Explicit(roles) => roles.clone(),
            };
            if !names.contains_key(&EditingRole::Normal) {
                return Err(format!(
                    "buffer-types.{kind} requires a normal starting role"
                ));
            }
            let mut roles = BTreeMap::new();
            for (role, name) in names {
                let id = graph.id(&name).ok_or_else(|| {
                    format!(
                        "buffer-types.{kind}.{} names missing mode {name:?}",
                        role.name()
                    )
                })?;
                if matches!(name.as_str(), "list" | "session-manager") {
                    return Err(format!(
                        "buffer-types.{kind} cannot use UI-owned mode {name:?}"
                    ));
                }
                if roles.values().any(|prior| *prior == id) {
                    return Err(format!(
                        "buffer-types.{kind} assigns mode {name:?} to multiple editing roles"
                    ));
                }
                roles.insert(role, id);
            }
            graph.roles.insert(kind.clone(), roles);
        }
        Ok(graph)
    }

    pub fn id(&self, name: &str) -> Option<ModeId> {
        self.names.get(name).copied()
    }
    /// IDs belong to the immutable graph that issued them. A reload must
    /// resolve retained names against the replacement graph, never reuse IDs.
    pub fn get(&self, id: ModeId) -> Option<&BindingMode> {
        (id.graph == self.generation)
            .then(|| self.modes.get(id.index))
            .flatten()
    }
    pub fn starting_mode(&self, buffer_type: &str) -> Option<ModeId> {
        self.role_mode(buffer_type, EditingRole::Normal)
    }
    pub fn role_mode(&self, buffer_type: &str, role: EditingRole) -> Option<ModeId> {
        self.roles.get(buffer_type)?.get(&role).copied()
    }
    /// Role names take precedence over direct named submode entry.
    pub fn entry_mode(&self, buffer_type: &str, argument: &str) -> Option<ModeId> {
        if let Some(role) = EditingRole::named(argument) {
            self.role_mode(buffer_type, role)
        } else {
            self.id(argument)
        }
    }
    /// Stage 4 compatibility semantics. Lookup still uses only the named chain.
    pub fn editing_role(&self, buffer_type: &str, mode: ModeId) -> Option<EditingRole> {
        self.get(mode)?;
        let roles = self.roles.get(buffer_type)?;
        for ancestor in self.chain(mode) {
            if let Some((&role, _)) = roles.iter().find(|(_, id)| **id == ancestor) {
                return Some(role);
            }
        }
        Some(EditingRole::Normal)
    }
    pub fn modes(&self) -> impl Iterator<Item = (ModeId, &BindingMode)> {
        self.modes.iter().enumerate().map(|(index, mode)| {
            (
                ModeId {
                    graph: self.generation,
                    index,
                },
                mode,
            )
        })
    }
    /// The only binding search order: own table followed by each parent.
    /// The originating mode's unbound policy applies after this chain ends.
    pub fn chain(&self, mode: ModeId) -> impl Iterator<Item = ModeId> + '_ {
        std::iter::successors(self.get(mode).map(|_| mode), |id| {
            self.get(*id).and_then(BindingMode::parent)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn compile(
        modes: &str,
        bindings: &[&str],
        entered: &[&str],
        starts: &str,
    ) -> Result<ModeGraph, String> {
        ModeGraph::compile(
            &serde_yaml::from_str(modes).unwrap(),
            bindings,
            entered,
            &serde_yaml::from_str(starts).unwrap(),
            &["text", "terminal"],
        )
    }
    #[test]
    fn explicit_and_implicit_modes_have_one_chain_and_own_unbound_policy() {
        let graph = compile("common: {unbound-keys: pass-to-buffer}\nnormal: {inherits: common}\nselect: {inherits: normal}\ninsert: {unbound-keys: pass-to-buffer}\nreplace: {unbound-keys: overwrite}",
            &["common", "terminal"], &["select"], "text: normal\nterminal: terminal").unwrap();
        assert_eq!(
            graph
                .chain(graph.id("select").unwrap())
                .map(|id| graph.get(id).unwrap().name())
                .collect::<Vec<_>>(),
            ["select", "normal", "common"]
        );
        assert_eq!(
            graph
                .get(graph.id("select").unwrap())
                .unwrap()
                .unbound_keys(),
            UnboundKeys::Report
        );
        assert_eq!(
            graph
                .get(graph.id("insert").unwrap())
                .unwrap()
                .unbound_keys(),
            UnboundKeys::PassToBuffer
        );
        assert_eq!(
            graph
                .get(graph.id("replace").unwrap())
                .unwrap()
                .unbound_keys(),
            UnboundKeys::Overwrite
        );
        assert_eq!(graph.starting_mode("text"), graph.id("normal"));
        assert_eq!(graph.starting_mode("terminal"), graph.id("terminal"));
    }
    #[test]
    fn missing_parents_starts_and_entered_only_typos_are_rejected() {
        for (modes, entered, starts, expected) in [
            (
                "normal: {inherits: typo}",
                vec![],
                "text: normal\nterminal: normal",
                "missing mode",
            ),
            (
                "normal: {}",
                vec!["typo"],
                "text: normal\nterminal: normal",
                "neither a declaration nor bindings",
            ),
            (
                "normal: {}",
                vec![],
                "text: normal",
                "requires a starting mode",
            ),
            (
                "normal: {}",
                vec![],
                "text: typo\nterminal: normal",
                "missing mode",
            ),
            (
                "normal: {}",
                vec![],
                "text: normal\nterminal: normal\nunknown: normal",
                "unknown buffer type",
            ),
        ] {
            assert!(
                compile(modes, &[], &entered, starts)
                    .unwrap_err()
                    .contains(expected)
            );
        }
    }
    #[test]
    fn cycles_and_graph_bounds_are_rejected() {
        for modes in [
            "normal: {inherits: normal}",
            "normal: {inherits: other}\nother: {inherits: normal}",
        ] {
            assert!(
                compile(modes, &[], &[], "text: normal\nterminal: normal")
                    .unwrap_err()
                    .contains("cycle")
            );
        }
        let names = (0..=MAX_MODES)
            .map(|n| format!("mode-{n}"))
            .collect::<Vec<_>>();
        assert!(
            compile(
                "{}",
                &names.iter().map(String::as_str).collect::<Vec<_>>(),
                &[],
                "{}"
            )
            .unwrap_err()
            .contains("exceeds")
        );
        let chain = (0..MAX_INHERITANCE_DEPTH)
            .map(|n| format!("mode-{n}: {{inherits: mode-{}}}\n", n + 1))
            .collect::<String>();
        assert!(
            compile(
                &chain,
                &[&format!("mode-{MAX_INHERITANCE_DEPTH}")],
                &[],
                "{}"
            )
            .unwrap_err()
            .contains("depth")
        );
        assert!(
            compile("{}", &[&"a".repeat(MAX_NAME_BYTES + 1)], &[], "{}")
                .unwrap_err()
                .contains("invalid mode name")
        );
    }
    #[test]
    fn mode_ids_cannot_cross_reload_graphs() {
        let original = compile(
            "normal: {}",
            &["zzz"],
            &[],
            "text: normal\nterminal: normal",
        )
        .unwrap();
        let changed = compile("aaa: {}", &[], &[], "text: aaa\nterminal: aaa").unwrap();
        for id in [original.id("normal").unwrap(), original.id("zzz").unwrap()] {
            assert!(changed.get(id).is_none());
            assert_eq!(changed.chain(id).count(), 0);
            assert!(original.clone().get(id).is_some());
        }
    }

    #[test]
    fn buffer_roles_resolve_shared_entry_and_submode_semantics() {
        let graph = compile(
            "normal: {}\nselect: {}\ninsert: {}\nreplace: {}\nmarkdown: {}\nmarkdown-select: {inherits: markdown}\nsubmode: {inherits: markdown-select}\nother: {}",
            &[], &[], "text: normal\nterminal: {normal: markdown, select: markdown-select, insert: insert}",
        ).unwrap();
        assert_eq!(
            graph.entry_mode("terminal", "select"),
            graph.id("markdown-select")
        );
        assert_eq!(graph.entry_mode("text", "select"), graph.id("select"));
        assert_eq!(graph.entry_mode("terminal", "replace"), None);
        assert_eq!(graph.entry_mode("terminal", "submode"), graph.id("submode"));
        assert_eq!(
            graph.editing_role("terminal", graph.id("submode").unwrap()),
            Some(EditingRole::Select)
        );
        assert_eq!(
            graph.editing_role("terminal", graph.id("other").unwrap()),
            Some(EditingRole::Normal)
        );
        assert!(
            compile(
                "normal: {}",
                &[],
                &[],
                "text: normal\nterminal: {normal: normal, insert: typo}"
            )
            .unwrap_err()
            .contains("missing mode")
        );
        assert!(
            compile(
                "normal: {}",
                &[],
                &[],
                "text: normal\nterminal: {insert: normal}"
            )
            .unwrap_err()
            .contains("normal starting role")
        );
        assert!(
            compile(
                "normal: {}",
                &[],
                &[],
                "text: normal\nterminal: {normal: normal, insert: normal}"
            )
            .unwrap_err()
            .contains("multiple editing roles")
        );
        assert!(
            serde_yaml::from_str::<BufferTypes>("text: {normal: normal, typo: insert}").is_err()
        );
    }

    #[test]
    fn schema_rejects_unknown_properties_and_unbound_values() {
        for value in [
            "normal: {fallback: report}",
            "normal: {unbound-keys: insert}",
            "normal: {selecting-motions: true}",
            "normal: {inherits: [other]}",
        ] {
            assert!(serde_yaml::from_str::<Definitions>(value).is_err());
        }
    }
}
