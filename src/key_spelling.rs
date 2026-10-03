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
    pub const MARKDOWN_SOURCE: &str = "rendered Markdown; {binding:?} returns to the source";

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
        MARKDOWN_SOURCE,
    ];
}

const MARKER_KINDS: [&str; 4] = ["key:", "binding:", "prefix:", "literal-key:"];

/// Makes text from outside Runyte, such as a plugin's help or labels, pass
/// through [`resolve_with_map`] unchanged. Only a brace that would open a
/// marker needs doubling; every other brace is already literal.
pub(crate) fn escape_markers(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for (at, character) in text.char_indices() {
        if character == '{'
            && MARKER_KINDS
                .iter()
                .any(|kind| text[at + 1..].starts_with(kind))
        {
            escaped.push('{');
        }
        escaped.push(character);
    }
    escaped
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
    let mut output_character = 0;
    while at < template.len() {
        offsets[source_character] = output_character;
        let rest = &template[at..];
        if ["{{key:", "{{binding:", "{{prefix:", "{{literal-key:"]
            .iter()
            .any(|prefix| rest.starts_with(prefix))
        {
            text.push('{');
            output_character += 1;
            source_character += 2;
            offsets[source_character - 1] = output_character - 1;
            offsets[source_character] = output_character;
            at += 2;
            continue;
        }
        if rest.starts_with('{')
            && let Some(kind) = MARKER_KINDS
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
            let start = output_character;
            text.push_str(&spelling);
            output_character += spelling.chars().count();
            substitutions.push(start..output_character);
            let consumed = rest[..=close].chars().count();
            for item in &mut offsets[source_character..source_character + consumed] {
                *item = start;
            }
            source_character += consumed;
            offsets[source_character] = output_character;
            at += close + 1;
            continue;
        }
        let character = rest.chars().next().expect("at is in bounds");
        text.push(character);
        at += character.len_utf8();
        source_character += 1;
        output_character += 1;
        offsets[source_character] = output_character;
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
    if let Some(original) = default_keymap().bindings().iter().find(|binding| {
        binding.scope == BindingScope::Global
            && binding.target.name() == name
            && binding.role == role
    }) {
        return binding_spelling(&original.sequence.to_string(), keymap);
    }
    let matches = keymap
        .bindings()
        .iter()
        .filter(|binding| {
            binding.scope == BindingScope::Global
                && binding.target.name() == name
                && binding.is_plain_action()
                && binding.role == role
                && matches!(binding.availability, BindingAvailability::Implemented)
        })
        .map(|binding| &binding.sequence)
        .collect::<HashSet<_>>();
    let mut matches = matches.into_iter().collect::<Vec<_>>();
    matches.sort_by_key(|sequence| (sequence.len(), sequence.to_string()));
    if let Some(sequence) = matches.first() {
        return Ok(sequence.to_string());
    }
    if default_keymap()
        .bindings()
        .iter()
        .any(|binding| binding.target.name() == name && binding.role == role)
    {
        return Ok(format!("[unbound: {name}]"));
    }
    Err(format!(
        "no implemented global {role:?} binding for {name:?}"
    ))
}

fn binding_spelling(body: &str, keymap: &Keymap) -> Result<String, String> {
    let default = KeySequence::parse(body)?;
    let originals = default_keymap()
        .bindings()
        .iter()
        .filter(|binding| binding.sequence == default || binding.alias.as_ref() == Some(&default))
        .collect::<Vec<_>>();
    if originals.is_empty() {
        return Err(format!("unknown default binding {body:?}"));
    }
    let original = originals[0];
    let mut by_mode = Vec::new();
    for &mode in original
        .alias_modes
        .filter(|_| original.alias.as_ref() == Some(&default))
        .unwrap_or(original.modes)
    {
        let mut candidates = keymap
            .bindings()
            .iter()
            .filter(|binding| {
                binding.is_plain_action()
                    && binding.target == original.target
                    && binding.scope == original.scope
                    && binding.is_active_in(mode)
            })
            .map(|binding| &binding.sequence)
            .collect::<Vec<_>>();
        candidates.sort_by_key(|sequence| (sequence.len(), sequence.to_string()));
        let chosen = keymap
            .spelling_for_default(&default)
            .filter(|preferred| candidates.contains(preferred))
            .or_else(|| candidates.first().copied());
        let spelling = chosen.map_or_else(
            || format!("[unbound: {}]", original.target.name()),
            ToString::to_string,
        );
        by_mode.push((mode, spelling));
    }
    if by_mode
        .iter()
        .all(|(_, spelling)| *spelling == by_mode[0].1)
    {
        return Ok(by_mode[0].1.clone());
    }
    Ok(by_mode
        .into_iter()
        .map(|(mode, spelling)| format!("{spelling} ({})", mode.label()))
        .collect::<Vec<_>>()
        .join(" / "))
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
        .map(|sequence| {
            if keymap
                .namespaces()
                .iter()
                .any(|namespace| namespace.sequence == *sequence)
            {
                sequence.to_string()
            } else {
                format!("[unbound prefix: {body}]")
            }
        })
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
    fn unicode_marker_offsets_and_escaped_boundaries_stay_exact() {
        let marker = "{literal-key:界}";
        let template = format!("λ{marker}e\u{301}{{{{key:nope}}");
        let (resolved, map) = resolve_with_map(&template, default_keymap()).unwrap();
        assert_eq!(resolved.text, "λ界e\u{301}{key:nope}");
        assert_eq!(resolved.substitutions, [1..2]);
        let after_marker = 1 + marker.chars().count();
        assert_eq!(map[0], 0);
        assert!(map[1..after_marker].iter().all(|offset| *offset == 1));
        assert_eq!(&map[after_marker..after_marker + 5], &[2, 3, 4, 4, 5]);
        assert_eq!(*map.last().unwrap(), resolved.text.chars().count());
    }

    #[test]
    fn large_unmarked_page_has_identity_character_offsets() {
        let template = "λe\u{301} literal prose\n".repeat(8192);
        let (resolved, map) = resolve_with_map(&template, default_keymap()).unwrap();
        assert_eq!(resolved.text, template);
        assert!(resolved.substitutions.is_empty());
        assert_eq!(map.len(), template.chars().count() + 1);
        assert!(
            map.iter()
                .enumerate()
                .all(|(index, mapped)| index == *mapped)
        );
    }

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
    fn escaped_text_resolves_to_itself() {
        for text in [
            "{key:nope}",
            "{binding:Space g l}",
            "{{prefix:x}",
            "{{{literal-key:Space x}}",
            "{n,m} {} {",
            "é{key:ß}",
            "{key:",
        ] {
            let escaped = escape_markers(text);
            let (resolved, map) = resolve_with_map(&escaped, default_keymap()).unwrap();
            assert_eq!(resolved.text, text);
            assert!(resolved.substitutions.is_empty());
            assert_eq!(map.len(), escaped.chars().count() + 1);
        }
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
