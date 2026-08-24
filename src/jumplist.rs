// SPDX-License-Identifier: MPL-2.0

//! Per-pane jump history.
//!
//! A jump records where the caret was *before* a non-local move — opening a
//! file, following a definition, choosing a picker result. Local motion does
//! not touch it, which is what keeps the list short enough to be useful.
//!
//! Traversal is browser-shaped: going back and then jumping somewhere new
//! discards the forward history, because a forward entry nobody can reach by
//! going back is a position the person has no way to think about.

use crate::selection::Selection;

/// Coordinate provenance for a remembered selection.
///
/// Runyte selections address characters inclusively, while Vim range
/// operations use half-open spans. Equal coordinates can therefore carry
/// different editing meaning and must travel with a jump.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SelectionSemantics {
    #[default]
    Runyte,
    HalfOpen,
    /// A half-open span that must enter and leave registers as whole lines.
    VimLinewise,
}

/// How many positions a pane remembers. Helix uses 30; the cost here is one
/// selection per entry, so the limit is about keeping the list comprehensible
/// rather than about memory.
const LIMIT: usize = 30;

/// A remembered position: which buffer, and the whole selection, not just the
/// caret. Returning from a jump should restore what was selected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Jump {
    pub buffer: usize,
    /// Host-lifetime terminal identity when the pane was showing a terminal
    /// over `buffer`. Kept as the stable numeric identity so this generic
    /// history module does not depend on terminal process or emulator state.
    pub terminal: Option<u64>,
    pub selection: Selection,
    pub semantics: SelectionSemantics,
}

impl Jump {
    pub fn new(buffer: usize, selection: Selection, semantics: SelectionSemantics) -> Self {
        Self {
            buffer,
            terminal: None,
            selection,
            semantics,
        }
    }

    pub fn with_terminal(mut self, terminal: Option<u64>) -> Self {
        self.terminal = terminal;
        self
    }

    fn same_surface(&self, other: &Self) -> bool {
        self.buffer == other.buffer && self.terminal == other.terminal
    }
}

#[derive(Clone, Debug, Default)]
pub struct JumpList {
    entries: Vec<Jump>,
    /// Where the pane sits within `entries`. Equal to `entries.len()` when the
    /// caret is past the newest entry, which is the state after any jump and
    /// the only state in which "going back" has somewhere new to record.
    current: usize,
}

impl JumpList {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Records the position being left.
    ///
    /// Forward history is discarded, and a jump from a position already at the
    /// head is folded into it so that repeating a jump does not fill the list
    /// with copies of one place.
    pub fn push(&mut self, jump: Jump) {
        self.entries.truncate(self.current);
        if self.entries.last() != Some(&jump) {
            self.entries.push(jump);
            if self.entries.len() > LIMIT {
                self.entries.remove(0);
            }
        }
        self.current = self.entries.len();
    }

    /// Steps back, returning where to go.
    ///
    /// `here` is the current position. The first step back records it, so that
    /// [`JumpList::forward`] can return to where the traversal started rather
    /// than losing it.
    pub fn backward(&mut self, here: Jump) -> Option<Jump> {
        if self.current == 0 {
            return None;
        }
        if self.current == self.entries.len() {
            self.entries.push(here);
        }
        self.current -= 1;
        self.entries.get(self.current).cloned()
    }

    /// Steps forward through positions a previous [`JumpList::backward`] left
    /// behind. Returns `None` at the newest entry.
    pub fn forward(&mut self) -> Option<Jump> {
        if self.current + 1 >= self.entries.len() {
            return None;
        }
        self.current += 1;
        self.entries.get(self.current).cloned()
    }

    /// Maps every remembered selection in `buffer` through a transaction.
    ///
    /// Without this a jump would point at wherever its offsets happened to
    /// land after later edits, which is worse than not remembering it at all.
    pub fn map(&mut self, buffer: usize, transaction: &crate::text::Transaction) {
        for jump in &mut self.entries {
            if jump.buffer == buffer {
                jump.selection = jump.selection.map(transaction);
            }
        }
    }

    /// The most recent buffer this pane was in before `exclude`, if any.
    ///
    /// Closing a buffer has to put the pane somewhere, and "where you came
    /// from" is the only answer a reader can predict. Scanning backwards from
    /// the current position rather than from the newest entry keeps it
    /// consistent with what one [`JumpList::backward`] would do.
    pub fn previous_buffer(&self, exclude: usize) -> Option<usize> {
        self.entries[..self.current]
            .iter()
            .rev()
            .map(|jump| jump.buffer)
            .find(|buffer| *buffer != exclude)
    }

    /// Steps back to the newest entry in a different buffer, skipping every
    /// position recorded within the current one.
    ///
    /// Reading a long document records a jump per section; stepping through
    /// them one at a time to leave the document is the same work as scrolling
    /// back by hand. `here` is recorded exactly as [`JumpList::backward`]
    /// records it, so the two traversals share one history.
    pub fn backward_across_buffers(&mut self, here: Jump) -> Option<Jump> {
        let origin = here.clone();
        let mut destination = self.backward(here)?;
        while destination.same_surface(&origin) {
            destination = self.backward(destination)?;
        }
        Some(destination)
    }

    /// Steps forward to the next entry in a different buffer.
    pub fn forward_across_buffers(&mut self, here: &Jump) -> Option<Jump> {
        let mut destination = self.forward()?;
        while destination.same_surface(here) {
            destination = self.forward()?;
        }
        Some(destination)
    }

    /// Drops every entry for a buffer, for callers that retire one.
    pub fn forget(&mut self, buffer: usize) {
        self.entries.retain(|jump| jump.buffer != buffer);
        self.current = self.current.min(self.entries.len());
    }

    /// Retires document positions while preserving terminal surface entries.
    ///
    /// A terminal jump needs a live backing buffer to reveal if the child
    /// exits, but the terminal identity remains valid after that buffer is
    /// closed. Ordinary positions into the closed document are simply gone.
    pub fn retire_buffer(&mut self, buffer: usize, replacement: usize) {
        self.entries.retain_mut(|jump| {
            if jump.buffer != buffer {
                return true;
            }
            if jump.terminal.is_some() {
                jump.buffer = replacement;
                true
            } else {
                false
            }
        });
        self.current = self.current.min(self.entries.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Transaction;

    fn jump(offset: usize) -> Jump {
        Jump::new(0, Selection::point(offset), SelectionSemantics::Runyte)
    }

    fn offset(jump: &Jump) -> usize {
        jump.selection.primary().head
    }

    #[test]
    fn an_empty_list_has_nowhere_to_go() {
        let mut jumps = JumpList::default();
        assert!(jumps.backward(jump(5)).is_none());
        assert!(jumps.forward().is_none());
        assert!(jumps.is_empty());
    }

    #[test]
    fn going_back_records_the_starting_point_so_forward_can_return() {
        let mut jumps = JumpList::default();
        jumps.push(jump(10));
        jumps.push(jump(20));
        // The pane is now at 30, having jumped away from 20.

        assert_eq!(offset(&jumps.backward(jump(30)).unwrap()), 20);
        assert_eq!(offset(&jumps.backward(jump(20)).unwrap()), 10);
        assert!(jumps.backward(jump(10)).is_none(), "the list has an end");

        assert_eq!(offset(&jumps.forward().unwrap()), 20);
        assert_eq!(offset(&jumps.forward().unwrap()), 30);
        assert!(jumps.forward().is_none(), "and so does the other end");
    }

    #[test]
    fn a_new_jump_discards_the_forward_history() {
        let mut jumps = JumpList::default();
        jumps.push(jump(10));
        jumps.push(jump(20));
        jumps.backward(jump(30));
        assert_eq!(jumps.len(), 3, "30 was recorded on the way back");

        // Jumping away from 20 to somewhere new: 30 is no longer reachable.
        jumps.push(jump(20));
        assert!(jumps.forward().is_none());
        assert_eq!(offset(&jumps.backward(jump(99)).unwrap()), 20);
    }

    #[test]
    fn repeating_a_jump_from_one_place_is_a_single_entry() {
        let mut jumps = JumpList::default();
        jumps.push(jump(10));
        jumps.push(jump(10));
        jumps.push(jump(10));
        assert_eq!(jumps.len(), 1);
    }

    #[test]
    fn the_list_is_bounded_and_drops_its_oldest_entry() {
        let mut jumps = JumpList::default();
        for offset in 0..LIMIT + 10 {
            jumps.push(jump(offset));
        }
        assert_eq!(jumps.len(), LIMIT);
        // The oldest reachable entry is the first that survived the cap.
        let mut oldest = jumps.backward(jump(9999)).unwrap();
        while let Some(previous) = jumps.backward(oldest.clone()) {
            oldest = previous;
        }
        assert_eq!(offset(&oldest), 10);
    }

    #[test]
    fn edits_move_remembered_positions_with_the_text() {
        let mut jumps = JumpList::default();
        jumps.push(Jump::new(
            0,
            Selection::point(10),
            SelectionSemantics::HalfOpen,
        ));
        jumps.push(Jump::new(
            1,
            Selection::point(10),
            SelectionSemantics::Runyte,
        ));

        // Five characters inserted at the start of buffer 0.
        jumps.map(0, &Transaction::insert(0, "abcde"));

        let first = jumps.backward(jump(0)).unwrap();
        assert_eq!(first.buffer, 1, "the newest entry is the other buffer");
        assert_eq!(offset(&first), 10, "an unrelated buffer is untouched");
        let second = jumps.backward(first).unwrap();
        assert_eq!(second.buffer, 0);
        assert_eq!(offset(&second), 15);
        assert_eq!(second.semantics, SelectionSemantics::HalfOpen);
    }

    fn in_buffer(buffer: usize, offset: usize) -> Jump {
        Jump::new(buffer, Selection::point(offset), SelectionSemantics::Runyte)
    }

    #[test]
    fn a_buffer_level_step_skips_every_position_within_one_buffer() {
        let mut jumps = JumpList::default();
        jumps.push(in_buffer(0, 1));
        jumps.push(in_buffer(1, 10));
        jumps.push(in_buffer(1, 20));
        jumps.push(in_buffer(1, 30));

        // One step leaves buffer 1 entirely, past all three of its positions.
        let back = jumps.backward_across_buffers(in_buffer(1, 40)).unwrap();
        assert_eq!(back.buffer, 0);
        assert_eq!(offset(&back), 1);

        // Coming forward lands on a recorded position, not the buffer's top.
        let forward = jumps.forward_across_buffers(&in_buffer(0, 1)).unwrap();
        assert_eq!(forward.buffer, 1);
        assert_eq!(offset(&forward), 10);
    }

    #[test]
    fn a_buffer_level_step_reports_the_end_rather_than_stopping_short() {
        let mut jumps = JumpList::default();
        jumps.push(in_buffer(2, 1));
        jumps.push(in_buffer(2, 2));

        // Every entry is the buffer we are already in, so there is nowhere to
        // go and the traversal must not land back where it started.
        assert!(jumps.backward_across_buffers(in_buffer(2, 3)).is_none());
        assert!(jumps.forward_across_buffers(&in_buffer(2, 3)).is_none());
    }

    #[test]
    fn a_surface_level_step_distinguishes_a_terminal_from_its_buffer() {
        let mut jumps = JumpList::default();
        let buffer = in_buffer(4, 2);
        let terminal = in_buffer(4, 2).with_terminal(Some(9));
        jumps.push(buffer.clone());

        assert_eq!(
            jumps.backward_across_buffers(terminal.clone()).unwrap(),
            buffer
        );
        assert_eq!(jumps.forward_across_buffers(&buffer).unwrap(), terminal);
    }

    #[test]
    fn the_previous_buffer_is_the_most_recent_one_that_is_not_excluded() {
        let mut jumps = JumpList::default();
        assert_eq!(jumps.previous_buffer(9), None);

        jumps.push(in_buffer(0, 1));
        jumps.push(in_buffer(3, 2));
        jumps.push(in_buffer(7, 3));

        assert_eq!(jumps.previous_buffer(9), Some(7));
        // Excluding the newest falls through to the one before it, which is
        // what closing a buffer needs.
        assert_eq!(jumps.previous_buffer(7), Some(3));
    }

    #[test]
    fn forgetting_a_buffer_keeps_the_cursor_within_the_list() {
        let mut jumps = JumpList::default();
        jumps.push(Jump::new(
            0,
            Selection::point(1),
            SelectionSemantics::Runyte,
        ));
        jumps.push(Jump::new(
            1,
            Selection::point(2),
            SelectionSemantics::Runyte,
        ));
        jumps.push(Jump::new(
            0,
            Selection::point(3),
            SelectionSemantics::Runyte,
        ));

        jumps.forget(0);

        assert_eq!(jumps.len(), 1);
        assert_eq!(offset(&jumps.backward(jump(9)).unwrap()), 2);
    }

    #[test]
    fn retiring_a_buffer_preserves_terminal_surfaces_with_a_live_backing() {
        let mut jumps = JumpList::default();
        jumps.push(jump(1));
        jumps.push(jump(2).with_terminal(Some(7)));

        jumps.retire_buffer(0, 4);

        assert_eq!(jumps.len(), 1);
        let target = jumps.backward(Jump::new(
            9,
            Selection::point(0),
            SelectionSemantics::Runyte,
        ));
        assert_eq!(target.unwrap().buffer, 4);
    }
}
