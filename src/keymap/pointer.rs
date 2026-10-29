// SPDX-License-Identifier: MPL-2.0

//! Exact pointer modifier bindings. A pane action is captured on button-down;
//! subsequent motion belongs to that action even when modifiers change.

use crate::input::{KeyStroke, Modifiers, PointerButton, PointerEvent, PointerEventKind};
use serde_yaml::Value;

#[cfg(test)]
#[path = "../tests/pointer_bindings.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneAction {
    Move,
    Extend,
}

/// One bit per exact combination of the six supported modifier bits. Fixed
/// size keeps the bundled frontend routing snapshot bounded independently of
/// configuration size. Both actions currently require a left-button drag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneGestures {
    pub move_pane: u64,
    pub extend_pane: u64,
}

impl Default for PaneGestures {
    fn default() -> Self {
        Self {
            move_pane: 1 << Modifiers::ALT.bits(),
            extend_pane: 1 << (Modifiers::ALT | Modifiers::SHIFT).bits(),
        }
    }
}

impl PaneGestures {
    pub fn action(self, event: PointerEvent) -> Option<PaneAction> {
        if event.kind != PointerEventKind::Down(PointerButton::Left) {
            return None;
        }
        let bit = 1u64 << event.modifiers.bits();
        if self.move_pane & bit != 0 {
            Some(PaneAction::Move)
        } else if self.extend_pane & bit != 0 {
            Some(PaneAction::Extend)
        } else {
            None
        }
    }

    pub fn labels(self, action: PaneAction) -> String {
        let mask = match action {
            PaneAction::Move => self.move_pane,
            PaneAction::Extend => self.extend_pane,
        };
        let labels: Vec<_> = (0..64)
            .filter(|bits| mask & (1u64 << bits) != 0)
            .map(|bits| {
                let key = KeyStroke::new(
                    crate::input::KeyCode::Left,
                    Modifiers::from_bits(bits).expect("six-bit modifiers"),
                );
                format!("{}Drag", key.label())
            })
            .collect();
        if labels.is_empty() {
            "unbound".into()
        } else {
            labels.join(" / ")
        }
    }
}

pub(super) fn compile(section: Option<&Value>, errors: &mut Vec<String>) -> PaneGestures {
    let defaults = PaneGestures::default();
    let Some(section) = section else {
        return defaults;
    };
    let Some(mapping) = section.as_mapping() else {
        errors.push("pointer rejected: expected action-to-gesture-list mapping".into());
        return defaults;
    };
    let mut candidate = defaults;
    let parsed = (|| -> Result<(), String> {
        if mapping.len() > 2 {
            return Err("unknown pointer actions or too many actions".into());
        }
        for (action, gestures) in mapping {
            let target = match action.as_str() {
                Some("pane-move") => &mut candidate.move_pane,
                Some("pane-extend") => &mut candidate.extend_pane,
                _ => return Err("expected pane-move or pane-extend action name".into()),
            };
            let gestures = gestures
                .as_sequence()
                .ok_or("expected a list; use [] to unbind")?;
            if gestures.len() > 64 {
                return Err("at most 64 gestures per action".into());
            }
            *target = 0;
            for gesture in gestures {
                let spelling = gesture.as_str().ok_or("gestures must be strings")?;
                if spelling.len() > 64 {
                    return Err("gesture spelling exceeds 64 bytes".into());
                }
                let prefix = spelling
                    .strip_suffix("LeftDrag")
                    .ok_or("expected LeftDrag with optional modifiers")?;
                // Parse a non-character key so Shift remains an exact modifier.
                let key = KeyStroke::parse(&format!("{prefix}Left"))?;
                let bit = 1u64 << key.modifiers.bits();
                if *target & bit != 0 {
                    return Err(format!("duplicate gesture {spelling:?}"));
                }
                *target |= bit;
            }
        }
        if candidate.move_pane & candidate.extend_pane != 0 {
            return Err("pane-move and pane-extend have overlapping gestures".into());
        }
        Ok(())
    })();
    match parsed {
        Ok(()) => candidate,
        Err(error) => {
            errors.push(format!(
                "pointer rejected: {error}; default gestures retained"
            ));
            defaults
        }
    }
}
