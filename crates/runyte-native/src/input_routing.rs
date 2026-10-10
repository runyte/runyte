// SPDX-License-Identifier: MPL-2.0
//! Preserve physical-input identity while waiting for editor-owned routing state.
use super::NativeInput;
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct Queue<T = ()> {
    inputs: VecDeque<(NativeInput, T)>,
    bytes: usize,
    pending: Option<(u64, u64)>,
    serial: u64,
}

#[cfg(test)]
#[path = "tests/input_routing.rs"]
mod tests;

impl<T> Queue<T> {
    pub fn clear(&mut self) {
        self.inputs.clear();
        self.bytes = 0;
        self.pending = None;
    }
    #[cfg(test)]
    pub fn push(&mut self, input: NativeInput, action: T) -> bool {
        self.push_coalesced(input, action, |_, _| false)
    }
    pub fn push_coalesced(
        &mut self,
        input: NativeInput,
        action: T,
        compatible: impl FnOnce(&T, &T) -> bool,
    ) -> bool {
        if let Some((previous, prior_action)) = self.inputs.back_mut()
            && super::input_queue::same_drag(previous, &input)
            && compatible(prior_action, &action)
        {
            *previous = input;
            *prior_action = action;
            return true;
        }
        let bytes = text_bytes(&input);
        if self.inputs.len() == 256 || bytes > (8 * 1024 * 1024usize).saturating_sub(self.bytes) {
            return false;
        }
        self.bytes += bytes;
        self.inputs.push_back((input, action));
        true
    }
    pub fn next(&mut self, attachment: u64, acknowledged: u64) -> Option<(NativeInput, T)> {
        if let Some((owner, serial)) = self.pending {
            if owner == attachment && acknowledged < serial {
                return None;
            }
            self.pending = None;
        }
        while let Some((input, action)) = self.inputs.pop_front() {
            self.bytes -= text_bytes(&input);
            if input.attachment == attachment {
                return Some((input, action));
            }
        }
        None
    }
    pub fn forward(&mut self, input: &mut NativeInput) {
        self.serial = self
            .serial
            .checked_add(1)
            .expect("input routing serial exhausted");
        input.routing_serial = self.serial;
        self.pending = Some((input.attachment, self.serial));
    }
}

fn text_bytes(input: &NativeInput) -> usize {
    match &input.event {
        super::Event::Paste(text) => text.len(),
        _ => 0,
    }
}

#[derive(Default)]
pub(super) enum Action {
    #[default]
    Direct,
    Down(super::MouseDownEvent),
    Up(super::MouseUpEvent),
    Move(super::MouseMoveEvent),
    Scroll(
        super::ScrollWheelEvent,
        super::CellMetrics,
        std::time::Instant,
    ),
}

impl super::NativeView {
    pub(super) fn queue_input(
        &mut self,
        event: super::Event,
        action: Action,
        cx: &mut super::Context<Self>,
    ) {
        let input = self.bridge.capture(event);
        if !self.routing.push_coalesced(input, action, |previous, next| {
            matches!((previous, next), (Action::Move(a), Action::Move(b)) if a.pressed_button == b.pressed_button)
        }) {
            self.bridge.input.note_overflow();
        }
        self.route_pending(cx);
    }

    pub(super) fn route_pending(&mut self, cx: &mut super::Context<Self>) {
        use self::Action as RoutingAction;
        use super::*;
        let attachment = self.bridge.attachment.load(Ordering::Acquire);
        let serial = self
            .frame
            .as_ref()
            .filter(|frame| frame.attachment == attachment)
            .map_or(0, |frame| frame.routing_serial);
        while let Some((mut input, action)) = self.routing.next(attachment, serial) {
            let mut copies = 1;
            // Local pointer geometry is valid only for the frame that was seen.
            // Otherwise forward its original identity for the host's stale-frame check.
            let current = self
                .frame
                .as_ref()
                .is_some_and(|frame| frame.id == input.presented);
            match action {
                RoutingAction::Direct => {
                    if let Event::Key(key) = input.event {
                        if self.preview_key(key, cx) {
                            continue;
                        }
                        if let Some(event) = self.window_shortcut(key, cx) {
                            let Some(event) = event else {
                                continue;
                            };
                            input.event = event;
                        }
                    }
                }
                RoutingAction::Down(event) => {
                    if current
                        && ((event.button == MouseButton::Left
                            && self.preview_pointer(event.position, 0, cx))
                            || self.media_mouse_down(&event, cx))
                    {
                        continue;
                    }
                    if event.button == MouseButton::Middle {
                        continue;
                    }
                }
                RoutingAction::Up(event) => {
                    if current
                        && ((event.button == MouseButton::Left
                            && self.preview_pointer(event.position, 2, cx))
                            || self.media_mouse_up(event.position, cx))
                    {
                        continue;
                    }
                    if event.button == MouseButton::Middle {
                        continue;
                    }
                }
                RoutingAction::Move(event) => {
                    if current
                        && (self.preview_pointer(event.position, 1, cx)
                            || self.media_mouse_move(&event, cx))
                    {
                        continue;
                    }
                    if event.pressed_button != Some(MouseButton::Left) {
                        continue;
                    }
                }
                RoutingAction::Scroll(event, metrics, when) => {
                    if current && (self.preview_scroll(&event, cx) || self.media_scroll(&event, cx))
                    {
                        continue;
                    }
                    if matches!(event.touch_phase, TouchPhase::Started) {
                        self.scroll = ScrollAccumulator::default();
                    }
                    let Some((kind, events)) = self
                        .scroll
                        .push(ScrollAccumulator::delta(event.delta, metrics), when)
                    else {
                        continue;
                    };
                    input.event = mouse_event(metrics, event.position, kind, event.modifiers);
                    copies = events;
                }
            }
            let barrier = self
                .frame
                .as_ref()
                .is_some_and(|frame| !frame.previews.is_empty());
            for index in 0..copies {
                if barrier && index + 1 == copies {
                    self.routing.forward(&mut input);
                }
                if !self.bridge.send_captured(input.clone()) {
                    self.routing.clear();
                    break;
                }
            }
        }
    }
}
