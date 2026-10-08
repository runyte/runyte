// SPDX-License-Identifier: MPL-2.0

use super::{
    MediaPane, NativeView,
    media::Page,
    viewport::{Drag, Selection, Viewport},
};
use gpui::{Context, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollWheelEvent};
use runyte::media::ViewAction;
use std::sync::Arc;

impl NativeView {
    fn media_at(&self, position: Point<Pixels>) -> Option<MediaPane> {
        if self
            .bridge
            .painted_attachment
            .load(std::sync::atomic::Ordering::Acquire)
            != self
                .bridge
                .attachment
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return None;
        }
        let [x, y] = [f32::from(position.x), f32::from(position.y)];
        self.bridge
            .painted_media
            .lock()
            .unwrap()
            .iter()
            .find(|pane| {
                let area = pane.body;
                x >= area.x as f32 * self.metrics.width
                    && x < (area.x + area.width) as f32 * self.metrics.width
                    && y >= area.y as f32 * self.metrics.height
                    && y < (area.y + area.height) as f32 * self.metrics.height
            })
            .cloned()
    }
    fn local(&self, pane: &MediaPane, position: Point<Pixels>) -> [f32; 2] {
        [
            f32::from(position.x) - pane.body.x as f32 * self.metrics.width,
            f32::from(position.y) - pane.body.y as f32 * self.metrics.height,
        ]
    }
    pub(super) fn media_area(&self, pane: &MediaPane) -> [f32; 2] {
        [
            pane.body.width as f32 * self.metrics.width,
            pane.body.height as f32 * self.metrics.height,
        ]
    }
    fn focus_media(&self, pane: &MediaPane, delta: i32) {
        let mut requests = self.bridge.media_pointer.lock().unwrap();
        if requests.len() < 256 {
            requests.push((
                self.bridge
                    .painted_attachment
                    .load(std::sync::atomic::Ordering::Acquire),
                pane.pane,
                pane.path.clone(),
                delta,
            ));
        }
        drop(requests);
        let (w, h) = *self.bridge.dimensions.lock().unwrap();
        self.send(crossterm::event::Event::Resize(w, h));
    }
    pub(super) fn media_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .bridge
            .blocked_media
            .lock()
            .unwrap()
            .blocks(event.position)
        {
            return true;
        }
        let Some(pane) = self.media_at(event.position) else {
            return false;
        };
        self.focus_media(&pane, 0);
        let Some(Ok(page)) = self.media.get(&pane.path, pane.page) else {
            return true;
        };
        let area = self.media_area(&pane);
        let point = self.local(&pane, event.position);
        let view = self
            .viewports
            .entry((pane.pane, pane.path.clone()))
            .or_insert_with(|| Viewport::new(pane.page));
        view.show_page(pane.page);
        view.show_source(&page);
        let document = view
            .document_point(point, [page.width, page.height], area)
            .map(|v| v.clamp(0., 1.));
        view.message.clear();
        view.drag = if event.button == MouseButton::Middle
            || event.button == MouseButton::Right
            || event.modifiers.alt
        {
            Some(Drag::Pan(point))
        } else if event.modifiers.shift
            || (pane
                .path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
                && page.words.is_empty())
        {
            view.selection = Some(Selection::Region(document, document));
            Some(Drag::Region(document))
        } else if let Some(word) = Viewport::nearest_word(&page, document) {
            view.selection = Some(Selection::Text(word, word));
            Some(Drag::Text(word))
        } else {
            if event.click_count == 2 {
                view.zoom_at(
                    if view.zoom <= 1.01 {
                        2.
                    } else {
                        1. / view.zoom
                    },
                    point,
                    [page.width, page.height],
                    area,
                );
            }
            Some(Drag::Pan(point))
        };
        cx.notify();
        true
    }
    pub(super) fn media_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .bridge
            .blocked_media
            .lock()
            .unwrap()
            .blocks(event.position)
        {
            for view in self.viewports.values_mut() {
                view.drag = None;
            }
            return true;
        }
        let Some(key) = self
            .viewports
            .iter()
            .find(|(_, view)| view.drag.is_some())
            .map(|(key, _)| key.clone())
        else {
            return false;
        };
        let pane = self
            .bridge
            .painted_media
            .lock()
            .unwrap()
            .iter()
            .find(|pane| (pane.pane, &pane.path) == (key.0, &key.1))
            .cloned();
        let Some(pane) = pane else {
            self.viewports.get_mut(&key).unwrap().drag = None;
            return true;
        };
        let Some(Ok(page)) = self.media.get(&pane.path, pane.page) else {
            return true;
        };
        let area = self.media_area(&pane);
        let point = self.local(&pane, event.position);
        let view = self.viewports.get_mut(&key).unwrap();
        view.show_source(&page);
        if event.pressed_button.is_none() {
            view.drag = None;
            return true;
        }
        let document = view
            .document_point(point, [page.width, page.height], area)
            .map(|v| v.clamp(0., 1.));
        match view.drag {
            Some(Drag::Pan(previous)) => {
                view.pan(
                    [point[0] - previous[0], point[1] - previous[1]],
                    [page.width, page.height],
                    area,
                );
                view.drag = Some(Drag::Pan(point));
            }
            Some(Drag::Text(start)) => {
                if let Some(end) = Viewport::nearest_word(&page, document) {
                    view.selection = Some(Selection::Text(start, end));
                }
            }
            Some(Drag::Region(start)) => view.selection = Some(Selection::Region(start, document)),
            None => {}
        }
        cx.notify();
        true
    }
    pub(super) fn media_mouse_up(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut dragged = false;
        for view in self.viewports.values_mut() {
            dragged |= view.drag.take().is_some();
        }
        if dragged {
            cx.notify();
        }
        dragged
            || self.media_at(position).is_some()
            || self.bridge.blocked_media.lock().unwrap().blocks(position)
    }
    pub(super) fn media_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .bridge
            .blocked_media
            .lock()
            .unwrap()
            .blocks(event.position)
        {
            return true;
        }
        let Some(pane) = self.media_at(event.position) else {
            return false;
        };
        let Some(Ok(page)) = self.media.get(&pane.path, pane.page) else {
            return true;
        };
        let delta = event.delta.pixel_delta(gpui::px(self.metrics.height));
        let mut delta = [f32::from(delta.x), f32::from(delta.y)];
        let area = self.media_area(&pane);
        let point = self.local(&pane, event.position);
        let view = self
            .viewports
            .entry((pane.pane, pane.path.clone()))
            .or_insert_with(|| Viewport::new(pane.page));
        view.show_page(pane.page);
        view.show_source(&page);
        if event.modifiers.control || event.modifiers.platform {
            view.zoom_at(
                (delta[1] * 0.008).exp().clamp(0.5, 2.),
                point,
                [page.width, page.height],
                area,
            );
        } else if view.zoom <= 1.01
            && !event.modifiers.shift
            && pane
                .path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
        {
            view.scroll_y += delta[1];
            let pages = (view.scroll_y / 60.).trunc() as i32;
            if pages != 0 {
                view.scroll_y -= pages as f32 * 60.;
                self.focus_media(&pane, -pages.clamp(-10, 10));
            }
        } else {
            if event.modifiers.shift && delta[0] == 0. {
                delta = [delta[1], 0.];
            }
            view.pan(delta, [page.width, page.height], area);
        }
        cx.notify();
        true
    }
    pub(super) fn apply_media_requests(&mut self, cx: &mut Context<Self>) {
        let requests = super::ready_media_requests(
            &mut self.bridge.media_requests.lock().unwrap(),
            self.frame.as_deref(),
            self.bridge
                .attachment
                .load(std::sync::atomic::Ordering::Acquire),
        );
        if !requests.is_empty() {
            cx.notify();
        }
        for request in requests {
            let Some(pane) = self
                .frame
                .as_ref()
                .and_then(|frame| {
                    frame
                        .media
                        .iter()
                        .find(|pane| pane.pane == request.pane && pane.path == request.path)
                })
                .cloned()
            else {
                continue;
            };
            if request.action == ViewAction::Back {
                if pane.page != request.page {
                    continue;
                }
                let view = self
                    .viewports
                    .entry((pane.pane, pane.path.clone()))
                    .or_insert_with(|| Viewport::new(pane.page));
                view.show_page(pane.page);
                if view.back() {
                    let mut back = self.bridge.media_back.lock().unwrap();
                    if back.len() < 256 {
                        back.push((
                            self.frame.as_ref().unwrap().attachment,
                            pane.pane,
                            pane.path,
                            pane.page,
                        ));
                    }
                    drop(back);
                    let (w, h) = *self.bridge.dimensions.lock().unwrap();
                    self.send(crossterm::event::Event::Resize(w, h));
                }
                continue;
            }
            let Some(Ok(page)) = self.media.get(&pane.path, request.page) else {
                let mut requests = self.bridge.media_requests.lock().unwrap();
                if requests.len() < 256 {
                    requests.push_back(super::PendingMediaRequest {
                        attachment: self.frame.as_ref().unwrap().attachment,
                        frame: self.frame.as_ref().unwrap().id.unwrap(),
                        request,
                    });
                }
                continue;
            };
            let area = self.media_area(&pane);
            let image = [page.width, page.height];
            let view = self
                .viewports
                .entry((pane.pane, pane.path))
                .or_insert_with(|| Viewport::new(pane.page));
            view.show_page(request.page);
            view.show_source(&page);
            view.message.clear();
            match request.action {
                ViewAction::ZoomIn => view.zoom_at(1.25, [area[0] / 2., area[1] / 2.], image, area),
                ViewAction::ZoomOut => view.zoom_at(0.8, [area[0] / 2., area[1] / 2.], image, area),
                ViewAction::Fit => {
                    view.zoom = 1.;
                    view.center = [0.5; 2];
                }
                ViewAction::ActualSize => {
                    view.zoom = 1. / Viewport::fit_scale(image, area);
                    view.clamp(image, area);
                }
                ViewAction::Center => view.center = [0.5; 2],
                ViewAction::PanLeft => view.pan([64., 0.], image, area),
                ViewAction::PanRight => view.pan([-64., 0.], image, area),
                ViewAction::PanUp => view.pan([0., 64.], image, area),
                ViewAction::PanDown => view.pan([0., -64.], image, area),
                ViewAction::Back => unreachable!("back does not require a rendered page"),
                ViewAction::BeginSelection => view.begin_selection(&page),
                ViewAction::ExtendLeft => view.extend_selection(&page, -1, 0),
                ViewAction::ExtendRight => view.extend_selection(&page, 1, 0),
                ViewAction::ExtendUp => view.extend_selection(&page, 0, -1),
                ViewAction::ExtendDown => view.extend_selection(&page, 0, 1),
                ViewAction::SelectAll => {
                    view.selection = Some(if page.words.is_empty() {
                        Selection::Region([0., 0.], [1., 1.])
                    } else {
                        Selection::Text(0, page.words.len() - 1)
                    })
                }
                ViewAction::CopySelection => {
                    copy_selection(view, &page, &mut self.image_clipboard, cx)
                }
            }
        }
    }
}

fn copy_selection(
    view: &mut Viewport,
    page: &Arc<Page>,
    clipboard: &mut Option<arboard::Clipboard>,
    cx: &mut Context<NativeView>,
) {
    if let Some(text) = view.selected_text(page) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        view.message = "Copied PDF text".into();
    } else if let Some(Selection::Region(a, b)) = view.selection {
        let result = (|| -> anyhow::Result<()> {
            let image = crop_rgba(page, a, b)?;
            if clipboard.is_none() {
                *clipboard = Some(arboard::Clipboard::new()?);
            }
            clipboard.as_mut().unwrap().set_image(arboard::ImageData {
                width: image.width() as usize,
                height: image.height() as usize,
                bytes: std::borrow::Cow::Owned(image.into_raw()),
            })?;
            Ok(())
        })();
        view.message = match result {
            Ok(()) => "Copied image region".into(),
            Err(error) => format!("Copy failed: {error}"),
        };
    } else {
        view.message = "Select PDF text or Shift-drag an image region first".into();
    }
}

pub(super) fn crop_rgba(page: &Page, a: [f32; 2], b: [f32; 2]) -> anyhow::Result<image::RgbaImage> {
    let width = page.width as u32;
    let height = page.height as u32;
    let x = (a[0].min(b[0]).clamp(0., 1.) * width as f32).floor() as u32;
    let y = (a[1].min(b[1]).clamp(0., 1.) * height as f32).floor() as u32;
    let right = (a[0].max(b[0]).clamp(0., 1.) * width as f32).ceil() as u32;
    let bottom = (a[1].max(b[1]).clamp(0., 1.) * height as f32).ceil() as u32;
    anyhow::ensure!(right > x && bottom > y, "selection is empty");
    let pixels = page
        .image
        .as_bytes(0)
        .ok_or_else(|| anyhow::anyhow!("image pixels unavailable"))?;
    let mut region = image::RgbaImage::new(right - x, bottom - y);
    for (dx, dy, pixel) in region.enumerate_pixels_mut() {
        let offset = (((dy + y) * width + dx + x) * 4) as usize;
        *pixel = image::Rgba([
            pixels[offset + 2],
            pixels[offset + 1],
            pixels[offset],
            pixels[offset + 3],
        ]);
    }
    Ok(region)
}
