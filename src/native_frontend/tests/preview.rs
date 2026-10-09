// SPDX-License-Identifier: MPL-2.0
use super::{Arc, AtomicBool, Ordering, Request, State, render_request};
#[test]
fn dropping_view_cancels_its_pending_helper_request() {
    let flag = Arc::new(AtomicBool::new(false));
    let state = State {
        cancel: Some(flag.clone()),
        generation: 0,
        serial: 0,
        request: None,
        output: None,
        scroll: [0.0; 2],
        selection: None,
        dragging: false,
        hidden: false,
        zoom: 1.0,
        keys: Default::default(),
    };
    drop(state);
    assert!(flag.load(Ordering::Acquire));
}
#[test]
fn obsolete_request_is_refused_before_starting_a_helper() {
    let request = Request {
        text: "hello".into(),
        selection_only: false,
        language: "text".into(),
        path: None,
        width: 100,
        height: 100,
        scale: 1.0,
        zoom: 1.0,
        scroll: [0.0, 0.0],
        selection: None,
    };
    let result = render_request(&request, &AtomicBool::new(true), &mut None);
    assert_eq!(result.err().as_deref(), Some("Preview cancelled"));
}

#[test]
fn large_hidpi_viewports_preserve_layout_with_a_bounded_raster() {
    for (w, h, display) in [
        (2020.0, 1100.0, 1.5),
        (3840.0, 2160.0, 2.0),
        (8000.0, 1200.0, 1.0),
    ] {
        let (width, height, scale) = super::raster_viewport(w, h, display);
        assert!(width <= 4096 && height <= 4096);
        assert!(u64::from(width) * u64::from(height) <= 4_000_000);
        assert!((width as f32 / scale - w).abs() <= 1.0 / scale);
        assert!((height as f32 / scale - h).abs() <= 1.0 / scale);
    }
    assert_eq!(super::raster_viewport(800.0, 600.0, 1.0), (800, 600, 1.0));
}

#[test]
fn document_navigation_uses_the_media_registry() {
    use super::{Navigation, preview_navigation};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut keys = runyte::keymap::KeySequence::default();
    let plain = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    assert_eq!(
        preview_navigation(&mut keys, plain('j'), 600.0),
        Some(Navigation::Scroll(0.0, 40.0))
    );
    assert_eq!(
        preview_navigation(
            &mut keys,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            600.0
        ),
        Some(Navigation::Scroll(0.0, 300.0))
    );
    assert_eq!(
        preview_navigation(
            &mut keys,
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
            600.0
        ),
        Some(Navigation::Scroll(0.0, 540.0))
    );
    assert_eq!(
        preview_navigation(&mut keys, plain('g'), 600.0),
        Some(Navigation::Prefix)
    );
    assert!(super::navigation_hints(&keys).contains("g g"));
    assert_eq!(
        preview_navigation(&mut keys, plain('g'), 600.0),
        Some(Navigation::Top)
    );
    assert_eq!(
        preview_navigation(&mut keys, plain('g'), 600.0),
        Some(Navigation::Prefix)
    );
    assert_eq!(
        preview_navigation(&mut keys, plain('e'), 600.0),
        Some(Navigation::Bottom)
    );
    assert_eq!(
        preview_navigation(&mut keys, plain('z'), 600.0),
        Some(Navigation::Prefix)
    );
    assert_eq!(
        preview_navigation(&mut keys, plain('h'), 600.0),
        Some(Navigation::Scroll(-40.0, 0.0))
    );
    assert_eq!(
        preview_navigation(&mut keys, plain('y'), 600.0),
        Some(Navigation::Copy)
    );
    assert_eq!(
        preview_navigation(&mut keys, plain('q'), 600.0),
        Some(Navigation::Back)
    );
    assert_eq!(preview_navigation(&mut keys, plain(':'), 600.0), None);
    assert!(keys.is_empty());
}
