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
        cache: None,
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
        adjacent: true,
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

fn cached_output() -> super::Output {
    super::Output {
        scale: 1.0,
        zoom: 1.0,
        reply: super::Reply {
            width: 1080,
            height: 800,
            selected: String::new(),
            link: None,
            scroll: [0.0; 2],
            max_scroll: [2000.0, 5000.0],
            raster_width: 1620,
            raster_height: 2400,
            origin: [0.0; 2],
            cacheable: true,
        },
        image: Arc::new(super::RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(1, 1),
        )])),
    }
}
#[test]
fn cached_scroll_prefetches_before_the_edge_and_never_exposes_blank_pixels() {
    let cache = cached_output();
    assert!(cache.covers([100.0, 600.25], true));
    assert!(cache.covers([500.0, 1300.0], false));
    assert!(
        !cache.covers([500.0, 1300.0], true),
        "prefetch while the viewport still fits"
    );
    assert_eq!(cache.display_scroll([100.25, 600.25]), [100.25, 600.25]);
    assert_eq!(
        cache.display_scroll([2000.0, 5000.0]),
        [540.0, 1600.0],
        "retain a fully painted viewport on a jump beyond the cache"
    );
    assert!(!cache.covers([2000.0, 5000.0], false));
}
#[test]
fn cache_uses_logical_coordinates_at_display_and_document_zoom() {
    let mut cache = cached_output();
    cache.scale = 2.0;
    cache.zoom = 1.5;
    assert!(cache.covers([0.0, 200.25], true));
    let displayed = cache.display_scroll([1000.0, 2000.0]);
    assert_eq!(displayed[0], 180.0);
    assert!((displayed[1] - 1600.0 / 3.0).abs() < 1e-9);
    cache.reply.cacheable = false;
    assert!(!cache.covers([0.0, 0.0], false));
    assert_eq!(
        cache.display_scroll([0.0, 20.0]),
        [0.0; 2],
        "viewport-anchored paint must not translate"
    );
}

#[test]
fn an_in_flight_cache_refresh_cannot_rewind_a_newer_cached_scroll() {
    use super::{Done, Job, Views, mpsc};
    let (jobs, _jobs_rx) = mpsc::sync_channel::<Job>(1);
    let (results, results_rx) = mpsc::sync_channel(1);
    let mut views = Views {
        worker: Some((jobs, results_rx)),
        ..Default::default()
    };
    let request = Request {
        text: "source".into(),
        language: "text".into(),
        selection_only: false,
        path: None,
        width: 1080,
        height: 800,
        scale: 1.0,
        zoom: 1.0,
        scroll: [0.0; 2],
        selection: None,
        adjacent: true,
    };
    assert!(cached_output().matches(&request));
    let mut resized = request.clone();
    resized.width = 600;
    assert!(!cached_output().matches(&resized));
    views.states.insert(
        1,
        State {
            cancel: None,
            generation: 2,
            serial: 3,
            request: Some(request),
            output: None,
            cache: None,
            scroll: [0.0, 600.25],
            selection: None,
            dragging: false,
            hidden: false,
            zoom: 1.0,
            keys: Default::default(),
        },
    );
    results
        .send(Done {
            pane: 1,
            generation: 2,
            serial: 3,
            result: Ok(cached_output()),
        })
        .unwrap();
    assert!(views.poll());
    assert_eq!(views.states[&1].scroll, [0.0, 600.25]);
    assert!(
        views.states[&1]
            .cache
            .as_ref()
            .unwrap()
            .covers([0.0, 600.25], false)
    );
    results
        .send(Done {
            pane: 1,
            generation: 2,
            serial: 3,
            result: Err("renderer failed".into()),
        })
        .unwrap();
    assert!(views.poll());
    assert!(
        views.states[&1].cache.is_none(),
        "an old cache must not hide a failed refresh"
    );
    assert!(views.states[&1].output.as_ref().unwrap().is_err());
}
