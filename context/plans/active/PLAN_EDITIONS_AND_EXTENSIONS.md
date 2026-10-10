# Terminal and desktop editions, helpers and extension surfaces

## Status

Approved for implementation on 2026-10-10. Recorded on `exp` at `9b83729`.
Phases 1–6 are implementation work. Phase 7 merges `exp` into `dev` and
requires the maintainer's explicit go-ahead. Phase 8 builds the signed and
notarized macOS disk image; it is manual and needs the maintainer's Apple
Developer credentials.

The plan is written for an agent that executes it phase by phase. Read
`AGENTS.md` first; its working conventions, gates, coverage rule and commit
rules apply to every phase below. Where this plan and `AGENTS.md` disagree,
stop and ask rather than choosing.

## Decisions already made

These are settled. Do not reopen them while implementing.

1. **The window is a product, equal to the terminal frontend.** It is not an
   experiment any more. Remove "experiment"/"experimental" wording about the
   window, the native frontend and document preview from user-facing documents
   and current references. Historical plans and resolved issues keep their wording.
2. **Two editions, named exactly "terminal edition" and "desktop edition".**
   - The **terminal edition** is the terminal frontend only. It is what
     crates.io (`cargo install runyte`), the curl installer's default and the
     terminal release archives provide. It keeps Rust 1.88 as its minimum
     version and its current dependency graph.
   - The **desktop edition** is a strict superset: the same terminal frontend,
     plus the window, built-in PDF/image viewing and document preview, all in
     **one executable**. It ships as release archives and as a macOS app
     bundle/disk image. It is never published to crates.io.
   - Both editions install an executable named `runyte` (plus `runed`). A
     person installs the desktop edition deliberately. Documentation must say
     clearly what each contains, its size, platforms and requirements.
3. **Modes stay runtime flags.** `--editor`, `--ide`, `--mux` (and `runed`) are
   unchanged and work identically in both editions. `--window` exists only in
   the desktop edition.
4. **The edition is chosen by which binary crate is built, not by a Cargo
   feature.** The `native` feature is removed from the published `runyte`
   package.
5. **Helpers live inside the desktop binary.** The PDF renderer (Hayro) already
   does. The Blitz document-preview engine moves into the same binary. Both
   keep running in a **separate child process**, started from the running
   executable with a helper-role argument, so a crash, hang or memory blow-up
   on hostile input only kills the child. The sibling `runyte-preview-helper`
   executable, the `RUNYTE_PREVIEW_HELPER` variable and the separate
   `contrib/document-preview` lockfile all go away.
6. **Blitz stays on its pinned Git revision** for now. The desktop crate is
   unpublished (`publish = false`), so Git dependencies are allowed there.
   Moving to crates.io Blitz releases is a later, separate change.
7. **Three extension tiers are documented as one taxonomy**: internal helpers,
   `runyte-1` plugins and the `runyte.context.v1` context profile with its MCP
   adapter. The private client/host protocol is listed alongside them as a
   non-extension wire. No protocol changes are part of this plan.
8. **Terminal release archives are renamed** to `runyte-terminal-<tag>-<target>`
   to match `runyte-desktop-<tag>-<target>`, with installer compatibility for
   older tags.
9. **macOS distribution is a signed, notarized, stapled universal disk image**
   (`Runyte-<version>.dmg`) containing `Runyte.app`. It is produced manually in
   Phase 8.

## Current state

These facts were verified on `exp` at `9b83729`. Re-verify any of them before
relying on it; line numbers drift.

- `Cargo.toml` defines one package, `runyte`, with `default = []`,
  `startup-timing` and `native` features. `native` enables `gpui`, `image`,
  `tempfile`, `async-channel`, `roxmltree`, `arboard`, `x11rb`, `hayro*` and
  `log`. `[patch.crates-io]` points `block`, `proc-macro-error2` and `gpui` at
  `vendor/`. `include` contains `/vendor/**`, `/assets/fonts/**` and
  `/logo/runyte_logo.svg`.
- `cargo package` drops `[patch.crates-io]` from the published manifest.
  (Checked by extracting `Cargo.toml` from the packaged `runyte-0.4.0.crate`.)
  So `cargo install runyte --features native` today builds **unpatched**
  upstream GPUI 0.2.2, without the fixes in `vendor/gpui/RUNYTE-PATCH.md`,
  while the crate still ships `vendor/`. The packaged crate is 8.2 MiB
  compressed.
- `src/main.rs` (9,406 lines) is the binary crate root. It holds `main`,
  `cli_main`, `run`, the standalone, attached and host event loops, Windows
  console handling and their tests. It declares `mod native_frontend` under
  `#[cfg(feature = "native")]`. Its native call sites are:
  - `main`: the `--native-pdf-helper` dispatch and the `--window` launch through
    `native_frontend::launch(cli_main, font_size)`;
  - `run`: `anyhow::ensure!(!arguments.window, "--window requires cargo build --features native")`
    when built without `native`;
  - the event loops: `native_frontend::Surface::new`, `Events::new`,
    `update_media`, `capture_media`, `render_frame`, `take_close_request`,
    `dimensions`, `begin_attachment`, `receive_media_action`, and the
    `AttachedEventSource::Native` variant.
- `src/native_frontend.rs` plus `src/native_frontend/` (9,307 lines: `animation`,
  `cells`, `grid`, `icon`, `input_queue`, `interactions`, `media`, `pdf`,
  `preview`, `viewport`, `tests`) is a module of the **binary**, not the
  library. It uses only two `crate::` paths, both its own. Everything else it
  reaches through `runyte::` (`layout`, `keymap`, `workspace`, `ui`, `tui`,
  `input`, `app`, `protocol`, `media`, `process_group`, `document_preview`,
  `command`, `key_hints`, `config`).
- PDF rendering: `src/native_frontend/pdf.rs` spawns `std::env::current_exe()`
  with `--native-pdf-helper <path> <page> …`; `main` dispatches to
  `native_frontend::pdf::helper_main` before anything else.
- Document preview: `src/native_frontend/preview.rs` `Helper::start` runs
  `RUNYTE_PREVIEW_HELPER` or `current_exe().with_file_name("runyte-preview-helper")`
  with `--serve`. On Linux it sets `RLIMIT_AS` to 2 GiB in `pre_exec`. On failure
  it says "Build contrib/document-preview and set RUNYTE_PREVIEW_HELPER."
- `contrib/document-preview/` is a separate crate, `runyte-preview-helper`
  (629 lines in `main.rs`, `format.rs`, `raster.rs`, `assets.rs`). It has its
  own `[workspace]`, `Cargo.lock`, `check.py`, `fixtures/` and `preview.css`.
  Blitz and Taffy are pinned by Git revision.
- `cargo audit` in the `security` CI job checks only the root `Cargo.lock`. The
  preview helper's lockfile is not audited today.
- `:preview` without the window fails with ":preview requires the native window
  (--window)" (`App::preview_document` in `src/app/presentation.rs`).
- `--version` prints `runyte <version>` (`run` in `src/main.rs`).
  `check_package.py` runs it; nothing found parses its output, but check again.
- Workflows:
  - `ci.yml` runs gates, coverage (`cargo llvm-cov --locked --workspace`, fails
    under 89%, Linux and macOS), `msrv` (`cargo +1.88 check --all-targets --locked`),
    `security`, Windows, installer, plugin and performance jobs.
  - `native-window.yml` (ubuntu-24.04, macos-15) lints and tests with
    `--features native`, builds the helper and packages Linux and macOS bundles.
  - `release.yml` builds five terminal archives and one Linux x86-64 desktop
    archive. The desktop job only runs for tags that contain
    `contrib/packaging/check_package.py`.
- `contrib/packaging/package.py`:
  - `linux --output` builds a directory with `runyte`, `runyte-preview-helper`,
    `runed`, notices, docs and icons. `linux` without `--output` registers a
    desktop entry.
  - `macos --output` builds `Runyte.app`. Its `CFBundleExecutable` is a
    **shell script**, `runyte-window`, that extends `PATH` and runs
    `runyte --window --editor`.
- `install.sh` and `tests/installer/` resolve archive names of the form
  `runyte-<tag>-<target>.tar.xz`. Other files that mention release archive
  names include `context/reference/releasing.md`, `contrib/packaging/README.md`,
  `skills/runyte-demo-videos/` and resolved issues. Grep again before renaming.
- `rust-toolchain.toml` pins 1.97.1. The package declares `rust-version = "1.88"`.
  Hayro needs 1.92, so the native graph already needs a newer Rust.
- `THIRD_PARTY_NOTICES.md` has sections "Experimental native frontend
  dependencies", "Bundled native font" and "Experimental native document
  preview helper". The last says the helper has its own `Cargo.lock` and is
  not linked into the terminal build.
- `context/reference/native-window-experiment.md` says the experiment stays on
  `exp` and must not be merged into `dev` or `main`. Decision 1 supersedes that;
  Phase 5 rewrites it.

## Target layout

```
Cargo.toml                    workspace root AND the `runyte` package (terminal edition)
src/                          runyte library + thin terminal `main.rs`
crates/runyte-native/         GPUI window frontend, media, PDF client and helper (library)
crates/runyte-preview/        Blitz document engine and its `serve` loop (library)
crates/runyte-desktop/        desktop executable: helper-role dispatch + CLI with window
contrib/packaging/            Linux directory/desktop entry, macOS app, DMG, signing
```

- **Root `Cargo.toml`**:
  - Keeps `[package] name = "runyte"` so release validation, `cargo publish`
    and every existing path keep working.
  - Adds `[workspace] members = ["crates/*"]` and `resolver = "3"`.
  - Adds `[workspace.package] version = "<current>"`. Every member uses
    `version.workspace = true`, so a release commit still touches only
    `Cargo.toml` and `Cargo.lock`.
  - Keeps `[patch.crates-io]`, which now only affects desktop members.
  - Gets no `[workspace] default-members`. With a root package, plain
    `cargo build/test/clippy` already act on `runyte` only, which keeps the
    `AGENTS.md` gates meaning the terminal edition.
- **`runyte`** (published): terminal edition. No `native` feature and no
  dependency on anything under `crates/`. `rust-version = "1.88"`.
- **`runyte-native`** (`publish = false`, `rust-version = "1.92"`): what is now
  `src/native_frontend*`, depending on `runyte` by path. It does **not**
  depend on `runyte-preview`; it reaches the engine only through the helper
  process.
- **`runyte-preview`** (`publish = false`): the Blitz engine. It exposes
  `serve(input: impl Read, output: impl Write) -> Result<…>` and the
  formatting, raster and asset policy functions used by tests.
- **`runyte-desktop`** (`publish = false`, `rust-version = "1.92"`): the
  executable. Its Cargo binary target is named **`runyte-desktop`**, so it
  never collides with the root `runyte` binary in `target/`. Packaging
  installs it as `runyte`. On macOS the crate also builds the small app
  launcher described in Phase 6.

All new files carry `// SPDX-License-Identifier: MPL-2.0`. Moved files keep
their existing headers.

## Phase 1: extension and process-boundary reference (documentation only)

Goal: write down the rules the later phases implement, so every later change
can be checked against them.

1. Create `context/reference/process-boundaries.md`. It is the register of
   record for each way code or data crosses a process boundary in Runyte. For
   each surface give: direction (Runyte starts it, or it connects in), who may
   use it, versioning rule, authority, transport, bounds and source location.
   - **Internal helpers.**
     - Private and tied to the exact Runyte build: no version range and no
       compatibility promise.
     - Started only by Runyte, with an argument vector (never a shell), from
       the running executable with a helper-role argument.
     - Used for engines that parse untrusted input or can exhaust memory or
       time, so failure is contained in a killable child with resource limits.
     - Rule for new helpers: link into the edition binary that needs them.
       Run in-process only when the engine handles no untrusted input and
       cannot block or exhaust the editor. A separate executable is allowed
       only when the helper must have a different code-signing identity,
       sandbox profile or architecture; record the reason in this file.
     - Current helpers: `pdf` (Hayro) and `preview` (Blitz), both desktop
       edition. Poppler is an external program fallback, not a helper.
   - **Plugins (`runyte-1`).** Public. Runyte starts them from configuration
     over stdio, with version ranges and capability grants. Link
     `docs/plugins.md` and `context/reference/plugin-compatibility.md`; do not
     repeat their content.
   - **Context profile (`runyte.context.v1`) and `runyte mcp`.** Public. External
     agents connect in over a natively granted socket or named pipe. Link the
     existing documents.
   - **Private client/host protocol.** Used between bundled Runyte processes
     only: persistent-session clients and hosts, `--wait`, the window
     attachment. It requires matching builds. It is not an extension surface.
   - **Choosing a tier.** A short decision list:
     - Third-party code must be a plugin or a context client.
     - Engine isolation inside Runyte is a helper.
     - Large raster frames or GPU resources never travel over `runyte-1`
       (1 MiB message limit, JSON).
     - A new public wire needs its own plan.
2. In `AGENTS.md`, add a bullet to the list of development records: "required
   when adding a helper, a plugin capability, a context operation or any new
   process boundary".
3. In `docs/plugins.md`, add one sentence near the top pointing out that
   bundled helpers are internal and not plugins. Link nothing under
   `context/` from `docs/`, which is published.
4. Commit: `Record process boundaries and extension tiers`.

Acceptance: documents only. `cargo fmt --check` is unaffected. Run
`git diff --check`.

## Phase 2: move the command-line program into the library behind a window seam

Goal: one CLI implementation that the terminal and desktop executables both
call. The window becomes something the desktop binary supplies, not a
`cfg(feature)` inside it. The repository is still one package at the end of
this phase, and the `native` feature still exists, so the change can be
checked in isolation.

1. Create `src/cli.rs` and `src/cli/` in the library. Move into it everything
   in `src/main.rs` except a minimal `fn main`: `cli_main`, `run`, the event
   loops, `TerminationSignals`, `HostSupervisor` users, Windows console glue,
   `StartupTrace` use and the tests.
   - Move the `#[path = "tui/…"]` modules declared in `main.rs`
     (`windows_frontend`, `windows_host`, the Windows acceptance modules) with
     it, keeping their files where they are or moving them under `src/cli/`.
   - Keep `pub` visibility minimal: export only the entry point and the seam
     below; everything else is `pub(crate)` or private.
2. Define the seam in `src/cli/window.rs`: an object-safe trait (suggested name
   `WindowFrontend`) implemented by the desktop edition. It must cover exactly
   the call sites listed under "Current state" — launch, surface creation,
   event source, per-frame media update/capture/render, close request,
   dimensions, attachment start and received media actions — and nothing else.
   - Pass the implementation explicitly from `main` into `cli::main` and down
     the call chain as `Option<&'static dyn WindowFrontend>`. A process-global
     `OnceLock` is acceptable only if explicit passing proves invasive. If
     used, it must be set in `main` before any thread or runtime starts, and
     the plan's progress notes must record why.
   - Generic types such as the Ratatui terminal type may need an enum or a
     boxed surface. Keep the per-frame path free of allocation that did not
     exist before.
3. Define `pub enum Edition { Terminal, Desktop }` in the library, passed to
   `cli::main` and stored where `App` can read it (Phase 5 uses it for
   messages). In this phase it only replaces `cfg(feature = "native")` checks
   that ask "is a window available".
4. `src/main.rs` becomes, apart from SPDX and lints:
   - `cfg(not(feature = "native"))`: call `runyte::cli::main(Edition::Terminal, None)`.
   - `cfg(feature = "native")` (temporary, removed in Phase 3): the current
     `--native-pdf-helper` dispatch, then
     `runyte::cli::main(Edition::Desktop, Some(&native_frontend::WINDOW))`.
5. Keep behavior identical:
   - No user-visible change in this phase. Messages, flags, exit codes and
     the startup order recorded in `context/reference/startup-performance.md`
     stay as they are. The early `mcp` dispatch and the `runed` exception stay
     before CLI parsing.
   - Tests that re-execute their own test binary (search for `current_exe` in
     `#[cfg(test)]` code and for "reexecuted helper") now run from the library
     test binary. Confirm each still finds its entry point.
6. Gates:
   - `cargo fmt --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test`
   - `cargo clippy --features native --all-targets -- -D warnings`
   - `cargo test --features native --bin runyte native_frontend`
   - `cargo +1.88 check --all-targets --locked`
   - Coverage: before the first edit of this phase, record
     `cargo llvm-cov --locked --workspace` on the base commit. After the
     phase, it must be at or above the floor. Report both totals in the
     commit body.
   - Windows: this host is Linux, so Windows CI is the evidence. Do not merge
     the phase until the `windows` job is green for its head commit. If CI is
     not available to the executor, stop and ask.
7. Commit: `Move the command-line program into the library`.

## Phase 3: Cargo workspace and the desktop executable

Goal: the edition is chosen by the crate built, and the published crate
carries nothing of the desktop edition.

1. Convert the root manifest as described in "Target layout". Move the version
   to `[workspace.package]` and set `version.workspace = true` on `runyte`.
   Confirm the `validate` job's `cargo metadata … .version` query in
   `release.yml` still resolves the root package's version, and add a test
   (workflow lint or script check) if it can be checked locally.
2. Create `crates/runyte-native/`:
   - Move `src/native_frontend.rs` and `src/native_frontend/` into it with
     `git mv`, so history follows.
   - Move the `native` feature's dependencies, with identical version
     requirements and features, from the root manifest into its
     `[dependencies]`, including the Linux-only `x11rb`. Each must leave the
     root manifest.
   - Replace the remaining `crate::native_frontend` paths.
   - Implement the Phase 2 trait as a `pub static` or a constructor.
3. Create `crates/runyte-desktop/`:
   - `src/main.rs` dispatches helper roles first (Phase 4 finalizes the names;
     keep `--native-pdf-helper` working until then), then calls
     `runyte::cli::main(Edition::Desktop, Some(…))`.
   - `[[bin]] name = "runyte-desktop"`.
4. Remove the `native` feature and every `#[cfg(feature = "native")]` from the
   `runyte` package. `grep -rn 'feature = "native"' src/` must return nothing.
   `--window` in the terminal edition must keep failing early, with a message
   that no longer mentions Cargo features. Phase 5 writes the final wording.
5. Trim the published package:
   - Remove `/vendor/**`, `/assets/fonts/**` and `/logo/runyte_logo.svg` from
     `include` **only if** `cargo package -p runyte --list` and a build of the
     packaged crate show they are no longer needed. The terminal ASCII logo
     stays.
   - Run `cargo publish -p runyte --locked --dry-run` (with `--allow-dirty`
     only before committing).
   - Extract the packaged `Cargo.toml` and confirm it has no `gpui`, `hayro`,
     `arboard`, `x11rb` or `native` entries.
   - Record the packaged size in the commit body.
6. Dependency graph checks:
   - **Terminal graph:** compare `cargo tree -p runyte -e normal,build --locked --target all`
     before and after. It must be identical; any difference needs a written
     reason in the commit body.
   - **MSRV:** run `cargo +1.88 check -p runyte --all-targets --locked`. A
     desktop dependency may raise a version that `runyte` shares in the common
     `Cargo.lock`. If that breaks Rust 1.88, pin the shared crate with `cargo
     update -p <crate> --precise <version>` and record why; do not raise
     `rust-version`. Change the CI `msrv` job to name `-p runyte` explicitly.
   - Run `cargo audit --deny unsound` on the workspace lock.
7. Update CI:
   - `native-window.yml`: rename the workflow to "Desktop edition". Use
     `cargo clippy -p runyte-native -p runyte-desktop --all-targets --locked -- -D warnings`
     and `cargo test -p runyte-native --locked`. Turn the two `--ignored --exact`
     PDF tests into `-p runyte-native` invocations with the same test names.
   - Build with `cargo build -p runyte-desktop --locked` and pass
     `target/debug/runyte-desktop` to packaging and acceptance scripts.
   - `release.yml` desktop job: build `-p runyte-desktop --release`. Keep the
     helper build until Phase 4 removes it.
   - `ci.yml`: the gates, coverage, MSRV and Windows jobs keep building only
     `runyte`.
8. Coverage:
   - Change the canonical command from `cargo llvm-cov --locked --workspace`
     to `cargo llvm-cov --locked --package runyte`. Before the split
     `--workspace` meant exactly this package, so the measured set is
     unchanged. Prove it by running both commands on the last pre-split
     commit and the new one on the split commit. Totals must agree within the
     variation recorded in `context/reference/test-coverage.md`.
   - Update `AGENTS.md`, `context/reference/test-coverage.md` and the CI job
     together. The 89% floor does not change.
   - Add a **desktop coverage** measurement in the "Desktop edition" workflow:
     `cargo llvm-cov --locked --package runyte-native --package runyte-desktop`
     on Linux. Record the first measured baseline in the coverage register
     under a dated heading. Set its enforced floor to the whole percentage
     below that baseline (round down, then subtract one), matching how the
     terminal floor relates to its baseline. Per `AGENTS.md`, never lower it
     afterwards.
9. Update `AGENTS.md`:
   - Replace the `src/native_frontend…` architecture entry with the three
     crates, and add `src/cli/`.
   - Under the gates, add the desktop gates, run when the change can affect
     the desktop edition:
     `cargo clippy -p runyte-native -p runyte-desktop --all-targets -- -D warnings`
     and `cargo test -p runyte-native`.
10. Commits, in order. Each must pass its gates:
    1. `Make the repository a Cargo workspace`: manifest and version
       inheritance only.
    2. `Move the window frontend into its own crate`: `git mv`, the
       dependencies and the seam implementation.
    3. `Build the desktop edition as its own executable`: the desktop crate,
       feature removal, CI, coverage and documentation.

Stop conditions: if a step needs to raise the terminal edition's
`rust-version`, add a dependency to `runyte`, or lower any coverage floor,
stop and report instead.

## Phase 4: helpers inside the desktop executable

Goal: one executable, one helper launcher and one helper argument
convention.

1. Move `contrib/document-preview/` to `crates/runyte-preview/` with `git mv`.
   - Delete its `[workspace]` table and its `Cargo.lock`. The workspace lock
     now carries its graph, with the same pinned Blitz and Taffy revisions
     and, where possible, the same versions as the deleted lock. Compare the
     two and list any version that changed.
   - Convert `main.rs` into `lib.rs`, exposing `serve`. Keep `format`, `raster`
     and `assets` as modules and keep their tests.
   - Move `check.py`, `fixtures/` and `preview.css` with the crate, and update
     their path references.
2. Define the helper convention in `runyte-native` (`src/helper.rs` in that crate):
   - **Invocation:** `runyte --helper <role> [role arguments…]`. It must be the
     first argument. Roles are `pdf` and `preview`. `--helper` is never shown
     in `--help`, and the terminal edition does not recognize it (it is an
     ordinary unknown option there).
   - **One launcher function** builds the `Command` for a role:
     - Executable: on Linux, `/proc/self/exe`, so a helper still starts after
       an update has replaced the file on disk. Elsewhere, `std::env::current_exe()`.
     - Argument vector only; stdin, stdout and stderr piped.
     - Role-specific resource limits applied in `pre_exec` where the platform
       supports them: preview keeps 2 GiB address space on Linux; PDF keeps
       its current limits. Move the existing limit code; do not re-derive the
       values.
     - Process-group handling as the existing helpers do.
   - **Replaced executable check:** helpers are tied to the exact build, so
     an editor must never talk to a helper from a newer file that an update
     put in its place.
     - On Linux, `/proc/self/exe` already guarantees the same file.
     - Elsewhere, `runyte-desktop` `main` records the file identity of
       `current_exe()` once, before anything else: device, inode, size and
       modification time from one `metadata` call.
     - Before each spawn, the launcher compares the current identity with
       the recorded one. If they differ, it does not spawn. The caller
       shows "Runyte was updated on disk; restart it to use <PDF
       viewing|document preview>".
     - No helper protocol change is needed. Make the identity source
       injectable so tests can simulate a replacement without touching real
       files.
3. Rewire both clients through the launcher:
   - `pdf.rs` spawns `--helper pdf …`.
   - `preview.rs` `Helper::start` spawns `--helper preview --serve`, or just
     `--helper preview` if `serve` becomes the only behavior.
   - Delete the `RUNYTE_PREVIEW_HELPER` lookup, the sibling lookup and the
     "Build contrib/document-preview" message. Replace that message with
     "Document preview could not start: <error>".
4. `runyte-desktop` `main`: dispatch `--helper pdf` to
   `runyte_native::pdf::helper_main` and `--helper preview` to
   `runyte_preview::serve(stdin, stdout)`, before anything else, including
   before the `mcp` check. Remove `--native-pdf-helper`. It is internal and
   always tied to the same build, so there is nothing to keep compatible.
5. Tests:
   - Unit tests for the launcher: argument vector, role parsing, unknown role,
     and refusal with the restart message when the recorded executable
     identity differs.
   - An integration test in `crates/runyte-desktop/tests/` that runs the built
     `runyte-desktop` (`env!("CARGO_BIN_EXE_runyte-desktop")`) as
     `--helper preview` and `--helper pdf` against fixtures. It checks that a
     rendered frame or page comes back and that an unknown role fails with a
     non-zero status. Use temporary directories and set `XDG_CONFIG_HOME`, as
     `AGENTS.md` requires. Never execute a file the test wrote.
   - Point `check.py` at the desktop binary: `--binary <path>` runs
     `<path> --helper preview`.
   - Update `tests/native_window.py`: drop `--preview-helper` and
     `--packaged-preview-helper`; preview now needs no extra file.
6. Packaging and CI:
   - `package.py`: remove `--preview-helper` and `preview_helper()`. Bundles
     contain one executable.
   - `check_package.py`: check for exactly one executable plus the `runed`
     link (Linux), and run engine acceptance through `runyte --helper preview`.
   - `release.yml` and the desktop workflow: delete the
     `--manifest-path contrib/document-preview/Cargo.toml` builds.
   - The `security` job now covers Blitz through the workspace lock.
7. Notices: rewrite the `THIRD_PARTY_NOTICES.md` preview section. It is now
   linked into the desktop executable and shares the workspace lock; the
   terminal edition still contains none of it. Rename the "Experimental
   native …" headings to "Desktop edition …". The published crate still
   includes this file. Make it say plainly which sections apply only to the
   desktop edition.
8. Measure and record in the commit body and in
   `context/reference/desktop-edition.md` (Phase 5 renames the file):
   - release `runyte-desktop` size before and after, stripped;
   - cold `runyte-desktop --version` time;
   - window startup time, using whatever `benchmarks/` and `tests/native_window.py`
     already measure;
   - the terminal edition's startup figures from `benchmarks/`, which must not
     change.
9. Commit: `Run document preview inside the desktop executable`.

## Phase 5: edition identity and how it is communicated

Goal: someone using either edition can always tell which one they have and
what the other offers.

1. `--version` prints `runyte <version> (terminal edition)` or
   `runyte <version> (desktop edition)`.
   - Search tests, scripts, plugins under `examples/`, `skills/`, the MCP
     adapter and `docs/` for parsers of the old output, and update them.
   - Note the change in the plan's progress section, so the next release's
     changes list includes it.
2. `--help`:
   - The desktop edition lists `--window` with a one-line description. The
     terminal edition omits it.
   - Both end with one line naming the edition. The terminal edition's line
     adds that the desktop edition is available, with the README anchor URL
     on GitHub.
3. Messages, in one table in the library so both editions share the wording:

   | Situation | Message |
   | --- | --- |
   | Terminal edition, `--window` | `--window is part of the Runyte desktop edition; this is the terminal edition. See https://github.com/runyte/runyte#editions` |
   | Terminal edition, `:preview` | `:preview is part of the Runyte desktop edition; this is the terminal edition.` |
   | Desktop edition in a terminal, `:preview` | `:preview needs the window; start Runyte with --window` |
   | Desktop edition, `--window` on Windows | `The desktop edition's window is not available on Windows yet` |

   Check the current behavior of opening a PDF or image in the terminal
   frontend. If it gives a window-only message, add matching rows. Each row
   needs a test at the behavior boundary: the CLI test for flags, the app
   test for commands. Messages pass through the existing action-failure path.
   Do not add new UI.
4. README:
   - Add an `## Editions` section near the top, before installation, with a
     two-column comparison:
     - what is included;
     - executable size, measured in Phase 4;
     - platforms;
     - requirements (desktop: Vulkan on Linux, the glibc floor, macOS version;
       Poppler as an optional PDF fallback);
     - how to install;
     - how to switch between editions.
   - State that the desktop edition also runs in a terminal, and that
     persistent sessions require the same Runyte build on `PATH` for both
     frontends.
   - Rewrite the "experimental native window" section as "Desktop edition",
     moving build-from-source instructions to `cargo run -p runyte-desktop --
     --window`.
5. `docs/user-guide.md`: same edition explanation in "Install and run". Rename
   the experimental native window section to "Desktop edition", and update
   its anchors and every link to them across `docs/`, `README.md` and `src/`
   (help, manual and tutorial text may link to it; grep).
6. References:
   - `git mv context/reference/native-window-experiment.md context/reference/desktop-edition.md`.
     Rewrite its status paragraph: product, not experiment. Remove the
     "do not merge into `dev` or `main`" rule and record that Decision 1
     supersedes it, with this plan's path. Keep the technical content.
   - Fold `context/reference/native-document-preview.md` into it, or keep it
     with "experiment" wording removed. Choose the one that leaves fewer
     duplicated statements, and update every link either way.
   - Update `AGENTS.md`, `contrib/packaging/README.md`, the moved preview
     `README.md` and `context/plans/README.md` links.
7. Commit: `Name the terminal and desktop editions throughout`.

## Phase 6: release packaging for both editions

Goal: every published artifact says which edition it is, and the desktop
edition is released on every platform where it has passed acceptance.

1. **Terminal archive rename.**
   - New names: `runyte-terminal-<tag>-<target>.tar.xz` and
     `runyte-terminal-<tag>-x86_64-pc-windows-msvc.zip`. The top-level
     directory inside becomes `runyte-terminal-<version>-<target>`.
   - `release.yml`: choose names by tag contents, the same way it already
     chooses the historical platform set. A tag containing a marker file
     added in this commit (for example `contrib/packaging/editions`) uses the
     new names; older tags keep the old ones. Rerunning an old tag must
     reproduce its original asset names exactly.
   - `install.sh`: try the new name first for the resolved version. Fall back
     to the old name only when `SHA256SUMS` lists the old name and not the
     new one. Never guess from version numbers. Add installer tests for both
     layouts and for a manifest listing neither.
   - Update `context/reference/releasing.md` (asset list, counts,
     `SHA256SUMS` coverage, verification commands), `contrib/runyte.ps1` if it
     names archives, and `skills/runyte-demo-videos/` if they download
     releases.
2. **Linux desktop archives.**
   - Keep x86-64 on Ubuntu 24.04.
   - Add `aarch64-unknown-linux-gnu` on `ubuntu-24.04-arm`, with the same
     lavapipe Xvfb acceptance. If lavapipe or window acceptance cannot run on
     that runner, do not publish the ARM64 desktop archive. Record the
     blocker in `desktop-edition.md` and continue.
   - Contents: `runyte`, `runed` link, notices, docs, icons and the
     registration script. `check_package.py` checks the exact file list.
3. **macOS app bundle.**
   - Replace the shell-script `CFBundleExecutable`. Shell scripts as a
     bundle's main executable are a signing and notarization hazard: the
     script's signature lives in extended attributes that copies can lose.
   - Add a tiny macOS-only binary target to `runyte-desktop`, named `Runyte`
     (the bundle executable). It:
     - finds the sibling `runyte` in `Contents/MacOS`;
     - appends `/opt/homebrew/bin` and `/usr/local/bin` to `PATH`, as the
       script does today;
     - drops a leading `-psn_*` argument if present;
     - `exec`s `runyte --window --editor <args>`.

     It has unit tests for argument and `PATH` construction, and needs no
     dependencies beyond `std` and `libc`.
   - `Info.plist` adds:
     - `LSMinimumSystemVersion`: the lowest macOS version that GPUI 0.2.2 and
       the build actually support. Determine it from GPUI's source and the
       `MACOSX_DEPLOYMENT_TARGET` used, set that variable explicitly in
       packaging, and record the value and its evidence.
     - `NSHumanReadableCopyright`.
     - `LSApplicationCategoryType = public.app-category.developer-tools`.

     Keep the identifier `com.runyte.Runyte`.
   - **Universal binary:** build `runyte-desktop` for `aarch64-apple-darwin`
     and `x86_64-apple-darwin`, combine each with `lipo -create`, and do the
     same for the `Runyte` launcher. `package.py macos` gains
     `--binary-x86_64` and `--binary-aarch64` (or accepts an already universal
     `--binary`). It verifies `lipo -archs` output.
   - **Release workflow:** add a macOS desktop job on `macos-15`. It:
     - builds both architectures from the validated tag;
     - creates the universal `Runyte.app`;
     - runs `check_package.py` headless engine acceptance (no window: GitHub
       macOS runners cannot be relied on for GPU acceptance);
     - uploads `Runyte-<version>-unsigned.app.zip` (made with `ditto -c -k
       --keepParent`) as a **workflow artifact only, never a release asset**.

     Phase 8 signs that artifact.
4. **Disk image tooling, written now and run manually in Phase 8.**
   - `contrib/packaging/package.py dmg --app <Runyte.app> --output <Runyte-<version>.dmg>`:
     - stages the app with an `/Applications` symbolic link and the README;
     - runs `hdiutil create -volname "Runyte <version>" -srcfolder <stage> -ov -format UDZO <out>`;
     - refuses to overwrite an existing output.
   - Unit tests mock `hdiutil`. A macOS CI step builds an unsigned DMG from
     the CI bundle and checks it mounts (`hdiutil attach -nobrowse`), contains
     the app and detaches.
   - `contrib/packaging/sign_macos.py`, with subcommands `app`, `dmg` and
     `verify`:
     - Takes `--identity "Developer ID Application: <Name> (<TEAMID>)"` and
       `--notary-profile <keychain profile name>`. It never reads credentials
       from files, environment variables or the repository.
     - Signs inside-out: the `runyte` executable, then the `Runyte` launcher,
       then the bundle. Each uses `codesign --force --options runtime
       --timestamp --sign <identity>`. No `--deep` when signing.
     - Notarizes with `xcrun notarytool submit <zip|dmg> --keychain-profile
       <profile> --wait` and staples with `xcrun stapler staple`.
     - `verify` runs:
       - `codesign --verify --strict --verbose=2` on the app;
       - `spctl --assess --type execute -vv` on the app;
       - `spctl --assess --type open --context context:primary-signature -vv`
         on the DMG;
       - `xcrun stapler validate` on both.
     - `--dry-run` prints every command without running it. Unit tests cover
       the command sequence in dry-run mode.
     - No entitlements file at first. GPUI uses Metal and the editor spawns
       children and PTYs, none of which needs an entitlement under the
       hardened runtime. If Phase 8 shows a failure that only an entitlement
       fixes, add the minimum one with its reason recorded.
5. **Installer edition option.**
   - `install.sh --edition terminal|desktop` (default `terminal`). `desktop`
     is accepted on Linux x86-64 (and ARM64 once published). It installs
     `runyte` and `runed` from the desktop archive, then prints the
     `package.py linux --binary <installed path>` command for launcher
     registration rather than running it.
   - On macOS, `--edition desktop` explains that the desktop edition is the
     DMG, with the releases URL, and exits non-zero.
   - Add installer tests for each case.
6. Update `context/reference/releasing.md`:
   - the complete asset list for new tags;
   - the desktop jobs;
   - that the macOS DMG is attached manually after the workflow (Phase 8);
   - the step in the runbook where that happens, after step 12 and before
     step 13's verification;
   - the asset counts in steps 12–13.
7. Commits, separated by concern:
   1. `Name terminal release archives by edition`, with installer
      compatibility;
   2. `Publish desktop archives for Linux ARM64` (if acceptance passes);
   3. `Launch the macOS app through a native executable`;
   4. `Build a universal macOS desktop app in the release workflow`;
   5. `Add disk image and signing tools for the macOS desktop edition`;
   6. `Install the desktop edition with the curl installer`.

## Phase 7: merge into `dev` (stop point)

Do not start this phase without the maintainer's explicit go-ahead in the
conversation. Prepare, then wait.

1. Prepare a merge report for the maintainer:
   - `git log --oneline dev..exp | wc -l` and `git diff --stat dev...exp`
     totals;
   - any conflicts from a trial merge in a temporary worktree
     (`git worktree add`, then delete it);
   - the state of every CI workflow on the `exp` head;
   - what `main` users would see change when this reaches a release.
2. After the go-ahead, the maintainer decides whether the agent or the
   maintainer performs the merge, and when.
3. Release preparation follows `context/reference/releasing.md` unchanged.
   This plan does not bump versions.

## Phase 8: signed and notarized macOS disk image (manual)

This phase runs on the maintainer's Mac with the maintainer's Apple
Developer account. The agent prepares commands and checklists, but must not
ask for, store or log credentials, certificates or passwords.

### One-time setup (maintainer)

1. Apple Developer Program membership (organization or individual).
2. A **Developer ID Application** certificate in the login keychain. Check
   with `security find-identity -v -p codesigning`, which must list
   `Developer ID Application: <Name> (<TEAMID>)`.
3. A notarization profile stored in the keychain:
   `xcrun notarytool store-credentials runyte-notary --apple-id <id> --team-id <TEAMID>`
   (prompts for an app-specific password), or the App Store Connect API key
   form of the same command.
4. Xcode command-line tools: `xcode-select -p`.

### Per release (maintainer, agent-assisted)

1. Download the CI artifact `Runyte-<version>-unsigned.app.zip` from the
   release workflow run for the exact tag. Confirm the run's source SHA
   matches the tag, as `releasing.md` step 12 requires. If that is not
   possible, build locally from a clean checkout of the tag with the same
   commands the workflow uses, and record that in the release notes draft.
2. `ditto -x -k Runyte-<version>-unsigned.app.zip ./stage`
3. `python3 contrib/packaging/sign_macos.py app --identity "…" --notary-profile runyte-notary ./stage/Runyte.app`
   signs, zips for submission, notarizes and staples the app.
4. `python3 contrib/packaging/package.py dmg --app ./stage/Runyte.app --output ./Runyte-<version>.dmg`
5. `python3 contrib/packaging/sign_macos.py dmg --identity "…" --notary-profile runyte-notary ./Runyte-<version>.dmg`
   signs, notarizes and staples the disk image.
6. `python3 contrib/packaging/sign_macos.py verify ./stage/Runyte.app ./Runyte-<version>.dmg`
   must pass.
7. Hands-on acceptance on a real Mac, Apple silicon at least, and Intel if
   one is available. Record results in `context/reference/desktop-edition.md`
   under a dated heading, listing exactly what was and was not tested.
   1. Download the DMG through a browser, so it carries quarantine. Open it,
      drag Runyte to Applications and launch it from Finder. There must be no
      Gatekeeper warning beyond the normal first-launch confirmation.
   2. The window opens in editor mode. Check:
      - fonts and Retina scaling;
      - typing, splits and the command palette;
      - an integrated terminal (`:terminal`) runs the login shell with the
        Homebrew `PATH`;
      - a PDF opens through the Hayro helper, and through Poppler when Hayro
        falls back, if Poppler is installed;
      - `:preview` on Markdown, HTML and SVG;
      - copy to the clipboard (Cmd-c and Ctrl-Shift-c).
   3. In Terminal.app, link the bundle's binaries onto `PATH`:
      `ln -s /Applications/Runyte.app/Contents/MacOS/runyte /usr/local/bin/runyte`
      and the same for `runed`. Then:
      - `runyte --version` reports the desktop edition;
      - `runyte --mux` attaches in the terminal, and `runyte --window --mux`
        attaches to the same persistent session.
   4. Quit and relaunch. Then replace the app with a rebuilt copy while it
      runs, and check that opening a new PDF reports the restart message
      from the Phase 4 replaced-executable check instead of failing obscurely.
8. Upload: `gh release upload v<version> Runyte-<version>.dmg` and a matching
   `Runyte-<version>.dmg.sha256` (`shasum -a 256`). Keep these out of the
   workflow-generated `SHA256SUMS`, because the workflow rewrites that file on
   rerun. Document this in `releasing.md`.
9. README and user guide: the macOS install instructions point to the DMG and
   explain the optional command-line links from step 7.3.

Until Phase 8 has produced one accepted DMG, the README must say macOS desktop
builds are available only as unsigned local bundles built from source.

## Out of scope

- A Windows desktop edition. The window frontend is not implemented on
  Windows. Desktop-edition parity on Windows needs its own plan: a GPUI
  Windows backend, ConPTY with the window, MSIX or installer packaging, and
  code signing.
- Splitting the core library (`app`, `workspace`, `buffer` and so on) into
  more crates. Revisit once the workspace exists, if compile times or
  ownership give a concrete reason.
- Moving Blitz from its Git pin to crates.io releases.
- Finder "open with" events, file associations, Homebrew casks, Flatpak,
  AppImage, Windows signing, automatic updates and in-app update checks.
- Browser panes (`context/plans/proposed/PLAN_BROWSER_PANES.md`). That plan
  must place its CEF processes under the Phase 1 helper rules; CEF requires
  its own helper executables, which those rules allow when the reason is
  recorded.
- Any change to `runyte-1`, `runyte.context.v1`, the MCP tools or the private
  protocol version.

## Risks and how to detect them

| Risk | Detection | Response |
| --- | --- | --- |
| The shared lock raises a crate `runyte` uses beyond Rust 1.88 | `msrv` job, `cargo tree` diff | Pin with `cargo update --precise`; never raise `rust-version` |
| Blitz/Stylo and GPUI require incompatible versions of a shared crate | Phase 4 build | Allow duplicate major versions; if Cargo cannot resolve, stop and report |
| Moving `main.rs` into the library breaks tests that re-execute their own binary | `cargo test`, Windows CI | Fix the entry point; do not skip tests |
| Windows-only code moved in Phase 2 compiles only on Windows | Windows CI | Merge only on green Windows CI |
| `/proc/self/exe` behaves differently under sandboxes or in containers | Helper tests in CI | Fall back to `current_exe()` with the replaced-executable check |
| The desktop executable becomes much larger | Phase 4 measurement | Record it; size alone does not block, since the desktop edition is a deliberate choice |
| Archive rename breaks `install.sh` for users installing old versions | Installer tests for both layouts | Manifest-based fallback as specified |
| Hardened runtime blocks something at run time | Phase 8 hands-on run | Add the minimum entitlement with its reason |
| The coverage figure changes when the command changes | Phase 3 before/after measurement | Investigate; the measured set must be identical |

## Progress

Record each phase's completion here with the commit hashes, the gate results
and any measurement taken. Note anything deferred and why. When every phase
is done, or the remaining phases are explicitly deferred by the maintainer,
move this file to `completed/` and update `context/plans/README.md`.
