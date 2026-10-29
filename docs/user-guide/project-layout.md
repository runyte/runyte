## Project layout

```text
src/
  app.rs          editor state, shared application types, and startup coordination
  app/            editor-level Git, workspace, pane, input, editing, terminal,
                  search, syntax, file, picker, settings, and LSP workflows
  buffer.rs       rope-backed buffers, file I/O, and transactional undo
  clipboard.rs    testable operating-system clipboard adapters
  text.rs         rope storage, character offsets, and transactions
  selection.rs    multi-range selections over character offsets
  grapheme.rs     bounded grapheme-cluster navigation and display widths
  syntax/         tree-sitter highlighting and the bundled grammar table
  lsp/            asynchronous language-server transport and typed events
  git/            bounded Git commands, projections, diffs, and status tracking
  terminal/       PTYs, emulation, bounded scrollback, and terminal sessions
  workspace/      persistent host state, attachment, and service lifecycle
  protocol/       private versioned DTOs for bundled local clients on Unix/Windows
  command.rs      editor commands and shared display metadata
  headless.rs     frontend-independent semantic editor test facade
  snapshot.rs     owned presentation-neutral editor and overlay snapshots
  config.rs       YAML settings and theme resolution
  directory_buffer.rs
                  editable directory projections and hidden entry identities
  fs_plan.rs      confirmed filesystem plans, conflicts, trash, and application
  diff.rs         line correspondence shared by Git and side-by-side comparison
  diff_view.rs    live paired-buffer comparison state and aligned scrolling
  hash.rs         stable content hashing used by buffers, Git, and transport
  path_safety.rs  canonical project-boundary checks
  external_open.rs
                  binary detection and the remembered programs that open them
  file_picker.rs  fuzzy matching, ignore-aware discovery, and text previews
  finder.rs       buffer and terminal ranking merged into the file scan
  picker.rs       shared presentation-neutral filterable result state
  help.rs         per-view prose and the registry-derived help document
  jump_labels.rs  proximity-ranked one- and two-key `gw` labels and narrowing
  key_hints.rs    registry-backed key discovery state
  keymap.rs       declarative bindings and sequence lookup
  layout.rs       recursive split tree
  notification.rs bounded workspace-lifetime history and its buffer document
  media.rs        native media classification and presentation operations
  document_preview.rs
                  bounded, engine-independent document captures
  ui.rs           Ratatui widgets and editor frame composition
  wrap.rs         Unicode cell-aware visual-line and soft-wrap geometry
  cli.rs, cli/    shared CLI, event loop, and terminal lifecycle
  main.rs         terminal edition entry point
crates/
  runyte-native/  GPUI window, clipboard, media/PDF workers and helper lifecycle
  runyte-preview/ isolated document rendering engine
  runyte-desktop/ desktop executable and early helper dispatch
```

The compatibility status for implemented, deviating, and removed Helix
bindings is tracked in `context/reference/helix-keymap-v1.md` in the
repository.

The UI is rendered into Ratatui's in-memory cell buffer. Ratatui compares each
completed frame with the previous one and sends only changed cells through its
Crossterm backend. The event loop blocks while idle, so unchanged screens are
neither reconstructed nor written to the terminal.

<!-- navigation -->
[Contents](README.md) · [Previous: Themes](themes.md) · [Next: License](license.md)
