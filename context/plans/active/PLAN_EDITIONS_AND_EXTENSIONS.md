# Terminal and desktop editions, helpers and extension surfaces

## Status

Approved for implementation on 2026-10-10. Recorded on `exp` at `9b83729`, and
revised the same day after an independent review against the repository.
Phases 1–6 are implementation work. Phase 7 merges `exp` into `dev` and
requires the maintainer's explicit go-ahead. Phase 8 builds the signed and
notarized macOS disk image; it is manual and needs the maintainer's Apple
Developer credentials.

## How to execute this plan

The plan is written for an agent that executes it phase by phase.

- Read `AGENTS.md` first. Its working conventions, gates, coverage rule, test
  isolation rules and commit rules apply to every phase. Where this plan and
  `AGENTS.md` disagree, stop and ask rather than choosing.
- Complete phases in order. Each phase ends with its commits and a short
  entry in **Progress** at the end of this file: commit hashes, gate results
  and measurements.
- **Pushing.** Some acceptance below requires CI results, including Windows
  CI that cannot run on a Linux host. `AGENTS.md` allows pushing only when
  asked. Ask the maintainer once, before the first push, whether you may push
  `exp` to `origin` for CI evidence during this plan. Without that
  permission, stop at the first gate that needs CI and report.
- A **stop condition** means: do not work around it; report what happened,
  what you measured and the options you see, then wait.

## Decisions already made

These are settled. Do not reopen them while implementing.

1. **The window is a product, equal to the terminal frontend.** It is not an
   experiment any more. Remove "experiment"/"experimental" wording about the
   window, the native frontend and document preview from user-facing documents
   and current references. Historical plans and resolved issues keep their
   wording. Only links to renamed files are updated in them.
2. **Two editions, named exactly "terminal edition" and "desktop edition".**
   - The **terminal edition** is the terminal frontend only. It is what
     crates.io (`cargo install runyte`), the curl installer and the terminal
     release archives provide. It keeps Rust 1.88 as its minimum version and
     its current dependency graph.
   - The **desktop edition** is a strict superset: the same terminal frontend,
     plus the window, built-in PDF and image viewing and document preview, in
     **one executable**. It ships as release archives and as a macOS app
     bundle on a disk image. It is never published to crates.io.
   - Both editions install an executable named `runyte`. A person installs the
     desktop edition deliberately; documentation says clearly what each
     edition contains, its size, platforms and requirements.
3. **Modes stay runtime flags.** `--editor`, `--ide`, `--mux` and `runed` are
   unchanged and behave identically in both editions. `--window` exists only
   in the desktop edition.
4. **The edition is chosen by which package is built, not by a Cargo
   feature.** The repository becomes one Cargo workspace. The `native` feature
   is removed from the published `runyte` package.
5. **Helpers live inside the desktop executable.** The PDF renderer (Hayro)
   already does. The Blitz document-preview engine moves into the same
   executable. Both keep running in a **separate child process**, started from
   the running executable with a helper-role argument, so a crash, hang or
   memory blow-up on hostile input kills only the child. The sibling
   `runyte-preview-helper` executable, the `RUNYTE_PREVIEW_HELPER` variable and
   the separate `contrib/document-preview` lockfile go away.
6. **Blitz stays on its pinned Git revision** for now. The desktop packages
   are unpublished (`publish = false`), so Git dependencies are allowed there.
   Moving to crates.io Blitz releases is a later, separate change.
7. **Three extension tiers are documented together**: internal helpers,
   `runyte-1` plugins, and the `runyte.context.v1` context profile with its MCP
   adapter. The private client/host protocol is listed alongside them as a
   non-extension wire. No protocol changes are part of this plan.
8. **Terminal release archives are renamed** to `runyte-terminal-<tag>-<target>`,
   to match `runyte-desktop-<tag>-<target>`. The installer stays compatible
   with older tags.
9. **macOS distribution is a signed, notarized and stapled universal disk
   image** (`Runyte-<version>.dmg`) containing `Runyte.app`. It is produced
   manually in Phase 8. The minimum macOS version is **11.0**.

## Current state

Verified on `exp` at `9b83729`. Re-verify anything before relying on it; line
numbers drift.

**Manifest and packaging**

- `Cargo.toml` has one package, `runyte`, with `default = []` and the features
  `startup-timing` and `native`. `native` enables `gpui`, `image`, `tempfile`,
  `async-channel`, `roxmltree`, `arboard`, `x11rb` (Linux), `hayro`,
  `hayro-interpret`, `hayro-syntax` and `log`.
- `[patch.crates-io]` points `block`, `proc-macro-error2` and `gpui` at
  `vendor/`. `include` contains `/vendor/**`, `/assets/fonts/**` and
  `/logo/runyte_logo.svg`.
- `cargo package` drops `[patch.crates-io]` from the published manifest
  (checked by extracting the packaged `Cargo.toml`).
  `cargo install runyte --features native` therefore builds **unpatched**
  GPUI 0.2.2, without the fixes in `vendor/gpui/RUNYTE-PATCH.md`, while the
  crate still ships `vendor/`. The packaged crate is 8.2 MiB compressed.

**Code that depends on `native`**

- `src/main.rs` (9,406 lines) is the binary crate root.
  - It holds `main`, `cli_main`, `run`, the standalone, attached and host
    event loops, Windows console handling, and inline tests (about lines
    7741–9406).
  - It declares the ordinary modules `host_requests` and `windows_host`. It
    declares the `#[path = "tui/…"]` modules `windows_frontend`,
    `windows_console_acceptance`, `windows_frontend_acceptance` and
    `windows_git_acceptance`. It also declares `mod native_frontend` under
    `#[cfg(feature = "native")]`.
- Its uses of `native_frontend`:
  - Before the runtime starts (`main`):
    - the `--native-pdf-helper` dispatch;
    - the `--window` pre-launch checks (refuse Windows, require ide, editor
      or mux mode, `Config::load` for the font size);
    - `native_frontend::launch(cli_main, font_size)`.
  - The terminal-edition refusal in `run`:
    `anyhow::ensure!(!arguments.window, "--window requires cargo build --features native")`.
  - Surface: `Surface::new`, plus `resize`, `draw` and `draw_host_frame`
    through the `AttachedSurface` implementation (about lines 4060–4105). The
    enum wraps `Terminal<GridBackend>`.
  - Events: `Events::new`, async `next`, `defer_frame`, `presented_frame`,
    `is_presentation_acknowledgement` and `accepts_input`. The
    `AttachedEventSource::Native` variant wraps them.
  - Per frame: `update_media`, `capture_media` and `render_frame`. Also
    `attached_media_requests`, `take_close_request`, `dimensions`,
    `begin_attachment` and `receive_media_action`.
- **The library also depends on `native`.** `src/clipboard.rs` (lines 6–17)
  and `src/clipboard/native.rs` use `arboard` and `image` under
  `cfg(all(feature = "native", not(windows)))`. `App` reaches that code
  through `live_clipboard()` (`src/app.rs`). Without it the desktop edition
  silently falls back to `CommandClipboard` and loses X11 selection ownership,
  which `tests/native_window.py --window-controls` exercises.

**The window frontend**

- `src/native_frontend.rs` and `src/native_frontend/` (9,307 lines) form a
  module of the **binary**, not the library.
  - It uses only two `crate::` paths, both its own. Everything else is
    reached through `runyte::`.
  - It embeds files with relative `include_bytes!` paths: fonts in
    `native_frontend.rs` around lines 726–735, the icon in `icon.rs` around
    line 50.
  - It contains no `CARGO_PKG_VERSION`. The private protocol uses the
    library's `CARGO_PKG_VERSION` (`protocol::CLIENT_VERSION`) to match
    clients with hosts, so same-version terminal and desktop builds can
    attach to each other.

**Helpers**

- **PDF.** `src/native_frontend/pdf.rs` spawns `std::env::current_exe()` with
  `--native-pdf-helper <path> <page> …`, in its own process group
  (`command.process_group(0)`, `runyte::process_group::claim_anchored_group`).
  - Its resource limits are applied **inside the helper** by
    `restrict_helper()`.
  - On Linux the address-space limit is an absolute 1 GiB. On macOS it is
    the helper's own virtual size plus 1 GiB.
  - `tests/native_pdf.py` (about line 99) calls `--native-pdf-helper`
    directly.
- **Preview.** `src/native_frontend/preview.rs` `Helper::start` runs
  `RUNYTE_PREVIEW_HELPER`, or `current_exe().with_file_name("runyte-preview-helper")`,
  with `--serve`.
  - It sets `RLIMIT_AS` to 2 GiB in `pre_exec` on Linux.
  - It has **no** process-group handling.
  - Its spawn error tells the user to "Build contrib/document-preview and set
    RUNYTE_PREVIEW_HELPER."
- **The preview crate.** `contrib/document-preview/` is a separate crate,
  `runyte-preview-helper` (629 lines in `main.rs`, `format.rs`, `raster.rs`,
  `assets.rs`).
  - It has its own `[workspace]`, `Cargo.lock`, `check.py`, `fixtures/` and
    `preview.css`. Blitz and Taffy are pinned by Git revision.
  - Besides `--serve`, it has a one-shot mode that `check.py` uses (about
    lines 17 and 79).
  - `tests/native_window.py` (about line 397) finds the running preview child
    by `/proc/<pid>/exe`.
- `:preview` without the window fails with ":preview requires the native window
  (--window)" (`App::preview_document` in `src/app/presentation.rs`).

**Things that read the version or the workflow text**

- `--version` prints `runyte <version>` (`run` in `src/main.rs`). Two
  consumers parse it exactly:
  - `release.yml` (about line 134): `test "$output" = "runyte $VERSION"`;
  - `docs/plugins/compatibility/check_frozen.py` (about line 113):
    `re.fullmatch(r'runyte ([^\s]+)\s*', …)`.
- The version is read from `Cargo.toml` text by
  `docs/plugins/compatibility/candidate.py` (`package_version` requires an
  explicit `version = "…"` under `[package]`, used by `ci.yml` jobs),
  `contrib/packaging/package.py` (about line 116) and `benchmarks/fuzzy.py`
  (about line 404).
- `tests/release_packaging.rs` pins workflow text:
  - the msrv command `cargo +1.88 check --all-targets --locked`;
  - `runyte-${RELEASE_TAG}-${TARGET}.tar.xz`;
  - the `sha256sum` glob;
  - the package's binaries `== ["runyte"]`.

  `release.yml` also requires exactly four `runyte-v*.tar.xz` files.

**Workflows**

- `ci.yml`:
  - gates and the Windows job;
  - coverage: `cargo llvm-cov --locked --workspace`, Linux and macOS, failing
    under 89%; the terminal baseline is 92.06%;
  - `msrv`: `cargo +1.88 check --all-targets --locked`;
  - `security`: `cargo audit --deny unsound`, which covers only the root
    `Cargo.lock`, so Blitz's graph is not audited today;
  - installer, plugin and performance jobs.
- `native-window.yml` (ubuntu-24.04, macos-15):
  - lints and tests with `--features native`, including
    `cargo test --locked --workspace`;
  - two `--ignored --exact` PDF tests named
    `native_frontend::media::tests::…`;
  - builds the helper and packages Linux and macOS bundles.
- `release.yml` builds five terminal archives and one Linux x86-64 desktop
  archive. The desktop job runs when the tag contains
  `contrib/packaging/check_package.py`; no existing tag does.

**Packaging, installer and documents**

- `contrib/packaging/package.py`:
  - `linux --output` makes a directory with `runyte`, `runyte-preview-helper`,
    `runed`, notices, docs and icons;
  - `linux` without `--output` registers a desktop entry;
  - `macos --output` makes `Runyte.app`. Its `CFBundleExecutable` is a
    **shell script** (`runyte-window`) that extends `PATH` and runs
    `runyte --window --editor`. The bundle has no `runed`.
- `install.sh` and `tests/installer/` use archive names
  `runyte-<tag>-<target>.tar.xz` and the matching top-level directory (about
  `install.sh:131`). Other files mentioning archive names include
  `context/reference/releasing.md`, `contrib/packaging/README.md`,
  `skills/runyte-demo-videos/` and resolved issues. Grep again before
  renaming.
- `rust-toolchain.toml` pins 1.97.1; the package declares
  `rust-version = "1.88"`. Hayro needs 1.92.
- `THIRD_PARTY_NOTICES.md` sections that concern only native builds:
  - "Native dependency compatibility patches" (line 7);
  - "Experimental native frontend dependencies";
  - "Bundled native font";
  - "Experimental native document preview helper".
- `context/reference/native-window-experiment.md` says the experiment stays on
  `exp` and must not be merged into `dev` or `main`. Decision 1 supersedes
  this. It is linked from `AGENTS.md`, five resolved issues and
  `context/plans/proposed/PLAN_BROWSER_PANES.md`.

## Target layout

```
Cargo.toml                    workspace root AND the `runyte` package (terminal edition)
src/                          runyte library + thin terminal `main.rs`
crates/runyte-native/         window frontend, media, clipboard, PDF client and helper (library)
crates/runyte-preview/        Blitz document engine (library)
crates/runyte-desktop/        desktop executable and the macOS app launcher
contrib/packaging/            Linux directory and desktop entry, macOS app, DMG, signing
```

- **Root `Cargo.toml`.**
  - Keeps `[package] name = "runyte"` and its explicit
    `version = "<current>"`, so release validation, `candidate.py`,
    `package.py`, `benchmarks/fuzzy.py` and the release runbook keep working.
    A release commit still touches only `Cargo.toml` and `Cargo.lock`.
  - Adds `[workspace] members = [...]` listing each member path
    **explicitly**. A glob such as `crates/*` fails when it matches nothing.
  - Has no `[workspace.package]`, no `default-members` and no explicit
    `resolver`: edition 2024 already implies resolver 3. With a root package
    and no `default-members`, plain `cargo build`, `cargo test` and
    `cargo clippy` act on `runyte` only, so the `AGENTS.md` gates keep meaning
    the terminal edition.
  - Keeps `[patch.crates-io]`, which now affects only desktop members. Path
    crates under `[patch]` do not become workspace members.
- **Members** are `publish = false` and **omit `version`**. Cargo then treats
  them as version `0.0.0`. Every version string Runyte prints or sends comes
  from the `runyte` library's `CARGO_PKG_VERSION`. Member code must never use
  its own `CARGO_PKG_VERSION`. Add a library constant
  (`runyte::VERSION`) if one does not exist, and use it.
- **`runyte`** (published, `rust-version = "1.88"`): the terminal edition, and
  the library everything else uses. No `native` feature, and no dependency on
  anything under `crates/`.
- **`runyte-native`** (`rust-version = "1.92"`): what is now
  `src/native_frontend*`, plus the native clipboard moved out of
  `src/clipboard/`. It depends on `runyte` by path. It does **not** depend on
  `runyte-preview`; it reaches that engine only through the helper process.
- **`runyte-preview`**: the Blitz engine, the formatting, raster and asset
  policy code, and the `serve` and one-shot entry points.
- **`runyte-desktop`** (`rust-version = "1.92"`): the executable.
  - Its binary target is named **`runyte-desktop`**, so it never collides with
    the root `runyte` binary in `target/`. Packaging installs it as `runyte`.
  - On macOS it also builds the app launcher binary, named
    `runyte-app-launcher`. It must not be named `Runyte`, which would collide
    with `target/<profile>/runyte` on a case-insensitive APFS volume.
    Packaging copies it to `Contents/MacOS/Runyte`.

**Consequence to accept and document.** Every Cargo command in the workspace
resolves the whole workspace. Building the terminal edition from a checkout
therefore fetches the Blitz and Taffy Git repositories too: in CI, in the
MSRV and Windows jobs, and when packaging for publication. Installing from
crates.io is unaffected. Record this in the README's build-from-source notes.

**Fallback, not a default.** If the shared `Cargo.lock` cannot keep `runyte`
building on Rust 1.88, stop (see Phase 3). The alternative is a separate
desktop workspace with its own lockfile. That decision belongs to the
maintainer.

All new files carry `// SPDX-License-Identifier: MPL-2.0` (or `#` for Python
and shell). Moved files keep their existing headers.

## Phase 1: process-boundary register (documentation only)

Goal: write down the rules the later phases implement.

1. Create `context/reference/process-boundaries.md`, the register of record
   for each way code or data crosses a process boundary in Runyte. For each
   surface, give:
   - direction (Runyte starts it, or it connects in);
   - who may use it;
   - versioning rule;
   - authority;
   - transport;
   - bounds;
   - source location.

   The surfaces:
   - **Internal helpers.**
     - Private and tied to the exact Runyte build: no version range and no
       compatibility promise.
     - Started only by Runyte, with an argument vector (never a shell), from
       the running executable with `--helper <role>`.
     - Used for engines that parse untrusted input or can exhaust memory or
       time, so failure stays in a killable child with resource limits and
       its own process group.
     - Rule for new helpers: link them into the edition executable that needs
       them. Run in-process only when the engine handles no untrusted input
       and cannot block or exhaust the editor. A separate executable is
       allowed only when the helper needs a different code-signing identity,
       sandbox profile or architecture; record the reason in this file.
     - Current helpers: `pdf` (Hayro) and `preview` (Blitz), both in the
       desktop edition. Poppler is an external fallback program, not a
       helper.
   - **Plugins (`runyte-1`).** Public. Runyte starts them from configuration,
     over stdio, with version ranges and capability grants. Link
     `docs/plugins.md` and `context/reference/plugin-compatibility.md`
     rather than repeating them.
   - **Context profile (`runyte.context.v1`) and `runyte mcp`.** Public.
     External agents connect in over a natively granted socket or named
     pipe. Link the existing documents.
   - **Private client/host protocol.** Used only between bundled Runyte
     processes: persistent-session clients and hosts, `--wait`, and the
     window attachment.
     - Matching is by library version, so terminal and desktop builds of the
       same version interoperate.
     - Host-side behavior must not depend on the host's edition for features
       the client renders.
     - It is not an extension surface.
   - **Choosing a tier.**
     - Third-party code is a plugin or a context client.
     - Isolating an engine inside Runyte is a helper.
     - Raster frames and GPU resources never travel over `runyte-1` (JSON,
       1 MiB message limit).
     - A new public wire needs its own plan.
2. In `AGENTS.md`, add the new reference to the list of development records:
   "required when adding a helper, a plugin capability, a context operation,
   or any other process boundary".
3. In `docs/plugins.md`, add one sentence near the top saying that bundled
   helpers are internal and not plugins. Do not link anything under
   `context/` from `docs/`, which is published.
4. Commit: `Record process boundaries and extension tiers`.

Acceptance: `git diff --check`; documents only.

## Phase 2: move the command-line program into the library behind a window seam

Goal: one CLI implementation that both executables call, with the window
supplied by the desktop executable instead of `cfg(feature)`. The repository
is still one package at the end of this phase, and the `native` feature still
exists, so the move can be checked on its own.

1. **Move the code.**
   - Create `src/cli.rs` and `src/cli/` in the library. Move into it
     everything in `src/main.rs` except a minimal `fn main`: `cli_main`, `run`,
     the event loops, signal and termination handling, `HostSupervisor` use,
     Windows console glue, `StartupTrace` use and the inline tests.
   - Move `host_requests` and `windows_host` under `src/cli/` (they are
     ordinary modules, so they resolve relative to `src/cli/` once declared
     from `src/cli.rs`). Keep the `#[path]` modules pointing at their current
     files, adjusting the relative paths.
   - **Keep the inline tests inline**, beside the code they test. Moving them
     to a `tests/` directory would change the coverage figure, because
     cargo-llvm-cov excludes those directories.
   - The Windows acceptance modules re-run their own test binary with
     module-path constants and `--exact`
     (`windows_console_acceptance.rs` about line 22,
     `windows_frontend_acceptance.rs` about lines 56–64). Update those paths
     to the new module path (for example `cli::…`). A stale path makes the
     child run zero tests and pass. Assert that the child reports exactly one
     passed test, as `ci.yml`'s Windows steps already do.
   - Search all `#[cfg(test)]` code for `current_exe` and re-exec patterns, and
     confirm each still reaches its entry point from the library test binary.
   - Keep `pub` visibility minimal: export only the entry point, the
     `Edition` type and the seam; everything else is `pub(crate)` or
     private.
   - Move `--version` printing into the library, using `runyte::VERSION`.
2. **Define the seam** in `src/cli/window.rs`. It must cover every call site
   listed under "Code that depends on `native`", including the clipboard.
   Design constraints:
   - **Surface and events:** use library enums whose window variant holds a
     boxed trait object, alongside the existing terminal variants. For
     example, extend `AttachedEventSource` with a `Window(Box<dyn WindowEvents>)`
     variant. The terminal-frontend paths must not gain allocation or
     dynamic dispatch. Async `next` on the window path may return a boxed
     future; that path already crosses threads.
   - **Launch:** the desktop executable owns the main thread. The seam's
     launch method takes the CLI worker as
     `Box<dyn FnOnce() -> anyhow::Result<()> + Send>`, not a `fn` pointer, so
     the worker can capture the frontend.
   - **Pre-launch checks** (Windows refusal, mode check, font-size config
     load) move into the library's CLI entry point. They run only when a
     window frontend is supplied and `--window` is given.
   - **Clipboard:** the seam supplies
     `fn system_clipboard(&self) -> Option<Box<dyn SystemClipboard>>` (name it
     after the existing trait or type in `src/clipboard.rs`).
     `live_clipboard()` uses it when present, and the current fallback
     otherwise. This applies in the **host** process as well as the client:
     a desktop-edition host serving a window needs the native clipboard.
   - Pass the implementation explicitly, from `main` into `cli::main` and
     down the call chain, as `Option<&'static dyn WindowFrontend>`. Where it
     must reach `App` (the clipboard), store it in the value `App` already
     receives at construction. Do not add a process-global unless explicit
     passing is impossible somewhere. If you add one, say why in the Progress
     entry, and set it in `main` before any thread or runtime starts.
3. **Edition.** Define `pub enum Edition { Terminal, Desktop }`, passed to
   `cli::main` and stored where the CLI and `App` can read it. Phase 5 uses
   it for messages. In this phase it only replaces `cfg(feature = "native")`
   checks that ask whether a window is available.
4. **`src/main.rs`** becomes, apart from its header and lint attributes:
   - `cfg(not(feature = "native"))`: `runyte::cli::main(Edition::Terminal, None)`;
   - `cfg(feature = "native")` (temporary, removed in Phase 3): the current
     `--native-pdf-helper` dispatch, then
     `runyte::cli::main(Edition::Desktop, Some(&native_frontend::WINDOW))`.

   The early `mcp` dispatch and its `runed` exception stay before CLI
   parsing, in the library entry point.
5. **No user-visible change in this phase.** Messages, flags, exit codes and
   the startup ordering recorded in `context/reference/startup-performance.md`
   are unchanged.
6. **Gates:**
   - `cargo fmt --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test`
   - `cargo clippy --features native --all-targets -- -D warnings`
   - `cargo test --features native --bin runyte native_frontend`
   - `cargo +1.88 check --all-targets --locked`
   - Coverage: before the first edit, record
     `cargo llvm-cov --locked --workspace` on the base commit, then again
     after the phase. The result must stay at or above the 89% floor, and
     must not drop by more than 0.10 percentage points; investigate any
     larger drop. Report both totals in the commit body.
   - On an X11 display reserved for the test, run
     `tests/native_window.py --window-controls` with the native build. The
     clipboard must still pass.
   - Windows CI green for the phase's head commit (see "Pushing").
7. Commit: `Move the command-line program into the library`.

## Phase 3: Cargo workspace and the desktop executable

Goal: the edition is chosen by the package built, and the published crate
carries nothing of the desktop edition.

### Commit 1: `Prepare the repository for a Cargo workspace`

1. Add `[workspace]` with `members = []` to the root manifest. This is valid
   with a root package and changes nothing that is built.
2. Make every command that will later depend on "only `runyte`" say so
   explicitly. Then adding members in commit 2 cannot pull GPUI into jobs
   that lack its system libraries:
   - change the canonical coverage command from
     `cargo llvm-cov --locked --workspace` to
     `cargo llvm-cov --locked --package runyte`, in `ci.yml`, `AGENTS.md` and
     `context/reference/test-coverage.md` together;
   - change the msrv job to `cargo +1.88 check -p runyte --all-targets --locked`;
   - update `tests/release_packaging.rs` to the new command strings.
3. Coverage proof: run the old and the new command on this commit. They
   measure the same set, so the totals must be identical. Record both. The
   89% floor does not change.
4. Gates: the `AGENTS.md` three, msrv and coverage.

### Commit 2: `Build the desktop edition as its own executable`

This is one commit because the steps cannot be separated without a broken
manifest: removing the feature's dependencies breaks the feature, and the
root binary cannot depend on a member that depends on the root library.

1. **`crates/runyte-native/`.**
   - `git mv` `src/native_frontend.rs` to `crates/runyte-native/src/lib.rs`
     and `src/native_frontend/` to `crates/runyte-native/src/`.
   - `git mv src/clipboard/native.rs` into the crate. Implement the
     clipboard part of the seam from it, and remove the `native` cfgs from
     `src/clipboard.rs`.
   - Move the `native` feature's dependencies, with identical version
     requirements and features, into this crate's manifest. That includes the
     Linux-only `x11rb` under the same `target.'cfg(...)'` table. Each one
     must leave the root manifest.
   - Fix the relative `include_bytes!` paths (fonts, icon) so they point at
     the same files. The files stay where they are.
   - Implement the Phase 2 seam as `pub static WINDOW` (or a constructor).
2. **`crates/runyte-desktop/`.**
   - `src/main.rs` keeps `--native-pdf-helper` working (Phase 4 renames it),
     then calls `runyte::cli::main(Edition::Desktop, Some(&runyte_native::WINDOW))`.
   - `[[bin]] name = "runyte-desktop"`.
3. Add both crates to `members` explicitly.
4. Delete the `native` feature and every `cfg(feature = "native")` from the
   `runyte` package. `grep -rn 'feature = "native"' src/` returns nothing.
   The root `src/main.rs` is just `runyte::cli::main(Edition::Terminal, None)`.
   `--window` in the terminal edition must keep failing early, with a message
   that no longer mentions Cargo features; Phase 5 sets the final wording.
5. **Trim the published package.**
   - Remove `/vendor/**`, `/assets/fonts/**` and `/logo/runyte_logo.svg` from
     `include` only if `cargo package -p runyte --list` and the dry run below
     show they are not needed. The terminal ASCII logo stays.
   - Run `cargo publish -p runyte --locked --dry-run` (add `--allow-dirty`
     only before committing).
   - Extract the packaged `Cargo.toml` and confirm it has no `gpui`, `hayro`,
     `arboard`, `x11rb` or `native`.
   - Record the packaged size in the commit body.
   - Update `context/reference/releasing.md` steps 7 and 10 to
     `cargo publish -p runyte --locked --dry-run` and
     `cargo publish -p runyte --locked`.
6. **Dependency graph gates.** These are repeated in Phase 4.
   - `cargo tree -p runyte -e normal,build --locked --target all` before and
     after must be identical. Explain any difference in the commit body.
   - `cargo +1.88 check -p runyte --all-targets --locked` must pass. If a
     desktop dependency raised a crate that `runyte` shares in the common
     lock beyond 1.88, pin it with `cargo update -p <crate> --precise <version>`
     and record why. If no pin works, this is a stop condition. Never raise
     `rust-version`.
   - `cargo audit --deny unsound` must pass. A new advisory is a stop
     condition.
7. **CI.**
   - Rename `native-window.yml`'s workflow to "Desktop edition".
   - Lint with
     `cargo clippy --locked -p runyte-native -p runyte-desktop --all-targets -- -D warnings`.
   - Test with `cargo test --locked -p runyte-native -p runyte-desktop`,
     replacing the old `cargo test --locked --workspace`.
   - The two ignored PDF tests: the module path loses its `native_frontend::`
     prefix once the module is the crate root. Use the new full names with
     `--ignored --exact`, and assert that the output says exactly
     `1 passed`. A filter that matches nothing exits successfully.
   - Build with `cargo build --locked -p runyte-desktop`, and pass
     `target/debug/runyte-desktop` to packaging and acceptance scripts.
   - `release.yml` desktop job: `cargo build --release --locked -p runyte-desktop`.
     The helper build stays until Phase 4.
   - Update `tests/release_packaging.rs` for every workflow string it pins
     that changed.
8. **Desktop coverage.**
   - In the "Desktop edition" workflow on Linux, measure
     `cargo llvm-cov --locked --package runyte-native --package runyte-desktop`.
   - Check whether the report includes `runyte` library files. If it does,
     exclude them with `--ignore-filename-regex` so the figure describes only
     desktop code.
   - Record the first baseline under a dated heading in
     `context/reference/test-coverage.md`.
   - Set the enforced floor three percentage points below the baseline,
     rounded down to a whole number. The terminal edition's 92.06% baseline
     and 89% floor are the precedent.
   - Phase 4 adds `runyte-preview` to this measurement and records the new
     baseline. The floor may rise then, but never fall.
9. **`AGENTS.md`.**
   - Replace the `src/native_frontend…` architecture entry with the three
     crates, and add `src/cli/`. Move the clipboard sentence accordingly.
   - Under the gates, add the desktop gates, run whenever a change can affect
     the desktop edition:
     `cargo clippy -p runyte-native -p runyte-desktop -p runyte-preview --all-targets -- -D warnings`
     and `cargo test -p runyte-native -p runyte-desktop -p runyte-preview`.
     Until Phase 4, `runyte-preview` does not exist; add it in Phase 4.
10. **Notices.** Move "Native dependency compatibility patches" under a
    desktop-edition heading in `THIRD_PARTY_NOTICES.md`. Phase 4 rewrites the
    rest.
11. Gates:
    - the `AGENTS.md` three;
    - the desktop gates;
    - msrv, terminal coverage and desktop coverage;
    - on an X11 display reserved for the test:
      `tests/native_window.py --binary target/debug/runyte-desktop` with
      `--window-controls` and the document-preview options;
    - CI green on Linux, macOS and Windows.

**Stop conditions:** raising the terminal edition's `rust-version`; adding a
dependency to `runyte`; lowering any coverage floor; a new advisory.

## Phase 4: helpers inside the desktop executable

Goal: one executable, one helper launcher and one helper argument convention.

1. **Move the preview crate.**
   - `git mv contrib/document-preview crates/runyte-preview`, and add it to
     `members`.
   - Delete its `[workspace]` table and its `Cargo.lock`. Keep the same pinned
     Blitz and Taffy revisions. Compare the deleted lock with the workspace
     lock, and list every crate whose version changed.
   - Turn `main.rs` into `lib.rs`. Expose `serve(input, output)` and
     `run_once(args, input, output)`, which keeps today's one-shot behavior
     and arguments for `check.py`. Keep `format`, `raster` and `assets` as
     modules, with their tests.
   - Move `check.py`, `fixtures/` and `preview.css` with the crate, and fix
     their path references.
2. **The helper convention** lives in `crates/runyte-native/src/helper.rs`.
   - **Invocation:** `runyte --helper <role> [role arguments…]`, which must be
     the first argument. Roles: `pdf`, `preview`. `--helper` is never shown in
     `--help`. The terminal edition does not recognize it, so it is an
     ordinary unknown option there.
   - **One launcher** builds the `Command` for a role:
     - Executable: `/proc/self/exe` on Linux, so a helper still starts the
       same build after an update replaced the file; `current_exe()`
       elsewhere.
     - Argument vector only; stdin, stdout and stderr piped.
     - Every helper in its own process group, claimed with
       `runyte::process_group` exactly as `pdf.rs` does today. Preview gains
       this.
     - Resource limits stay where they are: preview's `RLIMIT_AS` (2 GiB,
       Linux) in `pre_exec`; PDF's limits inside the helper through
       `restrict_helper()`, unchanged. Its macOS limit is relative to the
       helper's own size, so it must not be measured from the parent.
   - **Replaced-executable check** (all platforms except Linux, which
     `/proc/self/exe` already covers):
     - When the editor starts, `runyte-desktop`'s `main` records the file
       identity of `current_exe()` with one `metadata` call: device, inode,
       size and modification time. This happens after helper-role dispatch,
       so helpers do not pay for it.
     - Before each spawn, the launcher compares the current identity with
       the recorded one. If they differ, it does not spawn, and the caller
       shows: "Runyte was updated on disk; restart it to use PDF viewing" (or
       "document preview").
     - Make the identity source injectable, so tests can simulate a
       replacement without touching real files.
3. **Rewire both clients through the launcher.**
   - `pdf.rs` spawns `--helper pdf …`. `preview.rs` spawns
     `--helper preview --serve`.
   - Delete the `RUNYTE_PREVIEW_HELPER` lookup, the sibling lookup and the
     "Build contrib/document-preview" message. Replace that message with
     "Document preview could not start: <error>".
4. **`runyte-desktop` `main`** dispatches, before anything else (including the
   `mcp` check):
   - `--helper pdf` to `runyte_native::pdf::helper_main`;
   - `--helper preview --serve` to `runyte_preview::serve(stdin, stdout)`;
   - any other `--helper preview …` to `runyte_preview::run_once`.

   Remove `--native-pdf-helper`. It is internal and tied to the same build,
   so nothing needs to stay compatible.
5. **Tests.**
   - Unit tests for the launcher:
     - the argument vector;
     - role parsing and an unknown role;
     - process-group setup;
     - refusal with the restart message when the recorded identity differs.
   - An integration test in `crates/runyte-desktop/tests/` that runs
     `env!("CARGO_BIN_EXE_runyte-desktop")` with `--helper preview`,
     `--helper preview --serve` and `--helper pdf` against checked-in
     fixtures:
     - a frame or page comes back;
     - an unknown role exits non-zero.

     Use temporary directories, set `XDG_CONFIG_HOME` to fixture-owned
     storage, and never execute a file the test wrote.
   - `check.py`: take `--binary <path>` and run `<path> --helper preview …`.
   - `tests/native_window.py`:
     - drop `--preview-helper` and `--packaged-preview-helper`;
     - find the preview child by its command line (`--helper preview`)
       instead of `/proc/<pid>/exe`, which is now the editor's own
       executable.
   - `tests/native_pdf.py`: call `--helper pdf`.
6. **Packaging and CI.**
   - `package.py`: remove `--preview-helper` and `preview_helper()`.
     Bundles contain one Runyte executable.
   - `check_package.py`:
     - check the exact file list: one executable and, on Linux, the `runed`
       link;
     - run engine acceptance through `runyte --helper preview`.
   - `release.yml` and the desktop workflow: delete the
     `--manifest-path contrib/document-preview/Cargo.toml` builds.
   - Add `-p runyte-preview` to the desktop clippy and test commands, in CI
     and in `AGENTS.md`.
   - Add `runyte-preview` to the desktop coverage measurement, and record the
     new baseline and floor (Phase 3, step 8).
7. **Graph gates again,** because this merges the largest graph
   (Blitz/Stylo): the terminal `cargo tree` diff must still be identical, and
   `cargo +1.88 check -p runyte` and `cargo audit --deny unsound` must pass.
   Same stop conditions as Phase 3.
8. **Notices.** Rewrite the preview section of `THIRD_PARTY_NOTICES.md`: the
   engine is linked into the desktop executable and shares the workspace
   lock, and the terminal edition still contains none of it. Rename the
   "Experimental native …" headings to "Desktop edition …". The file is still
   published with the crate, so say plainly at the top of each desktop
   section that it applies only to the desktop edition.
9. **Measure** and record in the commit body and in
   `context/reference/native-window-experiment.md` (Phase 5 renames it):
   - stripped release `runyte-desktop` size, before and after;
   - cold `runyte-desktop --version` time;
   - window startup, with the existing `benchmarks/` or
     `tests/native_window.py` measurements;
   - on Linux, the PDF helper's peak address space (`VmPeak` from
     `/proc/<pid>/status`) under `--helper pdf` on the largest fixture,
     against its absolute 1 GiB limit. If the larger executable leaves less
     than 25% headroom, stop and report;
   - the terminal edition's `benchmarks/` startup figures, which must not
     change.
10. Commit: `Run document preview inside the desktop executable`.

## Phase 5: edition identity and how it is communicated

Goal: someone using either edition can always tell which one they have and
what the other offers.

1. **`--version`** prints `runyte <version> (terminal edition)` or
   `runyte <version> (desktop edition)`. Update its two parsers:
   - `release.yml`'s smoke test: expect the new form when the tag contains
     `crates/runyte-desktop/Cargo.toml`, and the old exact form otherwise, so
     reruns of old tags still pass. Update `tests/release_packaging.rs`.
   - `docs/plugins/compatibility/check_frozen.py`: accept an optional
     ` (… edition)` suffix, and keep accepting the old form.

   Grep `tests/`, `examples/`, `skills/`, `benchmarks/`, `src/mcp*` and `docs/`
   for any other parser. Note the change in Progress, so the next release's
   changes list includes it.
2. **`--help`.**
   - The desktop edition lists `--window` with a one-line description. The
     terminal edition omits it.
   - Both end with a line naming the edition. The terminal edition's line
     adds that the desktop edition exists, with the README anchor URL:
     `https://github.com/runyte/runyte#editions`.
3. **Messages.** Keep them in one table in the library, so both editions
   share the wording:

   | Situation | Message |
   | --- | --- |
   | Terminal edition, `--window` | `--window is part of the Runyte desktop edition; this is the terminal edition. See https://github.com/runyte/runyte#editions` |
   | Desktop edition, `--window` on Windows | `The desktop edition's window is not available on Windows yet` |
   | `:preview` when the current attachment is not a window (either edition, standalone or persistent) | `:preview needs the Runyte window: use the desktop edition with --window` |

   - The `:preview` message depends only on whether the **current attachment**
     is a window, never on the host's edition. A persistent host may be
     either edition while the window client is the desktop edition (see the
     process-boundary register). Add a test that serves a window attachment
     from a host built as the terminal edition, using the existing
     headless/attachment test support, and confirm `:preview` captures
     rather than refusing.
   - Check what opening a PDF or image does in the terminal frontend. If it
     shows a window-only message, add a matching row.
   - Each row needs a test at its behavior boundary: the CLI tests for flags,
     the app tests for commands. Messages go through the existing
     action-failure path; do not add new UI.
4. **README.**
   - Add an `## Editions` section near the top, before installation. It is a
     two-column comparison of:
     - what is included;
     - executable size, as measured in Phase 4;
     - platforms;
     - requirements (desktop: Vulkan on Linux, the glibc floor, macOS 11 or
       later; Poppler as an optional PDF fallback);
     - how to install each edition.
   - Say that the desktop edition also runs in a terminal. Say that
     persistent sessions require the same Runyte version for every client
     and host on the machine.
   - Rewrite the "experimental native window" section as "Desktop edition".
     Use `cargo run -p runyte-desktop -- --window` in the build-from-source
     instructions, and note the Git fetch consequence from "Target layout".
5. **`docs/user-guide.md`.**
   - Give the same edition explanation under "Install and run".
   - Rename the experimental native window section to "Desktop edition".
   - Update its anchors and every link to them. Grep `docs/`, `README.md`
     and `src/`; help, manual and tutorial text may link to it.
6. **References.**
   - `git mv context/reference/native-window-experiment.md context/reference/desktop-edition.md`.
     Rewrite its status paragraph: a product, not an experiment. Remove the
     "do not merge into `dev` or `main`" rule and note that Decision 1 of this
     plan supersedes it. Keep the technical content.
   - Fold `context/reference/native-document-preview.md` into it, or keep it
     separate with "experiment" wording removed. Choose whichever leaves
     fewer duplicated statements.
   - Update every link to renamed files: `AGENTS.md`,
     `contrib/packaging/README.md`, the moved preview `README.md`,
     `context/plans/README.md`, `PLAN_BROWSER_PANES.md` and the five resolved
     issues. Change link targets only; leave their prose alone.
7. Commit: `Name the terminal and desktop editions throughout`.

## Phase 6: release packaging for both editions

Goal: every published artifact names its edition, and the desktop edition is
released wherever it has passed acceptance.

The **edition marker** for release logic is the presence of
`crates/runyte-desktop/Cargo.toml` in the tagged tree. Releases are cut only
from `main` after Phase 7, so any tag containing it also contains Phases 4–6.

1. **Rename the terminal archives.**
   - New names: `runyte-terminal-<tag>-<target>.tar.xz` and
     `runyte-terminal-<tag>-x86_64-pc-windows-msvc.zip`. The top-level
     directory inside becomes `runyte-terminal-<version>-<target>`.
   - `release.yml`: choose the names from the edition marker, the same way it
     already chooses a historical tag's platform set. Rerunning an old tag
     must reproduce its original asset names exactly. Update the "exactly
     four `runyte-v*.tar.xz`" check and the `sha256sum` glob for both
     layouts.
   - Update `tests/release_packaging.rs`.
   - `install.sh`:
     - Download `SHA256SUMS` first. Pick the archive name it lists for the
       target: the new name if present, otherwise the old one, otherwise
       fail clearly. Never infer the name from a version number.
     - Use the matching top-level directory when extracting (today around
       line 131).
     - Before replacing an existing `runyte`, run it with `--version`. If it
       reports the desktop edition, refuse with a message explaining that
       the curl installer installs the terminal edition, and pointing to the
       desktop archive. Never silently replace a desktop install with the
       terminal edition. An unrecognized or old-format version string counts
       as terminal.
   - Installer tests: new layout, old layout, a manifest listing neither,
     and refusal over an installed desktop edition. Use the checked-in
     `src/fixtures/stand-in` for the installed executable, as `AGENTS.md`
     requires.
   - Update `context/reference/releasing.md` (asset list, counts, `SHA256SUMS`
     coverage, verification commands). Update `contrib/runyte.ps1` and
     `skills/runyte-demo-videos/` if they name archives.
2. **Linux desktop archives.**
   - Keep x86-64 on Ubuntu 24.04.
   - Add `aarch64-unknown-linux-gnu` on `ubuntu-24.04-arm`, with the same
     lavapipe and Xvfb acceptance. If that acceptance cannot run there, do not
     publish the ARM64 desktop archive. Record the blocker in
     `desktop-edition.md` and continue; it is not a stop condition.
   - Contents: `runyte`, the `runed` link, notices, docs, icons and the
     registration script. `check_package.py` checks the exact file list.
3. **macOS app bundle.**
   - **Native launcher.** Replace the shell-script `CFBundleExecutable` with
     `runyte-app-launcher`, a macOS-only binary target in `runyte-desktop`.
     It:
     - finds the sibling `runyte` in its own directory;
     - appends `/opt/homebrew/bin` and `/usr/local/bin` to `PATH`, as the
       script does;
     - drops a leading `-psn_*` argument;
     - `exec`s `runyte --window --editor <args>`.

     Use only `std` and `libc`, and add unit tests for the argument and
     `PATH` construction. A shell script as the main executable is a signing
     and notarization hazard, because its signature is kept in extended
     attributes that copies can lose.
   - **Bundle layout.** `package.py macos` copies the launcher to
     `Contents/MacOS/Runyte` and the editor to `Contents/MacOS/runyte`, and
     adds a relative `runed` → `runyte` symbolic link in `Contents/MacOS/`
     for command-line use.
   - **`Info.plist`.** Keep the identifier `com.runyte.Runyte` and
     `CFBundleExecutable = Runyte`. Add:
     - `LSMinimumSystemVersion = 11.0`;
     - `NSHumanReadableCopyright`;
     - `LSApplicationCategoryType = public.app-category.developer-tools`.
   - **Universal build.**
     - Set `MACOSX_DEPLOYMENT_TARGET=11.0` in the environment of the
       `cargo build` steps, not in packaging.
     - Build `runyte-desktop` and `runyte-app-launcher` for
       `aarch64-apple-darwin` and `x86_64-apple-darwin`, and combine each
       pair with `lipo -create`.
     - `package.py macos` accepts `--binary` and `--launcher` (each already
       universal), and checks that `lipo -archs` lists both architectures.
   - **Binary checks** in `check_package.py` for a macOS bundle:
     - `vtool -show-build` reports `minos 11.0` for both architectures of
       both executables;
     - `otool -L` lists only `/usr/lib/` and `/System/` libraries. The CI
       runner has Homebrew libraries installed, so an accidental Homebrew
       link must fail here.
   - **Release workflow.** Add a macOS desktop job on `macos-15`. It:
     - builds both architectures from the validated tag;
     - makes the universal `Runyte.app`;
     - runs `check_package.py` headless engine acceptance and the binary
       checks (window acceptance stays manual: hosted macOS runners cannot be
       relied on for the GPU);
     - uploads `Runyte-<version>-unsigned.app.zip`, made with
       `ditto -c -k --keepParent`, as a **workflow artifact only, never a
       release asset**.

     The `publish` job does **not** depend on this job, so a macOS desktop
     failure blocks only Phase 8, not the terminal and Linux releases.
4. **Disk image and signing tools** (written now, run manually in Phase 8).
   - `contrib/packaging/package.py dmg --app <Runyte.app> --output <Runyte-<version>.dmg>`:
     - refuses an existing output;
     - stages the app with an `Applications` → `/Applications` symbolic link
       and the README;
     - runs `hdiutil create -volname "Runyte <version>" -srcfolder <stage> -format UDZO <output>`,
       without `-ov`.

     Unit tests mock `hdiutil`. A macOS CI step builds an unsigned DMG from
     the CI bundle, attaches it with `hdiutil attach -nobrowse -readonly`,
     checks the app is inside, and detaches.
   - `contrib/packaging/sign_macos.py` has the subcommands `app`, `dmg` and
     `verify`, and options `--identity "Developer ID Application: <Name> (<TEAMID>)"`
     and `--notary-profile <keychain profile>`. It never reads credentials
     from files, environment variables or the repository.
     - **`app`:**
       1. Sign inside-out: `Contents/MacOS/runyte`, then
          `Contents/MacOS/Runyte`, then the bundle. Each uses
          `codesign --force --options runtime --timestamp --sign <identity>`.
          The `runed` link needs no signature. Never use `--deep` when
          signing.
       2. Zip with `ditto -c -k --keepParent`.
       3. `xcrun notarytool submit <zip> --keychain-profile <profile> --wait --output-format json`.
          Require `"status": "Accepted"`. Otherwise run
          `xcrun notarytool log <id> --keychain-profile <profile>`, print the
          log and fail.
       4. `xcrun stapler staple <app>`.
     - **`dmg`:**
       1. `codesign --force --timestamp --sign <identity> <dmg>` (no
          `--options runtime` for a disk image).
       2. Notarize as above.
       3. `xcrun stapler staple <dmg>`.
     - **`verify`:**
       - `codesign --verify --deep --strict --verbose=2 <app>`;
       - `spctl --assess --type execute -vv <app>`;
       - `spctl --assess --type open --context context:primary-signature -vv <dmg>`;
       - `xcrun stapler validate` on both.
     - `--dry-run` prints every command without running it. Unit tests cover
       the command sequence and the notarization status handling in dry-run
       mode.
     - No entitlements file at first. Metal, child processes and PTYs need
       none under the hardened runtime. If Phase 8 finds a failure that only
       an entitlement fixes, add the minimum one and record why.
5. Update `context/reference/releasing.md`:
   - the asset list for new tags;
   - the desktop jobs;
   - that the macOS DMG and its `.sha256` are attached manually after step
     12 (Phase 8), and are not in the workflow's `SHA256SUMS`, which reruns
     rewrite;
   - the asset counts in steps 12–13.
6. Commits, one per concern:
   - `Name terminal release archives by edition`;
   - `Publish desktop archives for Linux ARM64` (only if acceptance passes);
   - `Launch the macOS app through a native executable`;
   - `Build a universal macOS desktop app in the release workflow`;
   - `Add disk image and signing tools for the macOS desktop edition`.

## Phase 7: merge into `dev` (stop point)

Do not start without the maintainer's explicit go-ahead in the conversation.
Prepare, then wait.

1. Prepare a merge report:
   - commit and file totals (`git log --oneline dev..exp | wc -l`,
     `git diff --stat dev...exp`);
   - conflicts from a trial merge in a temporary `git worktree` that is
     removed afterwards;
   - the state of every CI workflow on the `exp` head;
   - what users of `main` will see change at the next release.
2. After the go-ahead, the maintainer decides who performs the merge and
   when.
3. Releases follow `context/reference/releasing.md`. This plan bumps no
   versions.

## Phase 8: signed and notarized macOS disk image (manual)

This runs on the maintainer's Mac with the maintainer's Apple Developer
account. An assisting agent prepares commands and checklists. It never asks
for, stores, or logs credentials, certificates or passwords.

### One-time setup (maintainer)

1. Apple Developer Program membership.
2. A **Developer ID Application** certificate in the login keychain.
   `security find-identity -v -p codesigning` lists
   `Developer ID Application: <Name> (<TEAMID>)`.
3. A notarization profile:
   `xcrun notarytool store-credentials runyte-notary --apple-id <id> --team-id <TEAMID>`
   (it prompts for an app-specific password), or the App Store Connect API
   key form of the same command.
4. Xcode command-line tools: `xcode-select -p` prints a path.

### Per release

1. Download `Runyte-<version>-unsigned.app.zip` from the release workflow run
   for the exact tag, and confirm the run's source SHA matches the tag.
   If that is not possible, build from a clean checkout of the tag with the
   workflow's commands and environment (including
   `MACOSX_DEPLOYMENT_TARGET=11.0`), and note it in the release notes draft.
2. `ditto -x -k Runyte-<version>-unsigned.app.zip ./stage`
3. `python3 contrib/packaging/sign_macos.py app --identity "…" --notary-profile runyte-notary ./stage/Runyte.app`
4. `python3 contrib/packaging/package.py dmg --app ./stage/Runyte.app --output ./Runyte-<version>.dmg`
5. `python3 contrib/packaging/sign_macos.py dmg --identity "…" --notary-profile runyte-notary ./Runyte-<version>.dmg`
6. `python3 contrib/packaging/sign_macos.py verify ./stage/Runyte.app ./Runyte-<version>.dmg`
7. **Hands-on acceptance before upload,** on Apple silicon at least, and
   Intel if available. Record results in `context/reference/desktop-edition.md`
   under a dated heading, listing what was and was not tested.
   1. Give a copy of the DMG the quarantine attribute a browser download
      would set:
      `cp Runyte-<version>.dmg /tmp/q.dmg && xattr -w com.apple.quarantine "0081;$(printf %x $(date +%s));Safari;" /tmp/q.dmg`.
      Open it, drag Runyte to Applications, and launch it from Finder. Only
      the normal first-launch confirmation may appear: no "damaged" or
      "unidentified developer" warning.
   2. The window opens in editor mode. Check:
      - fonts and Retina scaling;
      - typing, splits and the command palette;
      - `:terminal` starts the login shell with the Homebrew `PATH`;
      - a PDF through Hayro, and through Poppler on a file Hayro falls back
        on, if Poppler is installed;
      - `:preview` on Markdown, HTML and SVG;
      - clipboard copy with Cmd-c and Ctrl-Shift-c.
   3. Command-line use. Link into a directory you own that is on `PATH`
      (create `~/.local/bin` if needed; `/usr/local/bin` may not exist on
      Apple silicon and needs `sudo`):
      `ln -s /Applications/Runyte.app/Contents/MacOS/runyte ~/.local/bin/runyte` and
      `ln -s /Applications/Runyte.app/Contents/MacOS/runyte ~/.local/bin/runed`.
      Then check:
      - `runyte --version` reports the desktop edition;
      - `runed` opens in editor mode;
      - `runyte --mux` attaches in Terminal, and `runyte --window --mux`
        attaches to the same persistent session.
   4. While Runyte runs, replace `/Applications/Runyte.app` with another
      build. Opening a new PDF must show the restart message from Phase 4's
      replaced-executable check.
8. Upload: `shasum -a 256 Runyte-<version>.dmg > Runyte-<version>.dmg.sha256`,
   then `gh release upload v<version> Runyte-<version>.dmg Runyte-<version>.dmg.sha256`.
   Download it once through a browser and repeat step 7.1 against the real
   download.
9. README and user guide: macOS installation points to the DMG, and
   describes the optional command-line links from step 7.3.

Until the first DMG is accepted, the README says that macOS desktop builds are
available only as unsigned bundles built from source.

## Out of scope

- A Windows desktop edition. The window frontend is not implemented on
  Windows. Parity there needs its own plan: a GPUI Windows backend, ConPTY
  with the window, packaging and code signing.
- Installing the desktop edition with the curl installer (`--edition desktop`).
  It is a reasonable follow-up once desktop archives exist on more platforms.
  This plan only makes the installer refuse to replace a desktop install.
- Splitting the core library (`app`, `workspace`, `buffer` and so on) into more
  crates. Revisit once the workspace exists, if compile times or ownership give
  a concrete reason.
- Moving Blitz from its Git pin to crates.io releases.
- Finder "open with" events, file associations, Homebrew casks, Flatpak,
  AppImage, Windows signing, automatic updates and update checks.
- Browser panes (`context/plans/proposed/PLAN_BROWSER_PANES.md`). That plan
  must place its CEF processes under the Phase 1 helper rules. CEF needs its
  own helper executables, which the rules allow with a recorded reason.
- Any change to `runyte-1`, `runyte.context.v1`, the MCP tools or the private
  protocol version.

## Risks and how to detect them

| Risk | Detection | Response |
| --- | --- | --- |
| Shared lock raises a crate `runyte` uses beyond Rust 1.88 | msrv job, `cargo tree` diff (Phases 3 and 4) | Pin with `cargo update --precise`; otherwise stop |
| Blitz/Stylo and GPUI need incompatible versions of a shared crate | Phase 4 build | Duplicate major versions are acceptable; an unresolvable graph is a stop condition |
| Moving `main.rs` breaks tests that re-run their own binary | `cargo test`, Windows CI, "1 passed" assertions | Fix the module paths; never skip |
| Windows-only code moved in Phase 2 compiles only on Windows | Windows CI | Merge only on green Windows CI |
| Desktop edition silently loses the native clipboard | `native_window.py --window-controls` (Phases 2 and 3) | The clipboard is part of the seam; the test must pass |
| `/proc/self/exe` unavailable in a sandbox or container | Helper tests in CI | Fall back to `current_exe()` with the replaced-executable check |
| Larger executable squeezes the PDF helper's 1 GiB limit | Phase 4 `VmPeak` measurement | Stop below 25% headroom |
| Git dependencies make terminal builds fetch Blitz | Any workspace command | Accepted and documented; installing from crates.io is unaffected |
| Archive rename breaks installs of old versions | Installer tests for both layouts | Choose the name from `SHA256SUMS` |
| Hardened runtime blocks something at run time | Phase 8 hands-on run | Add the minimum entitlement with its reason |
| Coverage figure moves when the command changes | Phase 3, commit 1 | The measured set is identical; totals must match |

## Progress

Record each phase here: commit hashes, gate results, measurements, and
anything deferred with its reason. When every phase is done, or the
maintainer explicitly defers the remaining ones, move this file to
`completed/` and update `context/plans/README.md`.

### Phase 1 — 2026-10-10

Implemented in `d403915` (`Record process boundaries and extension tiers`).
The process-boundary register distinguishes the approved helper convention
from the pre-migration implementation. Independent subagent review found no
substantive issues; its pixel-limit precision correction was applied.
`git diff --check` passed. Documentation only; no runtime measurements or Rust
gates apply to this phase.

### Phase 2 — 2026-10-10

Implemented in `e977f35` (`Move the command-line program into the library`).
The CLI and its inline tests now live in the library; terminal paths retain
concrete surfaces and event streams, and the window adapter supplies boxed
window services. Edition and frontend are passed explicitly through startup
and host clipboard construction; no new process-global state was added.
Moved ordinary module support directories with their parent modules. Direct
`#[path = "tui/…"]` declarations still resolve from `src/` and keep their paths.

Independent subagent review was repeated after corrections. It caught one
remaining Windows inline-test reexecution name, which now has the `cli::`
prefix. Successful fixture wrappers check that one test passed. Regression
coverage verifies supplied clipboard use by desktop editor and host
construction without a window attachment. The frontend architecture guard
now names `cli.rs` and `cli/window.rs` instead of `main.rs`; core input
consumers remain subject to the same restrictions.

Linux x86-64 local gates passed:

- `cargo fmt --check`;
- `cargo clippy --all-targets -- -D warnings` and `cargo test`;
- `cargo clippy --features native --all-targets -- -D warnings`;
- `cargo test --features native --bin runyte native_frontend`: 87 passed,
  three existing ignored tests;
- `cargo +1.88 check --all-targets --locked`;
- `tests/native_window.py --window-controls`, standalone and `--mux`, on a
  dedicated Xvfb display with lavapipe: clipboard copy/paste, terminal paste,
  font resizing, saved font configuration and persistent parent-editor wait;
- `git diff --check`.

Canonical `cargo llvm-cov --locked --workspace`, Rust 1.97.1 and
cargo-llvm-cov 0.9.1:

| Measure | Before (`b6cdddf`) | After (`e977f35`) |
| --- | ---: | ---: |
| Covered lines | 149,882 | 150,029 |
| Total lines | 162,805 | 163,040 |
| Line coverage | 92.06% | 92.02% |

The decrease is 0.04 percentage points, within the 0.10 limit; the enforced
89% floor is unchanged.

Phase 2 CI follow-up: the maintainer authorized pushes of `exp` for CI and
continuation through the implementation before the manual Apple DMG work.
Pushed `f7146ab`; its Windows lint step failed. Independent review found
`ratatui::Terminal` remained imported on Windows after all uses became
Unix-only. Limited that import to Unix; formatting, denied-warning all-target
Clippy and the complete local test suite pass again. The corrective diff was
independently reviewed and committed as `bdabad3` (`Limit the relocated
terminal import to Unix`). Its [Windows CI job](https://github.com/runyte/runyte/actions/runs/38055937917/job/114224473471)
passed before Phase 3 began, including the full suite and required clipboard,
MCP, restart and language-server acceptance.

The same CI run exposed a scheduler race in a macOS clipboard regression:
its delayed marker could be written before cleanup resumed. `c50fe4f`
(`Observe clipboard descendants only after helper cleanup`) replaces the
delay with a post-cleanup release barrier, preserving the 100 ms helper
timeout and 500 ms observation window. Independent review and the focused
regression passed. The full Rust gates run again with Phase 3 preparation.

The macOS performance job initially measured one edit at 16.224125 ms against
the existing 16 ms budget; all other cases passed. Rerunning that job without
changes passed. No budget changed. Native preview acceptance exposed incorrect
selection coordinates at raster scale 2; `2ef8b04` fixes CSS-pixel conversion
and verifies exact selection and link hit testing across five scale/zoom pairs.
Independent review and the full real-engine acceptance script passed.

### Phase 3 — workspace preparation, 2026-10-10

Implemented preparation in `8cec197` (`Prepare the repository for a Cargo workspace`).
Added the empty workspace and made terminal CI, coverage documentation and
MSRV checks select `runyte` explicitly. Independent review passed. Formatting,
denied-warning all-target Clippy, the complete test suite and Rust 1.88
all-target locked checks passed.

Both canonical instrumented test runs passed on Linux x86-64, Rust 1.97.1,
cargo-llvm-cov 0.9.1. `--workspace` covered 150,015 of 163,044 lines (92.01%);
`--package runyte` covered 150,025 of 163,044 (92.02%). The ten-line variation
is in asynchronous/process execution. To verify the measured set exactly,
both selectors were reported with `--no-run` against the same retained
profiles, without rebuilding: all 261 per-file and total rows were identical,
including 150,025 covered of 163,044 lines. The 89% floor is unchanged.

### Phase 3 — desktop crate split, 2026-10-10

Moved the native adapter, clipboard and PDF implementation into unpublished
`runyte-native`, with unpublished `runyte-desktop` supplying the executable.
Both members omit version and declare Rust 1.92. The root retains Rust 1.88,
has no native feature or desktop dependencies, and keeps plain Cargo commands
scoped to the terminal package. Internal native implementation items remain
crate-private; only the window seam and PDF helper entry cross crates.

Repeated independent review found no remaining issues after narrowing two
crate-root enums and the remaining implementation exports. Formatting, root
and desktop denied-warning Clippy, the full root suite, desktop tests (87
passed, three existing ignored), and Rust 1.88 all-target checks passed. The
first root run timed out in the existing worktree attachment test; its exact
isolated rerun passed in 0.89 seconds, and the complete rerun passed unchanged.
Both exact ignored PDF tests each report one passed test. Real PDF helper
acceptance passes with Poppler absent from PATH. Dedicated Xvfb/lavapipe window
acceptance passes for standalone and persistent clipboard/font controls,
including parent-editor wait.

The terminal normal/build dependency tree is byte-for-byte identical to the
pre-split graph. Audit passes with the same nine existing allowed warnings;
no new advisory or dependency version was introduced. The publish dry run
passes, including verification of the extracted terminal crate. Its generated
manifest has no `gpui`, `hayro`, `arboard`, `x11rb` or native feature. The
archive is 3,100,772 bytes (777 files, 13.9 MiB unpacked). Native fonts, patches
and the SVG logo are removed from the published include list; the terminal
ASCII logo remains.

Linux terminal coverage is 150,030/163,044 lines (92.02%), with the 89% floor
unchanged. Desktop coverage excludes absolute root `src/` paths and measures
2,297/5,255 lines (43.71%); its new floor is 40%, following the specified
three-percentage-point headroom rounded down. The desktop workflow enforces
that floor and uses the two explicit member packages for lint/tests/build.
Packaging unit tests and `git diff --check` pass.

Real document-preview window acceptance also passes: composition, overlays,
Escape, clipboard, scrolling with a paused renderer, resize, scaling, SVG and
selected-section capture. Platform CI is the remaining phase gate.
