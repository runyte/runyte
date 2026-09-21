# Windows Phase 1

Scope: native standalone editing and file management with independent ConPTY
terminals on `x86_64-pc-windows-msvc`. The acceptance requirements remain in
`context/issues/windows_support.md`. Windows 11 24H2 or later with Windows
Terminal is the initial target; older Windows, ARM64 and MinGW remain untested.
The default shell is `COMSPEC`, falling back to `cmd.exe`. PowerShell 7 and Git
Bash are optional. LSP, persistent sessions, Git, plugins, context access, shell
filters, image paste, system opening, `--wait` and `:quit-here` may remain
explicitly unavailable on Windows for this phase.

Each package receives an independent subagent review after implementation.
Actionable findings are addressed and recorded before starting the next package.
The initial configuration work is included in this sequence. Completion requires
real behavior tests, not only a successful target check. Unix behavior and the
89% coverage floor must be preserved.

## Work packages

Delivery sequence agreed on 2026-09-20: finish Phase 1, incorporate its final
subagent review, commit the implementation and push `feat/windows-support` to
the remote branch of the same name. Immediately begin Phase 2 afterward,
divided into sub-phases and reviewed work packages. Phase 2.1 is integrated Git
support. Detect whether Git is available before enabling the integration;
Windows installations without Git must retain a disabled integration with a
clear unavailable state, without repeated failed subprocess launches or runtime
errors caused by the missing executable. Remaining Phase-2 sub-phases cover
language services, persistent sessions and private storage, plugins/context,
and the remaining integrations, with dependencies refined before each begins.

1. **Configuration foundation** — default path precedence, injected environment
   tests, documentation, and the two initial non-Unix compile repairs.
2. **Native build and service boundaries** — library/binary/test compilation,
   registry-backed unavailable commands, no deferred service processes even
   when configuration enables them, and clear standalone CLI errors.
3. **Console input and text clipboard** — safe bracketed paste, key/text events,
   Unicode, console restoration, native clipboard text and bounded failures.
4. **Local files and filesystem plans** — Windows path/name handling, native
   identity, exclusive rename, recoverable staging and rollback, save/CRLF
   acceptance, and non-destructive refusal at unsupported boundaries.
5. **ConPTY terminals** — native program parsing, executable discovery, bounded
   asynchronous I/O, independent splits, resize, output draining and owned
   process-tree cleanup on close, shutdown and failed launch.
6. **Acceptance and delivery** — native integration tests, full lint/test and
   applicable coverage gates, Windows CI and archives, supported-platform guide
   and explicit remaining limitations. Record unavailable manual evidence.

## Review record

Package 1: reviewed by `review_wp1`; no actionable findings. The reviewer
independently passed all six native path tests with warnings denied and noted
that existing lossy Windows LSP identity reinforces the required LSP exclusion
in package 2. Focused Clippy and formatting pass. Full native checks currently
fail on the existing platform boundaries addressed by package 2.

Package 2: `review_wp2` identified and corrected a Unix test-import regression.
Follow-up findings added platform reasons to generated help and direct-key
coverage, platform-aware synthetic availability assertions, and immediate
binary-opening refusal instead of an unusable program prompt. A portable file
picker fixture now uses a sequence number instead of invalid Windows thread
names; PATH fixture separators use `join_paths`.

The first executable native library suite reached 2,067 passing and 216 failing
tests. Review separated intentionally unavailable-service expectations and
Unix-only private-storage fixtures from actual Phase-1 failures. Save failures
also surface in syntax, language, provider-document and host-wait tests; those
must remain enabled and be fixed with package 4. Terminal/list/Finder/tutorial
failures remain for package 5. Package 6 will finish platform-aware assertions
and run every suite; this intermediate run is not acceptance evidence.

Package 2 review is complete after a second inspection confirmed the findings
were addressed. Native formatting and all-target Clippy pass. Five Windows
boundary tests and 80 focused hint, health and scanner tests pass. Remaining
known runtime failures are assigned above; there is no Windows release claim.

Package 3: `review_wp3` found that Crossterm mouse setup overwrites the input
mode and clears VT input, and that non-character/Alt-numpad records could become
NUL text. Startup now restores VT input after mouse setup with rollback owned
by the console guard; the decoder suppresses non-character records and retains
explicit Ctrl-Space. A follow-up fixed clipboard worker completion publication
so a successful request cannot leave a stale busy state for its successor.

Seven decoder tests and four clipboard tests pass. Clipboard native round trips
use a separate noninteractive window station and desktop, leaving the person's
clipboard untouched. An injected stalled operation verifies bounded waiting and
no accumulation of workers. The native binary test
`tests::windows_console_paste_and_restoration` passed under an isolated ConPTY
console: default mouse setup retains VT input, native records deliver one paste,
reader shutdown handles backpressure, and the exact original input mode returns.
This test is explicitly ignored in a redirected test process and must be run
under the native acceptance harness in package 6.

A running debug editor also accepted a Normal-mode multiline `:quit!` paste as
one text event without executing a command. This exposed ConPTY's conversion of
line endings to CR records; the adapter restores lone CR to LF while retaining
CRLF pairs. Real interactive retesting of the resulting multiline edit remains
part of package 6. Native text clipboard uses CF_UNICODETEXT; image clipboard
remains unavailable. Unix helper ownership and behavior remain in
`src/clipboard/helpers.rs`.

Package 4: `review_wp1` found case-alias directory self-copy, hardlink collision
preflight, and apostrophe completion defects. The fixes resolve existing native
ancestor/entry names without conflating hardlinks, reject self-contained moves
and copies before mutation, and choose double quotes for apostrophe filenames.
Follow-up review passed; all three regressions passed independently. Native
filesystem-plan tests: 27 passed. Save tests cover locked/read-only targets,
conflicts, no-replace races and recoverable temporary cleanup. Short-name alias
handling still needs a dedicated native acceptance case in package 6.

Private-file tests require the normal Windows token: the restricted development
sandbox cannot reopen owner-only temporary files reliably. Normal-token test
runs use fixture-owned temporary directories; production ACLs remain unchanged.

Package 5: `review_wp2` identified early ConPTY cleanup ordering, relative
executable resolution, and cmd.exe shell-payload quoting issues. Console owns
its undrained output pipe until reader startup, relative programs resolve from
the requested directory, and cmd /c or /k takes one decoded command string
framed with /s. Native tests cover nested quotes and a checked-in batch path
with spaces. All findings passed follow-up review.

A real child-argument test exposed a native startup stall when ConPTY received
an extended-prefix cwd. The same executable/arguments/directory work with the
ordinary spelling. Terminal launch now verifies an equivalent ordinary path
by native directory identity and refuses long or verbatim-only cwd names before
creation; cmd.exe also refuses UNC cwd. This limit is documented separately
from editor file opening. Ten ConPTY tests and four command/path tests pass,
including real Unicode/quoted arguments, a relative hardlinked compiled fixture,
resize, independent shells, process-tree cleanup, saturated output shutdown and
four failed-start checkpoints. Native all-target Clippy and formatting pass.

Package 6: `review_wp3` found a job-level GitHub context error, stale-artifact
selection in the console acceptance harness, an incomplete output assertion,
one missing terminal cleanup wait, and a startup-failure restoration coverage
gap. The CI environment moved to the test step; the console test now reexecutes
the current binary with fixture-owned configuration; the editor acceptance keeps
its transcript; native cleanup waits for lifecycle completion; the actual guard
is exercised after an injected startup error. The guide is included in archives
and the terminal reference no longer describes Windows as wholly unsupported.
Follow-up review found no remaining blocking code issue. Historical release
retries retain their original platform matrix and generic download notes.

The integration pass found path completion choosing a native separator despite
an authored slash and duplicating full-path details for equivalent mixed-separator
paths. Completion now preserves the last authored separator, and presentation
compares path components. Existing path-completion tests cover these fixes.
Deferred-service integration fixtures remain enabled on Unix, with explicit
Windows refusal tests. Private-file tests run under the normal Windows token.

Final native validation: `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings` and
`cargo test --locked --workspace --no-fail-fast` pass. The workspace suite has
2,601 passing tests and no failures, including 2,123 library tests and 26 binary
tests. The 31 explicitly ignored entries are subprocess fixtures or existing
performance/manual jobs; the Windows console and editor fixtures are invoked
by nonignored parent tests. `cargo build --release --locked` passes, and the
locally packaged MSVC executable passes `--version`. The review ZIP includes
the guide, configuration and license material and has a SHA-256 checksum.

The real editor acceptance uses isolated ConPTY, an empty temporary workspace,
the initial project prompts and fixture-owned configuration. Normal-mode paste
of `:quit!` remains inert; Insert-mode Unicode multiline paste saves with its LF
intact while the existing CRLF is preserved. The actual terminal guard restores
both exact input flags and original screen contents after normal exit and an
injected startup failure. Clipboard tests use a private window station; physical
keyboard-layout/IME testing and optional PowerShell 7/Git Bash remain unclaimed.

Linux/macOS checks and their 89% coverage gates will run remotely on the reviewed
Phase-1 commit. No passing remote result is claimed before that run completes.
Phase 2 starts with optional integrated Git as recorded in the delivery order.

The first pushed implementation is `3203878`. Remote run `35538777457`
passed Linux coverage, macOS tests, MSRV and the release build floor. Linux
Clippy failed: review by `review_wp1` identified four blank lines after outer
conditional attributes on items excluded from Windows. The lint was reproduced
independently; those blank lines were removed without changing behavior, and
formatting passes. The complete remote gate result remains pending.
