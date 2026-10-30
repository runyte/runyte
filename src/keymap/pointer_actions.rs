// SPDX-License-Identifier: MPL-2.0
//! Exact pointer bindings, grouped by the interaction that owns the hit region.
use crate::input::{KeyStroke, Modifiers, PointerButton, PointerEvent, PointerEventKind};
use serde_yaml::Value;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum Action {
    PaneMove,
    PaneExtend,
    Resize,
    TreeResize,
    Select,
    Extend,
    OpenLink,
    Copy,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
    TreeSelect,
    TreeOpen,
    SessionActivate,
    MediaPan,
    MediaRegion,
    MediaAutoSelect,
    MediaToggleZoom,
    MediaZoom,
    MediaHorizontal,
    MediaScroll,
    PreviewSelect,
    PreviewHorizontal,
    PreviewScroll,
    InputOverflowDismiss,
}
impl Action {
    pub const ALL: [Self; 26] = [
        Self::PaneMove,
        Self::PaneExtend,
        Self::Resize,
        Self::TreeResize,
        Self::Select,
        Self::Extend,
        Self::OpenLink,
        Self::Copy,
        Self::ScrollUp,
        Self::ScrollDown,
        Self::ScrollLeft,
        Self::ScrollRight,
        Self::TreeSelect,
        Self::TreeOpen,
        Self::SessionActivate,
        Self::MediaPan,
        Self::MediaRegion,
        Self::MediaAutoSelect,
        Self::MediaToggleZoom,
        Self::MediaZoom,
        Self::MediaHorizontal,
        Self::MediaScroll,
        Self::PreviewSelect,
        Self::PreviewHorizontal,
        Self::PreviewScroll,
        Self::InputOverflowDismiss,
    ];
    pub fn name(self) -> &'static str {
        [
            "pane-move",
            "pane-extend",
            "pane-resize",
            "tree-resize",
            "select",
            "extend",
            "open-link",
            "copy",
            "scroll-up",
            "scroll-down",
            "scroll-left",
            "scroll-right",
            "tree-select",
            "tree-open",
            "session-activate",
            "media-pan",
            "media-region",
            "media-auto-select",
            "media-toggle-zoom",
            "media-zoom",
            "media-horizontal",
            "media-scroll",
            "preview-select",
            "preview-horizontal",
            "preview-scroll",
            "input-overflow-dismiss",
        ][self as usize]
    }
    fn owner(self) -> usize {
        match self {
            Self::PaneMove | Self::PaneExtend => 0,
            Self::Resize => 1,
            Self::TreeResize => 2,
            Self::Select | Self::Extend => 3,
            Self::OpenLink => 4,
            Self::Copy => 5,
            Self::ScrollUp | Self::ScrollDown | Self::ScrollLeft | Self::ScrollRight => 6,
            Self::TreeSelect => 7,
            Self::TreeOpen => 8,
            Self::SessionActivate => 9,
            Self::MediaPan | Self::MediaRegion | Self::MediaAutoSelect => 10,
            Self::MediaToggleZoom => 11,
            Self::MediaZoom | Self::MediaHorizontal | Self::MediaScroll => 12,
            Self::PreviewSelect => 13,
            Self::PreviewHorizontal | Self::PreviewScroll => 14,
            Self::InputOverflowDismiss => 15,
        }
    }
    fn accepts(self, trigger: Trigger) -> bool {
        match self {
            Self::OpenLink
            | Self::Copy
            | Self::TreeSelect
            | Self::SessionActivate
            | Self::InputOverflowDismiss => trigger.is_click(),
            Self::TreeOpen | Self::MediaToggleZoom => trigger.is_double(),
            Self::ScrollUp | Self::ScrollDown | Self::ScrollLeft | Self::ScrollRight => {
                trigger.is_directional_wheel()
            }
            Self::MediaZoom
            | Self::MediaHorizontal
            | Self::MediaScroll
            | Self::PreviewHorizontal
            | Self::PreviewScroll => trigger == Trigger::Wheel,
            _ => trigger.is_drag(),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum Trigger {
    LeftClick,
    MiddleClick,
    RightClick,
    LeftDrag,
    MiddleDrag,
    RightDrag,
    LeftDoubleClick,
    MiddleDoubleClick,
    RightDoubleClick,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
    Wheel,
}
impl Trigger {
    const ALL: [Self; 14] = [
        Self::LeftClick,
        Self::MiddleClick,
        Self::RightClick,
        Self::LeftDrag,
        Self::MiddleDrag,
        Self::RightDrag,
        Self::LeftDoubleClick,
        Self::MiddleDoubleClick,
        Self::RightDoubleClick,
        Self::WheelUp,
        Self::WheelDown,
        Self::WheelLeft,
        Self::WheelRight,
        Self::Wheel,
    ];
    fn name(self) -> &'static str {
        [
            "LeftClick",
            "MiddleClick",
            "RightClick",
            "LeftDrag",
            "MiddleDrag",
            "RightDrag",
            "LeftDoubleClick",
            "MiddleDoubleClick",
            "RightDoubleClick",
            "WheelUp",
            "WheelDown",
            "WheelLeft",
            "WheelRight",
            "Wheel",
        ][self as usize]
    }
    fn is_click(self) -> bool {
        (self as usize) < 3
    }
    fn is_drag(self) -> bool {
        (3..6).contains(&(self as usize))
    }
    fn is_double(self) -> bool {
        (6..9).contains(&(self as usize))
    }
    fn is_directional_wheel(self) -> bool {
        (9..13).contains(&(self as usize))
    }
    pub fn click(button: PointerButton) -> Self {
        Self::ALL[super::pointer::button_index(button)]
    }
    pub fn drag(button: PointerButton) -> Self {
        Self::ALL[3 + super::pointer::button_index(button)]
    }
    pub fn double_click(button: PointerButton) -> Self {
        Self::ALL[6 + super::pointer::button_index(button)]
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bindings(Arc<[[u64; 14]; 26]>);
impl Default for Bindings {
    fn default() -> Self {
        use Action::*;
        use Trigger::*;
        let mut masks = [[0; 14]; 26];
        for bits in 0..64 {
            let modifiers = Modifiers::from_bits(bits).unwrap();
            let shift = modifiers.contains(Modifiers::SHIFT);
            let mut set = |action: Action, trigger: Trigger| {
                masks[action as usize][trigger as usize] |= 1 << bits
            };
            if modifiers == Modifiers::ALT {
                set(PaneMove, LeftDrag);
            }
            if modifiers == (Modifiers::ALT | Modifiers::SHIFT) {
                set(PaneExtend, LeftDrag);
            }
            set(Resize, LeftDrag);
            set(TreeResize, LeftDrag);
            set(if shift { Extend } else { Select }, LeftDrag);
            if modifiers
                == if cfg!(target_os = "macos") {
                    Modifiers::SUPER
                } else {
                    Modifiers::CONTROL
                }
            {
                set(OpenLink, LeftClick);
            }
            set(Copy, RightClick);
            set(ScrollUp, WheelUp);
            set(ScrollDown, WheelDown);
            set(ScrollLeft, WheelLeft);
            set(ScrollRight, WheelRight);
            set(TreeSelect, LeftClick);
            set(TreeOpen, LeftDoubleClick);
            set(SessionActivate, LeftClick);
            set(InputOverflowDismiss, LeftClick);
            let alt = modifiers.contains(Modifiers::ALT);
            let zoom =
                modifiers.contains(Modifiers::CONTROL) || modifiers.contains(Modifiers::SUPER);
            set(MediaPan, RightDrag);
            set(MediaPan, MiddleDrag);
            set(
                if alt {
                    MediaPan
                } else if shift {
                    MediaRegion
                } else {
                    MediaAutoSelect
                },
                LeftDrag,
            );
            if !alt && !shift {
                set(MediaToggleZoom, LeftDoubleClick);
            }
            set(
                if zoom {
                    MediaZoom
                } else if shift {
                    MediaHorizontal
                } else {
                    MediaScroll
                },
                Wheel,
            );
            set(PreviewSelect, LeftDrag);
            set(
                if shift {
                    PreviewHorizontal
                } else {
                    PreviewScroll
                },
                Wheel,
            );
        }
        Self(Arc::new(masks))
    }
}
impl Bindings {
    pub fn same_action(&self, other: &Self, action: Action) -> bool {
        self.0[action as usize] == other.0[action as usize]
    }
    pub fn is_default(&self, action: Action) -> bool {
        static DEFAULTS: std::sync::OnceLock<Bindings> = std::sync::OnceLock::new();
        self.0[action as usize] == DEFAULTS.get_or_init(Self::default).0[action as usize]
    }

    pub fn label(&self, action: Action) -> String {
        if self.is_default(action) {
            return match action {
                Action::PaneMove => "Alt-LeftDrag",
                Action::PaneExtend => "Shift-Alt-LeftDrag",
                Action::Resize | Action::TreeResize => "LeftDrag (any modifiers)",
                Action::Select => "LeftDrag (without Shift)",
                Action::Extend => "Shift-LeftDrag (any other modifiers)",
                Action::OpenLink => {
                    if cfg!(target_os = "macos") {
                        "Super-LeftClick"
                    } else {
                        "Ctrl-LeftClick"
                    }
                }
                Action::Copy => "RightClick (any modifiers)",
                Action::ScrollUp => "WheelUp (any modifiers)",
                Action::ScrollDown => "WheelDown (any modifiers)",
                Action::ScrollLeft => "WheelLeft (any modifiers)",
                Action::ScrollRight => "WheelRight (any modifiers)",
                Action::TreeSelect | Action::SessionActivate | Action::InputOverflowDismiss => {
                    "LeftClick (any modifiers)"
                }
                Action::TreeOpen => "LeftDoubleClick (any modifiers)",
                Action::MediaPan => "MiddleDrag / RightDrag / Alt-LeftDrag (any other modifiers)",
                Action::MediaRegion => "Shift-LeftDrag (without Alt)",
                Action::MediaAutoSelect => "LeftDrag (without Shift or Alt)",
                Action::MediaToggleZoom => "LeftDoubleClick (without Shift or Alt)",
                Action::MediaZoom => "Ctrl-Wheel / Super-Wheel (any other modifiers)",
                Action::MediaHorizontal => "Shift-Wheel (without Ctrl or Super)",
                Action::MediaScroll => "Wheel (without Shift, Ctrl or Super)",
                Action::PreviewSelect => "LeftDrag (any modifiers)",
                Action::PreviewHorizontal => "Shift-Wheel (any other modifiers)",
                Action::PreviewScroll => "Wheel (without Shift)",
            }
            .into();
        }
        let mut labels = Vec::new();
        for trigger in Trigger::ALL {
            let mask = self.0[action as usize][trigger as usize];
            for bits in 0..64 {
                if mask & (1u64 << bits) == 0 {
                    continue;
                }
                let spelling = KeyStroke::new(
                    crate::input::KeyCode::Left,
                    Modifiers::from_bits(bits).unwrap(),
                )
                .label();
                labels.push(format!(
                    "{}{}",
                    spelling.strip_suffix("Left").unwrap(),
                    trigger.name()
                ));
            }
        }
        if labels.is_empty() {
            "unbound".into()
        } else {
            labels.join(" / ")
        }
    }

    pub fn matches(&self, action: Action, trigger: Trigger, modifiers: Modifiers) -> bool {
        self.0[action as usize][trigger as usize] & (1u64 << modifiers.bits()) != 0
    }
    pub fn press(&self, action: Action, event: PointerEvent) -> bool {
        let PointerEventKind::Down(button) = event.kind else {
            return false;
        };
        let click = Trigger::click(button);
        let drag = Trigger::drag(button);
        self.matches(
            action,
            if action.accepts(click) {
                click
            } else if action.accepts(drag) {
                drag
            } else {
                Trigger::double_click(button)
            },
            event.modifiers,
        )
    }
    pub fn wheel(&self, event: PointerEvent) -> Option<Action> {
        let trigger = match event.kind {
            PointerEventKind::ScrollUp => Trigger::WheelUp,
            PointerEventKind::ScrollDown => Trigger::WheelDown,
            PointerEventKind::ScrollLeft => Trigger::WheelLeft,
            PointerEventKind::ScrollRight => Trigger::WheelRight,
            _ => return None,
        };
        [
            Action::ScrollUp,
            Action::ScrollDown,
            Action::ScrollLeft,
            Action::ScrollRight,
        ]
        .into_iter()
        .find(|action| self.matches(*action, trigger, event.modifiers))
    }
    pub fn pane_gestures(&self) -> super::pointer::PaneGestures {
        let masks = |action: Action| {
            [
                self.0[action as usize][3],
                self.0[action as usize][4],
                self.0[action as usize][5],
            ]
        };
        super::pointer::PaneGestures {
            move_pane: masks(Action::PaneMove),
            extend_pane: masks(Action::PaneExtend),
        }
    }
    pub fn masks(&self) -> &[[u64; 14]; 26] {
        &self.0
    }
    pub fn from_masks(masks: [[u64; 14]; 26]) -> Option<Self> {
        for action in Action::ALL {
            for trigger in Trigger::ALL {
                let mask = masks[action as usize][trigger as usize];
                if mask != 0 && !action.accepts(trigger) {
                    return None;
                }
                for prior in Action::ALL.into_iter().take(action as usize) {
                    if action.owner() == prior.owner()
                        && mask & masks[prior as usize][trigger as usize] != 0
                    {
                        return None;
                    }
                }
            }
        }
        Some(Self(Arc::new(masks)))
    }
}

pub(super) fn compile(section: Option<&Value>, errors: &mut Vec<String>) -> Bindings {
    let defaults = Bindings::default();
    let Some(section) = section else {
        return defaults;
    };
    let Some(mapping) = section.as_mapping() else {
        errors.push("pointer rejected: expected action-to-gesture-list mapping".into());
        return defaults;
    };
    if mapping.len() > Action::ALL.len()
        || mapping.keys().any(|key| {
            !Action::ALL
                .iter()
                .any(|action| Some(action.name()) == key.as_str())
        })
    {
        errors.push("pointer rejected: unknown action; default gestures retained".into());
        return defaults;
    }
    let mut masks = *defaults.0;
    for owner in 0..16 {
        let result = (|| -> Result<(), String> {
            for action in Action::ALL
                .into_iter()
                .filter(|action| action.owner() == owner)
            {
                let Some(value) = mapping.get(Value::String(action.name().into())) else {
                    continue;
                };
                let gestures = value
                    .as_sequence()
                    .ok_or("expected a gesture list; use [] to unbind")?;
                if gestures.len() > 192 {
                    return Err("at most 192 aliases per action".into());
                }
                masks[action as usize] = [0; 14];
                for value in gestures {
                    let spelling = value.as_str().ok_or("gestures must be strings")?;
                    if spelling.len() > 64 {
                        return Err("gesture spelling exceeds 64 bytes".into());
                    }
                    let (trigger, prefix) = Trigger::ALL
                        .into_iter()
                        .filter(|trigger| action.accepts(*trigger))
                        .find_map(|trigger| {
                            spelling
                                .strip_suffix(trigger.name())
                                .map(|prefix| (trigger, prefix))
                        })
                        .ok_or_else(|| format!("unsupported gesture for {}", action.name()))?;
                    let key = KeyStroke::parse(&format!("{prefix}Left"))?;
                    let bit = 1u64 << key.modifiers.bits();
                    let mask = &mut masks[action as usize][trigger as usize];
                    if *mask & bit != 0 {
                        return Err(format!("duplicate gesture {spelling:?}"));
                    }
                    *mask |= bit;
                }
            }
            for action in Action::ALL
                .into_iter()
                .filter(|action| action.owner() == owner)
            {
                for prior in Action::ALL
                    .into_iter()
                    .take(action as usize)
                    .filter(|prior| prior.owner() == owner)
                {
                    if masks[action as usize]
                        .iter()
                        .zip(masks[prior as usize])
                        .any(|(left, right)| left & right != 0)
                    {
                        return Err(format!(
                            "{} and {} overlap; explicitly replace both lists",
                            action.name(),
                            prior.name()
                        ));
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            for action in Action::ALL
                .into_iter()
                .filter(|action| action.owner() == owner)
            {
                masks[action as usize] = defaults.0[action as usize];
            }
            errors.push(format!(
                "pointer owner rejected: {error}; its default gestures retained"
            ));
        }
    }
    Bindings(Arc::new(masks))
}

#[cfg(test)]
#[path = "../tests/pointer_actions.rs"]
mod tests;
