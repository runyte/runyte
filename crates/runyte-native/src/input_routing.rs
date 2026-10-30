// SPDX-License-Identifier: MPL-2.0
//! Preserve physical-input identity while waiting for editor-owned routing state.
use super::NativeInput;
use std::collections::VecDeque;

pub(super) fn native_pointer_button(button: super::MouseButton) -> crossterm::event::MouseButton {
    match button {
        super::MouseButton::Left => crossterm::event::MouseButton::Left,
        super::MouseButton::Middle => crossterm::event::MouseButton::Middle,
        _ => crossterm::event::MouseButton::Right,
    }
}

pub(super) fn pointer_motion(
    button: Option<super::MouseButton>,
) -> crossterm::event::MouseEventKind {
    button.map_or(crossterm::event::MouseEventKind::Moved, |button| {
        crossterm::event::MouseEventKind::Drag(native_pointer_button(button))
    })
}

// Passive movement remains local, but must coalesce while routing waits just
// like a drag. Never cross attachment, painted geometry or modifier boundaries.
fn same_passive_motion(previous: &NativeInput, next: &NativeInput) -> bool {
    use crossterm::event::{Event, MouseEventKind};
    previous.attachment == next.attachment
        && previous.presented == next.presented
        && matches!((&previous.event, &next.event), (Event::Mouse(a), Event::Mouse(b))
            if a.kind == MouseEventKind::Moved && b.kind == MouseEventKind::Moved && a.modifiers == b.modifiers)
}

/// Overlay cells own wheel input even when native media lies underneath.
fn overlay_owns_scroll(overlays: &[runyte::layout::Rect], column: f32, row: f32) -> bool {
    overlays.iter().any(|area| {
        column >= f32::from(area.x)
            && column < f32::from(area.x.saturating_add(area.width))
            && row >= f32::from(area.y)
            && row < f32::from(area.y.saturating_add(area.height))
    })
}

#[derive(Default)]
pub(super) struct Queue<T = ()> {
    inputs: VecDeque<(NativeInput, T)>,
    bytes: usize,
    pending: Option<(u64, u64)>,
    serial: u64,
    pane_gesture: Option<(u64, crossterm::event::MouseButton)>,
}

#[cfg(test)]
#[path = "tests/input_routing.rs"]
mod tests;

impl<T> Queue<T> {
    pub fn clear(&mut self) {
        self.inputs.clear();
        self.bytes = 0;
        self.pending = None;
        self.pane_gesture = None;
    }
    /// Keep an editor-owned Alt-left gesture out of native image panning and
    /// document selection, including when Alt is released before the button.
    fn editor_pane_gesture(&mut self, input: &NativeInput) -> bool {
        use crossterm::event::{Event, MouseEventKind};
        if self
            .pane_gesture
            .is_some_and(|(attachment, _)| attachment != input.attachment)
        {
            self.pane_gesture = None;
        }
        match &input.event {
            Event::Mouse(event) => match event.kind {
                MouseEventKind::Down(button) => {
                    let claimed = matches!(runyte::tui::input::convert_event(input.event.clone()), Ok(Some(runyte::input::InputEvent::Pointer(pointer))) if input.pane_gestures.action(pointer).is_some());
                    self.pane_gesture = claimed.then_some((input.attachment, button));
                    self.pane_gesture.is_some()
                }
                MouseEventKind::Up(button) => {
                    let owned = self.pane_gesture.is_some();
                    if self.pane_gesture.is_some_and(|(_, captured)| {
                        captured == crossterm::event::MouseButton::Left || captured == button
                    }) {
                        self.pane_gesture = None;
                    }
                    owned
                }
                _ => self.pane_gesture.is_some(),
            },
            Event::FocusLost => {
                self.pane_gesture = None;
                false
            }
            // Keys must reach the editor so Escape can cancel its drag. Keep
            // owning the trailing mouse release after cancellation.
            _ => self.pane_gesture.is_some(),
        }
    }
    /// A release outside the window may never reach GPUI's mouse-up handler.
    /// Cancel on the first buttonless motion instead of turning hover into a
    /// synthetic left drag (or committing an unseen drop with a synthetic up).
    fn cancel_lost_pane_button(
        &mut self,
        input: &mut NativeInput,
        pressed: Option<crossterm::event::MouseButton>,
    ) {
        if self.pane_gesture.is_some_and(|(attachment, button)| {
            attachment == input.attachment && pressed != Some(button)
        }) {
            self.pane_gesture = None;
            input.event = crossterm::event::Event::FocusLost;
        }
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
            && (super::input_queue::same_drag(previous, &input)
                || same_passive_motion(previous, &input))
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
    pub fn next(
        &mut self,
        attachment: u64,
        acknowledged: u64,
        routing_ready: bool,
    ) -> Option<(NativeInput, T)> {
        if !routing_ready {
            return None;
        }
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

fn window_requires_barrier(
    captured: &runyte::keymap::native_window::Bindings,
    current: Option<&runyte::keymap::native_window::Bindings>,
) -> bool {
    captured.has_authored_characters()
        || current.is_some_and(|bindings| bindings.has_authored_characters())
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
        let routing_ready = self.preview_routing_ready
            || self.frame.as_ref().is_none_or(|frame| {
                !frame.media_input || !frame.previews.iter().any(|pane| pane.active)
            });
        while let Some((mut input, action)) = self.routing.next(attachment, serial, routing_ready) {
            let mut copies = 1;
            // Local pointer geometry is valid only for the frame that was seen.
            // Otherwise forward its original identity for the host's stale-frame check.
            let current = self
                .frame
                .as_ref()
                .is_some_and(|frame| frame.id == input.presented);
            if let RoutingAction::Move(event) = &action {
                self.routing.cancel_lost_pane_button(
                    &mut input,
                    event.pressed_button.map(native_pointer_button),
                );
            }
            let cancelled = matches!(input.event, Event::FocusLost);
            if cancelled
                || matches!(&action, RoutingAction::Move(event) if event.pressed_button != Some(MouseButton::Left))
                || (!current && matches!(&action, RoutingAction::Up(_)))
            {
                self.previews.cancel_drag();
            }
            if !self.routing.editor_pane_gesture(&input) && !cancelled {
                match action {
                    RoutingAction::Direct => {
                        if let Event::Key(key) = input.event {
                            let window_bindings = self
                                .frame
                                .as_ref()
                                .filter(|frame| frame.attachment == input.attachment)
                                .map_or_else(
                                    || input.window_bindings.clone(),
                                    |frame| {
                                        input
                                            .window_bindings
                                            .with_current_guard(&frame.window_bindings)
                                    },
                                );
                            if self.preview_key(key, &input, &window_bindings, cx) {
                                continue;
                            }
                            if let Some(event) = self.window_shortcut(key, &window_bindings, cx) {
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
                        let over_overlay = self.frame.as_ref().is_some_and(|frame| {
                            overlay_owns_scroll(
                                &frame.overlays,
                                f32::from(event.position.x) / metrics.width,
                                f32::from(event.position.y) / metrics.height,
                            )
                        });
                        if current
                            && !over_overlay
                            && (self.preview_scroll(&event, cx) || self.media_scroll(&event, cx))
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
            }
            let barrier = self
                .frame
                .as_ref()
                .is_some_and(|frame| !frame.previews.is_empty())
                || window_requires_barrier(
                    &input.window_bindings,
                    self.frame.as_ref().map(|frame| &frame.window_bindings),
                );
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
