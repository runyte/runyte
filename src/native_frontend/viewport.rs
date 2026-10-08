// SPDX-License-Identifier: MPL-2.0

//! Pane-local document coordinates shared by drawing, hit testing and copying.
use super::media::Page;

#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    Text(usize, usize),
    Region([f32; 2], [f32; 2]),
}
#[derive(Clone, Copy)]
pub enum Drag {
    Pan([f32; 2]),
    Text(usize),
    Region([f32; 2]),
}
pub struct Viewport {
    pub page: usize,
    source: Option<(Option<std::time::SystemTime>, u64)>,
    pub zoom: f32,
    pub scroll_y: f32,
    pub center: [f32; 2],
    pub selection: Option<Selection>,
    pub drag: Option<Drag>,
    pub message: String,
}
impl Viewport {
    /// Returns true only when Escape has no selection left to cancel.
    pub fn back(&mut self) -> bool {
        self.drag = None;
        self.selection.take().is_none()
    }

    pub fn new(page: usize) -> Self {
        Self {
            page,
            source: None,
            zoom: 1.,
            scroll_y: 0.,
            center: [0.5; 2],
            selection: None,
            drag: None,
            message: String::new(),
        }
    }
    pub fn show_source(&mut self, page: &Page) {
        if self.source.is_some_and(|source| source != page.source) {
            self.selection = None;
            self.drag = None;
            self.message.clear();
            self.center = [0.5; 2];
            self.zoom = 1.;
        }
        self.source = Some(page.source);
    }
    pub fn show_page(&mut self, page: usize) {
        if self.page != page {
            self.page = page;
            self.scroll_y = 0.;
            self.center = [0.5; 2];
            self.selection = None;
            self.drag = None;
            self.message.clear();
        }
    }
    pub fn fit_scale(image: [f32; 2], area: [f32; 2]) -> f32 {
        (area[0] / image[0]).min(area[1] / image[1]).max(0.0001)
    }
    pub fn geometry(&self, image: [f32; 2], area: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let scale = Self::fit_scale(image, area) * self.zoom;
        let size = [image[0] * scale, image[1] * scale];
        (
            [
                area[0] / 2. - self.center[0] * size[0],
                area[1] / 2. - self.center[1] * size[1],
            ],
            size,
        )
    }
    pub fn document_point(&self, point: [f32; 2], image: [f32; 2], area: [f32; 2]) -> [f32; 2] {
        let (origin, size) = self.geometry(image, area);
        [
            (point[0] - origin[0]) / size[0],
            (point[1] - origin[1]) / size[1],
        ]
    }
    pub fn zoom_at(&mut self, factor: f32, point: [f32; 2], image: [f32; 2], area: [f32; 2]) {
        let before = self.document_point(point, image, area);
        self.zoom = (self.zoom * factor).clamp(0.1, 64.);
        let (_, size) = self.geometry(image, area);
        self.center = [
            before[0] - (point[0] - area[0] / 2.) / size[0],
            before[1] - (point[1] - area[1] / 2.) / size[1],
        ];
        self.clamp(image, area);
    }
    pub fn pan(&mut self, delta: [f32; 2], image: [f32; 2], area: [f32; 2]) {
        let (_, size) = self.geometry(image, area);
        self.center[0] -= delta[0] / size[0];
        self.center[1] -= delta[1] / size[1];
        self.clamp(image, area);
    }
    pub fn clamp(&mut self, image: [f32; 2], area: [f32; 2]) {
        let (_, size) = self.geometry(image, area);
        for axis in 0..2 {
            let half = (area[axis] / size[axis] / 2.).min(0.5);
            self.center[axis] = self.center[axis].clamp(half, 1. - half);
        }
    }
    pub fn nearest_word(page: &Page, point: [f32; 2]) -> Option<usize> {
        page.words
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let distance = |r: [f32; 4]| {
                    let x = point[0] - point[0].clamp(r[0], r[2]);
                    let y = point[1] - point[1].clamp(r[1], r[3]);
                    x * x + y * y * 4.
                };
                distance(a.bounds).total_cmp(&distance(b.bounds))
            })
            .map(|(index, _)| index)
    }
    pub fn begin_selection(&mut self, page: &Page) {
        if self.selection.is_none() {
            self.selection = Some(if page.words.is_empty() {
                Selection::Region([0.5; 2], [0.5; 2])
            } else {
                Selection::Text(0, 0)
            });
        }
    }
    pub fn extend_selection(&mut self, page: &Page, horizontal: i32, vertical: i32) {
        self.begin_selection(page);
        match self.selection.as_mut().unwrap() {
            Selection::Region(_, end) => {
                end[0] = (end[0] + horizontal as f32 * 0.02).clamp(0., 1.);
                end[1] = (end[1] + vertical as f32 * 0.02).clamp(0., 1.);
            }
            Selection::Text(_, end) => {
                if horizontal != 0 {
                    *end = end
                        .saturating_add_signed(horizontal as isize)
                        .min(page.words.len().saturating_sub(1));
                }
                if vertical != 0
                    && let Some(word) = page.words.get(*end)
                {
                    let line = word.line.saturating_add_signed(vertical as isize);
                    if line != word.line
                        && let Some((index, _)) = page
                            .words
                            .iter()
                            .enumerate()
                            .filter(|(_, candidate)| candidate.line == line)
                            .min_by(|(_, a), (_, b)| {
                                (a.bounds[0] - word.bounds[0])
                                    .abs()
                                    .total_cmp(&(b.bounds[0] - word.bounds[0]).abs())
                            })
                    {
                        *end = index;
                    }
                }
            }
        }
    }
    pub fn selected_text(&self, page: &Page) -> Option<String> {
        let Selection::Text(a, b) = self.selection.as_ref()? else {
            return None;
        };
        let mut text = String::new();
        let mut last_line = None;
        for word in page.words.get((*a).min(*b)..=(*a).max(*b))? {
            if let Some(line) = last_line {
                text.push(if line == word.line { ' ' } else { '\n' });
            }
            text.push_str(&word.text);
            last_line = Some(word.line);
        }
        Some(text)
    }
    pub fn selected_bounds(&self, page: &Page) -> Vec<[f32; 4]> {
        match self.selection {
            Some(Selection::Region(a, b)) => vec![[
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[0].max(b[0]),
                a[1].max(b[1]),
            ]],
            Some(Selection::Text(a, b)) => page
                .words
                .get(a.min(b)..=a.max(b))
                .unwrap_or_default()
                .iter()
                .map(|word| word.bounds)
                .collect(),
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zoom_preserves_pointer_anchor_and_pan_never_loses_the_document() {
        let mut view = Viewport::new(1);
        let image = [1000., 800.];
        let area = [500., 400.];
        let pointer = [200., 160.];
        let before = view.document_point(pointer, image, area);
        view.zoom_at(2., pointer, image, area);
        let after = view.document_point(pointer, image, area);
        assert!((before[0] - after[0]).abs() < 0.0001);
        assert!((before[1] - after[1]).abs() < 0.0001);
        view.pan([100000., -100000.], image, area);
        assert_eq!(view.center, [0.25, 0.75]);
        view.show_page(2);
        assert_eq!(view.center, [0.5; 2]);
        assert_eq!(view.zoom, 2.);
    }
}

#[cfg(test)]
mod back_tests {
    use super::*;

    #[test]
    fn escape_clears_text_or_region_before_leaving_without_resetting_view() {
        for selection in [
            Selection::Text(1, 3),
            Selection::Region([0.1, 0.2], [0.4, 0.5]),
        ] {
            let mut view = Viewport::new(4);
            view.zoom = 2.;
            view.center = [0.4, 0.6];
            view.selection = Some(selection);
            view.drag = Some(Drag::Pan([1., 2.]));
            assert!(!view.back());
            assert!(view.selection.is_none());
            assert!(view.drag.is_none());
            assert!(view.back());
            assert_eq!(view.page, 4);
            assert_eq!(view.zoom, 2.);
            assert_eq!(view.center, [0.4, 0.6]);
        }
    }
}
