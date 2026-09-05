# Security review — September 5, 2026

Examined commit: `a1d526440dfbb2c75607eeab7963f8356bbcf44b`.

## Implementation follow-up

The working-tree implementation addresses findings 1–4 and the affected
`lru` dependency. The original findings below refer to the examined commit,
not the updated implementation.

- Editor state and the LSP manager begin denied. Production startup reads a
  bounded exact-workspace decision from private per-user cache storage and
  shows the shared choice overlay when no decision is available. Refusals and
  permanent approvals are remembered; a one-time grant removes any earlier
  remembered decision. `:lsp-trust` changes permission later. The host owns
  approval and revocation, which wakes the manager independently of its
  command queue and stops its servers. Events carry the manager's permission
  epoch, and the host receiver discards retired epochs even when revocation
  and reapproval happen before it drains a queued `Ready` and `ApplyEdit`.
  Protocol 49 prevents attachment to a
  host lacking that gate. The warning and workspace identity remain in the
  main choice surface when a narrow pane hides the preview.
- Logs, images, and permission records use `private_storage::Directory`.
  Unix directory walks and file operations are descriptor-relative, reject
  symlinks, and use owned regular files without hard links. File opens are
  nonblocking so FIFOs cannot stall startup. New private directories and files
  use `0700` and `0600`; existing relevant owned files are secured through
  their descriptors. Atomic writes create temporary files exclusively, and
  log rotation reads the held file and publishes through its held directory.
  Existing image entries must match the supplied bytes before reuse.
- The lockfile now uses `lru 0.18.2`. CI installs pinned `cargo-audit 0.22.2`
  and checks the locked graph, treating unsoundness advisories as failures.
- The shared benchmark setup disables LSP in its isolated configuration while
  retaining syntax highlighting. The first-open permission overlay therefore
  cannot intercept readiness edits or affect the quit and idle measurements.

Validation on Linux with Rust 1.97.1:

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo test` passed; 2,888 tests passed in the ordinary suite.
- `cargo llvm-cov --locked --workspace` passed with 96,991 of 105,927 lines
  covered (**91.56%**), above the unchanged 89% floor.
- `cargo-audit 0.22.2` passed against the database revision recorded below,
  including a run with the crates.io yanked-package check enabled.

Behavior coverage includes the two `workspace_lsp_permission_*` tests in
`src/app/tests/language.rs`; the launch-gating and live-revocation tests in
`tests/lsp_client.rs`; `first_open_lsp_choice_is_host_owned_and_refusal_survives_restart`
in `tests/persistent_host.rs`; and the five confinement, file-type,
permissions, rotation, and trust-record tests in `tests/security_storage.rs`.
Existing process fixtures explicitly disable LSP or seed decisions in their
own temporary storage. `revocation_discards_queued_ready_and_edits_before_reapproval_events`
in `tests/lsp_client.rs` covers both asynchronous and nonblocking queue drains.
All 91 Python benchmark tests passed with `RUNYTE_BENCH_BINARY` pointing at the
rebuilt debug editor, including
`test_first_open_with_empty_storage_accepts_and_saves_the_edit` in
`benchmarks/test_startup.py`. That test verifies complete saved edits and clean
exits for plain-text and Lua fixtures; it does not refresh release timings.
Process and socket suites ran outside the filesystem
sandbox; macOS execution remains for CI. No changes were made to the deferred
filesystem-plan race.

## Original review scope

Scope: targeted source review of process execution, language-server startup
and edits, local persistent-session transport, terminal escape handling,
runtime storage, filesystem plans, configuration loading, release workflows,
and locked dependencies. This is not a complete audit of every source line,
the native grammars, or all transitive dependency implementations. Severity
assumes that opening an unfamiliar project should not authorize execution of
its code, and that workspace files can be supplied by another party.

## 1. High: opening Rust code implicitly trusts executable project content

`src/config.rs:792` enables LSP and configures `rust-analyzer` by default,
without overriding its initialization options. `App::attach_lsp` and
`App::lsp_touch` in `src/app/language_workflows.rs:149` start language support
for opened files. `ensure_server` in `src/lsp/mod.rs:1613` checks configuration
and server state, but has no workspace trust decision. The transport starts
the server in the project root (`src/lsp/transport.rs:127`).

With rust-analyzer installed, opening a Rust file in an unfamiliar Cargo
project therefore starts a tool which can execute project-controlled code
with the editor user's privileges. Upstream explicitly documents default
execution of build scripts and procedural macros, plus executable overrides
through Cargo and toolchain configuration. See the
[rust-analyzer security documentation](https://rust-analyzer.github.io/book/security.html).

Verification: the startup path and default configuration were traced in
source and checked against upstream's documented behavior. An end-to-end
malicious-project execution was not run. This is an implicit trust boundary,
not a claim of a bug in rust-analyzer itself; it requires opening an
attacker-controlled project and an installed server.

Recommended fix: introduce a persistent, explicit workspace trust decision
before starting project-aware executable tools. Keep ordinary editing and
syntax highlighting available without trust. Merely disabling build scripts
does not cover the other executable configuration mechanisms. Until that
boundary exists, document that unfamiliar projects must be opened with LSP
disabled in the user-selected configuration; review other automatic tools
under the same trust policy.

## 2. Medium: image-cache temporary files follow symlinks and truncate targets

`pasted_image::store` in `src/pasted_image.rs:120` constructs a predictable
pending name from the image hash and process ID, then opens it using
`File::create` at line 143. The open follows symlinks and truncates an existing
file. Parent directories are also followed by `create_dir_all`. An existing
final pathname is accepted with `path.exists()` without checking either its
type or content.

A party able to populate the workspace cache can place a pending-file symlink
to another file writable by the editor user. A paste then overwrites that
file with the image bytes. The leaf attack requires knowing the image hash
and process ID; a pre-existing symlinked cache parent can redirect storage
without a race. This is not remote execution by itself, and a malicious
process already holding all of the same user's privileges generally has
equivalent write authority.

Verification: compiled the current `pasted_image.rs` and `hash.rs` into a
standalone harness. In a fresh temporary directory, created a sentinel
outside the state directory and symlinked the computed pending pathname to
it. Calling `store` replaced the sentinel bytes with the image bytes. No
repository runtime state or user files were used.

Recommended fix: anchor cache access to a validated private directory, reject
symlinked components, create pending files exclusively with restrictive
permissions, and publish atomically without following substituted objects.
Validate existing cache entries rather than trusting the filename alone.
Add regression coverage for pending-file and parent-directory symlinks.

## 3. Medium: diagnostic logs follow workspace-controlled symlinks

`open_log_file` in `src/log.rs:520` opens its destination with
`create(true).append(true)` and no no-follow or regular-file check. The default
host log has the fixed name `.runyte/host.log`. Logging is initialized at
`src/main.rs:1330`; transport endpoint protections do not protect this open.
`project_root::validate_state_root` only checks overlap with reserved Runyte
user-storage locations, not the log's type or ownership.

A supplied workspace containing a log symlink can redirect diagnostic writes
to a file outside the workspace. The bytes are formatted diagnostic records,
not arbitrary attacker-selected file contents. A FIFO in place of the log
can also block its synchronous open when no reader is present. Explicit log
rotation additionally uses pathname-based copying and truncation and needs
the same object-safety review.

Verification: compiled the current logging module with its existing compiled
dependencies. A temporary `host.log` symlink pointed to an outside sentinel;
starting the logger, emitting a warning, and flushing appended the warning
to that sentinel. The FIFO case is source-derived and was not executed.

Recommended fix: enforce regular files, ownership, and no-follow semantics
for default runtime logs and their parents; use nonblocking open followed by
type validation where needed to reject special files safely. Keep rotation
anchored to verified objects. Cover both initial open and rotated paths.

## 4. Medium: scratch images and diagnostic logs lack private creation modes

The same image-cache and logging paths use default directory/file creation
modes. On a conventional `022` umask, directories become `0755` and files
become `0644`. This allows other local users to read screenshots and log
metadata whenever ancestor-directory permissions permit traversal. A private
home directory limits exposure, but shared workspaces and locations under
publicly traversable roots do not provide that protection.

Verification: the temporary-directory harness observed a newly created log
mode of `0644` and image-cache directory mode of `0755`. Image creation uses
the same default file mode. The logger avoids document text by design, but
its paths and process metadata still have confidentiality value; pasted
screenshots may contain sensitive content.

Recommended fix: create runtime directories as `0700` and logs/images as
`0600`, using creation-time permissions. Validate existing storage before
changing permissions; do not chmod through an unverified symlink. Add tests
under a permissive umask in an isolated child process.

## Existing deferred security issue

The confirmed filesystem-plan check/use race remains present:
`FsPlan::preflight` (`src/fs_plan.rs:1103`) validates paths, while subsequent
operations reopen them and `parent_is_ready` (`src/fs_plan.rs:1192`) follows
directory symlinks. Concurrent replacement can redirect a confirmed operation
outside its reviewed scope or affect replacement contents. This is a medium
local issue with potentially serious data-loss consequences.

The existing [deferred issue](../issues/deferred/fs_plan_symlink_race.md)
already explains why descriptor-relative capabilities, exact-object handling,
and platform-specific trash/rollback behavior need a broader design. This
review did not implement or move that issue, and did not run a race exploit.

## Dependency result

Downloaded the public RustSec advisory database at
`5a0ebedfe8bdd2e295b171f4162f8c977bcad9a5` and inspected advisory entries for
package names in `Cargo.lock`. `cargo-audit` was not installed; this was a
manual advisory comparison, not a successful `cargo audit` run.

`Cargo.lock:599` pins `lru 0.18.1`, affected by
[RUSTSEC-2026-0253](https://rustsec.org/advisories/RUSTSEC-2026-0253).
`cargo tree --locked --offline -i lru` confirms the runtime dependency through
`ratatui-core 0.1.2`. The advisory concerns panic safety in `LruCache::pop()`
and requires a panicking key destructor plus caught unwinding. Inspection of
Ratatui's layout cache found no call to the affected operation and no evident
matching key-destructor behavior. Treat this as a dependency-maintenance
finding, with Runyte exploitability unproven. Update the lockfile to a
compatible patched version, at least `0.18.2`, and add an automated RustSec
check to CI. Do not interpret this manual review as a complete clean bill of
health for the dependency graph.

## Positive controls and limits

The reviewed local transport checks endpoint ownership and private modes and
bounds protocol input. LSP framing bounds message bodies and header lines;
workspace-edit handling checks scope and edit validity. The terminal emulator
explicitly rejects OSC 52 clipboard writes. Configuration is loaded from a
user-selected or per-user path rather than automatically from a project YAML
file. Release workflows pin actions to commits and pass the requested release
tag through an environment variable and validation before use in commands.

A limited working-tree scan for common private-key and token signatures found
no matches. This does not cover Git history or all secret formats. No fuzzing,
macOS runtime validation, native-code audit, or full Rust regression suite was
performed. Only this review document was added; production code and the
lockfile were left unchanged.
