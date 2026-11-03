// SPDX-License-Identifier: MPL-2.0
use super::*;
use runyte::keymap::pointer_actions::{Action, Bindings};
fn page(words: bool) -> Page {
    Page {
        source: (None, 42),
        image: std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(1, 1),
        )])),
        animation: None,
        width: 600.,
        height: 800.,
        words: if words {
            vec![super::super::media::Word {
                bounds: [0.1, 0.1, 0.4, 0.2],
                text: "word".into(),
                line: 0,
            }]
        } else {
            Vec::new()
        },
        text_error: None,
    }
}
#[test]
fn automatic_selection_keeps_pdf_region_text_and_image_pan_domains() {
    for (words, pdf) in [(false, true), (true, true), (false, false)] {
        let page = page(words);
        let surface = PointerSurface {
            page: &page,
            area: [400.; 2],
            pdf,
        };
        let mut view = Viewport::new(0);
        view.begin_pointer(Some(Action::MediaAutoSelect), true, [100.; 2], surface);
        if words {
            assert!(matches!(view.drag, Some(Drag::Text(0))));
            assert_eq!(view.zoom, 1.);
        } else if pdf {
            assert!(matches!(view.drag, Some(Drag::Region(_))));
            assert_eq!(view.zoom, 1.);
        } else {
            assert!(matches!(view.drag, Some(Drag::Pan(_))));
            assert_eq!(view.zoom, 2.);
        }
        view.move_pointer([140., 160.], &page, [400.; 2]);
        assert!(view.drag.is_some());
    }
}
#[test]
fn explicit_region_pan_and_independent_double_click_do_not_depend_on_modifiers() {
    let page = page(false);
    let surface = PointerSurface {
        page: &page,
        area: [400.; 2],
        pdf: false,
    };
    let mut view = Viewport::new(0);
    view.begin_pointer(Some(Action::MediaRegion), false, [100.; 2], surface);
    view.move_pointer([140., 160.], &page, [400.; 2]);
    assert!(matches!(view.selection, Some(Selection::Region(start, end)) if start != end));
    view.begin_pointer(Some(Action::MediaPan), true, [100.; 2], surface);
    assert_eq!(view.zoom, 2.);
    assert!(matches!(view.drag, Some(Drag::Pan(_))));
    view.begin_pointer(None, true, [100.; 2], surface);
    assert_eq!(view.zoom, 1.);
    assert!(view.drag.is_none());
    view.begin_pointer(None, false, [100.; 2], surface);
    assert_eq!(view.zoom, 1.);
}
#[test]
fn continuous_pdf_wheel_retains_remainders_across_zoom_and_horizontal_input() {
    let page = page(false);
    let surface = PointerSurface {
        page: &page,
        area: [400.; 2],
        pdf: true,
    };
    let bindings = Bindings::default();
    let mut view = Viewport::new(0);
    assert_eq!(
        view.pointer_scroll(
            Action::MediaScroll,
            [0., 35.],
            [100.; 2],
            surface,
            &bindings
        ),
        0
    );
    assert_eq!(
        view.pointer_scroll(
            Action::MediaHorizontal,
            [0., 20.],
            [100.; 2],
            surface,
            &bindings
        ),
        0
    );
    assert_eq!(
        view.pointer_scroll(Action::MediaZoom, [20., 0.], [100.; 2], surface, &bindings),
        0
    );
    assert_eq!(view.scroll_y, 35.);
    assert_eq!(
        view.pointer_scroll(
            Action::MediaScroll,
            [0., 30.],
            [100.; 2],
            surface,
            &bindings
        ),
        -1
    );
    assert_eq!(view.scroll_y, 5.);
    assert_eq!(
        view.pointer_scroll(
            Action::MediaScroll,
            [0., -1265.],
            [100.; 2],
            surface,
            &bindings
        ),
        10
    );
    assert_eq!(view.scroll_y, 0.);
    view.pointer_scroll(
        Action::MediaZoom,
        [0., 1000.],
        [100.; 2],
        surface,
        &bindings,
    );
    assert_eq!(view.zoom, 2.);
    let center = view.center;
    assert_eq!(
        view.pointer_scroll(Action::MediaScroll, [5., 8.], [100.; 2], surface, &bindings),
        0
    );
    assert_ne!(view.center, center);
}
#[test]
fn wheel_zoom_steps_by_notches_and_scales_touchpad_fractions() {
    let page = page(false);
    let surface = PointerSurface {
        page: &page,
        area: [400.; 2],
        pdf: false,
    };
    let bindings = Bindings::default();
    let mut view = Viewport::new(0);
    let zoom = |view: &mut Viewport, notches: f32| {
        view.pointer_scroll(
            Action::MediaZoom,
            [0., notches],
            [200.; 2],
            surface,
            &bindings,
        );
    };
    zoom(&mut view, 1.);
    assert!((view.zoom - WHEEL_ZOOM_PER_NOTCH).abs() < 1e-6);
    zoom(&mut view, -1.);
    assert!((view.zoom - 1.).abs() < 1e-6);
    for _ in 0..4 {
        zoom(&mut view, 0.25);
    }
    assert!((view.zoom - WHEEL_ZOOM_PER_NOTCH).abs() < 1e-5);
}
