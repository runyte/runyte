# Document readiness independent of syntax parsing

Status: completed — approved and implemented 2026-09-07

Created: 2026-09-07

Analysis baseline: `aee17b0`.

The implementation and final validation are recorded below. The numbered plan
retains the approved design and its analysis baseline.

## Implementation decisions

The application keeps pending requests separate from its optional stale-tree
highlight fallback. Each pending request has an application-issued generation
and a full target rope snapshot; the worker accepts either no base (initial
or replacement parsing) or an exact tree/text base (incremental updates).
Completion validates generation and globally unique text revision before any
tree becomes available. Language is part of the request and is checked again
against the live document on receipt.

A single dedicated parser thread uses a condition variable for requests and
Tokio notification for completion. Both queues coalesce by buffer, and locks
cover queue bookkeeping only. Dropping the handle or receiver stops queued
work without joining an active parser call. Catching a parser panic produces
a failure result for that request, preserving service availability for other
documents. A thread-start failure closes the service and settles pending work.

Production construction explicitly defers syntax even before worker attachment.
Existing constructors used by structural tests retain inline behavior. Worker
attachment opts subsequent opens and full replacements into deferred behavior
and submits all previously pending documents, initially visible ones first.

Tests control application of completion events and use an unattached deferred
editor to hold all parser work where first-frame ordering must be deterministic.
Worker tests additionally exercise coalescing, cancellation, and checkpoint
reuse directly. Review is a separate subagent pass after implementation; every
finding must be addressed or resolved with evidence, followed by another pass
until no findings remain.

The second independent review additionally identified last-owner tree disposal
as document-size work. Unread completions now retain their source snapshot and
stay in a hidden per-buffer slot when superseded, for checkpoint reuse or
worker-side cleanup. Cancellation transfers retired trees to the worker, which
drains them before another parse; repeated typing cannot append retired clones.
Shutdown signals ownership retirement without clearing trees on the caller.
Checkpoint pruning and all retired-tree destruction occur outside queue locks.
A deterministic disposal seam verifies thread ownership and continued input
while cleanup is held.

## Intended behavior

Once file contents are loaded into the authoritative text buffer, the editor
can display and edit them. Initial syntax parsing and subsequent full parses
run outside input handling and frame preparation. A completed current tree
adds highlighting and enables structural commands between frames.

The first document frame may use ordinary foreground colours. Its text,
selection, scroll position, wrapping, and undo state are already real editor
state. Applying initial syntax changes styling and command availability; it
does not replace the buffer, reset the caret, or automatically fold text.

Movement, selection, search, insert/replace, deletion, comments, undo/redo,
saving, and language-server workflows remain usable while parsing. Language
identity comes from the filename and bounded first-line detection independently
of a tree. Optional smart newline keeps its existing whitespace/list fallback
when syntax indentation is unavailable; it never adjusts an earlier edit after
syntax arrives.

`Space x` commands remain visible but dimmed while a current tree is missing.
Invoking one reports `Syntax is still parsing` without waiting or queuing the
command for later execution. The same readiness rule covers remapped keys,
colon commands, and other invocations of those command identities.

There is one existing binding outside `Space x` that also needs this rule:
`mm` (`match-bracket`) deliberately uses syntax to ignore unrelated brackets
in strings and comments. Keep that behavior and give it the same temporary
unavailability. Add its missing syntax capability metadata rather than
introducing a different textual bracket matcher for the initial interval.

No new loading overlay or recurring notification is needed during parsing.
Hints and the palette distinguish pending syntax from unsupported documents
and failed parsing. Service health retains actual failures. A failed parser
leaves an editable plain-text document and does not create a retry loop.

## Evidence and implementation at the analysis baseline

The 2026-09-05 measurements in
`context/reference/startup-performance.md` concern Runyte 0.1.10. For the
50,000-line Lua fixture, demonstrated editing readiness was 157.5 ms versus
Neovim's 32.2 ms. Runyte's separately measured file-loaded and syntax-ready
medians were 11.9 ms and 153.1 ms. The byte-identical plain-text fixture was
ready to edit in 23.0 ms. These observations support removing the parse
dependency, but do not establish a future latency or an isolated parsing cost.
The instrumented milestones and readiness samples came from separate launches.

At `aee17b0`, source still had that dependency:

- `src/app.rs::open_launch_targets` calls `parse_buffer` for every document
  before returning. `parse_buffer` calls `DocumentSyntax::new` synchronously.
- `src/main.rs` presents `Opening workspace…`, constructs the application,
  draws its first editor frame, and only then attaches services, including the
  syntax worker. The user guide explicitly promises a fully highlighted first
  document frame; the proposal intentionally changes that policy.
- `src/app/syntax_workflows.rs::reparse` already moves incremental updates to
  a worker. `StaleSyntax` preserves translated viewport highlights without
  exposing an old tree to structural commands.
- `src/syntax/background.rs::ParseRequest` requires an existing tree and base
  text. It cannot represent the first parse. `apply_syntax_event` likewise
  requires a `StaleSyntax` entry.
- `reparse_whole`, ordinary and protocol file opening, successful saves,
  language changes, and newly opened LSP edit targets still parse inline.
- Grammar/highlight configurations initialize lazily. Moving
  `DocumentSyntax::new` into the worker also moves the configurations it first
  uses there. Language detection and readiness presentation must not force
  their initialization on the editor thread.

## Implementation plan

### 1. Represent initial and replacement work explicitly

Keep `App::syntax` restricted to trees matching current text. Generalize the
pending lifecycle to represent an initial/full parse with no previous tree as
well as an incremental refresh with a `StaleSyntax` presentation fallback.
Represent unsupported, pending, ready, and failed outcomes distinctly; avoid
inferring all of them from `None`.

Each request/result identifies the buffer, a parse generation, language, and
target text revision. Incremental requests also identify their base. Text
revisions are already globally unique, so reuse them. A generation additionally
invalidates work when a path/language or parse lifecycle changes without a text
change. Only accept a result for a live buffer whose generation, language,
target revision, and applicable base still match. Late failures are rejected
under the same rules as late successes.

Centralize invalidation and scheduling in `src/app/syntax_workflows.rs`.
Closing or repurposing a buffer retires its pending work and retained syntax.
Pending edits must schedule the latest snapshot even when no initial tree
exists; audit the `watched`/pre-edit capture conditions, including LSP edits,
so absence of a tree never silently stops scheduling.

### 2. Extend the worker without a typing backlog

Support full requests containing language plus a cheap rope snapshot, and
incremental requests containing a parsed base plus the target snapshot. Keep
one active parse per workspace and at most one newest queued request per
buffer. Preserve the existing guarantee that distinct buffers cannot overwrite
one another's work or results.

If typing overtakes the initial parse, its result must not enter the editor as
a current tree. The worker can keep that completed tree and its exact source
snapshot as a private base, then incrementally advance it to the newest target
within the same generation. This avoids starting the same large document from
scratch repeatedly. Discard the checkpoint on generation/language changes or
failure. Retention is limited to pending work, not an additional permanent
cache of every open document. Bound/coalesce unpublished completion storage as
well as request storage.

Queue the initially visible document first and give visible buffers priority
at job boundaries, with fair service for other buffers. No editor-thread wait,
periodic parser polling, or thread per document is needed. Preserve the
existing injection-size policy and stale-highlight behavior after an accepted
initial parse.

The current worker awaits `spawn_blocking`, while `main` drops the Tokio
runtime at exit. An active parse can therefore delay process termination.
Use an owned parser thread with non-blocking request/event delivery and a
shutdown signal: shutdown discards queued work, and an active computation
owns only snapshots and registry references, may finish independently, and
cannot keep editor shutdown waiting on a join. It retires after the active
call when its owner is gone. Keep this syntax-specific; do not relax shutdown
for unrelated services. The pinned tree-house API exposes parse timeouts but
no caller cancellation parameter, so aborting a task must not be described as
preempting its parse. Closing a buffer invalidates an active result immediately
even if the underlying computation is still finishing.

Worker failure must settle affected pending state and report an unavailable
service instead of leaving commands dimmed forever. Detaching a persistent
client keeps the host's worker running normally.

### 3. Remove synchronous production parse entry points

Add an explicit deferred-syntax construction policy for production startup;
load buffers, detect languages, and record pending initial work without parsing.
Attach the worker and submit pending requests without delaying the first
document frame. Both standalone and persistent host startup use this policy.
Retain an explicit inline policy for deterministic existing headless/unit tests,
and exercise deferred construction in dedicated tests. Production must never
fall back to parsing inline simply because services have not attached yet.

Route ordinary opens, persistent protocol/`--wait` opens, undo/redo, reload,
language-changing edits, save-as, directory retargeting, and LSP-created
buffers through the same scheduler. Keep staged multi-file operations atomic:
enqueue syntax only after their buffers are committed to editor state.
An ordinary save that changes neither text nor language should retain current
syntax or pending work rather than request another full parse.

This proposal removes parsing from document readiness. File reads and
construction still precede display, and startup still loads all explicit
targets before returning. Streaming file loading or displaying the first
target before the remaining targets finish loading is a separate design.

### 4. Publish readiness consistently

Use the existing syntax capability snapshot for hints, palette availability,
and invocation feedback. Ensure capability state refreshes when completion is
applied, including an already open hint popup. Compact hints must say
`parsing` for pending work instead of collapsing it to `no syntax`.

All structural entry points, including `mm` and semantic/headless invocations,
must refuse unavailable syntax without blocking, changing selections, or
replaying an action later. Rendering without a tree already yields empty
highlight spans; retain that ordinary rendering path.

Drain completion through the existing `HostEvent::Syntax` boundary and request
the usual frame update. Attached clients receive the resulting semantic
snapshot; parsing and its requests remain host-owned. Check both a newly
starting host and files opened in an already attached persistent session.

### 5. Verify correctness and performance

Add deterministic tests with a held parser/completion boundary, rather than
depending on a large fixture taking long enough:

- Before the first parse is released, document snapshots contain the full
  text; navigation, multi-caret Unicode editing, undo/redo, and saving work.
  Save verification compares the complete expected file.
- Syntax hints and `mm` are temporarily unavailable, selections remain
  unchanged after refused commands, and the current result enables them.
- Typing during initial work rejects obsolete completion and eventually
  publishes the newest revision. Rapid typing coalesces; two buffers both
  receive results; worker checkpoints never become structural editor state.
- Reload, close, repurposing, save-as, shebang changes, and LSP replacements
  reject old generations. Ordinary saves preserve valid syntax work.
- Initial parse failure, timeout, and worker termination preserve editing and
  settle pending state. Unsupported documents never appear perpetually pending.
- Highlight arrival preserves text, cursor, selection, scroll, and undo history.
  Existing translated-highlight and injection-threshold tests continue to pass.
- Standalone startup and persistent open paths expose usable text before
  completion. Quitting during initial parsing remains responsive; persistent
  detach preserves parsing and reattachment observes its current state.

Extend `tests/background_syntax.rs`, relevant application and startup tests,
and `tests/performance.rs`. Keep synchronous tests where they intentionally
exercise structural behavior; do not rewrite all tests to wait on real threads.

Update `src/startup.rs`: syntax readiness is now independent of first-frame
ordering, so it cannot remain in the current strictly ordered phase sequence.
Record actual successful completion for the initial target, and distinguish
it from applying that tree if recording both milestones. Update
`benchmarks/runyte-milestones.patch` and its documented probe location; never
substitute enqueue time or an unsupported/failed result for syntax completion.

Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, and `cargo llvm-cov --locked --workspace`; retain the 89% total
line-coverage floor on affected first-class Linux/macOS targets. Run benchmark
harness tests when changing probes or milestone interpretation.

After builds and tests finish, establish a current release-build baseline and
repeat the shared `.txt`/`.lua` matrix with the same configurations, rotated
editor order, and whole-file save verification. Record readiness, file loading,
and syntax completion separately. Also check quit/idle cost, typing before
initial completion, multiple open targets, and persistent-session opening.

The desired performance result is Lua editing readiness approaching the
same-size plain-text path without worsening ordinary edit or quit latency.
The historical 23.0 ms plain-text median is evidence of potential, not a
promised outcome or a portable CI threshold. Publish new comparisons only
after measurement, retaining the old dated results.

## Documentation and delivery

Implement worker/lifecycle support first, production scheduling second, then
command presentation and startup instrumentation. Each stage needs its own
behavior coverage; the production switch is complete only when both startup
and later full parses use the new lifecycle.

Update `docs/user-guide.md`, `context/reference/helix-keymap-v1.md`, applicable
UI vocabulary, `benchmarks/README.md`, and the startup performance register.
The guide must explicitly permit initial plain-text presentation and describe
temporary structural unavailability. Preserve prior resolved diagnoses and
their original commit identifiers when updating affected records.


## Implementation outcome

Production startup and subsequent full parses now use the same deferred
lifecycle. The first frame exposes authoritative text and ordinary editing;
only syntax-dependent commands await a current tree. Same-generation worker
checkpoints allow edits made during initial parsing to advance incrementally.
Application checks reject late results after text, language, or buffer-lifecycle
changes. Existing inline constructors remain available for deterministic
structural tests.

The independent reviewer completed three passes. The second identified
last-owner tree destruction on the input thread or under the queue mutex;
worker-side retirement and unread-checkpoint reuse resolved it. The third
pass found no additional actionable issues. The comments and correction are
retained in `context/reviews/async_initial_syntax.md`.

The final Linux ordinary and canonical coverage suites each passed 3,029 tests
with 32 ignored. Line coverage is 91.69%, above the unchanged 89% floor.
Formatting, warnings-as-errors Clippy, all 13 benchmark-harness tests, and the
three selected release readiness/reparse performance tests passed. The
`startup-timing` release probe also builds successfully. Native macOS checks
remain for CI or a macOS host; this Linux result is not a macOS measurement.

Final before/after measurements and retained samples are linked from
`context/reference/startup-performance.md`. Timing runs follow all compilation
and test work and verify the complete saved document for every readiness
sample. Early quit is measured separately from settled-document shutdown. The final
50,000-line Lua readiness median is 22.3 ms, versus 160.9 ms before and
29.7 ms for Neovim in the final comparison; the parse itself completes around
156.9 ms in separate instrumented launches. Early quit is 2.7 ms median.

File loading remains synchronous, and all explicit startup targets load before
the first document frame. Accepted current trees can still incur their normal
cleanup cost on settled-document shutdown. An in-progress parser call is not
preempted; its dedicated thread owns its snapshots and does not delay process
exit. These boundaries were deliberate in this change.
