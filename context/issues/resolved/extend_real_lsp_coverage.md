---
title: "The real-server LSP matrix covered only its initial smoke contract"
status: resolved
reported: 2026-08-14
resolved: 2026-08-19
legacy_commit: 06baabb
---

## Resolution

Commit `06baabb` (`Extend real LSP compatibility coverage`) extended the
opt-in real-server suite. The suite's `smoke` path previously stopped after
initialization, `didOpen`, document symbols, definition, signature help where
available, and shutdown, so it did not prove that Runyte's production stdio
transport remained compatible with real servers for document changes,
diagnostics, or most advertised editor features.

`tests/lsp_real_servers.rs` now gives every server an explicit
`FeatureMatrix`. A feature is recorded as tested, advertised-only, or
unsupported, so absence of a probe is a deliberate compatibility statement
rather than an unrecorded gap. `exercise_extended` sends an incremental
`didChange` containing non-ASCII text, then requests a definition for a symbol
that exists only in the unsaved document. Completion, hover, references,
rename, formatting, and signature help are exercised where the pinned server
gives a stable meaningful answer. Clangd supplies the positive code-action
case through a deterministic misspelled-name quick fix. Rust-analyzer, gopls,
and typescript-language-server also resolve a definition in another fixture
file, covering normal project indexing.

Every programming-language fixture introduces a deterministic error in a
second incremental change. The suite verifies that a published diagnostic
overlaps the expected LSP range, replaces the invalid range with valid text,
and waits for a later publication in which that range is clear. The position
anchors occur after the inserted non-ASCII text, so request and diagnostic
coordinates exercise the encoding negotiated during initialization. A final
valid incremental nudge makes diagnostic republication deterministic for
rust-analyzer. Markdown remains the explicit no-diagnostics case.

The Docker image still pins all server toolchains and now installs the pinned
Rust `rustfmt` component required by the new formatting probe. The expanded
suite remains ignored by default and uses disposable temporary projects;
ordinary `cargo test` still needs neither Docker, a language-server
installation, nor network access. `README.md` describes the extended contract
and points to the per-server matrix.

Coverage is exercised by `python_pyright`, `swift_sourcekit_lsp`,
`c_clangd`, `cpp_clangd`, `javascript_typescript_language_server`,
`go_gopls`, `rust_rust_analyzer`, and `markdown_marksman` in
`tests/lsp_real_servers.rs`; run all eight with `tests/lsp/run.sh`. The normal
mock protocol, crash, cancellation, queueing, and editor-response coverage
continues to live in `tests/lsp_client.rs` and the ordinary Rust suite. The
implementation was also checked with `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`.

Known limitation: SourceKit-LSP's advertised references, formatting, and code
actions; Marksman's advertised rename and code actions; Pyright's advertised
code actions; gopls's advertised code actions; typescript-language-server's
advertised code actions; and rust-analyzer's advertised code actions are
recorded as advertised-only because the pinned servers do not provide a
stable, meaningful answer for the deterministic fixtures. They are not
silently presented as covered or unsupported.

## Report

The opt-in Docker LSP matrix proved that Pyright, SourceKit-LSP, clangd,
typescript-language-server, gopls, rust-analyzer, and Marksman accepted
Runyte's initialization and document-open sequence, returned document
symbols, resolved a definition, and shut down cleanly. It did not exercise
most of the language features Runyte exposed.

The real-server matrix needed to extend beyond its initial smoke contract. At
minimum, each applicable language needed an incremental `didChange`
containing non-ASCII text so the negotiated position encoding was exercised,
followed by a request whose answer depended on the changed document rather
than the file on disk. Where a server provided diagnostics, the matrix needed
to introduce a deterministic error, verify that Runyte received the expected
diagnostic range, repair the error, and verify that the diagnostic was
cleared.

Capability-specific cases were needed for completion, hover, signature help,
references, rename, formatting, and code actions. A feature did not need to
be forced onto a server that did not advertise or meaningfully implement it,
but the intended coverage matrix needed to be explicit so a missing case was
distinguishable from an unsupported capability. Servers for which project
indexing is part of the normal workflow also needed a cross-file definition
or rename case.

These tests needed to remain ignored, on-demand Docker tests using pinned
server versions and disposable projects. Ordinary `cargo test` needed to
remain independent of external language-server installations, Docker, and
network access. The existing mock suite remained responsible for malformed
protocol data, crashes, cancellation, queueing, and editor-side response
handling; the Docker matrix was responsible for compatibility with real
server behavior.
