// SPDX-License-Identifier: MPL-2.0

//! Pure pane-local structural selection and expansion-history mechanics.

use crate::{
    command::Mode,
    jumplist::SelectionSemantics,
    selection::{Range, Selection},
    syntax::{
        DelimiterPair, DocumentSyntax, Registry, SyntaxCapture, SyntaxError, SyntaxObject,
        SyntaxObjectPart, SyntaxRange, SyntaxSelectionRange, SyntaxSelectionTransform,
    },
    text::Text,
};

#[derive(Clone, Debug, Default)]
pub(crate) struct ExpansionHistory {
    frames: Vec<ExpansionFrame>,
}

pub(crate) fn select_delimiter(
    syntax: &DocumentSyntax,
    text: &Text,
    registry: &Registry,
    selection: &Selection,
    pair: Option<DelimiterPair>,
    part: SyntaxObjectPart,
) -> Result<Option<Selection>, SyntaxError> {
    let mut changed = false;
    let ranges = selection
        .ranges()
        .iter()
        .map(|range| {
            let selected = syntax.enclosing_delimiter(
                text,
                registry,
                SyntaxRange::new(range.from(), range.to())?,
                pair,
                part,
            )?;
            let Some(selected) = selected else {
                return Ok(*range);
            };
            changed = true;
            Ok(if range.anchor > range.head {
                Range::new(selected.to, selected.from)
            } else {
                Range::new(selected.from, selected.to)
            })
        })
        .collect::<Result<Vec<_>, SyntaxError>>()?;
    Ok(changed.then(|| Selection::new(ranges, selection.primary_index())))
}

#[derive(Clone, Debug)]
struct ExpansionFrame {
    before: SelectionSnapshot,
    after: SelectionSnapshot,
}

#[derive(Clone, Debug)]
struct SelectionSnapshot {
    ranges: Vec<DirectedRange>,
    primary: usize,
    mode: Mode,
    semantics: SelectionSemantics,
}

#[derive(Clone, Copy, Debug)]
struct DirectedRange {
    range: SyntaxSelectionRange,
    reversed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryReset {
    Stale,
    Mismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ShrinkResult {
    Restored {
        selection: Selection,
        mode: Mode,
        semantics: SelectionSemantics,
    },
    Empty,
    Reset(HistoryReset),
}

impl SelectionSnapshot {
    fn capture(
        syntax: &DocumentSyntax,
        text: &Text,
        selection: &Selection,
        mode: Mode,
        semantics: SelectionSemantics,
    ) -> Result<Self, SyntaxError> {
        let ranges = selection
            .ranges()
            .iter()
            .map(|range| {
                Ok(DirectedRange {
                    range: syntax
                        .selection_range(text, SyntaxRange::new(range.from(), range.to())?)?,
                    reversed: range.anchor > range.head,
                })
            })
            .collect::<Result<_, SyntaxError>>()?;
        Ok(Self {
            ranges,
            primary: selection.primary_index(),
            mode,
            semantics,
        })
    }

    fn resolve(&self, syntax: &DocumentSyntax, text: &Text) -> Result<Selection, SyntaxError> {
        let ranges = self
            .ranges
            .iter()
            .map(|directed| {
                let range = syntax.resolve_selection_range(text, directed.range)?;
                Ok(if directed.reversed {
                    Range::new(range.to, range.from)
                } else {
                    Range::new(range.from, range.to)
                })
            })
            .collect::<Result<_, SyntaxError>>()?;
        Ok(Selection::new(ranges, self.primary))
    }
}

impl ExpansionHistory {
    pub(crate) fn clear(&mut self) {
        self.frames.clear();
    }

    pub(crate) fn expand(
        &mut self,
        syntax: &DocumentSyntax,
        text: &Text,
        registry: &Registry,
        selection: &Selection,
        mode: Mode,
        semantics: SelectionSemantics,
    ) -> Result<Option<Selection>, SyntaxError> {
        if let Some(frame) = self.frames.last() {
            let connected = match frame.after.resolve(syntax, text) {
                Ok(after) => {
                    after == *selection
                        && frame.after.mode == mode
                        && frame.after.semantics == semantics
                }
                Err(SyntaxError::StaleRevision { .. } | SyntaxError::ForeignDocument) => false,
                Err(error) => return Err(error),
            };
            if !connected {
                self.clear();
            }
        }
        let before = SelectionSnapshot::capture(syntax, text, selection, mode, semantics)?;
        let Some(expanded) = transform_selection(
            syntax,
            text,
            registry,
            selection,
            SyntaxSelectionTransform::Expand,
        )?
        else {
            return Ok(None);
        };
        let after = SelectionSnapshot::capture(
            syntax,
            text,
            &expanded,
            Mode::Select,
            SelectionSemantics::HalfOpen,
        )?;
        self.frames.push(ExpansionFrame { before, after });
        Ok(Some(expanded))
    }

    pub(crate) fn shrink(
        &mut self,
        syntax: &DocumentSyntax,
        text: &Text,
        selection: &Selection,
        mode: Mode,
        semantics: SelectionSemantics,
    ) -> Result<ShrinkResult, SyntaxError> {
        let Some(frame) = self.frames.last() else {
            return Ok(ShrinkResult::Empty);
        };
        let after = match frame.after.resolve(syntax, text) {
            Ok(after) => after,
            Err(SyntaxError::StaleRevision { .. } | SyntaxError::ForeignDocument) => {
                self.clear();
                return Ok(ShrinkResult::Reset(HistoryReset::Stale));
            }
            Err(error) => return Err(error),
        };
        if after != *selection || frame.after.mode != mode || frame.after.semantics != semantics {
            self.clear();
            return Ok(ShrinkResult::Reset(HistoryReset::Mismatch));
        }
        let before = match frame.before.resolve(syntax, text) {
            Ok(before) => before,
            Err(SyntaxError::StaleRevision { .. } | SyntaxError::ForeignDocument) => {
                self.clear();
                return Ok(ShrinkResult::Reset(HistoryReset::Stale));
            }
            Err(error) => return Err(error),
        };
        let mode = frame.before.mode;
        let semantics = frame.before.semantics;
        self.frames.pop();
        Ok(ShrinkResult::Restored {
            selection: before,
            mode,
            semantics,
        })
    }
}

pub(crate) fn transform_selection(
    syntax: &DocumentSyntax,
    text: &Text,
    registry: &Registry,
    selection: &Selection,
    transform: SyntaxSelectionTransform,
) -> Result<Option<Selection>, SyntaxError> {
    let mut changed = false;
    let ranges = selection
        .ranges()
        .iter()
        .map(|range| {
            let current =
                syntax.selection_range(text, SyntaxRange::new(range.from(), range.to())?)?;
            let transformed =
                syntax.transform_selection_range(text, registry, current, transform)?;
            let transformed = transformed.unwrap_or(current).range;
            changed |= transformed.from != range.from() || transformed.to != range.to();
            Ok(if range.anchor > range.head {
                Range::new(transformed.to, transformed.from)
            } else {
                Range::new(transformed.from, transformed.to)
            })
        })
        .collect::<Result<Vec<_>, SyntaxError>>()?;
    Ok(changed.then(|| Selection::new(ranges, selection.primary_index())))
}

/// Selects the smallest capture whose logical bounds enclose each range.
/// Grouped captures remain separate ranges rather than becoming a hull.
pub(crate) fn select_text_object(
    syntax: &DocumentSyntax,
    text: &Text,
    registry: &Registry,
    selection: &Selection,
    object: SyntaxObject,
    part: SyntaxObjectPart,
) -> Result<Option<Selection>, SyntaxError> {
    let captures = syntax.text_object_captures(
        text,
        registry,
        object,
        part,
        SyntaxRange::new(0, text.len_chars())?,
    )?;
    let mut ranges = Vec::new();
    let mut primary = 0;
    for (index, range) in selection.ranges().iter().enumerate() {
        if index == selection.primary_index() {
            primary = ranges.len();
        }
        let Some(capture) = captures
            .iter()
            .filter(|capture| capture_encloses(capture, range))
            .min_by_key(|capture| capture_size(capture))
        else {
            ranges.push(*range);
            continue;
        };
        let reversed = range.anchor > range.head;
        ranges.extend(capture.ranges.iter().map(|range| {
            if reversed {
                Range::new(range.to, range.from)
            } else {
                Range::new(range.from, range.to)
            }
        }));
    }
    let selected = Selection::new(ranges, primary);
    Ok((selected != *selection).then_some(selected))
}

/// Moves each head to a strict previous/next capture start. In Select mode
/// the existing anchor is retained; otherwise the result is a caret.
pub(crate) fn navigate_text_object(
    syntax: &DocumentSyntax,
    text: &Text,
    registry: &Registry,
    selection: &Selection,
    object: SyntaxObject,
    forward: bool,
    mode: Mode,
) -> Result<Option<Selection>, SyntaxError> {
    let captures = syntax.text_object_captures(
        text,
        registry,
        object,
        SyntaxObjectPart::Around,
        SyntaxRange::new(0, text.len_chars())?,
    )?;
    let mut starts = captures
        .iter()
        .filter_map(|capture| capture_bounds(capture).map(|(from, _)| from))
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts.dedup();
    let mut moved = false;
    let selection = selection.transform(|range| {
        let target = if forward {
            starts.iter().copied().find(|start| *start > range.head)
        } else {
            starts
                .iter()
                .rev()
                .copied()
                .find(|start| *start < range.head)
        };
        let Some(target) = target else {
            return range;
        };
        moved = true;
        if mode == Mode::Select {
            range.extend_to(target)
        } else {
            Range::point(target)
        }
    });
    Ok(moved.then_some(selection))
}

fn capture_bounds(capture: &SyntaxCapture) -> Option<(usize, usize)> {
    Some((
        capture.ranges.iter().map(|range| range.from).min()?,
        capture.ranges.iter().map(|range| range.to).max()?,
    ))
}

fn capture_encloses(capture: &SyntaxCapture, range: &Range) -> bool {
    if capture.ranges.is_empty() {
        return false;
    }
    if range.is_empty() {
        capture
            .ranges
            .iter()
            .any(|part| part.from <= range.head && range.head < part.to)
    } else {
        let mut parts = capture.ranges.to_vec();
        parts.sort_unstable_by_key(|part| (part.from, part.to));
        let mut covered_to = range.from();
        for part in parts {
            if part.to <= covered_to {
                continue;
            }
            if part.from > covered_to {
                return false;
            }
            covered_to = covered_to.max(part.to);
            if covered_to >= range.to() {
                return true;
            }
        }
        false
    }
}

fn capture_size(capture: &SyntaxCapture) -> (usize, usize, usize) {
    let (from, to) = capture_bounds(capture).unwrap_or((usize::MAX, usize::MAX));
    let captured = capture
        .ranges
        .iter()
        .map(|range| range.to.saturating_sub(range.from))
        .sum();
    (to.saturating_sub(from), captured, capture.ranges.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Transaction;
    use std::path::Path;

    fn rust_document(source: &str) -> (Registry, Text, DocumentSyntax) {
        let registry = Registry::new();
        let language = registry.language_for_path(Path::new("fixture.rs")).unwrap();
        let text = Text::from(source);
        let syntax = DocumentSyntax::new(&text, language, &registry).unwrap();
        (registry, text, syntax)
    }

    #[test]
    fn expansion_history_restores_direction_primary_and_mode() {
        let source = "fn demo() { let first = 1; let second = 2; }";
        let (registry, text, syntax) = rust_document(source);
        let first = source.find("first").unwrap();
        let second = source.find("second").unwrap();
        let selection = Selection::new(
            vec![
                Range::new(first + "first".len(), first),
                Range::point(second),
            ],
            1,
        );
        let mut history = ExpansionHistory::default();

        let expanded = history
            .expand(
                &syntax,
                &text,
                &registry,
                &selection,
                Mode::Normal,
                SelectionSemantics::Runyte,
            )
            .unwrap()
            .unwrap();
        assert_eq!(expanded.primary_index(), 1);
        assert!(expanded.ranges()[0].anchor > expanded.ranges()[0].head);

        assert_eq!(
            history
                .shrink(
                    &syntax,
                    &text,
                    &expanded,
                    Mode::Select,
                    SelectionSemantics::HalfOpen,
                )
                .unwrap(),
            ShrinkResult::Restored {
                selection,
                mode: Mode::Normal,
                semantics: SelectionSemantics::Runyte,
            }
        );
    }

    #[test]
    fn transformed_ranges_normalize_atomically_and_keep_the_primary_region() {
        let source = "fn demo() { let value = call(argument); }";
        let (registry, text, syntax) = rust_document(source);
        let value = source.find("value").unwrap();
        let argument = source.find("argument").unwrap();
        let selection = Selection::new(vec![Range::point(value), Range::point(argument)], 1);

        let mut transformed = selection.clone();
        for _ in 0..8 {
            let Some(next) = transform_selection(
                &syntax,
                &text,
                &registry,
                &transformed,
                SyntaxSelectionTransform::Expand,
            )
            .unwrap() else {
                break;
            };
            transformed = next;
        }
        assert_eq!(transformed.len(), 1, "enclosing ranges should merge");
        assert_eq!(transformed.primary_index(), 0);

        let invalid = Selection::new(
            vec![Range::point(value), Range::point(text.len_chars() + 1)],
            0,
        );
        let before = invalid.clone();
        assert!(
            transform_selection(
                &syntax,
                &text,
                &registry,
                &invalid,
                SyntaxSelectionTransform::Expand,
            )
            .is_err()
        );
        assert_eq!(
            invalid, before,
            "a later failure cannot partially transform"
        );
    }

    #[test]
    fn delimiter_selection_handles_nested_pairs_parts_and_direction() {
        let source = "fn demo() { call([1, (2 + 3)]); }";
        let (registry, text, syntax) = rust_document(source);
        let two = source.find('2').unwrap();
        let selection = Selection::single(Range::new(two + 1, two));

        let inside = select_delimiter(
            &syntax,
            &text,
            &registry,
            &selection,
            Some(DelimiterPair::Parentheses),
            SyntaxObjectPart::Inside,
        )
        .unwrap()
        .unwrap();
        assert_eq!(inside.len(), 1);
        assert!(inside.primary().anchor > inside.primary().head);
        assert_eq!(
            text.slice_string(inside.primary().from(), inside.primary().to()),
            "2 + 3"
        );

        let around = select_delimiter(
            &syntax,
            &text,
            &registry,
            &Selection::point(two),
            Some(DelimiterPair::Parentheses),
            SyntaxObjectPart::Around,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            text.slice_string(around.primary().from(), around.primary().to()),
            "(2 + 3)"
        );

        let closest = select_delimiter(
            &syntax,
            &text,
            &registry,
            &Selection::point(two),
            None,
            SyntaxObjectPart::Around,
        )
        .unwrap()
        .unwrap();
        assert_eq!(closest, around);

        let outer = select_delimiter(
            &syntax,
            &text,
            &registry,
            &inside,
            Some(DelimiterPair::Parentheses),
            SyntaxObjectPart::Inside,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            text.slice_string(outer.primary().from(), outer.primary().to()),
            "[1, (2 + 3)]"
        );
    }

    #[test]
    fn delimiter_selection_supports_brackets_braces_angles_and_quotes() {
        let source = "fn demo<T>() { let array = [1, 2]; let text = \"hello\"; let ch = 'x'; }";
        let (registry, text, syntax) = rust_document(source);
        for (needle, pair, expected) in [
            ("T", DelimiterPair::AngleBrackets, "T"),
            ("1", DelimiterPair::SquareBrackets, "1, 2"),
            (
                "array",
                DelimiterPair::Braces,
                " let array = [1, 2]; let text = \"hello\"; let ch = 'x'; ",
            ),
            ("hello", DelimiterPair::DoubleQuotes, "hello"),
            ("'x'", DelimiterPair::SingleQuotes, "x"),
        ] {
            let point = source.find(needle).unwrap() + usize::from(needle == "'x'");
            let selected = select_delimiter(
                &syntax,
                &text,
                &registry,
                &Selection::point(point),
                Some(pair),
                SyntaxObjectPart::Inside,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                text.slice_string(selected.primary().from(), selected.primary().to()),
                expected
            );
        }
    }

    #[test]
    fn delimiter_selection_supports_javascript_backticks() {
        let registry = Registry::new();
        let language = registry.language_for_path(Path::new("fixture.js")).unwrap();
        let source = "const greeting = `hello`;";
        let text = Text::from(source);
        let syntax = DocumentSyntax::new(&text, language, &registry).unwrap();
        let selected = select_delimiter(
            &syntax,
            &text,
            &registry,
            &Selection::point(source.find("hello").unwrap()),
            Some(DelimiterPair::Backticks),
            SyntaxObjectPart::Inside,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            text.slice_string(selected.primary().from(), selected.primary().to()),
            "hello"
        );
    }

    #[test]
    fn shrink_resets_mismatched_and_stale_history_without_restoring() {
        let source = "fn demo() { let value = 1; }";
        let (registry, mut text, mut syntax) = rust_document(source);
        let selection = Selection::point(source.find("value").unwrap());
        let mut history = ExpansionHistory::default();
        let expanded = history
            .expand(
                &syntax,
                &text,
                &registry,
                &selection,
                Mode::Normal,
                SelectionSemantics::Runyte,
            )
            .unwrap()
            .unwrap();

        assert_eq!(
            history
                .shrink(
                    &syntax,
                    &text,
                    &Selection::point(0),
                    Mode::Select,
                    SelectionSemantics::HalfOpen,
                )
                .unwrap(),
            ShrinkResult::Reset(HistoryReset::Mismatch)
        );
        assert_eq!(
            history
                .shrink(
                    &syntax,
                    &text,
                    &expanded,
                    Mode::Select,
                    SelectionSemantics::HalfOpen,
                )
                .unwrap(),
            ShrinkResult::Empty
        );

        let expanded = history
            .expand(
                &syntax,
                &text,
                &registry,
                &selection,
                Mode::Normal,
                SelectionSemantics::Runyte,
            )
            .unwrap()
            .unwrap();
        let before = text.clone();
        let transaction = Transaction::insert(0, "// changed\n");
        text.apply(&transaction);
        assert!(syntax.update(&before, &text, &transaction, &registry));
        assert_eq!(
            history
                .shrink(
                    &syntax,
                    &text,
                    &expanded,
                    Mode::Select,
                    SelectionSemantics::HalfOpen,
                )
                .unwrap(),
            ShrinkResult::Reset(HistoryReset::Stale)
        );
    }

    #[test]
    fn expanding_after_a_selection_change_starts_a_fresh_history_chain() {
        let source = "fn demo() { let value = 1; }";
        let (registry, text, syntax) = rust_document(source);
        let first = Selection::point(source.find("value").unwrap());
        let fresh = Selection::point(source.find("demo").unwrap());
        let mut history = ExpansionHistory::default();
        history
            .expand(
                &syntax,
                &text,
                &registry,
                &first,
                Mode::Normal,
                SelectionSemantics::Runyte,
            )
            .unwrap()
            .unwrap();
        let expanded = history
            .expand(
                &syntax,
                &text,
                &registry,
                &fresh,
                Mode::Normal,
                SelectionSemantics::Runyte,
            )
            .unwrap()
            .unwrap();

        assert_eq!(
            history
                .shrink(
                    &syntax,
                    &text,
                    &expanded,
                    Mode::Select,
                    SelectionSemantics::HalfOpen,
                )
                .unwrap(),
            ShrinkResult::Restored {
                selection: fresh,
                mode: Mode::Normal,
                semantics: SelectionSemantics::Runyte,
            }
        );
        assert_eq!(
            history
                .shrink(
                    &syntax,
                    &text,
                    &first,
                    Mode::Normal,
                    SelectionSemantics::Runyte,
                )
                .unwrap(),
            ShrinkResult::Empty
        );
    }

    #[test]
    fn grouped_parameter_selection_keeps_ranges_direction_and_primary() {
        let source = "fn greet(α: usize, beta: usize) {}";
        let (registry, text, syntax) = rust_document(source);
        let alpha = source
            .chars()
            .position(|character| character == 'α')
            .unwrap();
        let beta = source[..source.find("beta").unwrap()].chars().count();
        let selection = Selection::new(
            vec![Range::new(alpha + 1, alpha), Range::new(beta + 1, beta)],
            1,
        );

        let selected = select_text_object(
            &syntax,
            &text,
            &registry,
            &selection,
            SyntaxObject::Parameter,
            SyntaxObjectPart::Around,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            selected.len(),
            3,
            "the first parameter keeps its comma range"
        );
        assert_eq!(selected.primary_index(), 2);
        assert!(
            selected
                .ranges()
                .iter()
                .all(|range| range.anchor > range.head)
        );
        let pieces = selected
            .ranges()
            .iter()
            .map(|range| text.slice_string(range.from(), range.to()))
            .collect::<Vec<_>>();
        assert_eq!(pieces, ["α: usize", ",", "beta: usize"]);
    }

    #[test]
    fn grouped_capture_gaps_do_not_enclose_ranges() {
        let source = "fn greet(first: usize , second: usize) {}";
        let (registry, text, syntax) = rust_document(source);
        let first = source.find("first").unwrap();
        let gap_from = source.find("usize ,").unwrap() + "usize".len();
        let comma = source.find(',').unwrap();

        let point = select_text_object(
            &syntax,
            &text,
            &registry,
            &Selection::point(first),
            SyntaxObject::Parameter,
            SyntaxObjectPart::Around,
        )
        .unwrap()
        .unwrap();
        assert_eq!(point.len(), 2, "captured parameter and comma stay grouped");

        for untouched in [
            Selection::single(Range::new(gap_from, comma)),
            Selection::single(Range::new(first, comma + 1)),
        ] {
            assert_eq!(
                select_text_object(
                    &syntax,
                    &text,
                    &registry,
                    &untouched,
                    SyntaxObject::Parameter,
                    SyntaxObjectPart::Around,
                )
                .unwrap(),
                None,
                "uncaptured whitespace cannot be filled by a capture hull"
            );
        }
    }

    #[test]
    fn text_object_selection_chooses_the_smallest_enclosing_capture() {
        let source = "fn outer() { fn inner() { let value = 1; } inner(); }";
        let (registry, text, syntax) = rust_document(source);
        let value = source.find("value").unwrap();
        let selected = select_text_object(
            &syntax,
            &text,
            &registry,
            &Selection::point(value),
            SyntaxObject::Function,
            SyntaxObjectPart::Around,
        )
        .unwrap()
        .unwrap();
        assert!(
            text.slice_string(selected.primary().from(), selected.primary().to())
                .starts_with("fn inner")
        );
    }

    #[test]
    fn object_navigation_uses_strict_starts_and_select_mode_keeps_the_anchor() {
        let source = "fn first() {}\nfn second() {}\n";
        let (registry, text, syntax) = rust_document(source);
        let first = source.find("fn first").unwrap();
        let second = source.find("fn second").unwrap();
        let inside_first = source.find("first").unwrap();

        assert_eq!(
            navigate_text_object(
                &syntax,
                &text,
                &registry,
                &Selection::point(inside_first),
                SyntaxObject::Function,
                false,
                Mode::Normal,
            )
            .unwrap(),
            Some(Selection::point(first))
        );
        assert_eq!(
            navigate_text_object(
                &syntax,
                &text,
                &registry,
                &Selection::point(first),
                SyntaxObject::Function,
                true,
                Mode::Normal,
            )
            .unwrap(),
            Some(Selection::point(second))
        );
        assert_eq!(
            navigate_text_object(
                &syntax,
                &text,
                &registry,
                &Selection::single(Range::new(inside_first, inside_first + 1)),
                SyntaxObject::Function,
                true,
                Mode::Select,
            )
            .unwrap(),
            Some(Selection::single(Range::new(inside_first, second)))
        );
    }

    #[test]
    fn structural_logic_stays_private_and_uses_only_owned_syntax_types() {
        let source = include_str!("structural_selection.rs");
        assert!(!source.contains(concat!("tree", "_house")));
        let library = include_str!("lib.rs");
        assert!(library.contains("mod structural_selection;"));
        assert!(!library.contains("pub mod structural_selection;"));
    }
}
