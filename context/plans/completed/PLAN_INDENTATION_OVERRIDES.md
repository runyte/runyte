# Aligned indentation and document overrides

Approved scope: Insert-mode Backspace removes leading whitespace to the previous
visual tab stop. Tab keeps advancing to the next stop. Selection deletion,
pair deletion, Markdown list unwinding, and newline joining retain their order.
Mixed tabs and spaces use actual display stops. One width controls both display
and indentation; the existing global defaults remain unchanged.

## Implementation

1. Add bounded indentation configuration with language overrides and ordered
   file-pattern overrides. Resolve each field independently: global, language,
   then matching patterns in declaration order. Explicit equal-to-default
   values remain overrides. Match slash-separated workspace-relative paths;
   basename patterns match at any depth. Pathless/provider documents can use
   language settings without pretending to have a local path. Compile patterns
   when loading configuration, not while drawing.
2. Share effective buffer settings across editing, newline/indent commands,
   LSP formatting, presentation, wrapping, mouse targeting, and snapshot caches.
   Keep terminal emulation independent. Add aligned indentation deletion through
   ordinary transactions, including multiple selections and undo.
3. Add contextual Tab actions on the global width/style rows in `[config]`:
   choose a language or enter a file pattern, then edit the value. Show only
   saved overrides beneath their parent row. Enter edits directly; Tab removes
   an override. Prompts explain inherited values and sources. Save atomically,
   retaining unrelated YAML, comments and ordering; reject unsafe shapes.
4. Cover configuration precedence, validation, mixed whitespace, multi-cursor
   editing, per-pane geometry, settings creation/edit/removal/cancellation,
   persistence failure and reload. Update the guide, example configuration,
   keymap register and UI vocabulary.
5. Run formatting, all-target denied-warning Clippy, the ordinary test suite,
   and canonical workspace coverage (89% floor). Obtain Astra High review and
   iterate on findings until clean, then retain this plan as completed.

## Boundaries

This change does not add EditorConfig, indentation autodetection, separate soft
tab widths, or language-dependent built-in defaults. Override matching remains
in memory and performs no file scans. Native macOS/Windows checks remain CI-owned
when only Linux execution is available.

## Completion

Implemented in `a95da12` and reviewed by Astra High with no remaining findings.
The subsequent merge from `exp` was also reviewed without findings. The merged
branch passed `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, and `cargo llvm-cov --locked --workspace` on Linux. Canonical line
coverage was 91.81% (143,000 of 155,762 lines), above the unchanged 89% floor.
Native macOS/Windows acceptance and macOS coverage remain part of dev CI.
