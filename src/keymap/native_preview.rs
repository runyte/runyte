// SPDX-License-Identifier: MPL-2.0
//! Bounded semantic navigation for bundled document previews. Media contributes
//! local navigation; the actual source scope and mode own forwarded commands.
use super::{Binding, BindingAvailability, BindingScope, BindingTarget, KeySequence, Keymap, ui};
use crate::{
    command::{EditorCommand as C, Mode},
    input::{KeyCode, KeyStroke, Modifiers},
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Down,
    Up,
    Left,
    Right,
    PageDown,
    PageUp,
    HalfPageDown,
    HalfPageUp,
    Top,
    Bottom,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    Copy,
    Back,
    Forward,
    DismissAndForward,
    ForwardInput,
}
impl Action {
    pub const ALL: [Self; 18] = [
        Self::Down,
        Self::Up,
        Self::Left,
        Self::Right,
        Self::PageDown,
        Self::PageUp,
        Self::HalfPageDown,
        Self::HalfPageUp,
        Self::Top,
        Self::Bottom,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::ResetZoom,
        Self::Copy,
        Self::Back,
        Self::Forward,
        Self::DismissAndForward,
        Self::ForwardInput,
    ];
    pub fn description(self) -> &'static str {
        match self {
            Self::Down => "Scroll down",
            Self::Up => "Scroll up",
            Self::Left => "Pan left",
            Self::Right => "Pan right",
            Self::PageDown => "Page down",
            Self::PageUp => "Page up",
            Self::HalfPageDown => "Half page down",
            Self::HalfPageUp => "Half page up",
            Self::Top => "Document start",
            Self::Bottom => "Document end",
            Self::ZoomIn => "Zoom in",
            Self::ZoomOut => "Zoom out",
            Self::ResetZoom => "Reset zoom",
            Self::Copy => "Copy selection",
            Self::Back => "Return to source",
            Self::Forward | Self::ForwardInput => "Editor input",
            Self::DismissAndForward => "Return to source and continue",
        }
    }
}
fn local(binding: &Binding) -> Option<Action> {
    if !binding.is_plain_action() || binding.availability != BindingAvailability::Implemented {
        return None;
    }
    let BindingTarget::Editor(command) = binding.target else {
        return None;
    };
    Some(match command {
        C::MediaMoveDown | C::MoveDown | C::ScrollViewDown => Action::Down,
        C::MediaMoveUp | C::MoveUp | C::ScrollViewUp => Action::Up,
        C::MoveLeft | C::MediaPanLeft => Action::Left,
        C::MoveRight | C::MediaPanRight => Action::Right,
        C::PageDown => Action::PageDown,
        C::PageUp => Action::PageUp,
        C::HalfPageDown => Action::HalfPageDown,
        C::HalfPageUp => Action::HalfPageUp,
        C::MoveFileStart => Action::Top,
        C::MoveFileEnd => Action::Bottom,
        C::MediaZoomIn => Action::ZoomIn,
        C::MediaZoomOut => Action::ZoomOut,
        C::MediaFit | C::MediaActualSize => Action::ResetZoom,
        C::MediaCopySelection => Action::Copy,
        C::MediaBack => Action::Back,
        _ => return None,
    })
}
fn pane(binding: &Binding) -> bool {
    binding.is_plain_action()
        && binding.availability == BindingAvailability::Implemented
        && binding.target.id().category() == crate::command::CommandCategory::Window
}
pub(super) fn legacy_ui(key: KeyStroke) -> Option<ui::Action> {
    use ui::Action::*;
    if (key.modifiers == Modifiers::CONTROL
        && matches!(
            key.code,
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
        ))
        || super::is_fast_pane_key(key)
    {
        return Some(ForwardPane);
    }
    if key.code == KeyCode::Escape {
        return Some(Back);
    }
    if matches!(key.code, KeyCode::Char('c' | 'C'))
        && key
            .modifiers
            .intersects(Modifiers::CONTROL | Modifiers::SUPER)
    {
        return Some(Copy);
    }
    if matches!(key.code, KeyCode::Char(':' | ' '))
        || key.code == KeyCode::Char('w') && key.modifiers.contains(Modifiers::CONTROL)
    {
        return Some(KeepOpenAndContinue);
    }
    None
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Bindings {
    entries: Arc<[(KeySequence, Action)]>,
    /// Exact raw compatibility keys; unlike registry keys these keep Shift.
    fallback: Arc<[(KeyStroke, Action)]>,
    index: Arc<HashMap<KeySequence, Action>>,
    prefixes: Arc<HashSet<KeySequence>>,
    pub unavailable: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lookup {
    Action(Action),
    Prefix,
    None,
}
impl Bindings {
    pub const MAX_ENTRIES: usize = 512;
    pub const MAX_FALLBACK: usize = 512;
    pub fn from_entries(
        entries: Vec<(KeySequence, Action)>,
        fallback: Vec<(KeyStroke, Action)>,
    ) -> Result<Self, String> {
        if entries.len() > Self::MAX_ENTRIES || fallback.len() > Self::MAX_FALLBACK {
            return Err("too many preview bindings".into());
        }
        let mut seen = HashSet::new();
        for (sequence, action) in &entries {
            if sequence.is_empty()
                || sequence.len() > 8
                || sequence
                    .as_slice()
                    .iter()
                    .any(|key| *key != key.canonical_for_binding())
                || !seen.insert(sequence)
            {
                return Err("invalid or duplicate preview sequence".into());
            }
            if matches!(
                action,
                Action::Forward | Action::ForwardInput | Action::DismissAndForward
            ) && sequence.len() != 1
            {
                return Err("preview forwarding must own its first stroke".into());
            }
        }
        for (sequence, _) in &entries {
            for len in 1..sequence.len() {
                if seen.contains(&KeySequence::new(
                    sequence.as_slice()[..len].iter().copied(),
                )) {
                    return Err("overlapping preview sequences".into());
                }
            }
        }
        let mut seen = HashSet::new();
        for (key, action) in &fallback {
            if !seen.insert(*key)
                || !matches!(
                    action,
                    Action::Back | Action::Copy | Action::Forward | Action::ForwardInput
                )
            {
                return Err("invalid preview fallback".into());
            }
        }
        let index = entries.iter().cloned().collect();
        let prefixes = entries
            .iter()
            .flat_map(|(sequence, _)| {
                (1..sequence.len())
                    .map(|len| KeySequence::new(sequence.as_slice()[..len].iter().copied()))
            })
            .collect();
        Ok(Self {
            entries: entries.into(),
            fallback: fallback.into(),
            index: Arc::new(index),
            prefixes: Arc::new(prefixes),
            unavailable: false,
        })
    }
    pub fn entries(&self) -> &[(KeySequence, Action)] {
        &self.entries
    }
    pub fn fallback(&self) -> &[(KeyStroke, Action)] {
        &self.fallback
    }
    pub fn lookup(&self, keys: &KeySequence) -> Lookup {
        if let Some(action) = self.index.get(keys) {
            Lookup::Action(*action)
        } else if self.prefixes.contains(keys) {
            Lookup::Prefix
        } else {
            Lookup::None
        }
    }
    pub fn fallback_action(&self, key: KeyStroke) -> Option<Action> {
        self.fallback
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, action)| *action)
    }
    pub fn label(&self, action: Action) -> String {
        let reachable = |key| match self.lookup(&KeySequence::from(key)) {
            Lookup::Action(found) => found == action,
            Lookup::None => self.fallback_action(key) == Some(action),
            Lookup::Prefix => false,
        };
        let escape = KeyStroke::plain(KeyCode::Escape);
        if action == Action::Back && reachable(escape) {
            return escape.label();
        }
        let mut labels = self
            .entries
            .iter()
            .filter(|(_, found)| *found == action)
            .map(|(sequence, _)| sequence.to_string())
            .chain(
                self.fallback
                    .iter()
                    .filter(|(key, found)| *found == action && reachable(*key))
                    .map(|(key, _)| key.label()),
            )
            .collect::<Vec<_>>();
        labels.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
        labels
            .into_iter()
            .next()
            .unwrap_or_else(|| "unbound".into())
    }
    pub fn hints(&self, keys: &KeySequence) -> String {
        self.entries
            .iter()
            .filter(|(sequence, action)| {
                sequence.starts_with(keys)
                    && !matches!(
                        action,
                        Action::Forward | Action::ForwardInput | Action::DismissAndForward
                    )
            })
            .map(|(sequence, action)| format!("{sequence}: {}", action.description()))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

#[derive(Debug, Default)]
pub(super) struct Cache(Mutex<HashMap<(Mode, BindingScope), Bindings>>);
impl Clone for Cache {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl Keymap {
    pub fn native_preview_help(&self, mode: Mode, source: BindingScope) -> String {
        let bindings = self.native_preview(mode, source);
        let mut text = String::from("\nNative document preview\n\n");
        if bindings.unavailable {
            text.push_str("  Preview keymap exceeds its limit; input returns to source\n");
            return text;
        }
        for (action, projected, description) in [
            (ui::Action::Back, Action::Back, "Return to source"),
            (ui::Action::Copy, Action::Copy, "Copy selected preview text"),
            (
                ui::Action::KeepOpenAndContinue,
                Action::Forward,
                "Continue editor input with the preview open",
            ),
            (
                ui::Action::ForwardPane,
                Action::Forward,
                "Forward compatibility pane shortcuts",
            ),
        ] {
            let label = self
                .ui()
                .reachable_label(ui::Owner::NativePreview, action, None, |key| match bindings
                    .lookup(&KeySequence::from(key))
                {
                    Lookup::Action(found) => {
                        found == projected
                            || projected == Action::Forward && found == Action::ForwardInput
                    }
                    Lookup::None => bindings.fallback_action(key).is_some_and(|found| {
                        found == projected
                            || projected == Action::Forward && found == Action::ForwardInput
                    }),
                    Lookup::Prefix => false,
                });
            text.push_str(&format!("  {label}  {description}\n"));
        }
        text.push_str("  Navigation follows the supported Media bindings.\n");
        text
    }
    pub fn native_preview(&self, mode: Mode, source: BindingScope) -> Bindings {
        let mut cache = self.preview.0.lock().unwrap();
        if cache.len() >= 16 && !cache.contains_key(&(mode, source)) {
            cache.clear();
        }
        cache
            .entry((mode, source))
            .or_insert_with(|| project(self, mode, source))
            .clone()
    }
}
fn authored(
    binding: &Binding,
    configured: &HashSet<KeySequence>,
    factory: &HashMap<KeySequence, (BindingTarget, bool)>,
) -> bool {
    configured.contains(&binding.sequence)
        || !binding.actions.is_empty()
        || factory.get(&binding.sequence) != Some(&(binding.target, binding.is_plain_action()))
}
fn project(map: &Keymap, mode: Mode, source: BindingScope) -> Bindings {
    use ui::{Action as U, Owner};
    let factory = |mode, scope| {
        super::default_keymap()
            .bindings_for_scope(mode, scope)
            .map(|binding| {
                (
                    binding.sequence.clone(),
                    (binding.target, binding.is_plain_action()),
                )
            })
            .collect::<HashMap<_, _>>()
    };
    let media_factory = factory(Mode::Normal, BindingScope::Media);
    let source_factory = factory(mode, source);
    let configured = |mode, scope| {
        map.overrides
            .governing(mode, scope)
            .keys()
            .map(|sequence| (*sequence).clone())
            .collect::<HashSet<_>>()
    };
    let media_configured = configured(Mode::Normal, BindingScope::Media);
    let source_configured = configured(mode, source);
    let mut entries = Vec::new();
    for binding in map.bindings_for_scope(Mode::Normal, BindingScope::Media) {
        let Some(action) = local(binding) else {
            continue;
        };
        let replaced = match action {
            Action::Back => map
                .ui()
                .explicitly_configured(Owner::NativePreview, U::Back),
            Action::Copy => map
                .ui()
                .explicitly_configured(Owner::NativePreview, U::Copy),
            _ => false,
        };
        if replaced && !authored(binding, &media_configured, &media_factory) {
            continue;
        }
        entries.push((binding.sequence.clone(), action));
    }
    let local_roots: HashSet<_> = entries
        .iter()
        .map(|(sequence, _)| sequence.as_slice()[0])
        .collect();
    let mut forwarding = HashMap::new();
    for binding in map.bindings_for_scope(mode, source) {
        let first = binding.sequence.as_slice()[0];
        if pane(binding) {
            forwarding.insert(first, Action::Forward);
        } else if local_roots.contains(&first)
            && local(binding).is_none()
            && authored(binding, &source_configured, &source_factory)
        {
            forwarding.entry(first).or_insert(Action::DismissAndForward);
        }
    }
    let mut fallback = Vec::new();
    for (key, action) in map.ui().keys(Owner::NativePreview) {
        let projected = match action {
            U::Back => Action::Back,
            U::Copy => Action::Copy,
            U::KeepOpenAndContinue
                if map.ui().explicitly_configured(Owner::NativePreview, action) =>
            {
                Action::ForwardInput
            }
            U::KeepOpenAndContinue | U::ForwardPane => Action::Forward,
            _ => unreachable!(),
        };
        if action == U::ForwardPane {
            forwarding.insert(key.canonical_for_binding(), Action::Forward);
        } else if map.ui().explicitly_configured(Owner::NativePreview, action) {
            let key = key.canonical_for_binding();
            entries.retain(|(sequence, _)| sequence.as_slice()[0] != key);
            entries.push((KeySequence::from(key), projected));
            fallback.push((key, projected));
            if matches!(key.code, KeyCode::Char(_)) {
                fallback.push((
                    KeyStroke::new(key.code, key.modifiers | Modifiers::SHIFT),
                    projected,
                ));
            }
        } else {
            fallback.push((key, projected));
            if matches!(key.code, KeyCode::Char(_)) {
                let shifted = KeyStroke::new(key.code, key.modifiers | Modifiers::SHIFT);
                if shifted != key && legacy_ui(shifted) == Some(action) {
                    fallback.push((shifted, projected));
                }
            }
        }
    }
    entries.retain(|(sequence, _)| !forwarding.contains_key(&sequence.as_slice()[0]));
    entries.extend(
        forwarding
            .into_iter()
            .map(|(key, action)| (KeySequence::from(key), action)),
    );
    entries.sort_by_key(|(sequence, _)| sequence.to_string());
    fallback.sort_by_key(|(key, _)| key.label());
    Bindings::from_entries(entries, fallback).unwrap_or_else(|_| Bindings {
        unavailable: true,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn configured(source: &str) -> std::sync::Arc<Keymap> {
        let result = super::super::configured::compile(
            &serde_yaml::from_str(source).unwrap(),
            super::super::default_keymap(),
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        result.keymap
    }
    fn sequence(source: &str) -> KeySequence {
        KeySequence::parse(source).unwrap()
    }
    #[test]
    fn preview_projection_uses_effective_media_navigation_and_source_forwarding() {
        let factory =
            super::super::default_keymap().native_preview(Mode::Normal, BindingScope::Global);
        assert_eq!(factory.lookup(&sequence("j")), Lookup::Action(Action::Down));
        assert_eq!(
            factory.lookup(&sequence("g g")),
            Lookup::Action(Action::Top)
        );
        assert_eq!(
            factory.lookup(&sequence("Ctrl-w")),
            Lookup::Action(Action::Forward)
        );
        assert_eq!(
            factory.fallback_action(KeyStroke::plain(KeyCode::Escape)),
            Some(Action::Back)
        );
        let map = configured("scopes: {media: {normal: {j: null, F8: media-move-down}}}");
        let preview = map.native_preview(Mode::Normal, BindingScope::Global);
        assert_eq!(preview.lookup(&sequence("j")), Lookup::None);
        assert_eq!(
            preview.lookup(&sequence("F8")),
            Lookup::Action(Action::Down)
        );
        assert!(
            preview
                .hints(&KeySequence::default())
                .contains("F8: Scroll down")
        );
    }
    #[test]
    fn preview_does_not_truncate_action_chains_or_steal_host_prefixes() {
        let map =
            configured("bind: {normal: {'g x': move-word-forward, F8: [move-down, move-up]}}");
        let preview = map.native_preview(Mode::Normal, BindingScope::Global);
        assert_eq!(
            preview.lookup(&sequence("g")),
            Lookup::Action(Action::DismissAndForward)
        );
        assert_eq!(preview.lookup(&sequence("g g")), Lookup::None);
        assert_eq!(preview.lookup(&sequence("F8")), Lookup::None);
        assert!(!preview.hints(&KeySequence::default()).contains("F8"));
    }
    #[test]
    fn preview_ui_replacement_suppresses_inherited_aliases_and_keeps_authored_media() {
        let map = configured(
            "ui: {native-preview: {back: [F8], copy: [], forward-pane: []}}\nscopes: {media: {normal: {F7: media-back}}}",
        );
        let preview = map.native_preview(Mode::Normal, BindingScope::Global);
        assert_eq!(
            preview.lookup(&sequence("F8")),
            Lookup::Action(Action::Back)
        );
        assert_eq!(
            preview.lookup(&sequence("F7")),
            Lookup::Action(Action::Back)
        );
        assert!(
            preview
                .fallback_action(KeyStroke::plain(KeyCode::Escape))
                .is_none()
        );
        assert!(preview.fallback_action(KeyStroke::ctrl('c')).is_none());
        assert_eq!(
            preview.lookup(&sequence("Ctrl-w")),
            Lookup::Action(Action::Forward)
        );
    }
    #[test]
    fn preview_back_messages_follow_effective_replacement_and_removal() {
        assert_eq!(
            super::super::default_keymap()
                .native_preview(Mode::Normal, BindingScope::Global)
                .label(Action::Back),
            "Esc"
        );
        let map = configured("ui: {native-preview: {back: [F8]}}");
        assert_eq!(
            map.native_preview(Mode::Normal, BindingScope::Global)
                .label(Action::Back),
            "F8"
        );
        let map = configured("ui: {native-preview: {back: []}}");
        assert_eq!(
            map.native_preview(Mode::Normal, BindingScope::Global)
                .label(Action::Back),
            "unbound"
        );
    }

    #[test]
    fn preview_projection_wire_admission_rejects_overlaps_duplicates_and_long_sequences() {
        for entries in [
            vec![(sequence("g"), Action::Top), (sequence("g g"), Action::Top)],
            vec![(sequence("g g"), Action::Forward)],
            vec![(sequence("g"), Action::Top); 2],
            vec![(KeySequence::default(), Action::Top)],
            vec![(sequence("a b c d e f g h i"), Action::Top)],
        ] {
            assert!(Bindings::from_entries(entries, vec![]).is_err());
        }
        assert!(Bindings::from_entries(vec![], vec![(KeyStroke::ctrl('x'), Action::Top)]).is_err());
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    #[test]
    fn preview_compatibility_preserves_all_raw_modifier_variants() {
        let map = super::super::default_keymap().native_preview(Mode::Normal, BindingScope::Global);
        assert!(!map.unavailable);
        for bits in 0..64 {
            for code in [
                KeyCode::Escape,
                KeyCode::Char('c'),
                KeyCode::Char('C'),
                KeyCode::Char(':'),
                KeyCode::Char(' '),
                KeyCode::Char('w'),
                KeyCode::Char('h'),
                KeyCode::Left,
            ] {
                let key = KeyStroke::new(code, Modifiers::from_bits(bits).unwrap());
                let expected = match legacy_ui(key) {
                    Some(ui::Action::Back) => Some(Action::Back),
                    Some(ui::Action::Copy) => Some(Action::Copy),
                    Some(ui::Action::KeepOpenAndContinue) => Some(Action::Forward),
                    Some(ui::Action::ForwardPane) => {
                        assert_eq!(
                            map.lookup(&KeySequence::from(key)),
                            Lookup::Action(Action::Forward)
                        );
                        None
                    }
                    _ => None,
                };
                assert_eq!(map.fallback_action(key), expected, "{key:?}");
            }
        }
    }
    #[test]
    fn preview_projection_budget_is_atomic_visible_and_cache_is_bounded() {
        let bindings = (0..=Bindings::MAX_ENTRIES)
            .map(|index| {
                Binding::implemented_in(
                    &[Mode::Normal],
                    BindingScope::Media,
                    KeyStroke::char(char::from_u32(0x1000 + index as u32).unwrap()),
                    C::MediaMoveDown,
                )
            })
            .collect();
        let map = Keymap::new(bindings).unwrap();
        let preview = map.native_preview(Mode::Normal, BindingScope::Global);
        assert!(preview.unavailable);
        assert!(preview.entries().is_empty());
        assert!(preview.fallback().is_empty());
        assert!(
            map.native_preview_help(Mode::Normal, BindingScope::Global)
                .contains("exceeds its limit")
        );
        for id in 0..40 {
            map.native_preview(Mode::Normal, BindingScope::Plugin(id));
            assert!(map.preview.0.lock().unwrap().len() <= 16);
        }
    }
}
