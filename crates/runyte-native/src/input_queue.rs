// SPDX-License-Identifier: MPL-2.0

//! Bounded nonblocking physical input, with coalesced drags and a separate
//! presentation slot. Overflow is retained until the window acknowledges it.
use super::NativeInput;
use crossterm::event::{Event, MouseEventKind};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::Notify;

const MAX_QUEUED_TEXT_BYTES: usize = 8 * 1024 * 1024;
struct State {
    physical: VecDeque<NativeInput>,
    acknowledgement: Option<NativeInput>,
    bytes: usize,
    closed: bool,
}
struct Shared {
    state: Mutex<State>,
    ready: Notify,
    capacity: usize,
    senders: AtomicUsize,
    overflow: AtomicBool,
}
pub(super) struct Sender(Arc<Shared>);
pub(crate) struct Receiver(Arc<Shared>);
#[derive(Debug)]
pub(super) enum SendError {
    Full,
    Closed,
}

pub(super) fn channel(capacity: usize) -> (Sender, Receiver) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            physical: VecDeque::new(),
            acknowledgement: None,
            bytes: 0,
            closed: false,
        }),
        ready: Notify::new(),
        capacity,
        senders: AtomicUsize::new(1),
        overflow: AtomicBool::new(false),
    });
    (Sender(shared.clone()), Receiver(shared))
}
fn text_bytes(input: &NativeInput) -> usize {
    match &input.event {
        Event::Paste(text) => text.len(),
        _ => 0,
    }
}
pub(super) fn same_drag(previous: &NativeInput, next: &NativeInput) -> bool {
    if previous.attachment != next.attachment || previous.presented != next.presented {
        return false;
    }
    match (&previous.event, &next.event) {
        (Event::Mouse(a), Event::Mouse(b)) => matches!((a.kind, b.kind),
            (MouseEventKind::Drag(left), MouseEventKind::Drag(right))
                if left == right && a.modifiers == b.modifiers),
        _ => false,
    }
}
impl Sender {
    pub fn note_overflow(&self) {
        self.0.overflow.store(true, Ordering::Release);
    }
    pub fn try_send(&self, input: NativeInput) -> Result<(), SendError> {
        let mut state = self.0.state.lock().unwrap();
        if state.closed {
            self.0.overflow.store(true, Ordering::Release);
            return Err(SendError::Closed);
        }
        if input.presentation_only {
            state.acknowledgement = Some(input);
        } else if let Some(previous) = state.physical.back_mut()
            && same_drag(previous, &input)
        {
            *previous = input;
        } else {
            let bytes = text_bytes(&input);
            if state.physical.len() >= self.0.capacity
                || bytes > MAX_QUEUED_TEXT_BYTES.saturating_sub(state.bytes)
            {
                self.0.overflow.store(true, Ordering::Release);
                return Err(SendError::Full);
            }
            state.bytes += bytes;
            state.physical.push_back(input);
        }
        drop(state);
        self.0.ready.notify_one();
        Ok(())
    }
    pub fn overflowed(&self) -> bool {
        self.0.overflow.load(Ordering::Acquire)
    }
    pub fn dismiss_overflow(&self) {
        self.0.overflow.store(false, Ordering::Release);
    }
    #[cfg(test)]
    pub async fn send(&self, input: NativeInput) -> Result<(), SendError> {
        self.try_send(input)
    }
}
impl Clone for Sender {
    fn clone(&self) -> Self {
        self.0.senders.fetch_add(1, Ordering::Relaxed);
        Self(self.0.clone())
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        if self.0.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.state.lock().unwrap().closed = true;
            self.0.ready.notify_one();
        }
    }
}
impl Receiver {
    /// Pointer and lifecycle events are publication barriers: their geometry
    /// must not inherit an earlier key's unpublished layout changes.
    pub(super) fn has_batchable_input(&self) -> bool {
        self.0
            .state
            .lock()
            .unwrap()
            .physical
            .front()
            .is_some_and(|input| matches!(input.event, Event::Key(_) | Event::Paste(_)))
    }

    pub async fn recv(&mut self) -> Option<NativeInput> {
        loop {
            let ready = self.0.ready.notified();
            {
                let mut state = self.0.state.lock().unwrap();
                if let Some(input) = state.physical.pop_front() {
                    state.bytes -= text_bytes(&input);
                    return Some(input);
                }
                if let Some(input) = state.acknowledgement.take() {
                    return Some(input);
                }
                if state.closed {
                    return None;
                }
            }
            ready.await;
        }
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap();
        state.closed = true;
        state.physical.clear();
        state.acknowledgement = None;
        state.bytes = 0;
    }
}

#[cfg(test)]
#[path = "tests/input_queue.rs"]
mod tests;
