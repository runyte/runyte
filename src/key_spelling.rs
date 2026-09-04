// SPDX-License-Identifier: MPL-2.0

//! Resolves authored key markers against one active keymap.

use std::collections::HashSet;
use std::ops::Range;

use crate::input::KeyStroke;
use crate::keymap::{
    BindingAvailability, BindingRole, BindingScope, KeySequence, Keymap, default_keymap,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedText {
    pub text: String,
    /// Character ranges occupied by resolved key spellings.
    pub substitutions: Vec<Range<usize>>,
}

pub(crate) mod actionable {
    pub const COMPARE_DISK: &str = "{binding:Space b d}";
    pub const RELOAD: &str = "{binding:Space r}";
    pub const HELP: &str = "{binding:Space ?}";
    pub const MACRO_RECORD: &str = "{binding:Space m m}";
    pub const WORKTREE_MANAGER: &str = "{binding:Space g w}";
    pub const FILE_RELOAD_NOTE: &str = "{binding:Space b d} compares without discarding changes.";
    pub const STALE_SAVE: &str = "file changed on disk; {binding:Space b d} compares, {binding:Space r} reloads, and :write! replaces it";
    pub const STARTUP_HELP: &str = ":? or {binding:Space ?} for help";

    #[cfg(test)]
    pub const ALL: &[&str] = &[
        COMPARE_DISK,
        RELOAD,
        HELP,
        MACRO_RECORD,
        WORKTREE_MANAGER,
        FILE_RELOAD_NOTE,
        STALE_SAVE,
        STARTUP_HELP,
    ];
}

pub fn resolve(template: &str, keymap: &Keymap) -> Result<ResolvedText, String> {
    resolve_with_map(template, keymap).map(|(resolved, _)| resolved)
}

pub(crate) fn resolve_with_map(
    template: &str,
    keymap: &Keymap,
) -> Result<(ResolvedText, Vec<usize>), String> {
    let mut text = String::new();
    let mut substitutions = Vec::new();
    let mut offsets = vec![0; template.chars().count() + 1];
    let mut at = 0;
    let mut source_character = 0;
    while at < template.len() {
        offsets[source_character] = text.chars().count();
        let rest = &template[at..];
        if ["{{key:", "{{binding:", "{{prefix:", "{{literal-key:"]
            .iter()
            .any(|prefix| rest.starts_with(prefix))
        {
            text.push('{');
            source_character += 2;
            offsets[source_character - 1] = text.chars().count() - 1;
            offsets[source_character] = text.chars().count();
            at += 2;
            continue;
        }
        if rest.starts_with('{')
            && let Some(kind) = ["key:", "binding:", "prefix:", "literal-key:"]
                .into_iter()
                .find(|kind| rest[1..].starts_with(kind))
        {
            let Some(close) = rest.find('}') else {
                return Err(format!("unterminated {{{kind} marker"));
            };
            let body = &rest[1 + kind.len()..close];
            let spelling = match kind {
                "key:" => command_spelling(body, keymap)?,
                "binding:" => binding_spelling(body, keymap)?,
                "prefix:" => prefix_spelling(body, keymap)?,
                "literal-key:" => KeyStroke::parse(body)?.label(),
                _ => unreachable!(),
            };
            let start = text.chars().count();
            text.push_str(&spelling);
            substitutions.push(start..start + spelling.chars().count());
            let consumed = rest[..=close].chars().count();
            for item in &mut offsets[source_character..source_character + consumed] {
                *item = start;
            }
            source_character += consumed;
            offsets[source_character] = text.chars().count();
            at += close + 1;
            continue;
        }
        let character = rest.chars().next().expect("at is in bounds");
        text.push(character);
        at += character.len_utf8();
        source_character += 1;
        offsets[source_character] = text.chars().count();
    }
    Ok((
        ResolvedText {
            text,
            substitutions,
        },
        offsets,
    ))
}

fn command_spelling(body: &str, keymap: &Keymap) -> Result<String, String> {
    let mut pieces = body.split(':');
    let name = pieces.next().unwrap_or_default();
    let role = match pieces.next() {
        None | Some("primary") => BindingRole::Primary,
        Some("fast") => BindingRole::Fast,
        Some("compatibility") => BindingRole::Compatibility,
        Some(value) => return Err(format!("unknown binding role {value:?}")),
    };
    if pieces.next().is_some() {
        return Err(format!("invalid key marker {body:?}"));
    }
    let matches = keymap
        .bindings()
        .iter()
        .filter(|binding| {
            binding.scope == BindingScope::Global
                && binding.target.name() == name
                && binding.role == role
                && matches!(binding.availability, BindingAvailability::Implemented)
        })
        .map(|binding| &binding.sequence)
        .collect::<HashSet<_>>();
    match matches.into_iter().collect::<Vec<_>>().as_slice() {
        [sequence] => Ok(sequence.to_string()),
        [] => Err(format!(
            "no implemented global {role:?} binding for {name:?}"
        )),
        _ => Err(format!("ambiguous global {role:?} binding for {name:?}")),
    }
}

fn binding_spelling(body: &str, keymap: &Keymap) -> Result<String, String> {
    let default = KeySequence::parse(body)?;
    let exists = default_keymap()
        .bindings()
        .iter()
        .any(|binding| binding.sequence == default || binding.alias.as_ref() == Some(&default));
    if !exists {
        return Err(format!("unknown default binding {body:?}"));
    }
    keymap
        .spelling_for_default(&default)
        .map(ToString::to_string)
        .ok_or_else(|| format!("default binding {body:?} has no live spelling"))
}

fn prefix_spelling(body: &str, keymap: &Keymap) -> Result<String, String> {
    let default = KeySequence::parse(body)?;
    if !default_keymap()
        .namespaces()
        .iter()
        .any(|namespace| namespace.sequence == default)
    {
        return Err(format!("unknown default prefix {body:?}"));
    }
    keymap
        .spelling_for_default(&default)
        .map(ToString::to_string)
        .ok_or_else(|| format!("default prefix {body:?} has no live spelling"))
}

#[cfg(test)]
pub(crate) fn stale_namespace_literals(template: &str) -> Vec<String> {
    let mut unmarked = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        unmarked.push_str(&rest[..open]);
        rest = &rest[open..];
        let marker = ["{key:", "{binding:", "{prefix:", "{literal-key:"]
            .into_iter()
            .any(|prefix| rest.starts_with(prefix));
        if marker && let Some(close) = rest.find('}') {
            unmarked.extend(std::iter::repeat_n(' ', rest[..=close].chars().count()));
            rest = &rest[close + 1..];
        } else {
            unmarked.push('{');
            rest = &rest[1..];
        }
    }
    unmarked.push_str(rest);

    let mut defaults = default_keymap()
        .bindings()
        .iter()
        .map(|binding| &binding.sequence)
        .chain(default_keymap().namespaces().iter().map(|namespace| &namespace.sequence))
        .filter(|sequence| {
            matches!(sequence.as_slice().first(), Some(key) if *key == crate::keymap::Key::char(' ') || *key == crate::keymap::Key::ctrl('w'))
        })
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    defaults.sort_by_key(|sequence| std::cmp::Reverse(sequence.len()));
    defaults.dedup();
    defaults
        .into_iter()
        .filter(|sequence| {
            unmarked.match_indices(sequence).any(|(at, _)| {
                let previous = unmarked[..at].chars().next_back();
                let next = unmarked[at + sequence.len()..].chars().next();
                let word = |character: char| character.is_ascii_alphanumeric() || character == '_';
                !previous.is_some_and(word) && !next.is_some_and(word)
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn assert_authored_template(template: &str) {
    assert_eq!(
        stale_namespace_literals(template),
        Vec::<String>::new(),
        "unmarked remappable key in {template:?}"
    );
    for fast_pane_keys in [false, true] {
        let keymap = crate::keymap::keymap_for(fast_pane_keys);
        resolve(template, &keymap).unwrap_or_else(|error| {
            panic!("marker failed against fast_pane_keys={fast_pane_keys}: {error}")
        });
    }
}

#[cfg(test)]
mod tests {
    use serde_yaml::Value;

    use super::*;

    #[test]
    fn actionable_message_inventory_has_only_resolvable_marked_keys() {
        for template in actionable::ALL {
            assert_authored_template(template);
        }
    }

    #[test]
    fn markers_resolve_and_other_braces_remain_literal() {
        let rendered = resolve(
            "{binding:Space g l} {prefix:Space g} {literal-key:Space} {n,m}",
            default_keymap(),
        )
        .unwrap();
        assert_eq!(rendered.text, "Space g l Space g Space {n,m}");
        assert_eq!(rendered.substitutions.len(), 3);
        assert!(resolve("{literal-key:Space x}", default_keymap()).is_err());
        assert_eq!(
            resolve("{{binding:Space g l}", default_keymap())
                .unwrap()
                .text,
            "{binding:Space g l}"
        );
    }

    #[test]
    fn configured_spellings_follow_the_resolved_registry() {
        let value: Value =
            serde_yaml::from_str("leader: Ctrl-x\nrebind:\n  Space g: Leader G\n").unwrap();
        let compiled = crate::keymap::configured::compile(&value, default_keymap());
        let rendered = resolve("{binding:Space g l}", &compiled.keymap).unwrap();
        assert_eq!(rendered.text, "Ctrl-x G l");
    }

    #[test]
    fn sentinel_map_moves_both_prefixes_a_namespace_and_an_alias_in_both_variants() {
        let value: Value = serde_yaml::from_str(
            "leader: Ctrl-x\nwindow: Ctrl-a\nrebind:\n  Space g: Leader G\n  \",\": F12\n",
        )
        .unwrap();
        for fast in [false, true] {
            let built_in = crate::keymap::keymap_for(fast);
            let compiled = crate::keymap::configured::compile(&value, &built_in);
            assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
            let rendered = resolve(
                "{prefix:Space} {prefix:Ctrl-w} {prefix:Space g} {binding:Space g l} {binding:,}",
                &compiled.keymap,
            )
            .unwrap();
            assert_eq!(rendered.text, "Ctrl-x Ctrl-a Ctrl-x G Ctrl-x G l F12");
            for template in actionable::ALL {
                let rendered = resolve(template, &compiled.keymap).unwrap();
                assert!(!rendered.text.contains("{binding:"));
                assert!(!rendered.text.contains("Space"));
            }
        }
    }
}
