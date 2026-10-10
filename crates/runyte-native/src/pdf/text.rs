// SPDX-License-Identifier: MPL-2.0

//! Geometry-based word grouping in PDF content order, without OCR or layout inference.
use super::super::media::Word;
use hayro::kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use hayro_interpret::{
    Device, DrawMode, DrawProps,
    font::{Glyph, GlyphRun},
};

#[derive(Clone, PartialEq)]
struct Letter {
    text: String,
    bounds: Rect,
    baseline: Point,
    axis: Vec2,
    em: f64,
}

#[derive(Default)]
pub(super) struct Collector {
    letters: Vec<Letter>,
    previous_run: Vec<Letter>,
    bytes: usize,
    pub error: Option<String>,
}
impl Collector {
    pub fn words(self, width: f64, height: f64) -> (Vec<Word>, Option<String>) {
        if self.error.is_some() {
            return (Vec::new(), self.error);
        }
        let mut words: Vec<Word> = Vec::new();
        let mut previous: Option<Letter> = None;
        let mut separated = true;
        let mut line = 0;
        for letter in self.letters {
            if letter.text.chars().all(char::is_whitespace) {
                separated = true;
                continue;
            }
            if let Some(previous) = &previous {
                let delta = letter.baseline - previous.baseline;
                let em = previous.em.max(letter.em).max(0.01);
                let normal = Vec2::new(-previous.axis.y, previous.axis.x);
                let along = delta.dot(previous.axis);
                let previous_end = corners(previous.bounds)
                    .into_iter()
                    .map(|p| (p - previous.baseline).dot(previous.axis))
                    .fold(f64::NEG_INFINITY, f64::max);
                let new_line = delta.dot(normal).abs() > em * 0.35
                    || along < -em * 0.5
                    || along - previous_end > em * 2.0
                    || previous.axis.dot(letter.axis) < 0.95;
                if new_line {
                    line += 1;
                    separated = true;
                } else if along - previous_end > em * 0.25 {
                    separated = true;
                }
            }
            let bounds = [
                (letter.bounds.x0 / width).clamp(0., 1.) as f32,
                (letter.bounds.y0 / height).clamp(0., 1.) as f32,
                (letter.bounds.x1 / width).clamp(0., 1.) as f32,
                (letter.bounds.y1 / height).clamp(0., 1.) as f32,
            ];
            if bounds[0] < bounds[2] && bounds[1] < bounds[3] {
                if !separated && let Some(word) = words.last_mut() {
                    word.text.push_str(&letter.text);
                    word.bounds = [
                        word.bounds[0].min(bounds[0]),
                        word.bounds[1].min(bounds[1]),
                        word.bounds[2].max(bounds[2]),
                        word.bounds[3].max(bounds[3]),
                    ];
                } else {
                    words.push(Word {
                        text: letter.text.clone(),
                        bounds,
                        line,
                    });
                }
                separated = false;
            }
            previous = Some(letter);
        }
        (words, None)
    }
}

fn corners(rect: Rect) -> [Point; 4] {
    [
        Point::new(rect.x0, rect.y0),
        Point::new(rect.x0, rect.y1),
        Point::new(rect.x1, rect.y0),
        Point::new(rect.x1, rect.y1),
    ]
}

impl<'a> Device<'a> for Collector {
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        if self.error.is_some() {
            return;
        }
        let collect = (|| -> anyhow::Result<Vec<Letter>> {
            anyhow::ensure!(
                run.glyphs().len() <= 100000usize.saturating_sub(self.letters.len()),
                "PDF text exceeds 100000 glyphs"
            );
            let mut letters = Vec::new();
            for glyph in run.glyphs() {
                let text = match glyph.as_unicode() {
                    Some(hayro_interpret::hayro_cmap::BfString::Char(c)) => c.to_string(),
                    Some(hayro_interpret::hayro_cmap::BfString::String(s)) => s,
                    None => anyhow::bail!("PDF glyph has no Unicode mapping"),
                };
                self.bytes += text.len();
                anyhow::ensure!(self.bytes <= 4 * 1024 * 1024, "PDF text exceeds 4 MiB");
                let transform: Affine = props.transform * glyph.transform();
                anyhow::ensure!(
                    transform.as_coeffs().iter().all(|v| v.is_finite()),
                    "invalid PDF glyph transform"
                );
                let baseline = transform * Point::ZERO;
                let direction = transform * Point::new(1000., 0.) - baseline;
                let em = direction.hypot();
                anyhow::ensure!(em.is_finite() && em > 0., "invalid PDF glyph scale");
                let bounds = if text.chars().all(char::is_whitespace) {
                    Rect::ZERO
                } else if let Glyph::Outline(outline) = &**glyph {
                    transform.transform_rect_bbox(outline.outline().bounding_box())
                } else {
                    anyhow::bail!("Type3 PDF text requires Poppler extraction");
                };
                anyhow::ensure!(
                    corners(bounds)
                        .iter()
                        .all(|p| p.x.is_finite() && p.y.is_finite()),
                    "invalid PDF glyph bounds"
                );
                letters.push(Letter {
                    text,
                    bounds,
                    baseline,
                    axis: direction / em,
                    em,
                });
            }
            Ok(letters)
        })();
        match collect {
            Ok(letters) => {
                // Filled-and-stroked text is dispatched twice by the interpreter.
                if letters != self.previous_run {
                    self.letters.extend(letters.iter().cloned());
                }
                self.previous_run = letters;
            }
            Err(error) => {
                self.letters.clear();
                self.error = Some(error.to_string());
            }
        }
    }
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &hayro_interpret::ClipPath) {}
    fn push_transparency_group(
        &mut self,
        _: f32,
        _: Option<hayro_interpret::SoftMask<'a>>,
        _: hayro_interpret::BlendMode,
    ) {
    }
    fn draw_image(
        &mut self,
        _: hayro_interpret::Image<'a, '_>,
        _: hayro_interpret::ImageDrawProps<'a>,
    ) {
    }
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}
