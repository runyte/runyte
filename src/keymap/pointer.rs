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
/// configuration size. Each action has exact modifier masks for left, middle and right drags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneGestures {
    pub move_pane: [u64; 3],
    pub extend_pane: [u64; 3],
}

impl Default for PaneGestures {
    fn default() -> Self {
        Self {
            move_pane: [1 << Modifiers::ALT.bits(), 0, 0],
            extend_pane: [1 << (Modifiers::ALT | Modifiers::SHIFT).bits(), 0, 0],
        }
    }
}

pub fn button_index(button: PointerButton) -> usize {
    match button {
        PointerButton::Left => 0,
        PointerButton::Middle => 1,
        PointerButton::Right => 2,
    }
}

impl PaneGestures {
    pub fn masks(self) -> [u64; 6] {
        [
            self.move_pane[0],
            self.extend_pane[0],
            self.move_pane[1],
            self.extend_pane[1],
            self.move_pane[2],
            self.extend_pane[2],
        ]
    }
    pub fn from_masks(masks: [u64; 6]) -> Option<Self> {
        (masks.chunks_exact(2).all(|pair| pair[0] & pair[1] == 0)).then_some(Self {
            move_pane: [masks[0], masks[2], masks[4]],
            extend_pane: [masks[1], masks[3], masks[5]],
        })
    }
    pub fn action(self, event: PointerEvent) -> Option<PaneAction> {
        let PointerEventKind::Down(button) = event.kind else {
            return None;
        };
        let button = button_index(button);
        let bit = 1u64 << event.modifiers.bits();
        if self.move_pane[button] & bit != 0 {
            Some(PaneAction::Move)
        } else if self.extend_pane[button] & bit != 0 {
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
        let labels: Vec<_> = mask
            .into_iter()
            .enumerate()
            .flat_map(|(button, mask)| {
                (0..64)
                    .filter(move |bits| mask & (1u64 << bits) != 0)
                    .map(move |bits| {
                        let key = KeyStroke::new(
                            [
                                crate::input::KeyCode::Left,
                                crate::input::KeyCode::Home,
                                crate::input::KeyCode::Right,
                            ][button],
                            Modifiers::from_bits(bits).expect("six-bit modifiers"),
                        );
                        format!("{}Drag", key.label().replace("Home", "Middle"))
                    })
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
            *target = [0; 3];
            for gesture in gestures {
                let spelling = gesture.as_str().ok_or("gestures must be strings")?;
                if spelling.len() > 64 {
                    return Err("gesture spelling exceeds 64 bytes".into());
                }
                let (button, prefix) = ["LeftDrag", "MiddleDrag", "RightDrag"]
                    .iter()
                    .enumerate()
                    .find_map(|(button, suffix)| {
                        spelling.strip_suffix(suffix).map(|prefix| (button, prefix))
                    })
                    .ok_or("expected LeftDrag, MiddleDrag or RightDrag with optional modifiers")?;
                // Parse a non-character key so Shift remains an exact modifier.
                let key = KeyStroke::parse(&format!("{prefix}Left"))?;
                let bit = 1u64 << key.modifiers.bits();
                if target[button] & bit != 0 {
                    return Err(format!("duplicate gesture {spelling:?}"));
                }
                target[button] |= bit;
            }
        }
        if PaneGestures::from_masks(candidate.masks()).is_none() {
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
