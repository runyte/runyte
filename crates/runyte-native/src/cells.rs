// SPDX-License-Identifier: MPL-2.0

//! Cell-local shaping preserves grid placement and prevents cross-cell ligatures.
//! Cache layouts and group ordinary pane-row spans into logical layers.
use super::{CellMetrics, FrameData, color};
use gpui::*;
use ratatui::style::Modifier;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

const CACHE_ENTRIES_PER_FACE: usize = 1024;
const MAX_CACHED_SYMBOL_BYTES: usize = 256;

struct FaceCache<T> {
    ascii: [Option<Arc<T>>; 128],
    entries: HashMap<String, Arc<T>>,
    order: VecDeque<String>,
}

impl<T> Default for FaceCache<T> {
    fn default() -> Self {
        Self {
            ascii: std::array::from_fn(|_| None),
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }
}
impl<T> FaceCache<T> {
    fn get_or_insert(&mut self, symbol: &str, shape: impl FnOnce() -> Arc<T>) -> Arc<T> {
        if symbol.len() == 1 && symbol.is_ascii() {
            return self.ascii[symbol.as_bytes()[0] as usize]
                .get_or_insert_with(shape)
                .clone();
        }
        if let Some(layout) = self.entries.get(symbol) {
            return layout.clone();
        }
        let layout = shape();
        // Both entry count and symbol size are bounded, including arbitrary PTY text.
        if symbol.len() <= MAX_CACHED_SYMBOL_BYTES {
            if self.entries.len() == CACHE_ENTRIES_PER_FACE - 128 {
                self.entries.remove(&self.order.pop_front().unwrap());
            }
            self.order.push_back(symbol.to_owned());
            self.entries.insert(symbol.to_owned(), layout.clone());
        }
        layout
    }
}

pub(super) struct GlyphCache {
    faces: [FaceCache<LineLayout>; 4],
    rows: Vec<Option<PreparedRow>>,
}
impl Default for GlyphCache {
    fn default() -> Self {
        Self {
            faces: std::array::from_fn(|_| FaceCache::default()),
            rows: Vec::new(),
        }
    }
}
impl GlyphCache {
    fn layout(
        &mut self,
        symbol: &str,
        modifier: Modifier,
        metrics: CellMetrics,
        window: &Window,
    ) -> Arc<LineLayout> {
        let face = usize::from(modifier.contains(Modifier::BOLD))
            | (usize::from(modifier.contains(Modifier::ITALIC)) << 1);
        self.faces[face].get_or_insert(symbol, || {
            let mut font = font("JetBrainsMono Nerd Font");
            font.weight = if face & 1 != 0 {
                FontWeight::BOLD
            } else {
                FontWeight::MEDIUM
            };
            if face & 2 != 0 {
                font.style = FontStyle::Italic;
            }
            window.text_system().layout_line(
                symbol,
                px(metrics.font_size),
                &[TextRun {
                    len: symbol.len(),
                    font,
                    color: rgb(0xffffff).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
        })
    }
}

#[derive(PartialEq)]
struct Coverage {
    media: bool,
    hidden: Vec<bool>,
    // Overlays reveal media cells without removing their scene-depth constraint.
    raw_media: Vec<bool>,
    boundaries: Vec<u16>,
}
impl Coverage {
    fn row(frame: &FrameData, y: u16) -> Self {
        let full_width_text_rows = *FULL_WIDTH_TEXT_ROWS
            .get_or_init(|| std::env::var_os("RUNYTE_NATIVE_FULL_WIDTH_TEXT_ROWS").is_some());
        Self::row_with_policy(frame, y, full_width_text_rows)
    }

    fn row_with_policy(frame: &FrameData, y: u16, full_width_text_rows: bool) -> Self {
        let media = media_row(frame, y);
        let mut hidden = if media {
            vec![false; frame.cells.area.width as usize]
        } else {
            Vec::new()
        };
        let mut raw_media = hidden.clone();
        let mut boundaries = vec![0, frame.cells.area.width];
        // Temporary diagnostic A/B: restore the full-row text topology only
        // when no media or preview exists anywhere in the frame.
        let skip_panes =
            full_width_text_rows && frame.media.is_empty() && frame.previews.is_empty();
        for rect in frame
            .pane_areas
            .iter()
            .filter(|_| !skip_panes)
            .chain(frame.media.iter().map(|pane| &pane.body))
            .chain(frame.previews.iter().map(|pane| &pane.body))
        {
            if y >= rect.y && y < rect.y.saturating_add(rect.height) {
                boundaries.push(rect.x.min(frame.cells.area.width));
                boundaries.push(
                    rect.x
                        .saturating_add(rect.width)
                        .min(frame.cells.area.width),
                );
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        if media {
            for (rect, covered) in frame
                .media
                .iter()
                .map(|pane| (&pane.body, true))
                .chain(frame.previews.iter().map(|pane| (&pane.body, true)))
                .chain(frame.overlays.iter().map(|rect| (rect, false)))
            {
                if y >= rect.y && y < rect.y.saturating_add(rect.height) {
                    let start = rect.x.min(frame.cells.area.width) as usize;
                    let end = rect
                        .x
                        .saturating_add(rect.width)
                        .min(frame.cells.area.width) as usize;
                    hidden[start..end].fill(covered);
                    if covered {
                        raw_media[start..end].fill(true);
                    }
                }
            }
        }
        Self {
            media,
            hidden,
            raw_media,
            boundaries,
        }
    }
    fn over_media(&self, x: u16) -> bool {
        self.raw_media.get(x as usize).copied().unwrap_or(false)
    }
    fn hides(&self, x: u16) -> bool {
        self.hidden.get(x as usize).copied().unwrap_or(false)
    }
}

type Colors = HashMap<(ratatui::style::Color, u32), Hsla>;
fn resolved(colors: &mut Colors, value: ratatui::style::Color, default: u32) -> Hsla {
    *colors
        .entry((value, default))
        .or_insert_with(|| color(value, default))
}
fn row_backgrounds(
    frame: &FrameData,
    y: u16,
    coverage: &Coverage,
    colors: &mut Colors,
) -> Vec<(u16, u16, Hsla)> {
    let mut runs: Vec<(u16, u16, Hsla)> = Vec::new();
    for x in 0..frame.cells.area.width {
        if coverage.hides(x) {
            continue;
        }
        let cell = &frame.cells[(x, y)];
        let bg = if cell.modifier.contains(Modifier::REVERSED) {
            resolved(colors, cell.fg, frame.foreground)
        } else {
            resolved(colors, cell.bg, frame.background)
        };
        if !coverage.over_media(x)
            && !coverage.boundaries.contains(&x)
            && let Some((start, width, previous)) = runs.last_mut()
            && *start + *width == x
            && *previous == bg
        {
            *width += 1;
        } else {
            runs.push((x, 1, bg));
        }
    }
    runs
}
#[cfg(test)]
fn backgrounds(frame: &FrameData, mut paint: impl FnMut(u16, u16, u16, Hsla)) {
    for y in 0..frame.cells.area.height {
        for (x, width, bg) in
            row_backgrounds(frame, y, &Coverage::row(frame, y), &mut Colors::new())
        {
            paint(x, y, width, bg);
        }
    }
}

#[derive(PartialEq)]
struct StyledCell {
    x: u16,
    width: usize,
    fg: Hsla,
    modifier: Modifier,
}
struct PreparedRow {
    source: Arc<[ratatui::buffer::Cell]>,
    coverage: Coverage,
    defaults: (u32, u32),
    backgrounds: Vec<(u16, u16, Hsla)>,
    cells: Vec<StyledCell>,
    segments: Vec<PreparedSegment>,
}
struct PreparedSegment {
    start: u16,
    end: u16,
    scene: SceneCache,
    placement: Option<(Point<Pixels>, bool)>,
}
impl GlyphCache {
    fn prepare(&mut self, frame: &FrameData) {
        self.rows.resize_with(frame.cells.rows.len(), || None);
        for y in 0..frame.cells.area.height {
            let source = &frame.cells.rows[y as usize];
            let coverage = Coverage::row(frame, y);
            let defaults = (frame.background, frame.foreground);
            if self.rows[y as usize].as_ref().is_some_and(|row| {
                Arc::ptr_eq(&row.source, source)
                    && row.coverage == coverage
                    && row.defaults == defaults
            }) {
                continue;
            }
            let mut colors = Colors::new();
            let backgrounds = row_backgrounds(frame, y, &coverage, &mut colors);
            let mut cells = Vec::new();
            let mut x = 0;
            while x < frame.cells.area.width {
                if coverage.hides(x) {
                    x += 1;
                    continue;
                }
                let cell = &source[x as usize];
                let symbol = cell.symbol();
                let width = if symbol.len() == 1 && symbol.is_ascii() {
                    1
                } else {
                    // The buffer placed this cell by the same measure.
                    usize::from(ratatui::buffer::CellWidth::cell_width(symbol)).max(1)
                };
                if symbol != " " && !cell.modifier.contains(Modifier::HIDDEN) {
                    let mut fg = if cell.modifier.contains(Modifier::REVERSED) {
                        resolved(&mut colors, cell.bg, frame.background)
                    } else {
                        resolved(&mut colors, cell.fg, frame.foreground)
                    };
                    if cell.modifier.contains(Modifier::DIM) {
                        fg.l *= 0.65;
                    }
                    cells.push(StyledCell {
                        x,
                        width,
                        fg,
                        modifier: cell.modifier,
                    });
                }
                x = (x as usize + width).min(frame.cells.area.width as usize) as u16;
            }
            // Only this row owns source cells. Retained segment scenes never pin
            // historical full-row allocations after changes in another pane.
            let mut previous = self.rows[y as usize].take();
            let segments = coverage
                .boundaries
                .windows(2)
                .map(|bounds| {
                    let (start, end) = (bounds[0], bounds[1]);
                    if let Some(old) = previous.as_mut()
                        && old.defaults == defaults
                        && (start..end).all(|x| {
                            old.coverage.hides(x) == coverage.hides(x)
                                && old.coverage.over_media(x) == coverage.over_media(x)
                        })
                        && old
                            .cells
                            .iter()
                            .filter(|cell| cell.x >= start && cell.x < end)
                            .eq(cells.iter().filter(|cell| cell.x >= start && cell.x < end))
                        && old.source.get(start as usize..end as usize)
                            == source.get(start as usize..end as usize)
                        && let Some(index) = old
                            .segments
                            .iter()
                            .position(|segment| segment.start == start && segment.end == end)
                    {
                        return old.segments.remove(index);
                    }
                    PreparedSegment {
                        start,
                        end,
                        scene: SceneCache::default(),
                        placement: None,
                    }
                })
                .collect();
            self.rows[y as usize] = Some(PreparedRow {
                source: source.clone(),
                coverage,
                defaults,
                backgrounds,
                cells,
                segments,
            });
        }
    }
}

pub(super) fn paint_cells(
    frame: &FrameData,
    origin: Point<Pixels>,
    metrics: CellMetrics,
    cache: &mut GlyphCache,
    window: &mut Window,
    cx: &mut App,
) {
    let timing = *TIMING.get_or_init(|| std::env::var_os("RUNYTE_NATIVE_PAINT_TIMING").is_some());
    let started = timing.then(std::time::Instant::now);
    cache.prepare(frame);
    let prepared = timing.then(std::time::Instant::now);
    let mut background_count = 0usize;
    let mut styled_cell_count = 0usize;
    let mut segment_count = 0usize;
    let mut rebuilt_segments = 0usize;
    for (y, row) in cache.rows.iter().enumerate() {
        let row = row.as_ref().unwrap();
        if timing {
            background_count += row.backgrounds.len();
            styled_cell_count += row.cells.len();
            segment_count += row.segments.len();
        }
        for &(x, width, bg) in &row.backgrounds {
            let position =
                origin + point(px(x as f32 * metrics.width), px(y as f32 * metrics.height));
            window.paint_quad(fill(
                Bounds::new(
                    position,
                    size(px(width as f32 * metrics.width), px(metrics.height)),
                ),
                bg,
            ));
        }
    }
    let backgrounds_painted = timing.then(std::time::Instant::now);
    let mut painted = Vec::with_capacity(frame.cells.area.width as usize);
    for y in 0..cache.rows.len() {
        // Prepared rows retain styles and source cells, not shaped layouts.
        // Layout retention stays bounded by the per-face glyph cache.
        let mut row = cache.rows[y].take().unwrap();
        let offset = origin + point(px(0.), px(y as f32 * metrics.height));
        for segment in &mut row.segments {
            let cursor_row = frame.cursor.is_some_and(|cursor| {
                cursor.y as usize == y && cursor.x >= segment.start && cursor.x < segment.end
            });
            let placement = (offset, cursor_row);
            let reuse = segment.placement == Some(placement);
            window.paint_cached_scene(&mut segment.scene, reuse, |window| {
                // The caller's reuse request can still miss GPUI's window,
                // generation, scale, clipping or opacity checks. Count actual
                // execution rather than treating the request as a cache hit.
                if timing {
                    rebuilt_segments += 1;
                }
                painted.clear();
                let mut separate = row.coverage.over_media(segment.start) || cursor_row;
                for cell in row
                    .cells
                    .iter()
                    .filter(|cell| cell.x >= segment.start && cell.x < segment.end)
                {
                    let layout = cache.layout(
                        row.source[cell.x as usize].symbol(),
                        cell.modifier,
                        metrics,
                        window,
                    );
                    separate |= layout.width <= px(0.)
                        || layout.width > px(cell.width as f32 * metrics.width);
                    painted.push(PaintedCell {
                        height: metrics.height,
                        layout,
                        fg: cell.fg,
                        modifier: cell.modifier,
                        position: point(px(cell.x as f32 * metrics.width), px(0.)),
                    });
                }
                if separate {
                    for cell in &painted {
                        window.paint_layer(cell.bounds(offset), |window| {
                            cell.paint(offset, window, cx)
                        });
                    }
                } else if let (Some(first), Some(last)) = (painted.first(), painted.last()) {
                    // Keep logical row-height bounds and layer ordering, including
                    // adjacent-row overhangs, emoji and decorated/oversized glyphs.
                    let bounds = Bounds::new(
                        first.position + offset,
                        size(
                            last.position.x + last.layout.width - first.position.x,
                            px(metrics.height),
                        ),
                    );
                    window.paint_layer(bounds, |window| {
                        for cell in &painted {
                            cell.paint(offset, window, cx);
                        }
                    });
                }
            });
            segment.placement = Some(placement);
        }
        cache.rows[y] = Some(row);
    }
    if let Some(cursor) = frame.cursor.filter(|c| !frame.under_media(c.x, c.y)) {
        let position = origin
            + point(
                px(cursor.x as f32 * metrics.width),
                px(cursor.y as f32 * metrics.height),
            );
        window.paint_quad(fill(
            Bounds::new(position, size(px(2.), px(metrics.height))),
            rgb(0xffffff),
        ));
    }
    if let Some(started) = started {
        let finished = std::time::Instant::now();
        let prepared = prepared.expect("timed preparation");
        let backgrounds_painted = backgrounds_painted.expect("timed backgrounds");
        eprintln!(
            "native-paint {}x{} {}us",
            frame.cells.area.width,
            frame.cells.area.height,
            finished.duration_since(started).as_micros()
        );
        eprintln!(
            "native-paint-stages prepare_us={} background_us={} glyph_us={} rows={} segments={} rebuilt={} replayed={} backgrounds={} styled_cells={}",
            prepared.duration_since(started).as_micros(),
            backgrounds_painted.duration_since(prepared).as_micros(),
            finished.duration_since(backgrounds_painted).as_micros(),
            cache.rows.len(),
            segment_count,
            rebuilt_segments,
            segment_count - rebuilt_segments,
            background_count,
            styled_cell_count,
        );
    }
}
fn media_row(frame: &FrameData, y: u16) -> bool {
    frame
        .media
        .iter()
        .map(|p| p.body)
        .chain(frame.previews.iter().map(|p| p.body))
        .any(|body| y >= body.y && y < body.y.saturating_add(body.height))
}

struct PaintedCell {
    height: f32,
    layout: Arc<LineLayout>,
    position: Point<Pixels>,
    fg: Hsla,
    modifier: Modifier,
}
impl PaintedCell {
    fn bounds(&self, offset: Point<Pixels>) -> Bounds<Pixels> {
        Bounds::new(
            self.position + offset,
            size(self.layout.width, px(self.height)),
        )
    }
    fn paint(&self, offset: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let position = self.position + offset;
        let baseline =
            (px(self.height) - self.layout.ascent - self.layout.descent) / 2. + self.layout.ascent;
        if let Some(first) = self.layout.runs.iter().flat_map(|r| &r.glyphs).next() {
            let mut start = position.x + first.position.x;
            let end = start + self.layout.width;
            if self.layout.width == px(0.)
                && let Some(last_run) = self.layout.runs.last()
            {
                start -= cx
                    .text_system()
                    .bounding_box(last_run.font_id, self.layout.font_size)
                    .size
                    .width
                    / 2.;
            }
            if self.modifier.contains(Modifier::UNDERLINED) {
                window.paint_underline(
                    point(start, position.y + baseline + self.layout.descent * 0.618),
                    end - start,
                    &UnderlineStyle {
                        thickness: px(1.),
                        color: Some(self.fg),
                        wavy: false,
                    },
                );
            }
            if self.modifier.contains(Modifier::CROSSED_OUT) {
                window.paint_strikethrough(
                    point(
                        start,
                        position.y + (self.layout.ascent * 0.5 + baseline) * 0.5,
                    ),
                    end - start,
                    &StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(self.fg),
                    },
                );
            }
        }
        for run in &self.layout.runs {
            for glyph in &run.glyphs {
                // GPUI's unwrapped paint_line uses the shaped x position and
                // a shared baseline (not glyph.position.y).
                let glyph_origin = position + point(glyph.position.x, baseline);
                if glyph.is_emoji {
                    let _ = window.paint_emoji(
                        glyph_origin,
                        run.font_id,
                        glyph.id,
                        self.layout.font_size,
                    );
                } else {
                    let _ = window.paint_glyph(
                        glyph_origin,
                        run.font_id,
                        glyph.id,
                        self.layout.font_size,
                        self.fg,
                    );
                }
            }
        }
    }
}

static TIMING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static FULL_WIDTH_TEXT_ROWS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

#[cfg(test)]
mod tests {
    use super::{
        CACHE_ENTRIES_PER_FACE, FaceCache, FrameData, MAX_CACHED_SYMBOL_BYTES, backgrounds, color,
    };
    use ratatui::style::Modifier;
    use ratatui::{buffer::Buffer, layout::Rect, style::Color};
    use std::sync::Arc;

    fn frame() -> FrameData {
        FrameData {
            attachment: 0,
            background: super::super::FALLBACK_BACKGROUND,
            foreground: super::super::FALLBACK_FOREGROUND,
            id: None,
            cells: Buffer::empty(Rect::new(0, 0, 8, 2)).into(),
            previews: vec![],
            media: vec![],
            cursor: None,
            overlays: vec![],
            pane_areas: vec![],
            media_input: false,
            pointer_bindings: Default::default(),
            window_bindings: Default::default(),
            preview_bindings: Default::default(),
            preview_source: None,
            preview_terminal: None,
            preview_structural: false,
            routing_serial: 0,
            metadata_paths: vec![],
        }
    }

    #[test]
    fn full_width_diagnostic_keeps_media_boundaries_and_default_pane_topology() {
        let mut frame = frame();
        frame.pane_areas.push(runyte::layout::Rect {
            x: 0,
            y: 0,
            width: 3,
            height: 2,
        });
        assert_eq!(
            super::Coverage::row_with_policy(&frame, 0, false).boundaries,
            [0, 3, 8]
        );
        assert_eq!(
            super::Coverage::row_with_policy(&frame, 0, true).boundaries,
            [0, 8]
        );
        frame.media.push(super::super::MediaPane {
            pane: 0,
            path: "image.png".into(),
            page: 1,
            body: runyte::layout::Rect {
                x: 4,
                y: 1,
                width: 4,
                height: 1,
            },
        });
        // A media pane on another row still disables the text-only experiment.
        assert_eq!(
            super::Coverage::row_with_policy(&frame, 0, true).boundaries,
            [0, 3, 8]
        );
        assert_eq!(
            super::Coverage::row_with_policy(&frame, 1, true).boundaries,
            [0, 3, 4, 8]
        );
    }

    #[test]
    fn backgrounds_merge_colors_and_preserve_media_overlay_occlusion() {
        let mut frame = frame();
        for x in 0..4 {
            frame.cells[(x, 0)].set_bg(Color::Red);
        }
        frame.cells[(4, 0)]
            .set_fg(Color::Red)
            .set_bg(Color::Blue)
            .set_style(Modifier::REVERSED);
        let mut runs = vec![];
        backgrounds(&frame, |x, y, w, c| runs.push((x, y, w, c)));
        assert_eq!(
            runs,
            vec![
                (0, 0, 5, color(Color::Red, 0)),
                (5, 0, 3, color(Color::Reset, 0x181818)),
                (0, 1, 8, color(Color::Reset, 0x181818))
            ]
        );
        frame.media.push(super::super::MediaPane {
            pane: 0,
            path: "image.png".into(),
            page: 1,
            body: runyte::layout::Rect {
                x: 2,
                y: 0,
                width: 6,
                height: 2,
            },
        });
        frame.overlays.push(runyte::layout::Rect {
            x: 6,
            y: 0,
            width: 2,
            height: 2,
        });
        runs.clear();
        backgrounds(&frame, |x, y, w, c| runs.push((x, y, w, c)));
        assert_eq!(
            runs,
            vec![
                (0, 0, 2, color(Color::Red, 0)),
                (6, 0, 1, color(Color::Reset, 0x181818)),
                (7, 0, 1, color(Color::Reset, 0x181818)),
                (0, 1, 2, color(Color::Reset, 0x181818)),
                (6, 1, 1, color(Color::Reset, 0x181818)),
                (7, 1, 1, color(Color::Reset, 0x181818))
            ]
        );
    }

    #[test]
    fn reset_colours_follow_the_frame_theme() {
        let mut frame = frame();
        frame.background = 0x0b1f2a;
        frame.foreground = 0xe0e0e0;
        frame.cells[(1, 0)].set_bg(Color::Red);
        frame.cells[(2, 0)]
            .set_fg(Color::Reset)
            .set_bg(Color::Blue)
            .set_style(Modifier::REVERSED);
        let mut runs = vec![];
        backgrounds(&frame, |x, y, w, c| runs.push((x, y, w, c)));
        assert_eq!(
            runs,
            vec![
                (0, 0, 1, color(Color::Rgb(0x0b, 0x1f, 0x2a), 0)),
                (1, 0, 1, color(Color::Red, 0)),
                // Reverse video takes the foreground default as its background.
                (2, 0, 1, color(Color::Rgb(0xe0, 0xe0, 0xe0), 0)),
                (3, 0, 5, color(Color::Rgb(0x0b, 0x1f, 0x2a), 0)),
                (0, 1, 8, color(Color::Rgb(0x0b, 0x1f, 0x2a), 0)),
            ]
        );
    }

    #[test]
    fn glyph_cache_reuses_symbols_and_bounds_retained_text() {
        let mut cache = FaceCache::<usize>::default();
        let a = cache.get_or_insert("a", || Arc::new(1));
        let reused = cache.get_or_insert("a", || panic!("reshaped cached symbol"));
        assert!(Arc::ptr_eq(&a, &reused));
        for n in 0..CACHE_ENTRIES_PER_FACE {
            cache.get_or_insert(&n.to_string(), || Arc::new(n));
        }
        assert_eq!(cache.entries.len(), CACHE_ENTRIES_PER_FACE - 128);
        assert_eq!(cache.order.len(), CACHE_ENTRIES_PER_FACE - 128);
        assert!(Arc::ptr_eq(
            &a,
            &cache.get_or_insert("a", || panic!("ASCII slot was evicted"))
        ));
        let large = "x".repeat(MAX_CACHED_SYMBOL_BYTES + 1);
        cache.get_or_insert(&large, || Arc::new(9));
        assert!(!cache.entries.contains_key(&large));
        assert_eq!(cache.entries.len(), CACHE_ENTRIES_PER_FACE - 128);
    }
}

#[cfg(test)]
#[path = "tests/cells.rs"]
mod prepared_tests;
