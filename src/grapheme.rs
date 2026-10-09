// SPDX-License-Identifier: MPL-2.0

//! User-perceived characters: the extended grapheme clusters of UAX #29.
//!
//! Text is stored and addressed by `char` offsets, but one visible character
//! can be several of them: `🤷‍♀️` is a base emoji, a zero-width joiner, a sign
//! and a variation selector. A caret or selection end must never stop inside
//! one, and its width must be measured as a whole, because the sum of its
//! code points' widths is not what a terminal or the native grid draws.
//!
//! [`width`] is the one measure every layout path uses. It is the measure the
//! cell buffer both frontends draw from places each cluster by, so layout and
//! drawing cannot disagree about where a character ends.
//!
//! Every search here is bounded by [`MAX_CLUSTER_CHARS`]. A hostile line of
//! nothing but combining marks must not turn one keystroke or one frame into
//! a scan of the whole line; past the bound it is split into pieces that
//! long, which only such text can notice.

use ratatui::buffer::CellWidth;
use ropey::RopeSlice;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};
use unicode_width::UnicodeWidthChar;

/// The most `char`s one cluster may span. Real clusters are far shorter: the
/// longest standard emoji, a kiss with two skin tones, has ten.
pub const MAX_CLUSTER_CHARS: usize = 32;

/// Terminal cells one cluster occupies, as the cell buffer measures it. That
/// is the `unicode-width` string width plus a cell for each halfwidth katakana
/// sound mark, which terminals draw beside its kana. A single code point
/// keeps its own width, so control characters stay zero-width as before.
pub fn width(cluster: &str) -> usize {
    let mut characters = cluster.chars();
    match (characters.next(), characters.next()) {
        (None, _) => 0,
        (Some(character @ ('\u{FF9E}' | '\u{FF9F}')), None) => {
            usize::from(cluster.cell_width()).max(UnicodeWidthChar::width(character).unwrap_or(0))
        }
        (Some(character), None) => UnicodeWidthChar::width(character).unwrap_or(0),
        _ => usize::from(cluster.cell_width()),
    }
}

/// Terminal cells a string occupies when every cluster is measured whole.
pub fn str_width(text: &str) -> usize {
    clusters(text.chars()).map(|cluster| cluster.width).sum()
}

/// One cluster read from a stream of `char`s.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cluster {
    /// The cluster's first code point.
    pub first: char,
    /// Code points in the cluster, at least one.
    pub chars: usize,
    /// Terminal cells, as [`width`] measures them.
    pub width: usize,
}

/// Splits a stream of `char`s into clusters. The stream must start at a
/// cluster boundary, as a line start or any offset taken from [`floor`] is.
pub fn clusters<I: Iterator<Item = char>>(characters: I) -> Clusters<I> {
    Clusters {
        characters: characters.peekable(),
        cluster: String::new(),
    }
}

pub struct Clusters<I: Iterator<Item = char>> {
    characters: std::iter::Peekable<I>,
    cluster: String,
}

impl<I: Iterator<Item = char>> Clusters<I> {
    /// The width of a cluster that is the single ASCII character `first`,
    /// read without segmenting when the next character is ASCII too, which
    /// it is for nearly all source text.
    fn ascii(&mut self, first: char) -> Option<usize> {
        (first.is_ascii()
            && first != '\r'
            && self.characters.peek().is_none_or(|next| next.is_ascii()))
        .then(|| UnicodeWidthChar::width(first).unwrap_or(0))
    }

    /// Reads the cluster that `first` begins, leaving its text in
    /// `self.cluster`.
    fn read(&mut self, first: char, mut visit: impl FnMut(char)) -> Cluster {
        visit(first);
        self.cluster.clear();
        self.cluster.push(first);
        let mut chars = 1;
        while chars < MAX_CLUSTER_CHARS
            && let Some(&next) = self.characters.peek()
            && extends(&mut self.cluster, next)
        {
            visit(next);
            self.characters.next();
            chars += 1;
        }
        Cluster {
            first,
            chars,
            width: width(&self.cluster),
        }
    }
}

impl<I: Iterator<Item = char>> Iterator for Clusters<I> {
    type Item = Cluster;

    fn next(&mut self) -> Option<Cluster> {
        let first = self.characters.next()?;
        if let Some(width) = self.ascii(first) {
            return Some(Cluster {
                first,
                chars: 1,
                width,
            });
        }
        Some(self.read(first, |_| {}))
    }
}

/// Pairs each `char` with the cells it contributes: a cluster's whole width
/// on its first code point and zero on the rest. Summing the second field
/// measures text. The stream must start at a cluster boundary.
pub fn cells<I: Iterator<Item = char>>(characters: I) -> impl Iterator<Item = (char, usize)> {
    starts(characters).map(|(character, width)| (character, width.unwrap_or(0)))
}

/// Pairs each `char` with its cluster's width when it starts that cluster,
/// and `None` when it continues one. Unlike [`cells`], this tells a cluster
/// of zero width apart from the code points that follow a visible one.
pub fn starts<I: Iterator<Item = char>>(characters: I) -> Starts<I> {
    Starts {
        clusters: clusters(characters),
        pending: Vec::new(),
        next: 0,
    }
}

pub struct Starts<I: Iterator<Item = char>> {
    clusters: Clusters<I>,
    pending: Vec<(char, Option<usize>)>,
    next: usize,
}

impl<I: Iterator<Item = char>> Iterator for Starts<I> {
    type Item = (char, Option<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(&item) = self.pending.get(self.next) {
            self.next += 1;
            return Some(item);
        }
        self.pending.clear();
        let first = self.clusters.characters.next()?;
        if let Some(width) = self.clusters.ascii(first) {
            return Some((first, Some(width)));
        }
        let pending = &mut self.pending;
        let cluster = self
            .clusters
            .read(first, |character| pending.push((character, None)));
        self.pending[0].1 = Some(cluster.width);
        self.next = 1;
        Some(self.pending[0])
    }
}

/// The `char` index where the cluster before `index` starts, for editing a
/// short single-line value such as a prompt by whole characters.
pub fn str_previous(text: &str, index: usize) -> usize {
    let mut start = 0;
    for cluster in clusters(text.chars()) {
        let end = start + cluster.chars;
        if end >= index {
            return start;
        }
        start = end;
    }
    start
}

/// The `char` index just past the cluster at `index` in a short value.
pub fn str_next(text: &str, index: usize) -> usize {
    let mut end = 0;
    for cluster in clusters(text.chars()) {
        end += cluster.chars;
        if end > index {
            return end;
        }
    }
    end
}

/// Whether `next` belongs to the cluster that `cluster` begins. On return
/// `cluster` holds `next` as well when it does, and is unchanged otherwise.
pub(crate) fn extends(cluster: &mut String, next: char) -> bool {
    // Nothing joins an ASCII character to what precedes it except the line
    // feed of a CRLF pair, so plain text never pays for segmentation.
    if next.is_ascii() {
        if next == '\n' && cluster == "\r" {
            cluster.push(next);
            return true;
        }
        return false;
    }
    let length = cluster.len();
    cluster.push(next);
    if cluster.graphemes(true).nth(1).is_none() {
        true
    } else {
        cluster.truncate(length);
        false
    }
}

/// The start of the cluster containing `offset`: `offset` itself when it is
/// already a boundary. Offsets at or past the end clamp to the end.
pub fn floor(text: RopeSlice<'_>, offset: usize) -> usize {
    if offset >= text.len_chars() {
        return text.len_chars();
    }
    if quick_boundary(text, offset) == Some(true) {
        return offset;
    }
    cluster_at(text, offset).0
}

/// The boundary after the cluster containing `offset`, or the end of text.
pub fn next(text: RopeSlice<'_>, offset: usize) -> usize {
    cluster_at(text, offset).1
}

/// The start of the cluster before the one containing `offset`, or zero.
pub fn previous(text: RopeSlice<'_>, offset: usize) -> usize {
    let start = floor(text, offset);
    if start == 0 {
        return 0;
    }
    floor(text, start - 1)
}

/// The cluster containing `offset` as a half-open `char` span. At or past the
/// end of text it is the empty span at the end.
pub fn cluster_at(text: RopeSlice<'_>, offset: usize) -> (usize, usize) {
    let len = text.len_chars();
    if offset >= len {
        return (len, len);
    }
    // Most code points are a cluster of their own, which the boundaries on
    // either side settle without segmenting a window.
    if quick_boundary(text, offset) == Some(true) && quick_boundary(text, offset + 1) == Some(true)
    {
        return (offset, offset + 1);
    }
    let start = window_start(text, offset);
    let end = offset.saturating_add(MAX_CLUSTER_CHARS).min(len);
    let mut position = start;
    for cluster in clusters(text.slice(start..end).chars()) {
        let cluster_end = position + cluster.chars;
        if cluster_end > offset {
            return (position, cluster_end);
        }
        position = cluster_end;
    }
    (offset, offset + 1)
}

/// Whether a cluster boundary falls before the code point at `offset`, read
/// from the code points around it. ASCII on both sides is settled at once;
/// anything else asks the segmenter about this one position in the rope's own
/// chunks. `None` when that needs more preceding text than a few chunks, as a
/// long run of flags does, and the caller falls back to a bounded window.
fn quick_boundary(text: RopeSlice<'_>, offset: usize) -> Option<bool> {
    let len = text.len_chars();
    if offset == 0 || offset >= len {
        return Some(true);
    }
    let before = text.char(offset - 1);
    let at = text.char(offset);
    if before.is_ascii() && at.is_ascii() {
        return Some(!(before == '\r' && at == '\n'));
    }
    let byte = text.char_to_byte(offset);
    // The chunk holding the boundary's right side; the segmenter asks for
    // the text before it when it needs that too.
    let (chunk, chunk_start, _, _) = text.chunk_at_byte(byte);
    let mut cursor = GraphemeCursor::new(byte, text.len_bytes(), true);
    for _ in 0..QUICK_CONTEXT_CHUNKS {
        match cursor.is_boundary(chunk, chunk_start) {
            Ok(boundary) => return Some(boundary),
            Err(GraphemeIncomplete::PreContext(end)) => {
                let (context, context_start, _, _) = text.chunk_at_byte(end - 1);
                cursor.provide_context(&context[..end - context_start], context_start);
            }
            Err(_) => return None,
        }
    }
    None
}

/// Preceding chunks [`quick_boundary`] may read before deferring to a window.
const QUICK_CONTEXT_CHUNKS: usize = 4;

/// A known boundary at or before `offset` from which segmenting reaches it.
///
/// A line start always is one, and so is any ASCII character other than the
/// line feed of a CRLF pair. The scan back stops at the cluster bound; text
/// with no such position that close is split at the bound instead.
fn window_start(text: RopeSlice<'_>, offset: usize) -> usize {
    let limit = offset.saturating_sub(MAX_CLUSTER_CHARS);
    let mut position = offset;
    let mut characters = text.chars_at(offset);
    while position > limit {
        let Some(character) = characters.prev() else {
            return 0;
        };
        position -= 1;
        if character == '\n' {
            return position + 1;
        }
        if character.is_ascii() && character != '\r' {
            return position;
        }
    }
    // Regional indicators pair from the start of their run, so a window that
    // opens inside a long run of flags must open on a pair. The count back is
    // bounded too; a run longer than that is paired from wherever it stops.
    let preceding = text
        .chars_at(limit)
        .reversed()
        .take(REGIONAL_INDICATOR_SCAN_LIMIT)
        .take_while(|character| is_regional_indicator(*character))
        .count();
    if text.get_char(limit).is_some_and(is_regional_indicator) && preceding % 2 == 1 {
        limit + 1
    } else {
        limit
    }
}

const REGIONAL_INDICATOR_SCAN_LIMIT: usize = 4096;

fn is_regional_indicator(character: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&character)
}

#[cfg(test)]
#[path = "grapheme/tests.rs"]
mod tests;
