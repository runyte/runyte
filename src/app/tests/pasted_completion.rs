// SPDX-License-Identifier: MPL-2.0

use super::language::{drain, ready_language_with_capabilities, rust_app};
use super::*;

#[test]
fn pasted_text_requests_language_help_only_at_the_final_caret() {
    for (suffix, expected) in [(".", "completion"), ("(", "signature help")] {
        let (mut app, _, mut queue) = rust_app("");
        ready_language_with_capabilities(
            &mut app,
            "rust",
            Encoding::Utf8,
            Capabilities {
                completion_triggers: vec!['.'],
                signature_triggers: vec!['('],
                signature_retriggers: Vec::new(),
                ..Capabilities::everything_for_test()
            },
        );
        press(&mut app, 'i');
        drain(&mut queue);
        let inserted = format!("{}{suffix}", ".(".repeat(20));
        app.handle_input(InputEvent::Text(inserted.clone()))
            .unwrap();
        assert_eq!(text(&app), inserted);
        let requests = drain(&mut queue)
            .into_iter()
            .filter_map(|command| match command {
                LspCommand::Request { kind, .. } => Some(kind.label()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(requests, [expected]);
    }
}

#[test]
fn pasted_text_refreshes_an_explicit_completion_once_and_keeps_its_filter() {
    let (mut app, _, mut queue) = rust_app("a");
    ready_language_with_capabilities(
        &mut app,
        "rust",
        Encoding::Utf8,
        Capabilities {
            completion_triggers: vec!['.'],
            ..Capabilities::everything_for_test()
        },
    );
    press(&mut app, 'a');
    app.start_explicit_lsp_completion();
    drain(&mut queue);
    app.handle_input(InputEvent::Text("bc".to_owned())).unwrap();
    assert_eq!(app.completion.as_ref().unwrap().filter, "abc");
    assert!(
        drain(&mut queue)
            .into_iter()
            .all(|command| !matches!(command, LspCommand::Request { .. }))
    );
    app.handle_input(InputEvent::Text(".".repeat(20))).unwrap();
    let requests = drain(&mut queue)
        .into_iter()
        .filter(|command| matches!(command, LspCommand::Request { .. }))
        .count();
    assert_eq!(requests, 1);
    assert!(app.explicit_completion_session().is_some());
    assert_eq!(app.completion.as_ref().unwrap().filter, "");
    app.handle_input(InputEvent::Text("different token".to_owned()))
        .unwrap();
    assert!(
        app.explicit_completion_session().is_none(),
        "whitespace ends the pinned session"
    );
}

#[test]
fn pasted_text_filters_existing_automatic_completion_with_the_whole_insertion() {
    for source in [CompletionSource::Language, CompletionSource::Word] {
        let mut app = App::new(Config::default(), None).unwrap();
        seed(&mut app, "a");
        press(&mut app, 'a');
        app.completion = Some(CompletionState {
            items: vec![Completion {
                label: "alphabet".to_owned(),
                filter_text: None,
                sort_text: None,
                detail: String::new(),
                kind: "text",
                insert: "alphabet".to_owned(),
                edit: None,
                additional: Vec::new(),
            }],
            selected: 0,
            buffer: 0,
            anchor: 0,
            filter: "a".to_owned(),
            source,
            explicit_session: None,
        });
        app.handle_input(InputEvent::Text("lph".to_owned()))
            .unwrap();
        assert_eq!(app.completion.as_ref().unwrap().filter, "alph");
        app.signature = Some(SignatureState {
            signatures: Vec::new(),
        });
        app.handle_input(InputEvent::Text(")different token".to_owned()))
            .unwrap();
        assert!(app.completion.is_none());
        assert!(
            app.signature.is_none(),
            "a pasted closing delimiter ends stale help"
        );
    }
}
