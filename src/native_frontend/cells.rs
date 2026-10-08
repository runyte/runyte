// SPDX-License-Identifier: MPL-2.0

//! Cell-local shaping preserves grid placement and prevents cross-cell ligatures.
//! Cache layouts and group ordinary rows into logical layers for GPUI batching.
use super::{CellMetrics, FrameData, color};
use gpui::*;
use ratatui::style::Modifier;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

const CACHE_ENTRIES_PER_FACE: usize = 1024;
const MAX_CACHED_SYMBOL_BYTES: usize = 256;

#[derive(Default)]
struct FaceCache<T> {
    entries: HashMap<String, Arc<T>>,
    order: VecDeque<String>,
}

impl<T> FaceCache<T> {
    fn get_or_insert(&mut self, symbol: &str, shape: impl FnOnce() -> Arc<T>) -> Arc<T> {
        if let Some(layout) = self.entries.get(symbol) {
            return layout.clone();
        }
        let layout = shape();
        // Both entry count and symbol size are bounded, including arbitrary PTY text.
        if symbol.len() <= MAX_CACHED_SYMBOL_BYTES {
            if self.entries.len() == CACHE_ENTRIES_PER_FACE {
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
}
impl Default for GlyphCache {
    fn default() -> Self {
        Self {
            faces: std::array::from_fn(|_| FaceCache {
                entries: HashMap::new(),
                order: VecDeque::new(),
            }),
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

fn background(frame: &FrameData, x: u16, y: u16) -> Option<Hsla> {
    if frame.under_media(x, y) {
        return None;
    }
    let cell = &frame.cells[(x, y)];
    let bg = if cell.modifier.contains(Modifier::REVERSED) {
        color(cell.fg, frame.foreground)
    } else {
        color(cell.bg, frame.background)
    };
    // Keep even root-coloured backgrounds as merged runs: their logical
    // ordering floor must match neighbouring rows when glyphs overhang.
    Some(bg)
}

fn backgrounds(frame: &FrameData, mut paint: impl FnMut(u16, u16, u16, Hsla)) {
    for y in 0..frame.cells.area.height {
        let mut x = 0;
        while x < frame.cells.area.width {
            let start = x;
            let bg = background(frame, x, y);
            x += 1;
            while !media_row(frame, y)
                && x < frame.cells.area.width
                && background(frame, x, y) == bg
            {
                x += 1;
            }
            if let Some(bg) = bg {
                paint(start, y, x - start, bg);
            }
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
    backgrounds(frame, |x, y, width, bg| {
        let position = origin + point(px(x as f32 * metrics.width), px(y as f32 * metrics.height));
        window.paint_quad(fill(
            Bounds::new(
                position,
                size(px(width as f32 * metrics.width), px(metrics.height)),
            ),
            bg,
        ));
    });
    let mut row = Vec::with_capacity(frame.cells.area.width as usize);
    for y in 0..frame.cells.area.height {
        row.clear();
        let mut x = 0;
        let mut separate = media_row(frame, y) || frame.cursor.is_some_and(|cursor| cursor.y == y);
        while x < frame.cells.area.width {
            if frame.under_media(x, y) {
                x += 1;
                continue;
            }
            let cell = &frame.cells[(x, y)];
            let width = unicode_width::UnicodeWidthStr::width(cell.symbol()).max(1);
            if cell.symbol() != " " && !cell.modifier.contains(Modifier::HIDDEN) {
                let layout = cache.layout(cell.symbol(), cell.modifier, metrics, window);
                // Oversized advances can overlap subsequent logical cell layers.
                // Keep exact per-cell ordering for that row, including zero-width
                // layouts and rows crossing media layers with different depths.
                separate |=
                    layout.width <= px(0.) || layout.width > px(width as f32 * metrics.width);
                let mut fg = if cell.modifier.contains(Modifier::REVERSED) {
                    color(cell.bg, frame.background)
                } else {
                    color(cell.fg, frame.foreground)
                };
                if cell.modifier.contains(Modifier::DIM) {
                    fg.l *= 0.65;
                }
                row.push(PaintedCell {
                    height: metrics.height,
                    layout,
                    fg,
                    modifier: cell.modifier,
                    position: origin
                        + point(px(x as f32 * metrics.width), px(y as f32 * metrics.height)),
                });
            }
            x = (x as usize + width).min(frame.cells.area.width as usize) as u16;
        }
        if separate {
            for cell in &row {
                window.paint_layer(cell.bounds(), |window| cell.paint(window, cx));
            }
        } else if let (Some(first), Some(last)) = (row.first(), row.last()) {
            // Use logical row bounds, not raster bounds: adjacent rows must keep
            // GPUI's original atlas ordering when italic glyphs overhang vertically.
            let bounds = Bounds::new(
                first.position,
                size(
                    last.position.x + last.layout.width - first.position.x,
                    px(metrics.height),
                ),
            );
            window.paint_layer(bounds, |window| {
                for cell in &row {
                    cell.paint(window, cx);
                }
            });
        }
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
        eprintln!(
            "native-paint {}x{} {}us",
            frame.cells.area.width,
            frame.cells.area.height,
            started.elapsed().as_micros()
        );
    }
}
fn media_row(frame: &FrameData, y: u16) -> bool {
    frame
        .media
        .iter()
        .any(|pane| y >= pane.body.y && y < pane.body.y.saturating_add(pane.body.height))
}

struct PaintedCell {
    height: f32,
    layout: Arc<LineLayout>,
    position: Point<Pixels>,
    fg: Hsla,
    modifier: Modifier,
}
impl PaintedCell {
    fn bounds(&self) -> Bounds<Pixels> {
        Bounds::new(self.position, size(self.layout.width, px(self.height)))
    }
    fn paint(&self, window: &mut Window, cx: &mut App) {
        let baseline =
            (px(self.height) - self.layout.ascent - self.layout.descent) / 2. + self.layout.ascent;
        if let Some(first) = self.layout.runs.iter().flat_map(|r| &r.glyphs).next() {
            let mut start = self.position.x + first.position.x;
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
                    point(
                        start,
                        self.position.y + baseline + self.layout.descent * 0.618,
                    ),
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
                        self.position.y + (self.layout.ascent * 0.5 + baseline) * 0.5,
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
                let glyph_origin = self.position + point(glyph.position.x, baseline);
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
            cells: Buffer::empty(Rect::new(0, 0, 8, 2)),
            media: vec![],
            cursor: None,
            overlays: vec![],
            media_input: false,
            metadata_paths: vec![],
        }
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
            width: 1,
            height: 2,
        });
        runs.clear();
        backgrounds(&frame, |x, y, w, c| runs.push((x, y, w, c)));
        assert_eq!(
            runs,
            vec![
                (0, 0, 1, color(Color::Red, 0)),
                (1, 0, 1, color(Color::Red, 0)),
                (6, 0, 1, color(Color::Reset, 0x181818)),
                (0, 1, 1, color(Color::Reset, 0x181818)),
                (1, 1, 1, color(Color::Reset, 0x181818)),
                (6, 1, 1, color(Color::Reset, 0x181818))
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
        assert_eq!(cache.entries.len(), CACHE_ENTRIES_PER_FACE);
        assert_eq!(cache.order.len(), CACHE_ENTRIES_PER_FACE);
        assert!(!cache.entries.contains_key("a"));
        let large = "x".repeat(MAX_CACHED_SYMBOL_BYTES + 1);
        cache.get_or_insert(&large, || Arc::new(9));
        assert!(!cache.entries.contains_key(&large));
        assert_eq!(cache.entries.len(), CACHE_ENTRIES_PER_FACE);
    }
}
