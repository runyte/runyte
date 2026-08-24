// SPDX-License-Identifier: MPL-2.0

//! One- and two-character jump labels.
//!
//! `goto-word` paints a short label over every word on screen and jumps to the
//! one whose label you type. This module owns prefix-free label assignment and
//! narrowing; it knows nothing about how a label is drawn, nor about what makes
//! an offset worth labelling. The editor hands it target offsets ordered from
//! nearest to farthest and asks, per offset, which label character belongs
//! there.

use crate::text::Offset;

/// Label characters, in the order labels are handed out.
///
/// Home-row-first ordering means the labels people reach for most often are
/// the ones under their fingers.
const ALPHABET: &[char] = &[
    'a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', 'q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p',
    'z', 'x', 'c', 'v', 'b', 'n', 'm',
];

/// How many targets two characters from the alphabet can name.
const CAPACITY: usize = ALPHABET.len() * ALPHABET.len();

/// What a displayed label character asks for next.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabelPart {
    /// This character completes the label immediately.
    Immediate,
    /// The first character of a two-key label.
    Prefix,
    /// The second character of a two-key label before narrowing.
    Suffix,
}

/// What a keystroke did to the labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Press {
    /// The key narrowed the labels; more are still showing.
    Narrowed,
    /// The key completed a label. Jump here.
    Jumped(Offset),
    /// The key matched nothing. The labels are spent.
    Missed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Label {
    One(char),
    Two([char; 2]),
}

impl Label {
    fn len(self) -> usize {
        match self {
            Self::One(_) => 1,
            Self::Two(_) => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Target {
    offset: Offset,
    label: Label,
}

/// A live set of jump labels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JumpLabels {
    targets: Vec<Target>,
    /// A two-key label's prefix, once it has been typed.
    typed: Option<char>,
}

impl JumpLabels {
    /// Labels ranked `offsets`, nearest first. `None` when there is nothing to
    /// label. Every target is assumed to have room for a two-character label.
    pub fn new(offsets: impl IntoIterator<Item = Offset>) -> Option<Self> {
        Self::with_visible_lengths(offsets.into_iter().map(|offset| (offset, 2)))
    }

    /// Labels ranked targets while respecting how many cells each can show.
    ///
    /// Assignment is regenerated after a target rejects a label that does not
    /// fit. Removing targets can only make labels shorter, so this converges in
    /// at most the number of input targets. Targets beyond [`CAPACITY`] are
    /// dropped rather than left with an unusable label.
    pub fn with_visible_lengths(
        targets: impl IntoIterator<Item = (Offset, usize)>,
    ) -> Option<Self> {
        let mut candidates: Vec<(Offset, usize)> = targets.into_iter().take(CAPACITY).collect();
        loop {
            if candidates.is_empty() {
                return None;
            }
            let labels = labels_for(candidates.len());
            let retained: Vec<(Offset, usize)> = candidates
                .iter()
                .copied()
                .zip(labels.iter().copied())
                .filter_map(|(candidate, label)| (label.len() <= candidate.1).then_some(candidate))
                .collect();
            if retained.len() == candidates.len() {
                let targets = candidates
                    .into_iter()
                    .zip(labels)
                    .map(|((offset, _), label)| Target { offset, label })
                    .collect();
                return Some(Self {
                    targets,
                    typed: None,
                });
            }
            candidates = retained;
        }
    }

    pub fn len(&self) -> usize {
        self.targets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// The label character sitting at `offset`, if any.
    ///
    /// Before input, a two-key label occupies the two cells starting at its
    /// target. After its prefix is typed, only the suffix remains and moves to
    /// the target cell, where it is immediately actionable.
    pub fn label_at(&self, offset: Offset) -> Option<(char, LabelPart)> {
        self.targets
            .iter()
            .find_map(|target| match (self.typed, target.label) {
                (None, Label::One(key)) if target.offset == offset => {
                    Some((key, LabelPart::Immediate))
                }
                (None, Label::Two([first, _])) if target.offset == offset => {
                    Some((first, LabelPart::Prefix))
                }
                (None, Label::Two([_, second])) if target.offset + 1 == offset => {
                    Some((second, LabelPart::Suffix))
                }
                (Some(typed), Label::Two([first, second]))
                    if first == typed && target.offset == offset =>
                {
                    Some((second, LabelPart::Immediate))
                }
                _ => None,
            })
    }

    /// Feeds the next typed character.
    pub fn press(&mut self, key: char) -> Press {
        match self.typed {
            None => match self
                .targets
                .iter()
                .find(|target| target.label == Label::One(key))
            {
                Some(target) => Press::Jumped(target.offset),
                None if self.targets.iter().any(
                    |target| matches!(target.label, Label::Two([first, _]) if first == key),
                ) =>
                {
                    self.typed = Some(key);
                    Press::Narrowed
                }
                None => Press::Missed,
            },
            Some(first) => self
                .targets
                .iter()
                .find(|target| target.label == Label::Two([first, key]))
                .map_or(Press::Missed, |target| Press::Jumped(target.offset)),
        }
    }
}

/// Produces a prefix-free set whose shortest labels come first.
fn labels_for(count: usize) -> Vec<Label> {
    let mut labels: Vec<Label> = ALPHABET.iter().copied().map(Label::One).collect();
    if count <= labels.len() {
        labels.truncate(count);
        return labels;
    }
    while labels.len() < count {
        let index = labels
            .iter()
            .rposition(|label| matches!(label, Label::One(_)))
            .expect("two-key capacity covers every accepted target");
        let Label::One(prefix) = labels.remove(index) else {
            unreachable!();
        };
        let children = (count - labels.len()).min(ALPHABET.len());
        labels.splice(
            index..index,
            ALPHABET
                .iter()
                .copied()
                .take(children)
                .map(|suffix| Label::Two([prefix, suffix])),
        );
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_label_produces_no_labels() {
        assert!(JumpLabels::new(Vec::new()).is_none());
    }

    #[test]
    fn nearby_targets_receive_immediate_labels() {
        let labels = JumpLabels::new(vec![10, 20, 30]).unwrap();
        assert_eq!(labels.label_at(10), Some(('a', LabelPart::Immediate)));
        assert_eq!(labels.label_at(20), Some(('s', LabelPart::Immediate)));
        assert_eq!(labels.label_at(30), Some(('d', LabelPart::Immediate)));
        assert_eq!(labels.label_at(11), None);
    }

    #[test]
    fn the_twenty_seventh_target_expands_the_farthest_single_key() {
        let offsets: Vec<Offset> = (0..ALPHABET.len() + 1).map(|index| index * 10).collect();
        let labels = JumpLabels::new(offsets).unwrap();
        // The first 25 targets stay single-key. The former farthest single key
        // becomes a prefix for the final two targets.
        assert_eq!(
            labels.label_at((ALPHABET.len() - 2) * 10),
            Some(('n', LabelPart::Immediate))
        );
        assert_eq!(
            labels.label_at((ALPHABET.len() - 1) * 10),
            Some(('m', LabelPart::Prefix))
        );
        assert_eq!(
            labels.label_at((ALPHABET.len() - 1) * 10 + 1),
            Some(('a', LabelPart::Suffix))
        );
        assert_eq!(
            labels.label_at(ALPHABET.len() * 10 + 1),
            Some(('s', LabelPart::Suffix))
        );
    }

    #[test]
    fn an_immediate_key_jumps_without_narrowing() {
        let mut labels = JumpLabels::new(vec![10, 20, 30]).unwrap();
        assert_eq!(labels.press('s'), Press::Jumped(20));
    }

    #[test]
    fn two_keystrokes_pick_a_distant_target() {
        let offsets: Vec<Offset> = (0..ALPHABET.len() + 1).map(|index| index * 10).collect();
        let mut labels = JumpLabels::new(offsets).unwrap();
        assert_eq!(labels.press('m'), Press::Narrowed);
        assert_eq!(labels.press('s'), Press::Jumped(ALPHABET.len() * 10));
    }

    #[test]
    fn narrowing_hides_other_labels_and_moves_red_suffixes_to_the_target() {
        let offsets: Vec<Offset> = (0..ALPHABET.len() + 1).map(|index| index * 10).collect();
        let mut labels = JumpLabels::new(offsets).unwrap();
        assert_eq!(labels.press('m'), Press::Narrowed);
        assert_eq!(labels.label_at(0), None);
        assert_eq!(
            labels.label_at((ALPHABET.len() - 1) * 10),
            Some(('a', LabelPart::Immediate))
        );
        assert_eq!(labels.label_at((ALPHABET.len() - 1) * 10 + 1), None);
        assert_eq!(
            labels.label_at(ALPHABET.len() * 10),
            Some(('s', LabelPart::Immediate))
        );
    }

    #[test]
    fn an_unmatched_key_spends_the_labels() {
        let mut labels = JumpLabels::new(vec![10, 20]).unwrap();
        assert_eq!(labels.press('!'), Press::Missed);

        let offsets: Vec<Offset> = (0..ALPHABET.len() + 1).map(|index| index * 10).collect();
        let mut labels = JumpLabels::new(offsets).unwrap();
        assert_eq!(labels.press('m'), Press::Narrowed);
        assert_eq!(labels.press('z'), Press::Missed);
    }

    #[test]
    fn labels_that_do_not_fit_are_removed_and_assignment_is_regenerated() {
        let targets = (0..ALPHABET.len() + 1).map(|index| {
            let visible = if index == ALPHABET.len() - 1 { 1 } else { 2 };
            (index * 10, visible)
        });
        let labels = JumpLabels::with_visible_lengths(targets).unwrap();
        assert_eq!(labels.len(), ALPHABET.len());
        assert_eq!(labels.label_at((ALPHABET.len() - 1) * 10), None);
        assert_eq!(
            labels.label_at(ALPHABET.len() * 10),
            Some(('m', LabelPart::Immediate))
        );
    }

    #[test]
    fn generated_labels_are_prefix_free() {
        for count in [1, ALPHABET.len(), ALPHABET.len() + 1, 51, 52, CAPACITY] {
            let labels = labels_for(count);
            for (index, left) in labels.iter().enumerate() {
                for right in labels.iter().skip(index + 1) {
                    assert!(!is_prefix(*left, *right));
                    assert!(!is_prefix(*right, *left));
                }
            }
        }

        fn is_prefix(left: Label, right: Label) -> bool {
            matches!((left, right), (Label::One(a), Label::Two([b, _])) if a == b)
        }
    }

    #[test]
    fn more_targets_than_labels_are_dropped_rather_than_left_blank() {
        let offsets: Vec<Offset> = (0..CAPACITY + 5).map(|index| index * 10).collect();
        let labels = JumpLabels::new(offsets).unwrap();
        assert_eq!(labels.len(), CAPACITY);
        assert_eq!(labels.label_at(CAPACITY * 10), None);
    }
}
