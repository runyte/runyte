// SPDX-License-Identifier: MPL-2.0

//! Opt-in compatibility tests against real language servers.
//!
//! The hermetic protocol and lifecycle coverage lives in `lsp_client.rs`.
//! These tests cover the remaining boundary: whether widely used servers for
//! Runyte's built-in languages accept its handshake, document identity, and
//! requests. They are ignored by default because the server executables are
//! external toolchain dependencies.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use runyte::{
    config::{LanguageServerConfig, LspConfig},
    lsp::{
        Capabilities, Diagnostic, Encoding, LspCommand, LspEvent, LspPosition, LspRange,
        RequestKind, Response, SignatureContext, TextDocumentContentChangeEvent,
    },
    text::Text,
};
use tokio::sync::mpsc;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

const READY_TIMEOUT: Duration = Duration::from_secs(120);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

struct TempProject(PathBuf);

impl TempProject {
    fn new(language: &str, files: &[(&str, &str)]) -> Self {
        let unique = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the system clock should follow the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-lsp-{language}-{}-{started}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("the real-server fixture directory should be created");
        for (relative, contents) in files {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, contents).unwrap();
        }
        Self(root)
    }

    fn root(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct ServerSpec {
    language: &'static str,
    opt_in: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    files: &'static [(&'static str, &'static str)],
    document: &'static str,
    symbol: &'static str,
    declaration: &'static str,
    use_site: &'static str,
    /// Where to ask for signature help, or `None` for a language whose server
    /// is expected not to offer it at all.
    signature: Option<SignatureProbe>,
    extended: ExtendedProbe,
}

/// The intended real-server coverage. `Unsupported` means that the pinned
/// server does not advertise (or meaningfully implement) the feature; it is
/// deliberately different from forgetting to add a probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Coverage {
    Tested,
    /// Advertised by the server, but the matrix deliberately has no stable,
    /// meaningful assertion for it.
    AdvertisedOnly,
    Unsupported,
}

#[derive(Clone, Copy)]
struct FeatureMatrix {
    completion: Coverage,
    hover: Coverage,
    signature_help: Coverage,
    references: Coverage,
    rename: Coverage,
    formatting: Coverage,
    code_actions: Coverage,
    diagnostics: Coverage,
    cross_file: Coverage,
}

#[derive(Clone, Copy)]
struct DiagnosticProbe {
    /// Invalid text appended in a separate incremental change.
    text: &'static str,
    /// Text on the line the server must diagnose.
    anchor: &'static str,
    /// Valid replacement text sent for the same range.
    repair: &'static str,
    /// A title fragment expected from a quick-fix action, when code actions
    /// are part of this server's matrix.
    action: Option<&'static str>,
}

#[derive(Clone, Copy)]
struct ExtendedProbe {
    /// Valid, unsaved text appended with `didChange`. Non-ASCII text precedes
    /// every request anchor so UTF-8/UTF-16 position negotiation is exercised.
    text: &'static str,
    declaration: &'static str,
    use_site: &'static str,
    completion_at: &'static str,
    completion: &'static str,
    hover_at: &'static str,
    rename_to: &'static str,
    diagnostic: Option<DiagnosticProbe>,
    matrix: FeatureMatrix,
}

/// Where to ask a server for signature help, and what its answer must name.
///
/// The anchors are matched from the front of the fixture and the caret sits at
/// the end of the match, so each one has to be long enough to be unique — a
/// bare `pair(` would find the declaration rather than the call.
#[derive(Clone, Copy)]
struct SignatureProbe {
    /// Text whose end is the caret just inside an opening call.
    call_site: &'static str,
    /// Text whose end is the caret just after a nested call's `)`, where the
    /// enclosing signature is what asking again on a closing delimiter is
    /// meant to recover.
    after_nested_call: &'static str,
    /// Whether this server actually answers there. Servers disagree, and
    /// advertising `)` is not the same as answering after one: clangd returns
    /// the enclosing signature, while Pyright lists `)` among its trigger
    /// characters and still answers nothing, which closes the popup instead.
    answers_after_nested_call: bool,
    /// A substring every acceptable signature label contains. Parameter names
    /// are used rather than the function's, because servers disagree about
    /// whether the label carries the name at all.
    expected: &'static str,
}

async fn smoke(spec: ServerSpec) {
    if !enabled(spec.opt_in) {
        return;
    }

    let project = TempProject::new(spec.language, spec.files);
    let path = project.root().join(spec.document);
    let source = fs::read_to_string(&path).unwrap();
    let mut servers = HashMap::new();
    servers.insert(
        spec.language.to_owned(),
        LanguageServerConfig {
            command: PathBuf::from(spec.command),
            args: spec
                .args
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect(),
            initialization_options: None,
        },
    );
    let config = LspConfig {
        enable: true,
        servers,
    };
    let (handle, mut events) = runyte::lsp::spawn(config, project.root().to_owned());

    assert!(handle.send(LspCommand::Ensure {
        language: spec.language.to_owned(),
    }));
    let (encoding, capabilities) = wait_until_ready(spec.language, &mut events).await;
    assert!(handle.send(LspCommand::Open {
        language: spec.language.to_owned(),
        path: path.clone(),
        version: 1,
        text: source.clone(),
    }));

    let symbols = request_until(
        &handle,
        &mut events,
        &spec,
        &path,
        1,
        || RequestKind::DocumentSymbols,
        |response| match response {
            Response::Symbols(symbols)
                if symbols.iter().any(|symbol| {
                    symbol
                        .name
                        .to_lowercase()
                        .contains(&spec.symbol.to_lowercase())
                }) =>
            {
                Some(symbols)
            }
            Response::Symbols(_) | Response::Empty => None,
            other => panic!(
                "{} returned an unexpected symbol response: {other:?}",
                spec.command
            ),
        },
    )
    .await;
    assert!(
        symbols.iter().any(|symbol| {
            symbol.location.path.as_os_str().is_empty() || symbol.location.path == path
        }),
        "{} returned the expected symbol only for another file",
        spec.command
    );

    let document = Text::from_str(&source);
    let use_offset = source
        .rfind(spec.use_site)
        .unwrap_or_else(|| panic!("fixture should contain use site {:?}", spec.use_site));
    let use_offset = source[..use_offset].chars().count() + 1;
    let position = runyte::lsp::to_lsp_position(&document, use_offset, encoding);
    let locations = request_until(
        &handle,
        &mut events,
        &spec,
        &path,
        10_000,
        || RequestKind::Definition(position),
        |response| match response {
            Response::Locations(locations) if !locations.is_empty() => Some(locations),
            Response::Locations(_) | Response::Empty => None,
            Response::Failed(message) if message.contains("content modified") => None,
            other => panic!(
                "{} returned an unexpected definition response: {other:?}",
                spec.command
            ),
        },
    )
    .await;

    assert!(
        locations.iter().any(|location| {
            let declaration_source =
                fs::read_to_string(&location.path).unwrap_or_else(|_| source.clone());
            let declaration_byte = declaration_source
                .find(spec.declaration)
                .unwrap_or_else(|| {
                    panic!("fixture should contain declaration {:?}", spec.declaration)
                });
            let declaration = declaration_source[..declaration_byte].chars().count();
            let declaration_document = Text::from_str(&declaration_source);
            let (from, to) =
                runyte::lsp::from_lsp_range(&declaration_document, location.range, encoding);
            from <= declaration && declaration < to.max(from + 1)
        }),
        "{} did not resolve {:?} to its declaration in {}: {locations:?}",
        spec.command,
        spec.use_site,
        path.display()
    );

    if spec.extended.matrix.cross_file == Coverage::Tested {
        assert!(
            locations.iter().any(|location| location.path != path),
            "{} was meant to cover project indexing, but definition stayed in {}",
            spec.command,
            path.display()
        );
    }

    exercise_extended(
        &handle,
        &mut events,
        &spec,
        &path,
        &source,
        encoding,
        &capabilities,
    )
    .await;

    // Signature help is the request the unsupported-request report was written
    // about: it is sent as a side effect of typing rather than on a key the
    // person chose, so a server that does not offer it used to produce an
    // error per call and per argument. What the server advertised now decides
    // both whether it is asked at all and on which characters.
    let advertises_signature_help = capabilities.supports(&RequestKind::SignatureHelp {
        position,
        context: SignatureContext::default(),
    });
    println!(
        "{}: signature help {}, asked on {}",
        spec.command,
        if advertises_signature_help {
            "advertised"
        } else {
            "not advertised"
        },
        SIGNATURE_CHARACTERS
            .iter()
            .filter(|character| capabilities.triggers_signature_help(**character, true))
            .map(|character| format!("`{character}`"))
            .collect::<Vec<_>>()
            .join(" ")
    );

    let Some(probe) = spec.signature else {
        assert!(
            !advertises_signature_help,
            "{} advertises signature help, so its fixture should probe it",
            spec.command
        );
        assert!(handle.send(LspCommand::Shutdown));
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            while events.recv().await.is_some() {}
        })
        .await;
        return;
    };
    assert!(
        advertises_signature_help,
        "{} does not advertise signature help, so its fixture should not probe it",
        spec.command
    );
    // Whatever else a server names, an opening `(` has to reach it: either it
    // listed one or it listed nothing and took the editor's fallback.
    assert!(
        capabilities.triggers_signature_help('(', false),
        "{} advertises signature help but would never be asked for it",
        spec.command
    );

    let caret_after = |anchor: &str| {
        let byte = source
            .find(anchor)
            .unwrap_or_else(|| panic!("fixture should contain {anchor:?}"));
        let offset = source[..byte + anchor.len()].chars().count();
        runyte::lsp::to_lsp_position(&document, offset, encoding)
    };

    let opening = caret_after(probe.call_site);
    let signatures = request_until(
        &handle,
        &mut events,
        &spec,
        &path,
        20_000,
        || RequestKind::SignatureHelp {
            position: opening,
            context: SignatureContext {
                trigger: Some('('),
                retrigger: false,
            },
        },
        |response| match response {
            Response::Signatures(signatures) if !signatures.is_empty() => Some(signatures),
            Response::Signatures(_) | Response::Empty => None,
            Response::Failed(message) if message.contains("content modified") => None,
            other => panic!(
                "{} returned an unexpected signature response: {other:?}",
                spec.command
            ),
        },
    )
    .await;
    assert!(
        signatures
            .iter()
            .any(|signature| signature.label.contains(probe.expected)),
        "{} did not name {:?} for an opening call: {:?}",
        spec.command,
        probe.expected,
        labels(&signatures)
    );

    // Asked once rather than retried: the probe above has already waited for
    // this server to warm up, so nothing here is answered with silence for
    // want of an index. A server that answers nothing is recording a real
    // answer, not a slow one.
    let response = request_once(
        &handle,
        &mut events,
        &spec,
        &path,
        30_000,
        RequestKind::SignatureHelp {
            position: caret_after(probe.after_nested_call),
            context: SignatureContext {
                trigger: Some(')'),
                retrigger: true,
            },
        },
    )
    .await;
    match (&response, probe.answers_after_nested_call) {
        (Response::Signatures(signatures), true) => assert!(
            signatures
                .iter()
                .any(|signature| signature.label.contains(probe.expected)),
            "{} answered after a nested call without naming the enclosing {:?}: {:?}",
            spec.command,
            probe.expected,
            labels(signatures)
        ),
        (Response::Empty, false) => {}
        (Response::Signatures(signatures), false) => panic!(
            "{} now answers after a nested call ({:?}); the fixture says it does not",
            spec.command,
            labels(signatures)
        ),
        (Response::Empty, true) => panic!(
            "{} no longer answers after a nested call; the fixture says it does",
            spec.command
        ),
        (other, _) => panic!(
            "{} returned an unexpected signature response: {other:?}",
            spec.command
        ),
    }

    assert!(handle.send(LspCommand::Shutdown));
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while events.recv().await.is_some() {}
    })
    .await;
}

fn labels(signatures: &[runyte::lsp::SignatureLine]) -> Vec<&str> {
    signatures
        .iter()
        .map(|signature| signature.label.as_str())
        .collect()
}

async fn exercise_extended(
    handle: &runyte::lsp::LspHandle,
    events: &mut mpsc::Receiver<LspEvent>,
    spec: &ServerSpec,
    path: &Path,
    original: &str,
    encoding: Encoding,
    capabilities: &Capabilities,
) {
    let probe = spec.extended;
    print_matrix(spec, capabilities);

    let mut source = original.to_owned();
    let end =
        runyte::lsp::to_lsp_position(&Text::from_str(&source), source.chars().count(), encoding);
    assert!(handle.send(LspCommand::Change {
        language: spec.language.to_owned(),
        path: path.to_owned(),
        version: 2,
        changes: vec![TextDocumentContentChangeEvent {
            range: Some(LspRange::new(end, end)),
            range_length: None,
            text: probe.text.to_owned(),
        }],
    }));
    source.push_str(probe.text);
    let document = Text::from_str(&source);

    // This definition exists only in the unsaved didChange text. Its anchors
    // follow a non-ASCII comment, so producing the request and reading the
    // answer both pass through the negotiated position encoding.
    let use_position = position_in(&source, probe.use_site, 1, &document, encoding);
    let locations = request_until(
        handle,
        events,
        spec,
        path,
        40_000,
        || RequestKind::Definition(use_position),
        |response| match response {
            Response::Locations(locations) if !locations.is_empty() => Some(locations),
            Response::Locations(_) | Response::Empty => None,
            Response::Failed(message) if message.contains("content modified") => None,
            other => panic!(
                "{} returned an unexpected changed-definition response: {other:?}",
                spec.command
            ),
        },
    )
    .await;
    let declaration = first_char_offset(&source, probe.declaration);
    assert!(
        locations.iter().any(|location| {
            location.path == path && {
                let (from, to) = runyte::lsp::from_lsp_range(&document, location.range, encoding);
                from <= declaration && declaration < to.max(from + 1)
            }
        }),
        "{} did not resolve a symbol introduced by didChange: {locations:?}",
        spec.command
    );

    if probe.matrix.completion == Coverage::Tested {
        println!("{}: completion", spec.command);
        let position = position_after(&source, probe.completion_at, &document, encoding);
        let completions = request_until(
            handle,
            events,
            spec,
            path,
            41_000,
            || RequestKind::Completion(position),
            |response| match response {
                Response::Completions(items)
                    if items
                        .iter()
                        .any(|item| item.label.contains(probe.completion)) =>
                {
                    Some(items)
                }
                Response::Completions(_) | Response::Empty => None,
                other => panic!(
                    "{} returned an unexpected completion response: {other:?}",
                    spec.command
                ),
            },
        )
        .await;
        assert!(
            completions
                .iter()
                .any(|item| item.label.contains(probe.completion))
        );
    }

    if probe.matrix.hover == Coverage::Tested {
        println!("{}: hover", spec.command);
        let position = position_in(&source, probe.hover_at, 1, &document, encoding);
        let hover = request_until(
            handle,
            events,
            spec,
            path,
            42_000,
            || RequestKind::Hover(position),
            |response| match response {
                Response::Hover(text) if !text.trim().is_empty() => Some(text),
                Response::Hover(_) | Response::Empty => None,
                other => panic!(
                    "{} returned an unexpected hover response: {other:?}",
                    spec.command
                ),
            },
        )
        .await;
        assert!(!hover.trim().is_empty());
    }

    if probe.matrix.references == Coverage::Tested {
        println!("{}: references", spec.command);
        let references = request_until(
            handle,
            events,
            spec,
            path,
            43_000,
            || RequestKind::References(use_position),
            |response| match response {
                Response::Locations(locations) if !locations.is_empty() => Some(locations),
                Response::Locations(_) | Response::Empty => None,
                other => panic!(
                    "{} returned an unexpected references response: {other:?}",
                    spec.command
                ),
            },
        )
        .await;
        assert!(!references.is_empty());
    }

    if probe.matrix.rename == Coverage::Tested {
        println!("{}: rename", spec.command);
        let renamed = request_until(
            handle,
            events,
            spec,
            path,
            44_000,
            || RequestKind::Rename {
                position: use_position,
                new_name: probe.rename_to.to_owned(),
            },
            |response| match response {
                Response::Edits { edits, skipped, .. }
                    if edits.iter().any(|document| {
                        document
                            .edits
                            .iter()
                            .any(|edit| edit.new_text.contains(probe.rename_to))
                    }) =>
                {
                    Some((edits, skipped))
                }
                Response::Edits { .. } | Response::Empty => None,
                other => panic!(
                    "{} returned an unexpected rename response: {other:?}",
                    spec.command
                ),
            },
        )
        .await;
        assert_eq!(
            renamed.1, 0,
            "{} rename included unsupported file operations",
            spec.command
        );
    }

    if probe.matrix.formatting == Coverage::Tested {
        println!("{}: formatting", spec.command);
        let edits = request_until(
            handle,
            events,
            spec,
            path,
            45_000,
            || RequestKind::Format {
                tab_size: 4,
                insert_spaces: true,
            },
            |response| match response {
                Response::Edits { edits, .. }
                    if edits.iter().any(|document| !document.edits.is_empty()) =>
                {
                    Some(edits)
                }
                Response::Edits { .. } | Response::Empty => None,
                other => panic!(
                    "{} returned an unexpected formatting response: {other:?}",
                    spec.command
                ),
            },
        )
        .await;
        assert!(edits.iter().any(|document| !document.edits.is_empty()));
    }

    if let Some(diagnostic_probe) = probe.diagnostic {
        println!("{}: diagnostics", spec.command);
        let invalid_start = source.chars().count();
        let start = runyte::lsp::to_lsp_position(&document, invalid_start, encoding);
        assert!(handle.send(LspCommand::Change {
            language: spec.language.to_owned(),
            path: path.to_owned(),
            version: 3,
            changes: vec![TextDocumentContentChangeEvent {
                range: Some(LspRange::new(start, start)),
                range_length: None,
                text: diagnostic_probe.text.to_owned()
            }],
        }));
        source.push_str(diagnostic_probe.text);
        assert!(handle.send(LspCommand::Save {
            language: spec.language.to_owned(),
            path: path.to_owned(),
            text: source.clone(),
        }));
        let invalid_document = Text::from_str(&source);
        let expected = range_of(
            &source,
            diagnostic_probe.anchor,
            &invalid_document,
            encoding,
        );
        let diagnostics = wait_for_diagnostics(events, spec, path, |diagnostics| {
            diagnostics
                .iter()
                .any(|diagnostic| ranges_overlap(diagnostic.range, expected))
        })
        .await;
        let matching: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| ranges_overlap(diagnostic.range, expected))
            .collect();
        assert!(
            !matching.is_empty(),
            "{} did not diagnose the expected changed range {expected:?}: {diagnostics:?}",
            spec.command
        );

        if probe.matrix.code_actions == Coverage::Tested {
            println!("{}: code actions", spec.command);
            let raws = matching
                .iter()
                .map(|diagnostic| diagnostic.raw.clone())
                .collect::<Vec<_>>();
            let actions = request_until(
                handle,
                events,
                spec,
                path,
                46_000,
                || RequestKind::CodeActions {
                    range: expected,
                    diagnostics: raws.clone(),
                },
                |response| match response {
                    Response::Actions(actions)
                        if actions.iter().any(|action| {
                            diagnostic_probe.action.is_none_or(|fragment| {
                                action
                                    .title
                                    .to_lowercase()
                                    .contains(&fragment.to_lowercase())
                            })
                        }) =>
                    {
                        Some(actions)
                    }
                    Response::Actions(_) | Response::Empty => None,
                    other => panic!(
                        "{} returned an unexpected code-action response: {other:?}",
                        spec.command
                    ),
                },
            )
            .await;
            assert!(!actions.is_empty());
        }

        let invalid_end = source.chars().count();
        let removal = LspRange::new(
            runyte::lsp::to_lsp_position(&invalid_document, invalid_start, encoding),
            runyte::lsp::to_lsp_position(&invalid_document, invalid_end, encoding),
        );
        assert!(handle.send(LspCommand::Change {
            language: spec.language.to_owned(),
            path: path.to_owned(),
            version: 4,
            changes: vec![TextDocumentContentChangeEvent {
                range: Some(removal),
                range_length: None,
                text: diagnostic_probe.repair.to_owned()
            }],
        }));
        source.truncate(source.len() - diagnostic_probe.text.len());
        source.push_str(diagnostic_probe.repair);
        assert!(handle.send(LspCommand::Save {
            language: spec.language.to_owned(),
            path: path.to_owned(),
            text: source.clone(),
        }));
        let repaired_document = Text::from_str(&source);
        let repaired_end = runyte::lsp::to_lsp_position(
            &repaired_document,
            repaired_document.len_chars(),
            encoding,
        );
        assert!(handle.send(LspCommand::Change {
            language: spec.language.to_owned(),
            path: path.to_owned(),
            version: 5,
            changes: vec![TextDocumentContentChangeEvent {
                range: Some(LspRange::new(repaired_end, repaired_end)),
                range_length: None,
                text: "\n".to_owned()
            }],
        }));
        println!("{}: diagnostic clear", spec.command);
        let _ = wait_for_diagnostics(events, spec, path, |diagnostics| {
            diagnostics
                .iter()
                .all(|diagnostic| !ranges_overlap(diagnostic.range, expected))
        })
        .await;
    }
}

fn char_offset(source: &str, anchor: &str) -> usize {
    let byte = source
        .rfind(anchor)
        .unwrap_or_else(|| panic!("fixture should contain {anchor:?}"));
    source[..byte].chars().count()
}

fn first_char_offset(source: &str, anchor: &str) -> usize {
    let byte = source
        .find(anchor)
        .unwrap_or_else(|| panic!("fixture should contain {anchor:?}"));
    source[..byte].chars().count()
}

fn position_in(
    source: &str,
    anchor: &str,
    inside: usize,
    document: &Text,
    encoding: Encoding,
) -> LspPosition {
    runyte::lsp::to_lsp_position(document, char_offset(source, anchor) + inside, encoding)
}

fn position_after(source: &str, anchor: &str, document: &Text, encoding: Encoding) -> LspPosition {
    position_in(source, anchor, anchor.chars().count(), document, encoding)
}

fn range_of(source: &str, anchor: &str, document: &Text, encoding: Encoding) -> LspRange {
    let start = char_offset(source, anchor);
    LspRange::new(
        runyte::lsp::to_lsp_position(document, start, encoding),
        runyte::lsp::to_lsp_position(document, start + anchor.chars().count(), encoding),
    )
}

fn ranges_overlap(left: LspRange, right: LspRange) -> bool {
    if left.start == left.end {
        return right.start <= left.start && left.start <= right.end;
    }
    if right.start == right.end {
        return left.start <= right.start && right.start <= left.end;
    }
    left.start < right.end && right.start < left.end
}

async fn wait_for_diagnostics(
    events: &mut mpsc::Receiver<LspEvent>,
    spec: &ServerSpec,
    path: &Path,
    accept: impl Fn(&Vec<Diagnostic>) -> bool,
) -> Vec<Diagnostic> {
    tokio::time::timeout(REQUEST_TIMEOUT, async {
        loop {
            match events.recv().await {
                Some(LspEvent::Diagnostics {
                    path: reported,
                    diagnostics,
                    ..
                }) if reported == path => {
                    println!(
                        "{}: published {} diagnostics",
                        spec.command,
                        diagnostics.len()
                    );
                    if accept(&diagnostics) {
                        return diagnostics;
                    }
                }
                Some(LspEvent::Stopped { message, .. }) => panic!("{message}"),
                Some(_) => {}
                None => panic!("the LSP manager stopped while waiting for diagnostics"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "{} did not publish expected diagnostics within {REQUEST_TIMEOUT:?}",
            spec.command
        )
    })
}

fn print_matrix(spec: &ServerSpec, capabilities: &Capabilities) {
    let matrix = spec.extended.matrix;
    let position = LspPosition::new(0, 0);
    let features = [
        (
            "completion",
            matrix.completion,
            RequestKind::Completion(position),
        ),
        ("hover", matrix.hover, RequestKind::Hover(position)),
        (
            "references",
            matrix.references,
            RequestKind::References(position),
        ),
        (
            "rename",
            matrix.rename,
            RequestKind::Rename {
                position,
                new_name: "probe".to_owned(),
            },
        ),
        (
            "formatting",
            matrix.formatting,
            RequestKind::Format {
                tab_size: 4,
                insert_spaces: true,
            },
        ),
        (
            "code actions",
            matrix.code_actions,
            RequestKind::CodeActions {
                range: LspRange::new(position, position),
                diagnostics: vec![],
            },
        ),
    ];
    for (name, intended, request) in features {
        let advertised = capabilities.supports(&request);
        match intended {
            Coverage::Tested | Coverage::AdvertisedOnly => assert!(
                advertised,
                "{} capability matrix is stale for {name}: intended {intended:?}, advertised=false",
                spec.command
            ),
            Coverage::Unsupported => assert!(
                !advertised,
                "{} capability matrix is stale for {name}: intended Unsupported, advertised=true",
                spec.command
            ),
        }
    }
    assert_eq!(
        matrix.signature_help == Coverage::Tested,
        spec.signature.is_some()
    );
    assert_eq!(
        matrix.diagnostics == Coverage::Tested,
        spec.extended.diagnostic.is_some()
    );
    println!(
        "{} coverage: completion={:?} hover={:?} signature={:?} references={:?} rename={:?} formatting={:?} code-actions={:?} diagnostics={:?} cross-file={:?}",
        spec.command,
        matrix.completion,
        matrix.hover,
        matrix.signature_help,
        matrix.references,
        matrix.rename,
        matrix.formatting,
        matrix.code_actions,
        matrix.diagnostics,
        matrix.cross_file
    );
}

/// Sends one request and returns whatever came back, without the retry loop
/// [`request_until`] uses to wait a server out.
async fn request_once(
    handle: &runyte::lsp::LspHandle,
    events: &mut mpsc::Receiver<LspEvent>,
    spec: &ServerSpec,
    path: &Path,
    token: u64,
    kind: RequestKind,
) -> Response {
    assert!(handle.send(LspCommand::Request {
        token,
        language: spec.language.to_owned(),
        path: path.to_owned(),
        kind: Box::new(kind),
    }));
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match events.recv().await {
                Some(LspEvent::Response {
                    token: answered,
                    response,
                }) if answered == token => return response,
                Some(LspEvent::Stopped { message, .. }) => panic!("{message}"),
                Some(_) => {}
                None => panic!("the LSP manager stopped while waiting for a response"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{} stopped answering requests", spec.command))
}

/// The delimiters worth reporting a server's signature-help answer for. Only
/// used to print the compatibility record; nothing is asserted about which of
/// them a given server names.
const SIGNATURE_CHARACTERS: [char; 7] = ['(', ')', ',', '<', '>', '{', '}'];

fn enabled(flag: &str) -> bool {
    std::env::var(flag).ok().as_deref() == Some("1")
        || std::env::var("RUNYTE_SMOKE_LSP_ALL").ok().as_deref() == Some("1")
}

async fn wait_until_ready(
    language: &str,
    events: &mut mpsc::Receiver<LspEvent>,
) -> (Encoding, Capabilities) {
    tokio::time::timeout(READY_TIMEOUT, async {
        loop {
            match events.recv().await {
                Some(LspEvent::Ready {
                    language: ready,
                    encoding,
                    capabilities,
                    ..
                }) if ready == language => return (encoding, capabilities),
                Some(LspEvent::Stopped { message, .. }) => panic!("{message}"),
                Some(_) => {}
                None => panic!("the LSP manager stopped before {language} became ready"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{language} did not finish its handshake in {READY_TIMEOUT:?}"))
}

#[allow(clippy::too_many_arguments)]
async fn request_until<T>(
    handle: &runyte::lsp::LspHandle,
    events: &mut mpsc::Receiver<LspEvent>,
    spec: &ServerSpec,
    path: &Path,
    first_token: u64,
    request: impl Fn() -> RequestKind,
    mut accept: impl FnMut(Response) -> Option<T>,
) -> T {
    let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
    let mut token = first_token;
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "{} did not produce a useful response within {REQUEST_TIMEOUT:?}",
            spec.command
        );
        assert!(handle.send(LspCommand::Request {
            token,
            language: spec.language.to_owned(),
            path: path.to_owned(),
            kind: Box::new(request()),
        }));
        let response = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match events.recv().await {
                    Some(LspEvent::Response {
                        token: response_token,
                        response,
                    }) if response_token == token => return response,
                    Some(LspEvent::Stopped { message, .. }) => panic!("{message}"),
                    Some(_) => {}
                    None => panic!("the LSP manager stopped while waiting for a response"),
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{} stopped answering requests", spec.command));
        if let Some(value) = accept(response) {
            return value;
        }
        token += 1;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

macro_rules! real_server_test {
    ($name:ident, $spec:expr, $reason:literal) => {
        #[tokio::test]
        #[ignore = $reason]
        async fn $name() {
            smoke($spec).await;
        }
    };
}

real_server_test!(
    python_pyright,
    ServerSpec {
        language: "python",
        opt_in: "RUNYTE_SMOKE_LSP_PYTHON",
        command: "pyright-langserver",
        args: &["--stdio"],
        files: &[(
            "main.py",
            "def target():\n    return 1\n\nresult = target()\n\n\ndef pair(alpha, beta):\n    return alpha + beta\n\n\ndef wrap(only):\n    return only\n\n\nsimple = pair(1, 2)\nnested = pair(wrap(1), 2)\n",
        )],
        document: "main.py",
        symbol: "target",
        declaration: "target",
        use_site: "target",
        signature: Some(SignatureProbe {
            call_site: "simple = pair(",
            after_nested_call: "nested = pair(wrap(1)",
            answers_after_nested_call: false,
            expected: "alpha",
        }),
        extended: ExtendedProbe {
            text: "\n# café 😀: unsaved Unicode precedes every probe\ndef live_target(value: int) -> int:\n    return value\n\nlive_result = live_target(7)\n",
            declaration: "live_target",
            use_site: "live_target",
            completion_at: "live_",
            completion: "live_target",
            hover_at: "live_target",
            rename_to: "renamed_target",
            diagnostic: Some(DiagnosticProbe {
                text: "\nbroken: int = \"text\"\n",
                anchor: "\"text\"",
                repair: "\nbroken: int = 1\n",
                action: None
            }),
            matrix: FeatureMatrix {
                completion: Coverage::Tested,
                hover: Coverage::Tested,
                signature_help: Coverage::Tested,
                references: Coverage::Tested,
                rename: Coverage::Tested,
                formatting: Coverage::Unsupported,
                code_actions: Coverage::AdvertisedOnly,
                diagnostics: Coverage::Tested,
                cross_file: Coverage::Unsupported
            },
        },
    },
    "requires RUNYTE_SMOKE_LSP_PYTHON=1 and pyright-langserver on PATH"
);

real_server_test!(
    swift_sourcekit_lsp,
    ServerSpec {
        language: "swift",
        opt_in: "RUNYTE_SMOKE_LSP_SWIFT",
        command: "sourcekit-lsp",
        args: &[],
        files: &[(
            "main.swift",
            "func target() -> Int { 1 }\nlet result = target()\n",
        )],
        document: "main.swift",
        symbol: "target",
        declaration: "target",
        use_site: "target",
        // sourcekit-lsp advertises no signature help, so Runyte never asks it
        // for one. Before the capability gate that was an error per `(` and
        // per `,` typed in a Swift file.
        signature: None,
        extended: ExtendedProbe {
            text: "\n// café 😀: unsaved Unicode precedes every probe\nfunc live_target(_ value: Int) -> Int { value }\nlet live_result = live_target(7)\n",
            declaration: "live_target",
            use_site: "live_target",
            completion_at: "live_",
            completion: "live_target",
            hover_at: "live_target",
            rename_to: "renamed_target",
            diagnostic: Some(DiagnosticProbe {
                text: "\nlet broken: Int = \"text\"\n",
                anchor: "\"text\"",
                repair: "\nlet broken: Int = 1\n",
                action: None
            }),
            matrix: FeatureMatrix {
                completion: Coverage::Tested,
                hover: Coverage::Tested,
                signature_help: Coverage::Unsupported,
                references: Coverage::AdvertisedOnly,
                rename: Coverage::Tested,
                formatting: Coverage::AdvertisedOnly,
                code_actions: Coverage::AdvertisedOnly,
                diagnostics: Coverage::Tested,
                cross_file: Coverage::Unsupported
            },
        },
    },
    "requires RUNYTE_SMOKE_LSP_SWIFT=1 and sourcekit-lsp on PATH"
);

real_server_test!(
    c_clangd,
    ServerSpec {
        language: "c",
        opt_in: "RUNYTE_SMOKE_LSP_C",
        command: "clangd",
        args: &[],
        files: &[(
            "main.c",
            "int target(void) { return 1; }\nint main(void) { return target(); }\nint pair(int alpha, int beta) { return alpha + beta; }\nint wrap(int only) { return only; }\nint simple(void) { return pair(1, 2); }\nint nested(void) { return pair(wrap(1), 2); }\n",
        )],
        document: "main.c",
        symbol: "target",
        declaration: "target",
        use_site: "target",
        signature: Some(SignatureProbe {
            call_site: "simple(void) { return pair(",
            after_nested_call: "nested(void) { return pair(wrap(1)",
            answers_after_nested_call: true,
            expected: "alpha",
        }),
        extended: ExtendedProbe {
            text: "\n// café 😀: unsaved Unicode precedes every probe\nint live_target(int value){return value;}\nint live_result(void){return live_target(7);}\n",
            declaration: "live_target",
            use_site: "live_target",
            completion_at: "live_",
            completion: "live_target",
            hover_at: "live_target",
            rename_to: "renamed_target",
            diagnostic: Some(DiagnosticProbe {
                text: "\nint broken = live_targte(1);\n",
                anchor: "live_targte",
                repair: "\nint broken = 1;\n",
                action: Some("live_target")
            }),
            matrix: FeatureMatrix {
                completion: Coverage::Tested,
                hover: Coverage::Tested,
                signature_help: Coverage::Tested,
                references: Coverage::Tested,
                rename: Coverage::Tested,
                formatting: Coverage::Tested,
                code_actions: Coverage::Tested,
                diagnostics: Coverage::Tested,
                cross_file: Coverage::Unsupported
            },
        },
    },
    "requires RUNYTE_SMOKE_LSP_C=1 and clangd on PATH"
);

real_server_test!(
    cpp_clangd,
    ServerSpec {
        language: "cpp",
        opt_in: "RUNYTE_SMOKE_LSP_CPP",
        command: "clangd",
        args: &[],
        files: &[(
            "main.cpp",
            "int target() { return 1; }\nint main() { return target(); }\nint pair(int alpha, int beta) { return alpha + beta; }\nint wrap(int only) { return only; }\nint simple() { return pair(1, 2); }\nint nested() { return pair(wrap(1), 2); }\n",
        )],
        document: "main.cpp",
        symbol: "target",
        declaration: "target",
        use_site: "target",
        signature: Some(SignatureProbe {
            call_site: "simple() { return pair(",
            after_nested_call: "nested() { return pair(wrap(1)",
            answers_after_nested_call: true,
            expected: "alpha",
        }),
        extended: ExtendedProbe {
            text: "\n// café 😀: unsaved Unicode precedes every probe\nint live_target(int value){return value;}\nint live_result(){return live_target(7);}\n",
            declaration: "live_target",
            use_site: "live_target",
            completion_at: "live_",
            completion: "live_target",
            hover_at: "live_target",
            rename_to: "renamed_target",
            diagnostic: Some(DiagnosticProbe {
                text: "\nint broken = live_targte(1);\n",
                anchor: "live_targte",
                repair: "\nint broken = 1;\n",
                action: Some("live_target")
            }),
            matrix: FeatureMatrix {
                completion: Coverage::Tested,
                hover: Coverage::Tested,
                signature_help: Coverage::Tested,
                references: Coverage::Tested,
                rename: Coverage::Tested,
                formatting: Coverage::Tested,
                code_actions: Coverage::Tested,
                diagnostics: Coverage::Tested,
                cross_file: Coverage::Unsupported
            },
        },
    },
    "requires RUNYTE_SMOKE_LSP_CPP=1 and clangd on PATH"
);

real_server_test!(
    javascript_typescript_language_server,
    ServerSpec {
        language: "javascript",
        opt_in: "RUNYTE_SMOKE_LSP_JAVASCRIPT",
        command: "typescript-language-server",
        args: &["--stdio"],
        files: &[
            ("package.json", "{\"type\":\"module\"}\n"),
            ("lib.js", "export function target() { return 1; }\n"),
            (
                "main.js",
                "import * as lib from './lib.js';\nconst result = lib.target();\nfunction pair(alpha, beta) { return alpha + beta; }\nfunction wrap(only) { return only; }\nconst simple = pair(1, 2);\nconst nested = pair(wrap(1), 2);\n",
            ),
        ],
        document: "main.js",
        symbol: "pair",
        declaration: "target",
        use_site: "target",
        signature: Some(SignatureProbe {
            call_site: "const simple = pair(",
            after_nested_call: "const nested = pair(wrap(1)",
            answers_after_nested_call: true,
            expected: "alpha",
        }),
        extended: ExtendedProbe {
            text: "\n// café 😀: unsaved Unicode precedes every probe\nfunction live_target(value) {return value;}\nconst live_result = live_target(7);\n",
            declaration: "live_target",
            use_site: "live_target",
            completion_at: "live_",
            completion: "live_target",
            hover_at: "live_target",
            rename_to: "renamed_target",
            diagnostic: Some(DiagnosticProbe {
                text: "\nconst broken = ;\n",
                anchor: ";",
                repair: "\nconst broken = 1;\n",
                action: None
            }),
            matrix: FeatureMatrix {
                completion: Coverage::Tested,
                hover: Coverage::Tested,
                signature_help: Coverage::Tested,
                references: Coverage::Tested,
                rename: Coverage::Tested,
                formatting: Coverage::Tested,
                code_actions: Coverage::AdvertisedOnly,
                diagnostics: Coverage::Tested,
                cross_file: Coverage::Tested
            },
        },
    },
    "requires RUNYTE_SMOKE_LSP_JAVASCRIPT=1 and typescript-language-server on PATH"
);

real_server_test!(
    go_gopls,
    ServerSpec {
        language: "go",
        opt_in: "RUNYTE_SMOKE_LSP_GO",
        command: "gopls",
        args: &[],
        files: &[
            (
                "go.mod",
                "module example.com/runyte-lsp-fixture\n\ngo 1.22\n"
            ),
            (
                "main.go",
                "package fixture\n\nvar result = target()\n\nfunc pair(alpha int, beta int) int { return alpha + beta }\n\nfunc wrap(only int) int { return only }\n\nvar simple = pair(1, 2)\n\nvar nested = pair(wrap(1), 2)\n",
            ),
            (
                "target.go",
                "package fixture\n\nfunc target() int { return 1 }\n"
            ),
        ],
        document: "main.go",
        symbol: "pair",
        declaration: "target",
        use_site: "target",
        signature: Some(SignatureProbe {
            call_site: "var simple = pair(",
            after_nested_call: "var nested = pair(wrap(1)",
            answers_after_nested_call: true,
            expected: "alpha",
        }),
        extended: ExtendedProbe {
            text: "\n// café 😀: unsaved Unicode precedes every probe\nfunc live_target(value int) int {return value}\nvar live_result = live_target(7)\n",
            declaration: "live_target",
            use_site: "live_target",
            completion_at: "live_",
            completion: "live_target",
            hover_at: "live_target",
            rename_to: "renamed_target",
            diagnostic: Some(DiagnosticProbe {
                text: "\nvar broken int = \"text\"\n",
                anchor: "\"text\"",
                repair: "\nvar broken int = 1\n",
                action: None
            }),
            matrix: FeatureMatrix {
                completion: Coverage::Tested,
                hover: Coverage::Tested,
                signature_help: Coverage::Tested,
                references: Coverage::Tested,
                rename: Coverage::Tested,
                formatting: Coverage::Tested,
                code_actions: Coverage::AdvertisedOnly,
                diagnostics: Coverage::Tested,
                cross_file: Coverage::Tested
            },
        },
    },
    "requires RUNYTE_SMOKE_LSP_GO=1 and gopls on PATH"
);

real_server_test!(
    rust_rust_analyzer,
    ServerSpec {
        language: "rust",
        opt_in: "RUNYTE_SMOKE_LSP_RUST",
        command: "rust-analyzer",
        args: &[],
        files: &[
            (
                "Cargo.toml",
                "[package]\nname = \"runyte-lsp-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
            ),
            (
                "src/main.rs",
                "use runyte_lsp_fixture::target;\nfn main() { let _ = target(); }\nfn pair(alpha: i32, beta: i32) -> i32 { alpha + beta }\nfn wrap(only: i32) -> i32 { only }\nfn simple() -> i32 { pair(1, 2) }\nfn nested() -> i32 { pair(wrap(1), 2) }\n",
            ),
            ("src/lib.rs", "pub fn target() -> i32 { 1 }\n"),
        ],
        document: "src/main.rs",
        symbol: "main",
        declaration: "target",
        use_site: "target",
        signature: Some(SignatureProbe {
            call_site: "fn simple() -> i32 { pair(",
            after_nested_call: "fn nested() -> i32 { pair(wrap(1)",
            answers_after_nested_call: true,
            expected: "alpha",
        }),
        extended: ExtendedProbe {
            text: "\n// café 😀: unsaved Unicode precedes every probe\nfn live_target(value: i32) -> i32 {value}\nfn live_result() -> i32 {live_target(7)}\n",
            declaration: "live_target",
            use_site: "live_target",
            completion_at: "live_",
            completion: "live_target",
            hover_at: "live_target",
            rename_to: "renamed_target",
            diagnostic: Some(DiagnosticProbe {
                text: "\nfn broken( {\n",
                anchor: "broken(",
                repair: "\nfn broken() {}\n",
                action: None
            }),
            matrix: FeatureMatrix {
                completion: Coverage::Tested,
                hover: Coverage::Tested,
                signature_help: Coverage::Tested,
                references: Coverage::Tested,
                rename: Coverage::Tested,
                formatting: Coverage::Tested,
                code_actions: Coverage::AdvertisedOnly,
                diagnostics: Coverage::Tested,
                cross_file: Coverage::Tested
            },
        },
    },
    "requires RUNYTE_SMOKE_LSP_RUST=1 and rust-analyzer on PATH"
);

real_server_test!(
    markdown_marksman,
    ServerSpec {
        language: "markdown",
        opt_in: "RUNYTE_SMOKE_LSP_MARKDOWN",
        command: "marksman",
        args: &["server"],
        files: &[("README.md", "# Target\n\nSee [the target](#target).\n")],
        document: "README.md",
        symbol: "target",
        declaration: "Target",
        use_site: "target",
        // Markdown has no calls to describe; this is the negative case the
        // report was about, where every request used to be sent anyway.
        signature: None,
        extended: ExtendedProbe {
            text: "\n<!-- café 😀: unsaved Unicode precedes every probe -->\n## Live Target\n\nSee [live target](#live-target).\n",
            declaration: "Live Target",
            use_site: "live-target",
            completion_at: "#live-",
            completion: "Live Target",
            hover_at: "live-target",
            rename_to: "renamed-target",
            diagnostic: None,
            matrix: FeatureMatrix {
                completion: Coverage::Tested,
                hover: Coverage::Tested,
                signature_help: Coverage::Unsupported,
                references: Coverage::Tested,
                rename: Coverage::AdvertisedOnly,
                formatting: Coverage::Unsupported,
                code_actions: Coverage::AdvertisedOnly,
                diagnostics: Coverage::Unsupported,
                cross_file: Coverage::Unsupported
            },
        },
    },
    "requires RUNYTE_SMOKE_LSP_MARKDOWN=1 and marksman on PATH"
);
