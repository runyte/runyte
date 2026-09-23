# Elixir language support

This plan implements `context/issues/elixir_language_support.md` through the
existing statically linked syntax registry. The issue authorizes a language
addition; editor commands, parser scheduling, language-server integration, and
EEx/HEEx template grammars are outside this change.

## Scope and capability decisions

Register `elixir` for the case-insensitive extensions `ex` and `exs`. The
extension rule already covers `mix.exs`, `config.exs`, and `.formatter.exs`, so
no exact-filename exceptions are needed. Use `#` as the line-comment marker.
Do not add shebang detection or short language-name aliases in this change.
Markdown fences named `elixir` resolve through the existing registry and
remain subject to its 128 KB injection limit.

Provide highlighting, ordinary syntax-tree navigation, and generic structural
selection. Leave dedicated function/class/parameter text objects, outlines,
syntax indentation, and folds unsupported, matching the existing XML, HCL,
Ruby, and PHP capability boundary. Register their empty capabilities explicitly
and document the limitation. Leave Elixir-owned injections and locals empty:
Elixir string interpolation is already part of its syntax tree, and embedded
sigil languages and template grammars need separate scope and fidelity choices.

## Dependency and query audit

The preferred dependency is the exact pin `tree-sitter-elixir = "=0.3.5"`.
The [published Rust API](https://docs.rs/tree-sitter-elixir/0.3.5/tree_sitter_elixir/)
exports `LANGUAGE`, `HIGHLIGHTS_QUERY`, `INJECTIONS_QUERY`, and `TAGS_QUERY`.
Its [versioned upstream license](https://raw.githubusercontent.com/elixir-lang/tree-sitter-elixir/v0.3.5/LICENSE)
is Apache-2.0. Archive verification is an implementation prerequisite: the
planning environment could inspect these public sources but could not resolve
the crate-download host through its restricted shell network.

1. Fetch the published 0.3.5 crate and inspect its manifest, Rust binding,
   build script, parser/scanner, query files, license/notice files, and
   `.cargo_vcs_info.json`. Record its actual checksum, packaged revision,
   dirty flag if present, query provenance, and parser ABI. Do not substitute
   metadata inferred from an upstream branch.
2. Confirm its `LanguageFn` integrates with the existing
   `tree-sitter-language 0.1.7` and `tree-house 0.4.0` /
   `tree-house-bindings 0.3.2`. The installed bindings accept grammar ABI
   versions 13 through 15. Keep those parsing-stack pins unchanged; prove
   compatibility by constructing `DocumentSyntax` and compiling its query.
3. Add only the grammar dependency and required lockfile changes. The generated
   C parser and scanner must build with the normal project toolchain; editor
   use must require neither an Elixir runtime nor a server.
4. Use the packaged `HIGHLIGHTS_QUERY` with explicit version provenance in
   `src/syntax/grammars.rs`. Audit supported predicates and capture precedence
   against tree-house. If needed, append a small Runyte-authored query fragment
   under `src/syntax/queries/elixir/` using the existing fragment-composition
   pattern. Avoid copying the whole upstream query without a concrete need.

The [upstream highlight query](https://raw.githubusercontent.com/elixir-lang/tree-sitter-elixir/main/queries/highlights.scm)
currently exposes module names through `@module`, which Runyte does not map,
and atoms through `@string.special.symbol`, which falls back to `string`.
Verify the exact packaged query before adapting it. Prefer Elixir-local
captures mapping module aliases to `namespace` and atoms/keyword keys to
`constant` so the requested distinctions use existing theme scopes. Check
interpolation expressions and delimiters, sigil helper captures, ordinary
module attributes, documentation attributes, guarded definitions, and calls
without parentheses against actual highlight spans. An unmapped helper on a
node can override an earlier mapped capture in tree-house; query compilation
alone does not prove correct colours. Do not add theme scopes or make broad
capture-mapping changes merely to accommodate this one grammar.

## Implementation and regression coverage

Add the language definition in `src/syntax/grammars.rs`, then extend the
line-comment inventory test in `src/syntax/mod.rs` and the grammar-load matrix
in `tests/syntax.rs`. Existing registry construction stays lazy: no query
compilation, new worker, or timer belongs in startup.

Place Elixir behavior tests alongside the current language tests in
`tests/syntax.rs`, using its `parse`, `scopes`, `spans_of`, and character-offset
helpers. Cover these acceptance cases:

- Detection by canonical language name and `.ex`/`.exs`, both upper-case
  extensions, and each named issue example; `.eex` and `.heex` must not be
  registered as Elixir.
- Successful first-use grammar/query compilation and an empty registry-error
  list after parsing representative input.
- The issue's complete greeting example, asserting exact token spans for
  comments, `defmodule`/`def`/`when`/`fn`/`do`/`end`, module aliases, function
  definitions and local/remote calls, atoms, strings, interpolation expression
  tokens/delimiters, and `|>`/`->` operators.
- A second fixture covering ordinary module attributes, pattern matching and
  pin/match operators, keyword keys, integers/floats, string/regex sigils,
  multiline strings, and Unicode before and inside highlighted content.
  Include a zero-argument definition and a call without parentheses to expose
  differences hidden by the greeting example.
- Several transactional edits, including deletion and restoration of a
  closing delimiter or `end`, that leave temporarily incomplete code. Compare
  the full incremental span sequence with a fresh parse after every edit and
  assert expected unaffected highlights, valid character bounds, and no
  registry errors. Include a Unicode edit rather than only ASCII appends.
- Markdown containing an `elixir` fence: verify Elixir language identity
  inside the fence and representative highlight spans. Repeat after an edit
  inside the fence. Verify a document over the existing injection threshold
  drops embedded Elixir while retaining Markdown block highlights.
- Explicit unsupported outcomes for dedicated text objects, outline,
  indentation, and fold capabilities; generic syntax expansion still works.

No LSP implementation or configuration changes are needed. Server definitions
are configured separately, and only rust-analyzer is enabled by default.

## Documentation and attribution

Update `README.md` from 31 to 32 bundled grammars and add Elixir to both the
language list and syntax-support section in `docs/user-guide.md`. State file
detection, offline highlighting, Markdown fences, the structural capability
limits, and the exclusion of EEx/HEEx and bundled Elixir server setup.

Add the audited dependency and query facts to
`docs/dependency-license-inventory.md`, `docs/source-provenance.md`, and
`THIRD_PARTY_NOTICES.md`. Include Elixir among the Apache-2.0 exceptions in
the grammar license summary. Reuse the existing complete Apache-2.0 license
text if sufficient, retaining any package-specific attribution or NOTICE.
Local query headers must distinguish original MPL-2.0 additions from any
adapted Apache-2.0 material. Do not update historical performance measurements
without running the corresponding harness.

## Validation and completion

Run focused syntax tests first. Before handoff, run `cargo fmt --check`,
`cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked`.
Once dependencies are cached, verify syntax tests with `--locked --offline`.
Run the canonical `cargo llvm-cov --locked --workspace`; preserve the enforced
89% total line-coverage floor. Linux results do not constitute native macOS or
Windows verification; identify any platform checks left to CI explicitly.
The new C grammar must be included in existing platform build/test gates.

After implementation, Astra at medium reasoning reviews the complete change
for issue coverage, query correctness, dependency provenance, capability
claims, and regression risk. Sol at medium reasoning addresses findings, with
validation repeated only where the correction changes the tested behavior.

The orchestrator owns the repository's two-commit issue-resolution procedure.
The first commit contains the implementation, tests, documentation, and
neutral issue report. Only after its hash exists does the second commit move
the issue to `context/issues/resolved/elixir_language_support.md`, add the
required resolution frontmatter and diagnosis, preserve the complete report,
name the regression tests, and record deliberate capability limitations. Move
this plan to `context/plans/completed/` when the work is complete. Implementation
agents must preserve the original untracked issue until that coordinated step.
