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
        keys_bindings: None,
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
        theme: None,
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
        theme: None,
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
            themed: false,
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
        theme: None,
    };
    assert!(cached_output().matches(&request));
    let mut resized = request.clone();
    resized.width = 600;
    assert!(!cached_output().matches(&resized));
    // A theme change or toggling the setting replaces a raster painted in the old colours.
    let mut themed = request.clone();
    themed.theme = Some(palette());
    assert!(!cached_output().matches(&themed));
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
            keys_bindings: None,
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

#[test]
fn pane_focus_shortcuts_route_to_editor_without_dismissing_preview() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    for direction in [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down] {
        assert!(super::editor_pane_key(KeyEvent::new(
            direction,
            KeyModifiers::CONTROL
        )));
        assert!(!super::editor_pane_key(KeyEvent::new(
            direction,
            KeyModifiers::NONE
        )));
    }
    for key in ['i', 'q', 'j', 'f'] {
        assert!(!super::editor_pane_key(KeyEvent::new(
            KeyCode::Char(key),
            KeyModifiers::NONE
        )));
    }
}

#[test]
fn native_fast_pane_strokes_and_repeats_reach_the_editor() {
    use crossterm::event::KeyEventKind;
    use runyte::{
        command::{EditorCommand as C, Mode},
        keymap::{BindingTarget, KeySequence, Lookup},
    };
    for (spelling, command) in [
        ("ctrl-h", C::FocusWindowLeft),
        ("ctrl-j", C::FocusWindowDown),
        ("ctrl-k", C::FocusWindowUp),
        ("ctrl-l", C::FocusWindowRight),
    ] {
        let mut key =
            super::super::translate_key(&gpui::Keystroke::parse(spelling).unwrap()).unwrap();
        for kind in [KeyEventKind::Press, KeyEventKind::Repeat] {
            key.kind = kind;
            assert!(super::editor_pane_key(key), "{spelling}: {kind:?}");
        }
        let Some(runyte::input::InputEvent::Key(stroke)) =
            runyte::tui::input::convert_event(crossterm::event::Event::Key(key)).unwrap()
        else {
            panic!("native pane key was lost");
        };
        let sequence = KeySequence::new([stroke]);
        assert!(matches!(
            runyte::keymap::keymap_for(true).lookup(Mode::Normal, &sequence),
            Lookup::Exact(binding) if binding.target == BindingTarget::Editor(command)
        ));
        assert!(matches!(
            runyte::keymap::keymap_for(false).lookup(Mode::Normal, &sequence),
            Lookup::NoMatch
        ));
    }
}

fn palette() -> runyte::document_preview::Palette {
    let theme = runyte::config::Config::default().startup_theme().unwrap().1;
    runyte::document_preview::Palette::from_theme(&theme)
}

#[test]
fn themed_request_carries_channels_the_helper_reads() {
    let request = Request {
        text: "# hi".into(),
        selection_only: false,
        language: "markdown".into(),
        path: None,
        width: 100,
        height: 100,
        scale: 1.0,
        zoom: 1.0,
        scroll: [0.0; 2],
        selection: None,
        adjacent: false,
        theme: Some(palette()),
    };
    let wire = serde_json::to_value(&request).unwrap();
    let theme = &wire["theme"];
    for key in [
        "background",
        "foreground",
        "muted",
        "surface",
        "code",
        "border",
        "link",
        "heading",
        "error",
    ] {
        assert_eq!(theme[key].as_array().map(Vec::len), Some(3), "{key}");
    }
    assert!(theme["dark"].is_boolean());
    let syntax = theme["syntax"].as_array().unwrap();
    assert!(!syntax.is_empty());
    assert!(
        syntax
            .iter()
            .all(|entry| entry[0].is_string() && entry[1].as_array().map(Vec::len) == Some(3))
    );
    let light = Request {
        theme: None,
        ..request
    };
    assert!(serde_json::to_value(&light).unwrap()["theme"].is_null());
}

#[test]
fn layer_colours_follow_the_palette_and_keep_the_light_page_without_one() {
    use super::LayerColours;
    assert_eq!(
        LayerColours::new(None),
        LayerColours {
            ground: 0xffffff,
            text: 0x333333,
            link: 0x0969da,
            error: 0x990000,
        }
    );
    let mut palette = palette();
    palette.background = [0x01, 0x02, 0x03];
    palette.foreground = [0x04, 0x05, 0x06];
    palette.link = [0x07, 0x08, 0x09];
    palette.error = [0x0a, 0x0b, 0x0c];
    assert_eq!(
        LayerColours::new(Some(&palette)),
        LayerColours {
            ground: 0x010203,
            text: 0x040506,
            link: 0x070809,
            error: 0x0a0b0c,
        }
    );
}

#[test]
fn authored_pages_keep_a_white_layer_under_a_themed_editor() {
    use super::{Pane, page_palette};
    let pane = |language: &str| Pane {
        pane: 1,
        active: true,
        body: runyte::layout::Rect::default(),
        document: runyte::document_preview::DocumentPreview {
            generation: 1,
            source: 0,
            text: String::new(),
            selection_only: false,
            language: language.into(),
            path: None,
        },
        palette: Some(palette()),
    };
    // Before the first frame the language decides.
    assert!(page_palette(&pane("markdown"), None).is_some());
    assert!(page_palette(&pane("html"), None).is_none());
    assert!(page_palette(&pane("svg"), None).is_none());
    let error = String::from("failed");
    assert!(page_palette(&pane("rust"), Some(Err(&error))).is_some());
    // Afterwards the helper's reply does, including for a broken drawing.
    let mut output = cached_output();
    assert!(page_palette(&pane("markdown"), Some(Ok(&output))).is_none());
    output.reply.themed = true;
    assert!(page_palette(&pane("svg"), Some(Ok(&output))).is_some());
    let light = Pane {
        palette: None,
        ..pane("markdown")
    };
    assert!(page_palette(&light, Some(Ok(&output))).is_none());
}

fn pointer_fixture() -> (super::Views, super::Pane, super::CellMetrics) {
    let mut views = super::Views::default();
    views.states.insert(
        1,
        State {
            request: Some(Request {
                text: "source".into(),
                language: "text".into(),
                path: None,
                selection_only: false,
                width: 1080,
                height: 800,
                scale: 1.0,
                zoom: 1.0,
                scroll: [0.0; 2],
                selection: None,
                adjacent: true,
                theme: None,
            }),
            cache: Some(Arc::new(cached_output())),
            zoom: 1.0,
            cancel: None,
            generation: 1,
            serial: 0,
            output: None,
            scroll: [0.0; 2],
            selection: None,
            dragging: false,
            hidden: false,
            keys: Default::default(),
            keys_bindings: None,
        },
    );
    let pane = super::Pane {
        pane: 1,
        active: true,
        body: runyte::layout::Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 40,
        },
        document: runyte::document_preview::DocumentPreview {
            generation: 1,
            source: 0,
            text: "source".into(),
            selection_only: false,
            language: "text".into(),
            path: None,
        },
        palette: None,
    };
    (views, pane, super::CellMetrics::new(15))
}

#[test]
fn inactive_preview_forwards_only_focus_press_and_respects_overlays() {
    use super::{PointerResult, point, px};
    let (mut views, mut pane, metrics) = pointer_fixture();
    let position = point(px(20.0), px(30.0));
    assert_eq!(
        views.pointer(Some(&pane), true, position, metrics, 0),
        PointerResult::Changed
    );
    let selection = views.states[&1].selection;
    pane.active = false;
    assert_eq!(
        views.pointer(Some(&pane), true, position, metrics, 0),
        PointerResult::Forward
    );
    assert_eq!(views.states[&1].selection, selection);
    assert!(!views.states[&1].dragging);
    for phase in [1, 2] {
        assert_eq!(
            views.pointer(Some(&pane), true, position, metrics, phase),
            PointerResult::Consumed
        );
    }
    assert_eq!(
        views.pointer(Some(&pane), false, position, metrics, 0),
        PointerResult::Consumed
    );
    assert_eq!(views.states[&1].selection, selection);
}

#[test]
fn preview_release_outside_body_or_over_another_pane_ends_selection() {
    use super::{PointerResult, point, px};
    for other_pane in [false, true] {
        let (mut views, pane, metrics) = pointer_fixture();
        let start = point(px(20.0), px(30.0));
        let end = point(px(90.0), px(60.0));
        assert_eq!(
            views.pointer(Some(&pane), true, start, metrics, 0),
            PointerResult::Changed
        );
        assert_eq!(
            views.pointer(Some(&pane), true, end, metrics, 1),
            PointerResult::Changed
        );
        let selection = views.states[&1].selection;
        let mut other = pane.clone();
        other.pane = 2;
        other.active = false;
        views.states.insert(2, State::default());
        views.pointer(other_pane.then_some(&other), true, end, metrics, 2);
        assert!(!views.states[&1].dragging);
        assert_eq!(
            views.pointer(Some(&pane), true, start, metrics, 1),
            PointerResult::Consumed
        );
        assert_eq!(views.states[&1].selection, selection);
        assert!(views.states[&2].selection.is_none());
    }
}

#[test]
fn cancelling_preview_capture_preserves_selection_and_stops_later_motion() {
    use super::{PointerResult, point, px};
    let (mut views, pane, metrics) = pointer_fixture();
    let start = point(px(20.0), px(30.0));
    let end = point(px(90.0), px(60.0));
    views.pointer(Some(&pane), true, start, metrics, 0);
    views.pointer(Some(&pane), true, end, metrics, 1);
    let selection = views.states[&1].selection;
    // Focus loss and buttonless motion use the same capture cancellation.
    views.cancel_drag();
    assert!(!views.states[&1].dragging);
    assert_eq!(
        views.pointer(Some(&pane), true, start, metrics, 1),
        PointerResult::Consumed
    );
    assert_eq!(views.states[&1].selection, selection);
    assert_eq!(
        views.pointer(Some(&pane), true, start, metrics, 0),
        PointerResult::Changed
    );
    assert!(views.states[&1].dragging);
}

#[test]
fn preview_input_fences_replacement_but_preserves_intentional_source_handoff() {
    use super::{InputRoute, Target, input_route};
    let target = Target {
        pane: 2,
        source: 7,
        generation: 11,
    };
    let source = Some((2, 7));
    assert_eq!(
        input_route(Some(target), Some(target), source, true, None),
        InputRoute::Local
    );
    assert_eq!(
        input_route(Some(target), Some(target), source, false, None),
        InputRoute::Host
    );
    assert_eq!(
        input_route(None, Some(target), source, true, None),
        InputRoute::Host
    );
    for replacement in [
        Target { pane: 3, ..target },
        Target {
            source: 8,
            ..target
        },
        Target {
            generation: 12,
            ..target
        },
    ] {
        assert_eq!(
            input_route(Some(target), Some(replacement), source, true, None),
            InputRoute::Discard
        );
        assert_eq!(
            input_route(Some(target), Some(replacement), source, true, Some(target)),
            InputRoute::Discard
        );
    }
    assert_eq!(
        input_route(Some(target), None, source, true, None),
        InputRoute::Discard
    );
    // i intentionally returns to source; a/b/c already queued from the same
    // painted preview remain source input after the dismissal acknowledgement.
    for _ in ['a', 'b', 'c'] {
        assert_eq!(
            input_route(Some(target), None, source, true, Some(target)),
            InputRoute::Host
        );
    }
    assert_eq!(
        input_route(Some(target), None, Some((3, 7)), true, Some(target)),
        InputRoute::Discard
    );
}

#[test]
fn effective_preview_navigation_keeps_pane_precedence_and_remapped_semantics() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use runyte::{
        command::Mode,
        keymap::{BindingScope, KeySequence},
    };
    let section = serde_json::from_value(serde_json::json!({"ui": {"native-preview": {"back": ["F8"]}}, "scopes": {"media": {"normal": {"F7": "media-zoom-in"}}}})).unwrap();
    let compiled = runyte::keymap::configured::compile(&section, runyte::keymap::default_keymap());
    assert!(compiled.errors.is_empty());
    let bindings = compiled
        .keymap
        .native_preview(Mode::Normal, BindingScope::Global);
    let mut keys = KeySequence::parse("g").unwrap();
    assert_eq!(
        super::effective_navigation(
            &mut keys,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL),
            600.0,
            &bindings
        ),
        Some(super::Navigation::Forward)
    );
    assert!(keys.is_empty());
    assert_eq!(
        super::effective_navigation(
            &mut keys,
            KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE),
            600.0,
            &bindings
        ),
        Some(super::Navigation::Zoom(1.2))
    );
    assert_eq!(
        super::effective_navigation(
            &mut keys,
            KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE),
            600.0,
            &bindings
        ),
        Some(super::Navigation::Back)
    );
    keys = KeySequence::parse("g").unwrap();
    assert_eq!(
        super::effective_navigation(
            &mut keys,
            KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE),
            600.0,
            &bindings
        ),
        None
    );
    assert_eq!(
        bindings.fallback_action(runyte::input::KeyStroke::plain(
            runyte::input::KeyCode::Function(8)
        )),
        Some(runyte::keymap::native_preview::Action::Back)
    );
}

#[test]
fn authored_keep_open_hands_insert_and_operand_input_to_current_host_owner() {
    use super::{Navigation, Target, effective_navigation, forwards_structural_input};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use runyte::{
        command::Mode,
        keymap::{BindingScope, KeySequence},
    };
    let section = serde_json::from_value(
        serde_json::json!({"ui": {"native-preview": {"keep-open-and-continue": ["i", "f"]}}}),
    )
    .unwrap();
    let compiled = runyte::keymap::configured::compile(&section, runyte::keymap::default_keymap());
    assert!(compiled.errors.is_empty());
    let map = compiled
        .keymap
        .native_preview(Mode::Normal, BindingScope::Global);
    let target = Some(Target {
        pane: 2,
        source: 7,
        generation: 11,
    });
    for starter in ['i', 'f'] {
        let mut keys = KeySequence::default();
        assert_eq!(
            effective_navigation(
                &mut keys,
                KeyEvent::new(KeyCode::Char(starter), KeyModifiers::NONE),
                600.0,
                &map
            ),
            Some(Navigation::ForwardInput)
        );
        let mut forwarded = target;
        // The queue waits for the host acknowledgement; its current frame says
        // Insert/operand, even though the captured map still contains j/k.
        for _ in ['j', 'k', 'x'] {
            assert!(forwards_structural_input(&mut forwarded, target, true));
        }
        assert!(!forwards_structural_input(&mut forwarded, target, false));
        assert!(forwarded.is_none());
        assert!(!forwards_structural_input(&mut forwarded, target, true));
    }
}

#[test]
fn preview_queue_combines_captured_maps_prefixes_handoff_and_host_acknowledgements() {
    use super::{InputOwnership, InputRoute, Navigation, Target};
    use crate::{NativeInput, input_routing::Queue};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use runyte::{
        input::{KeyCode as CoreCode, KeyStroke},
        keymap::{
            KeySequence,
            native_preview::{Action, Bindings},
        },
    };
    let target = Some(Target {
        pane: 2,
        source: 7,
        generation: 11,
    });
    let source = Some((2, 7));
    let map = |top| {
        Bindings::from_entries(
            vec![
                (KeySequence::parse("g g").unwrap(), top),
                (KeySequence::parse("i").unwrap(), Action::ForwardInput),
                (KeySequence::parse("j").unwrap(), Action::Down),
            ],
            vec![(KeyStroke::plain(CoreCode::Escape), Action::Back)],
        )
        .unwrap()
    };
    let a = map(Action::Top);
    let b = map(Action::Bottom);
    let input = |code, map: &Bindings| NativeInput {
        attachment: 1,
        event: Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        presented: Some(runyte::protocol::FrameId::from_raw(42).into()),
        presentation_only: false,
        pointer_target: None,
        pointer_bindings: Default::default(),
        window_bindings: Default::default(),
        preview_bindings: map.clone(),
        preview_target: target,
        routing_serial: 0,
    };
    let navigation = |state: &mut State, input: &NativeInput| {
        let Event::Key(key) = input.event else {
            panic!("key fixture")
        };
        state.navigation(key, 600.0, &input.preview_bindings)
    };
    let mut queue: Queue = Queue::default();
    queue.push(input(KeyCode::Char('g'), &a), ());
    queue.push(input(KeyCode::Char('g'), &a), ());
    let mut state = State::default();
    // Current map B must not reinterpret keys already captured from map A.
    assert_ne!(a, b);
    assert_eq!(
        navigation(&mut state, &queue.next(1, 0, true).unwrap().0),
        Some(Navigation::Prefix)
    );
    assert_eq!(
        navigation(&mut state, &queue.next(1, 0, true).unwrap().0),
        Some(Navigation::Top)
    );
    assert_eq!(
        navigation(&mut state, &input(KeyCode::Char('g'), &a)),
        Some(Navigation::Prefix)
    );
    assert_eq!(
        navigation(&mut state, &input(KeyCode::Char('g'), &b)),
        Some(Navigation::Prefix),
        "map change cancels the old prefix"
    );
    assert_eq!(
        navigation(&mut state, &input(KeyCode::Char('g'), &b)),
        Some(Navigation::Bottom)
    );

    let mut ownership = InputOwnership::default();
    queue.push(input(KeyCode::Char('i'), &a), ());
    queue.push(input(KeyCode::Char('j'), &a), ());
    let mut start = queue.next(1, 0, true).unwrap().0;
    assert_eq!(
        ownership.route(start.preview_target, target, source, true, false),
        InputRoute::Local
    );
    assert_eq!(
        navigation(&mut state, &start),
        Some(Navigation::ForwardInput)
    );
    ownership.forward(start.preview_target, true, false);
    queue.forward(&mut start);
    assert!(queue.next(1, 0, true).is_none());
    let text = queue.next(1, start.routing_serial, true).unwrap().0;
    assert_eq!(
        ownership.route(text.preview_target, target, source, true, true),
        InputRoute::Host,
        "host Insert receives j instead of preview scroll"
    );
    assert_eq!(
        ownership.route(text.preview_target, target, source, true, false),
        InputRoute::Local
    );

    queue.push(input(KeyCode::Esc, &a), ());
    queue.push(input(KeyCode::Char('i'), &a), ());
    queue.push(input(KeyCode::Char('x'), &a), ());
    let back = queue.next(1, start.routing_serial, true).unwrap().0;
    assert_eq!(navigation(&mut state, &back), None);
    assert_eq!(
        back.preview_bindings
            .fallback_action(KeyStroke::plain(CoreCode::Escape)),
        Some(Action::Back)
    );
    ownership.back(back.preview_target);
    let mut insert = queue.next(1, start.routing_serial, true).unwrap().0;
    assert_eq!(
        ownership.route(insert.preview_target, target, source, true, false),
        InputRoute::Host
    );
    queue.forward(&mut insert);
    assert!(queue.next(1, start.routing_serial, true).is_none());
    let text = queue.next(1, insert.routing_serial, true).unwrap().0;
    assert_eq!(
        ownership.route(text.preview_target, None, source, true, true),
        InputRoute::Host
    );
    assert_eq!(
        ownership.route(text.preview_target, None, Some((2, 8)), true, true),
        InputRoute::Discard
    );
    queue.push(text, ());
    assert!(
        queue.next(2, 0, true).is_none(),
        "replacement attachment discards old captured input"
    );
}
