# Exp reliability and performance review — 2026-10-03

## Scope and disposition

The review starts at `1778900` on `exp`. All implementation and documentation
commits stay on that branch; no merge, push, release, or dependency upgrade is
part of this work. The reviewed implementation endpoint is `e97ee3d`.

**77 confirmed issues were repaired.** Each new issue has its own
implementation commit and a separate resolution-record commit naming that
implementation. Follow-up findings about an already resolved issue preserve its
original implementation reference and update its record alongside the code.
The issue index below links the complete diagnoses, reproductions, regression
names, and deliberate limitations.

The changes target correctness, bounded resource ownership, and work that grew
quadratically or needlessly with document size. The selection-first workflow,
ordinary bindings, and overall UX remain the basis of the editor. There is no
new public plugin protocol version or broad architectural rewrite.

## Review coverage and method

Subsystem owners read production source, reproduced actionable findings, added
behavior coverage, and repaired their owned files. Nested reviewers inspected
independent slices and revisited repairs. Shared ownership was coordinated in
one checkout; Git commits were serialized. Review continued through newly
exposed defects, including malformed transaction validation, Finder truncation,
PTY setup cleanup, directory request identities, and tooling contract drift.
Final crossreviews reported no remaining confirmed in-scope finding. That is a
review result, not a proof that the editor has no undiscovered defects.

| Review area | Source covered |
| --- | --- |
| Text and selections | Rope transactions and offset mapping; buffers, file/provider identity, undo; selections, indentation, text objects, structural selections, word indexing, jump history and labels. |
| Editor workflows | `app.rs`, application commands, headless facade, editing, input dispatch, completion, navigation, search, panes, file/directory operations, prompts, settings, external opening and tutorial coordination. |
| Filesystem and discovery | Directory buffers/listings/tree/plans, platform staging, file monitoring, file picker and Finder, workspace search, Markdown and navigation targets, pasted images, project roots and path containment. |
| Presentation and input | Snapshots and Ratatui rendering, layout/wrapping/tables/diffs, hints/help/manual, key registry/compiler/validation, input grammar, configuration and themes, notifications and session strip. |
| Language services | Syntax registry/background parsing, grammars and all 111 query files; application syntax coordination; LSP transport, diagnostics, Windows transport/path handling and language workflows. |
| Terminals | Parser, emulator, grid, scrollback accounting, reads/proposals/key encoding, Unix PTY and Windows ConPTY ownership, pending activation and editor terminal workflows. |
| Git | Git argument/IO boundaries, service ordering, discovery, status/history/patch/blame, tracker/monitor, merges/conflicts/fetch/network/worktrees and all application Git workflows. |
| Workspace and context | Shared/private protocol DTOs, host requests/service/buffers, identity/catalog/history/lifecycle/transport; full Windows endpoint, catalog, names, control, transport, service and lifecycle paths; context wire/storage/discovery/transports, native approval, reads and edits. |
| Plugins | Stable wire types, validation, process/worker lifetime, application/view/state/settings machinery, filesystem and provider reads/writes, host integration and application approval/recovery workflows. |
| OS and frontend | CLI/event loop, startup and health helpers, Unix/Windows TUI input and switching, clipboard, process cleanup, private storage, logs, external open, pipe execution, platform path/process/executable helpers and fixture infrastructure. |
| Shipped clients and tools | MCP bridge production; plugin Python/JavaScript/C/Rust examples, current/frozen SDKs and schemas, remote transports, compatibility gates; benchmark Python/Rust/Lua helpers; installer, CI/release configuration, contribution helpers and screenshot/demo scripts. |

Platform-specific source was inspected even where the current Linux host could
not execute it. Test code was examined at changed behavior boundaries, through
regressions and integration fixtures; no claim is made that every possible test
interleaving or native platform path was exercised.

## Implementation changes

### Editor state and data correctness

- Host saves now report success only after a completed write. Activated host
  opens publish coherent prepared state and retain the buffer identities the
  caller requested, including directory aliases and multi-directory requests.
- Late Git commit completions preserve message edits made after submission.
  Git read coalescing respects intervening mutations; history pagination keeps
  merge ancestry. Repository and status paths retain raw identity while display
  labels escape characters that would corrupt rows.
- Multi-cursor line-prefix deletion merges overlaps, scoped searches preserve
  half-open boundaries, and Markdown reflow recognizes actual closing fences.
  Raw invalid transactions still reach normal validation after indexing.
- Pointer mapping accounts for horizontally scrolled tabs; wheel events scroll
  the hovered wrapped pane. Clipped text no longer invents end-of-line cells.
  Typed optional command arguments match supported command semantics, and
  configured remaps consumed by the input grammar are rejected explicitly.
- Bracket jumps require real delimiter partners. Terminal review line-end motion
  stays on blank rows, padded cursor coordinates match displayed rows, SGR mouse
  release retains the released button, cancelled control sequences recover, and
  unsupported CSI intermediates cannot masquerade as basic commands.
- Pasted image links encode reserved path characters. Configured help text
  remains literal; LSP location titles and extreme row values render correctly.

### Lifetime, blocking and resource bounds

- LSP connection shutdown cancels retained transport tasks. File-monitor stop
  survives queue overflow, Finder workers release their own wake ownership,
  and the main event loop stops polling closed service channels.
- File, settings, metadata, history, log, cache, audit, Finder preview and debug
  trace boundaries validate regular files through opened descriptors where
  applicable. Unix FIFO opens are nonblocking and opened non-regular
  descriptors are rejected; bounded cache/preview reads reject excessive data. Regular-file
  IO can still stall on a stalled filesystem.
- Detached persistent-host startup drains diagnostics with bounded ownership.
  Partial PTY setup cancels pending activation, closes endpoints and reaps in
  the correct order. Manual external open reserves a bounded reaper before
  spawning, and clipboard success requires complete input delivery.
- Hidden primary terminal history remains charged while the alternate screen
  is active. Future runtime state paths resolve existing ancestor aliases and
  parent traversal before overlap decisions.
- Word indexing shares capped completed word lists. Windows directory cache
  validation checks directory identity, including replacement at the same path.
  Finder result budgets count authoritative content and expose truncation even
  inside one dense file or the last live buffer.

### Performance refactoring

- Terminal character moves use slice copying and row operations use rotation
  instead of repeating a one-cell or one-row shift.
- Transaction offset mapping indexes cumulative deltas. Scoped search advances
  through ordered regions, and multi-selection paste locates retained changes
  with ordered bounds. Paste requests completion once at the final caret.
- Directory plans index identities and path groups; directory trees reuse
  sorted listings. Diff filler lookup locates the containing alignment run.
- Workspace searches avoid repeatedly scanning the same line for columns and
  previews. Session reads copy only the bounded rope prefix requested.
- Overlay formatting visits retained/visible rows. Key marker offsets advance
  incrementally, long Markdown marker runs are skipped in one pass, URL suffix
  punctuation uses indexed counts, and jump labels avoid measuring offscreen
  word prefixes.
- Newline indentation queries only the relevant tree range and prunes clean
  parse-error subtrees. A small private native shim stays inside `syntax/`
  because the pinned bindings do not expose the runtime error-bit accessor.

### Shipped client, fixture and benchmark repairs

The published context Python client/schema now includes documented
`buffer.append`. The client enforces permission checks, UTF-8 byte limits and
uncertain mutation outcomes; the schema defines closed request shapes and
character-count bounds. Example memory reads refuse a byte window that cannot make
UTF-8 progress; failed file-browser pagination releases its directory handle.
Benchmark harnesses check process-handle capability before launch, maintain
exit deadlines after PTY EOF, retain fragmented terminal queries across reads,
and isolate inherited diagnostic instrumentation.

Regression fixtures were also made deterministic around explicit log ownership,
external-opener cleanup and revised search expectations. These changes retain
failure evidence rather than hiding it by relaxing production bounds.

## Measured performance evidence

The table records targeted before/after stress measurements from this checkout
and retained earlier source. They demonstrate the diagnosed scaling changes;
they are not representative whole-editor speedup ratios. Timings vary by host.
No fresh claim about overall startup, idle CPU or end-to-end typing latency is
inferred from these isolated operations.

| Workload | Before | After | Measurement scope |
| --- | ---: | ---: | --- |
| 32,768-column terminal row; four 16,000-cell insert/delete pairs | 858.429 ms | 0.091462 ms | Actual grid source, `rustc -O`, three alternating samples after warm-up. |
| 32,768-row terminal region; 16,000-row scroll/insert/delete operations | 1,216.661 ms | 0.602435 ms | Actual grid source, optimized, three alternating samples after warm-up. |
| 100 diff filler lookups near a million-row gap | 146.793 ms | 0.000450 ms | Optimized lookup workload, five samples; excludes alignment construction. |
| Directory plan for 16,384 entries | 177.471 ms | 10.825 ms | Three samples; excludes filesystem IO and fixture cloning. |
| URL with 65,536 trailing punctuation characters | 4,879.092 ms | 0.127053 ms | Optimized navigation-target stress harness. |
| 20,000 transaction cursor pairs | 205.47 ms | 1.172 ms | Optimized actual mapping source with debug dependencies; excludes transaction construction. |
| 20,000 search regions and 40,000 matches | 84.25 ms | 4.18 ms | Optimized actual search source with debug dependencies. |
| Warm newline indentation in a 2 MB repeated-function document | about 1.82 s | 18–20 µs | Debug build; excludes parsing, rendering and other input processing. |

Reproduction harnesses are under `benchmarks/`: `terminal_grid.py`,
`diff_filler.rs`, `directory_plan.py`, `navigation_target.py`,
`transaction_mapping.py`, and `scoped_search.py`. The ignored manual
`newline_indentation_scaling` test is in `tests/syntax.rs`, runnable with
`cargo test --locked --test syntax newline_indentation_scaling -- --ignored --exact --nocapture`.
Invocation and measurement boundaries are documented in the benchmark README and linked
issue records.

Independent semantic comparisons checked 265,349 URL cases, 2,400 Markdown
cases (full text/scopes and sampled offset/link checks), 7,872 indentation
decisions, and 5,000 terminal-query streams over 102,611 read boundaries. These
complement targeted regression tests; they do not replace native integration or
performance budgets.

## Validation

Native environment: Linux x86-64, Rust 1.97.1, cargo-llvm-cov 0.9.0. Local
socket, PTY and subprocess fixtures ran with the required operating-system
access. Configuration and fixture storage remained temporary.

| Check | Result |
| --- | --- |
| `cargo fmt --check` | Passed. |
| `cargo clippy --all-targets -- -D warnings` | Passed. |
| `cargo test --locked` | 4,538 passed, 54 intentionally ignored; no exclusions or test filters. |
| `cargo llvm-cov --locked --workspace` | Passed: 91.94% total Lines (145,756 / 158,528); 4,538 tests passed, 54 ignored. |
| `cargo test --release --locked --test performance -- --ignored --test-threads=1` | All 19 release performance budgets passed serially (8.15 s for the test run). |
| Plugin `check_all.py --require-backends` | All 30 suites passed: 442 tests, including all 69 Python/C/Rust todo cases; no backend skips. |
| Benchmark unit discovery | 126 passed, 2 native acceptance cases skipped by default. The affected real-editor first-open acceptance was run explicitly and passed for plain text and Lua while preserving the caller trace. |
| MCP bridge with the actual debug editor | 46 passed, 7 native Windows cases skipped; all four real Unix host cases executed. |
| Installer | 18 tests passed; shell syntax check passed. |
| Screenshot/demo and compatibility tooling | 4 capture, 9 recording and 14 compatibility-gate tests passed. |
| Shipped examples in a real persistent host | Explicit ignored acceptance passed, with Python remote dependencies, Node and built C/Rust examples available. |
| Rust todo example | Formatting, denied-warning Clippy and its Cargo test target passed; behavioral verification is in the conformance suites above. |

The final complete Rust run includes the LSP graceful-shutdown follow-up
`e97ee3d`; an earlier full run exposed that regression and was not accepted as
validation. No failing test was excluded to obtain the final result.
The coverage floor, CI threshold and README badge remain unchanged at 89%.

## Constraints and remaining limits

- Validation here is native Linux. macOS ARM64 coverage and Darwin PTY drain
  behavior still need the existing native CI gates; native Windows builds,
  clipboard/session acceptance and directory-cache behavior were not executed.
  Static review and Linux fixtures do not establish those platform results.
- The unchanged deferred issues are
  [filesystem substitution races](../issues/deferred/fs_plan_symlink_race.md)
  and [MCP tool-list refresh](../issues/deferred/mcp_client_tool_list_refresh.md).
  Their broader design decisions were not reopened by this review.
- Existing open requests for installation channels, Windows chooser shortcuts
  and Windows stop-without-selector remain outside this reliability repair.
- Bounded word-list publication still allows a transient distinct-word count
  map while scanning one buffer. Overlay ranking still considers candidates;
  the repair removes unnecessary formatting. Goto-word still scans line text.
- The syntax error accessor relies on the pinned `tree-house-bindings` Node ABI
  and must be rechecked on dependency updates. Malformed trees can still inspect
  siblings on error-bearing branches. Existing large-source parser limits remain.
- Pasted-image link parsing retains the documented literal-first ambiguity when
  both literal and decoded spellings identify different existing files.
- Ordinary suites intentionally skip native acceptance and manual performance
  measurements. Subprocess helper entrypoints are ignored when run directly,
  but their owning ordinary tests execute them as fixtures. Explicit extra
  acceptance and measurement runs are identified above.
  Coverage is the repository's canonical total Lines measure, which includes
  compiler-instrumented inline tests; it is not production-only coverage.

## Suggested manual acceptance on exp

1. Edit and save real projects with multiple cursors; exercise Enter in large
   syntax-highlighted files, paste, undo/redo, scoped search and long URLs.
2. Use Finder with open unsaved buffers, directory navigation, aliases and
   multiple panes; verify tabs, soft wrap and pointer/wheel targeting.
3. Run interactive terminal applications through alternate-screen entry/exit,
   resize, mouse input, scrollback and persistent-session attach/detach.
4. Exercise Git status/history/merge views and edit a commit message while a
   commit is finishing. Try context append and provider/example applications.
5. Repeat the native platform cases above before deciding whether to merge.

## Issue-by-issue implementation index

Each link contains the original report, diagnosis, exact regression names and
known limitations. Commit identifiers name the implementation, not the following
documentation move.

| Implementation | Confirmed issue and resolution record |
| --- | --- |
| `fd151ef` | [Benchmark children can overwrite caller trace files](../issues/resolved/benchmark_inherits_caller_trace_paths.md) |
| `f96d899` | [PTY EOF bypasses the benchmark quit deadline](../issues/resolved/benchmark_pty_eof_bypasses_quit_deadline.md) |
| `f9d414e` | [PTY benchmarks lose fragmented terminal capability queries](../issues/resolved/benchmark_terminal_queries_lost_across_reads.md) |
| `b3c90d2` | [Plugin benchmarks assume unavailable Python process-handle APIs](../issues/resolved/plugin_benchmark_assumes_python_pidfd_support.md) |
| `75cb009` | [Clipboard helpers report successful copies with incomplete input](../issues/resolved/clipboard_incomplete_input_reports_success.md) |
| `7dc376f` | [Typed commands reject valid optional arguments](../issues/resolved/typed_command_optional_arguments_rejected.md) |
| `82cfd0f` | [Windows path completion can reuse a replaced directory listing](../issues/resolved/windows_path_completion_directory_replacement.md) |
| `e4842fb` | [Configuration reads can hang on special files](../issues/resolved/special_file_config_read_hang.md) |
| `bc6f6a1` | [Published context Python client and schema omit buffer append](../issues/resolved/context_python_client_omits_buffer_append.md) |
| `e2bac56` | [Special input trace files block debug startup](../issues/resolved/special_file_input_trace_startup_hang.md) |
| `3ff1c7f` | [Same-line cursors leave requested prefix text undeleted](../issues/resolved/multicursor_delete_line_start_leaves_text.md) |
| `f9587dc` | [Markdown reflow can alter code after an invalid closing fence](../issues/resolved/markdown_reflow_closes_fences_early.md) |
| `e42d1c6` | [Memory provider UTF-8 reads can acknowledge empty non-final chunks](../issues/resolved/plugin_memory_read_utf8_boundary_no_progress.md) |
| `7b2dd47` | [Failed file-manager pagination leaks a directory snapshot handle](../issues/resolved/plugin_file_browser_failed_page_retains_directory.md) |
| `4c5729c` | [Program cache files can block startup or grow without a bound](../issues/resolved/program_cache_special_file_hangs.md) |
| `06bc2b3` | [Special-file document reads can stall the editor or its workers](../issues/resolved/special_file_document_read_hang.md) |
| `f9c8f3d` | [Manual external opens create unbounded infallible reaper threads](../issues/resolved/manual_external_open_unbounded_reaper_threads.md) |
| `9671138` | [File monitor can retain its worker after shutdown](../issues/resolved/file_monitor_shutdown_queue_overflow.md) |
| `16952e2` | [Special files can block Finder scans and previews](../issues/resolved/finder_special_files_block_workers.md) |
| `35ff39e` | [Dense matching files conceal Finder result truncation](../issues/resolved/finder_dense_file_hides_truncation.md) |
| `a847564` | [Stale saved contents exhaust Content Finder's result budget](../issues/resolved/finder_stale_disk_results_exhaust_budget.md) |
| `1070431` | [Finder workers retain their own wake-channel senders](../issues/resolved/finder_workers_retain_their_own_wake_senders.md) |
| `e20e236` | [Commit detail reads execute configured Git text converters](../issues/resolved/git_commit_detail_runs_textconv.md) |
| `7c3df5b` | [Control characters in Git status paths shift action rows](../issues/resolved/git_status_filename_newlines_shift_action_rows.md) |
| `272f12c` | [Git reads coalesce across queued repository changes](../issues/resolved/git_read_coalescing_crosses_mutation_barriers.md) |
| `8b202fc` | [Git history pages lose merge ancestry](../issues/resolved/git_history_pages_lose_merge_ancestry.md) |
| `0b0119b` | [Git commit completion discards newer message edits](../issues/resolved/git_commit_completion_discards_new_message_edits.md) |
| `cb59e84` | [Git repository discovery changes valid pathname bytes](../issues/resolved/git_discovery_changes_path_bytes.md) |
| `6c5e5c4` | [Direct Git metadata reads can wait indefinitely on named pipes](../issues/resolved/special_file_git_metadata_read_hang.md) |
| `3fc286a` | [Configured direct key descriptions can panic while opening help](../issues/resolved/configured_direct_key_help_marker_panic.md) |
| `5900be7` | [Pasted image links misinterpret reserved pathname characters](../issues/resolved/pasted_image_link_reserved_path_characters.md) |
| `984a06b` | [Pointer coordinates disagree with tab rendering after horizontal scrolling](../issues/resolved/pointer_tabs_after_horizontal_scroll.md) |
| `71be29b` | [Configured key remaps can advertise unreachable commands](../issues/resolved/configured_key_remaps_unreachable.md) |
| `8e6b060` | [Opening a diagnostic log can block on a special file](../issues/resolved/special_file_diagnostic_log_read_hang.md) |
| `277ee71` | [Language-server transport tasks survive connection teardown](../issues/resolved/lsp_transport_tasks_outlive_connection.md) |
| `f2e6818` | [LSP location picker titles duplicate their initial letter](../issues/resolved/lsp_location_picker_duplicated_initial.md) |
| `ee64775` | [Language-server location rows overflow while building a picker](../issues/resolved/lsp_location_picker_row_overflow.md) |
| `6bcf501` | [Soft-wrap wheel scrolling changes the active pane instead of the hovered pane](../issues/resolved/soft_wrap_wheel_scrolls_active_pane.md) |
| `1da9d33` | [Process audit storage can block cleanup signals](../issues/resolved/process_audit_special_file_blocks_cleanup.md) |
| `7cdf1bb` | [Clipped text rows invent line endings and duplicate end carets](../issues/resolved/clipped_rows_false_line_endings.md) |
| `0efc559` | [Closed service channels repeatedly wake the editor event loop](../issues/resolved/closed_service_channels_spin_event_loop.md) |
| `ec5150a` | [Unsaved buffer changes can hide workspace search results](../issues/resolved/workspace_search_stale_disk_budget.md) |
| `a8f2500` | [Scoped search includes text outside half-open selections](../issues/resolved/scoped_search_includes_half_open_selection_boundary.md) |
| `05f8f2a` | [Detached host stderr can block startup beyond its readiness deadline](../issues/resolved/detached_host_startup_stderr_can_block.md) |
| `87b74ee` | [Session listing paths can corrupt terminal table rows](../issues/resolved/session_cli_control_characters_corrupt_rows.md) |
| `46fa60c` | [Persistent-session metadata reads can block on special files](../issues/resolved/special_file_workspace_metadata_read_hang.md) |
| `d1eb280` | [Recent-workspace history reads can block indefinitely on named pipes](../issues/resolved/special_file_workspace_history_read_hang.md) |
| `e79a6b2` | [Host saves report success without writing the buffer](../issues/resolved/host_save_reports_unwritten_buffers_as_saved.md) |
| `4035a9a` | [Missing state directories conceal overlap through symlink aliases](../issues/resolved/state_root_overlap_misses_missing_alias_descendants.md) |
| `b6ed7d1` | [Syntax bracket matching invents partners in incomplete or quoted text](../issues/resolved/syntax_bracket_matching_invents_partners.md) |
| `962be54` | [Terminal view padding separates the cursor from its displayed row](../issues/resolved/terminal_padded_view_cursor_misaligned.md) |
| `36addad` | [Cancelled terminal sequences swallow subsequent output](../issues/resolved/terminal_sequence_cancellation_loses_output.md) |
| `17fe8c6` | [Alternate terminal screens hide retained history from the shared budget](../issues/resolved/terminal_alternate_screen_hides_history_budget.md) |
| `8a94dff` | [Partial PTY setup can wait before releasing its endpoints](../issues/resolved/pty_setup_failure_waits_before_closing_endpoints.md) |
| `a09e163` | [Unsupported CSI functions execute unrelated basic commands](../issues/resolved/terminal_csi_intermediates_alias_basic_commands.md) |
| `5e051a1` | [Terminal review line-end motion leaves empty rows](../issues/resolved/terminal_review_line_end_leaves_blank_row.md) |
| `aa05cd0` | [SGR mouse releases lose the released button identity](../issues/resolved/terminal_sgr_mouse_release_button.md) |
| `214f74a` | [Host open errors can publish partial buffer changes](../issues/resolved/host_open_failure_publishes_refreshed_buffers.md) |
| `9fa0fdf` | [Host directory activation retargets another requested buffer](../issues/resolved/host_directory_open_retargets_another_requested_buffer.md) |
| `bd64610` | [Pasted text repeats completion requests at the final caret](../issues/resolved/pasted_text_repeats_completion_requests.md) |
| `24a622a` | [Completion retains uncapped word counts and rebuilds unchanged buffers](../issues/resolved/word_index_retains_and_rebuilds_uncapped_counts.md) |
| `4cca101` | [Diff viewport mapping scans entire filler gaps](../issues/resolved/diff_filler_row_lookup_scans_entire_gap.md) |
| `c1b0aa5` | [Multi-selection paste repeatedly scans retained replacements](../issues/resolved/multiselection_paste_repeatedly_scans_retained_changes.md) |
| `a6c5ad5` | [Directory plan identity matching repeats full scans](../issues/resolved/directory_plan_quadratic_identity_matching.md) |
| `eea17b1` | [Directory tree navigation repeatedly clones and sorts cached listings](../issues/resolved/directory_tree_repeated_listing_sort.md) |
| `ff5e663` | [Key-marker resolution repeatedly scans the growing page](../issues/resolved/quadratic_key_marker_resolution.md) |
| `57db4ce` | [Long Markdown marker runs cause repeated unbounded scans](../issues/resolved/markdown_long_marker_runs_quadratic.md) |
| `2a5cde6` | [URL punctuation trimming repeatedly rescans candidate text](../issues/resolved/navigation_url_punctuation_quadratic.md) |
| `376479a` | [Jump labels repeatedly measure offscreen word prefixes](../issues/resolved/jump_labels_measure_every_offscreen_word.md) |
| `106b22f` | [Large overlays format offscreen rows on every redraw](../issues/resolved/overlay_formats_offscreen_rows.md) |
| `297414c` | [Scoped search rescans every selection for each match](../issues/resolved/scoped_search_rechecks_every_selection_for_each_match.md) |
| `6684665` | [Dense workspace matches repeatedly rescan long lines](../issues/resolved/workspace_search_repeated_line_scans.md) |
| `c81e0f3` | [Bounded session reads copy whole documents before truncating](../issues/resolved/workspace_bounded_reads_copy_entire_documents.md) |
| `b528dad` | [Newline indentation scans unrelated document syntax](../issues/resolved/syntax_newline_indent_scans_entire_document.md) |
| `d26624a` | [Terminal character shifts repeatedly copy the same cells](../issues/resolved/terminal_character_shift_quadratic.md) |
| `2e742e2` | [Terminal line operations perform quadratic row shifts](../issues/resolved/terminal_line_shift_quadratic.md) |
| `4903c0f` | [Transaction offset mapping scales quadratically with cursor count](../issues/resolved/transaction_mapping_quadratic_for_many_cursors.md) |

Additional follow-up commits preserve their original issue references:
`654e798` (search regression expectations), `0929f12` (hint scrolling prefixes),
`0c7ba91` (future-path parent traversal), `bb9d8c2` (invalid raw transactions),
`fcd90c7` (live-buffer truncation), `d5c5690` (opener fixture cleanup),
`8222781` (explicit logging fixture ownership), `50f7958` (CSI control-flow lint),
`c9de066` (key-marker assertion lint), and `e97ee3d` (bounded LSP shutdown-frame
drain before task cancellation).
