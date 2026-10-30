// SPDX-License-Identifier: MPL-2.0
//! Painted pointer targets and local gesture ownership, independent of hit tests.
use runyte::input::PointerButton;
use std::path::PathBuf;

pub(super) type Source = (Option<std::time::SystemTime>, u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Target {
    Media {
        pane: usize,
        path: PathBuf,
        page: usize,
        source: Option<Source>,
    },
    Preview(super::preview::Target),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MediaAdmission {
    Start,
    FocusOnly,
    Discard,
}
impl Target {
    fn media_admission(
        &self,
        current: &super::MediaPane,
        loaded: Option<Source>,
    ) -> MediaAdmission {
        match self {
            Self::Media {
                pane,
                path,
                page,
                source,
            } if *pane == current.pane && *path == current.path && *page == current.page => {
                match source {
                    None => MediaAdmission::FocusOnly,
                    Some(source) if loaded == Some(*source) => MediaAdmission::Start,
                    _ => MediaAdmission::Discard,
                }
            }
            _ => MediaAdmission::Discard,
        }
    }
}
#[derive(Clone)]
pub(super) struct Surface {
    pub body: runyte::layout::Rect,
    pub target: Target,
}
impl Surface {
    pub fn contains(&self, column: f32, row: f32) -> bool {
        column >= f32::from(self.body.x)
            && column < f32::from(self.body.x.saturating_add(self.body.width))
            && row >= f32::from(self.body.y)
            && row < f32::from(self.body.y.saturating_add(self.body.height))
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Step {
    Move,
    Release,
    Cancel,
    Consume,
}
#[derive(Clone, Debug)]
pub(super) struct Capture {
    pub attachment: u64,
    pub target: Target,
    pub button: PointerButton,
    /// Factory media gestures historically follow any held button. Effective
    /// factory maps retain that policy; replacement maps use the initiating one.
    pub strict: bool,
    pub cancelled: bool,
}
impl Capture {
    pub fn motion(
        &mut self,
        attachment: u64,
        pressed: Option<PointerButton>,
        current: bool,
    ) -> Step {
        if self.cancelled {
            return Step::Consume;
        }
        if attachment != self.attachment
            || !current
            || pressed.is_none()
            || ((self.strict || matches!(self.target, Target::Preview(_)))
                && pressed != Some(self.button))
        {
            self.cancelled = true;
            Step::Cancel
        } else {
            Step::Move
        }
    }
    /// Returns whether the initiating physical button has ended. Unmatched
    /// releases may cancel the action, but never give its remaining tail away.
    pub fn release(&mut self, button: PointerButton) -> (Step, bool) {
        if button == self.button {
            return (
                if self.cancelled {
                    Step::Consume
                } else {
                    Step::Release
                },
                true,
            );
        }
        // Default preview selection used only its left release; retain that
        // compatibility while consuming unrelated releases on the owned surface.
        if matches!(self.target, Target::Preview(_)) && !self.strict {
            return (Step::Consume, false);
        }
        self.cancelled = true;
        (Step::Cancel, false)
    }
}

pub(super) fn event(input: &super::NativeInput) -> Option<runyte::input::PointerEvent> {
    match runyte::tui::input::convert_event(input.event.clone()) {
        Ok(Some(runyte::input::InputEvent::Pointer(event))) => Some(event),
        _ => None,
    }
}

pub(super) fn reset_capture(capture: &mut Option<Capture>, attachment: u64, fresh: bool) -> bool {
    if fresh
        || capture
            .as_ref()
            .is_some_and(|capture| capture.attachment != attachment)
    {
        *capture = None;
        true
    } else {
        false
    }
}
fn factory_preview_fallback(
    target: &Target,
    button: PointerButton,
    bindings: &runyte::keymap::pointer_actions::Bindings,
) -> bool {
    matches!(target, Target::Preview(_))
        && button == PointerButton::Right
        && bindings.is_default(runyte::keymap::pointer_actions::Action::PreviewSelect)
}

impl super::NativeView {
    pub(super) fn pointer_target_current(&mut self, target: &Target) -> bool {
        let Some(frame) = &self.frame else {
            return false;
        };
        match target {
            Target::Media { path, page, .. } => {
                let loaded = self
                    .media
                    .get(path, *page)
                    .and_then(|loaded| loaded.ok())
                    .map(|page| page.source);
                frame
                    .media
                    .iter()
                    .any(|pane| target.media_admission(pane, loaded) == MediaAdmission::Start)
            }
            Target::Preview(target) => {
                frame.previews.iter().any(|pane| {
                    pane.pane == target.pane
                        && pane.document.source == target.source
                        && pane.document.generation == target.generation
                }) && self.previews.pointer_visible(target.pane)
            }
        }
    }
    pub(super) fn cancel_local_pointer(&mut self) {
        self.previews.cancel_drag();
        for view in self.viewports.values_mut() {
            view.drag = None;
        }
    }
    /// An owned tail never falls through to another pane's hit test.
    pub(super) fn local_pointer_tail(
        &mut self,
        input: &super::NativeInput,
        action: &super::input_routing::Action,
        cx: &mut super::Context<Self>,
    ) -> bool {
        use super::input_routing::Action;
        let Some(mut capture) = self.local_pointer.take() else {
            return false;
        };
        let current = self.pointer_target_current(&capture.target)
            && self
                .frame
                .as_ref()
                .is_some_and(|f| f.media_input && f.attachment == capture.attachment);
        let (step, clear) = match action {
            Action::Move(event) => (
                capture.motion(
                    input.attachment,
                    event.pressed_button.map(|b| match b {
                        super::MouseButton::Left => PointerButton::Left,
                        super::MouseButton::Middle => PointerButton::Middle,
                        _ => PointerButton::Right,
                    }),
                    current,
                ),
                false,
            ),
            Action::Up(event) => {
                if !current {
                    capture.cancelled = true;
                }
                capture.release(match event.button {
                    super::MouseButton::Left => PointerButton::Left,
                    super::MouseButton::Middle => PointerButton::Middle,
                    _ => PointerButton::Right,
                })
            }
            _ => {
                self.local_pointer = Some(capture);
                return false;
            }
        };
        match step {
            Step::Move => match (&capture.target, action) {
                (Target::Media { .. }, Action::Move(event)) => {
                    self.media_mouse_move(event, cx);
                }
                // Crossing into another preview cannot retarget selection.
                (Target::Preview(target), Action::Move(event))
                    if input.pointer_target.as_ref() == Some(&Target::Preview(*target)) =>
                {
                    self.preview_pointer(event.position, 1, cx);
                }
                _ => {}
            },
            Step::Release => {
                if let (Target::Preview(target), Action::Up(event)) = (&capture.target, action)
                    && input.pointer_target.as_ref() == Some(&Target::Preview(*target))
                {
                    self.preview_pointer(event.position, 2, cx);
                }
                self.cancel_local_pointer();
                cx.notify();
            }
            Step::Cancel => {
                self.cancel_local_pointer();
                cx.notify();
            }
            Step::Consume => {}
        }
        if !clear {
            self.local_pointer = Some(capture);
        }
        true
    }
    pub(super) fn begin_local_pointer(
        &mut self,
        input: &super::NativeInput,
        event: &super::MouseDownEvent,
        cx: &mut super::Context<Self>,
    ) -> bool {
        use runyte::keymap::pointer_actions::Action;
        let Some(target) = input.pointer_target.clone() else {
            return false;
        };
        let Some(pointer) = event_pointer(input) else {
            return true;
        };
        let runyte::input::PointerEventKind::Down(button) = pointer.kind else {
            return true;
        };
        // Factory right-clicks on a preview historically reach host copy.
        // Removing/replacing preview selection instead makes unmatched buttons inert.
        if factory_preview_fallback(&target, button, &input.pointer_bindings) {
            return false;
        }
        let current = self.pointer_target_current(&target)
            && self.frame.as_ref().is_some_and(|f| f.id == input.presented);
        let strict = match &target {
            Target::Media { .. } => [
                Action::MediaPan,
                Action::MediaRegion,
                Action::MediaAutoSelect,
                Action::MediaToggleZoom,
            ]
            .into_iter()
            .find(|action| input.pointer_bindings.press(*action, pointer))
            .is_some_and(|action| !input.pointer_bindings.is_default(action)),
            Target::Preview(_) => !input.pointer_bindings.is_default(Action::PreviewSelect),
        };
        self.local_pointer = Some(Capture {
            attachment: input.attachment,
            target: target.clone(),
            button,
            strict,
            cancelled: !current,
        });
        if !current {
            // Loading surfaces historically accept focus before their raster
            // arrives. Never start selection against content not yet painted.
            if matches!(target, Target::Media { source: None, .. })
                && self
                    .frame
                    .as_ref()
                    .is_some_and(|f| f.id == input.presented && f.media_input)
                && !self
                    .bridge
                    .blocked_media
                    .lock()
                    .unwrap()
                    .blocks(event.position)
                && [
                    Action::MediaPan,
                    Action::MediaRegion,
                    Action::MediaAutoSelect,
                ]
                .into_iter()
                .any(|action| input.pointer_bindings.press(action, pointer))
                && let Some(pane) = self
                    .frame
                    .as_ref()
                    .and_then(|f| {
                        f.media.iter().find(|pane| {
                            target.media_admission(pane, None) == MediaAdmission::FocusOnly
                        })
                    })
                    .cloned()
            {
                self.focus_media(&pane, 0);
            }
            return true;
        }
        match target {
            Target::Media { .. } => self.media_mouse_down(event, input, cx),
            Target::Preview(_) => {
                if !input.pointer_bindings.press(Action::PreviewSelect, pointer) {
                    return true;
                }
                self.preview_pointer(event.position, 0, cx)
            }
        }
    }
}
fn event_pointer(input: &super::NativeInput) -> Option<runyte::input::PointerEvent> {
    event(input)
}

#[cfg(test)]
#[path = "tests/pointer.rs"]
mod tests;

pub(super) fn overflow_hint(bindings: &runyte::keymap::pointer_actions::Bindings) -> String {
    use runyte::keymap::pointer_actions::Action;
    if bindings.is_default(Action::InputOverflowDismiss) {
        "Some input was lost. Click to dismiss.".into()
    } else {
        format!(
            "Some input was lost. Dismiss: {}.",
            bindings.label(Action::InputOverflowDismiss)
        )
    }
}
pub(super) fn dismiss_overflow(
    queue: &super::input_queue::Sender,
    bindings: &runyte::keymap::pointer_actions::Bindings,
    physical: crossterm::event::Event,
) -> bool {
    if let Ok(Some(runyte::input::InputEvent::Pointer(event))) =
        runyte::tui::input::convert_event(physical)
        && bindings.press(
            runyte::keymap::pointer_actions::Action::InputOverflowDismiss,
            event,
        )
    {
        queue.dismiss_overflow();
        true
    } else {
        false
    }
}
