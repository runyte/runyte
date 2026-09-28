# Session stop without a workspace is refused on Windows

On Linux and macOS, `:session-stop` and `runyte --session-stop` (`-s`) with no
argument stop the persistent session for the current project root.

On Windows both forms require an explicit workspace selector:

- `:session-stop` with no argument fails with
  `session-stop needs an explicit selector on Windows`
  (`src/app/input.rs`, the `Colon::SessionStop` arm).
- `runyte --session-stop` with no argument fails with
  `--session-stop on Windows requires an explicit workspace selector`
  (`run_native_control_cli` in `src/main.rs`).

Naming the workspace by ID, name or path works. `docs/user-guide.md` lists the
gap under "Not available on Windows".

Expected: with no argument, both forms stop the session for the current
project on Windows too, or refuse with a specific reason when that cannot be
decided safely.

Constraints:

- Windows can run two hosts for one project at the same time, and a path or ID
  selector that matches both is already refused as ambiguous. An implicit stop
  must follow the same rule: when the current project matches more than one
  live publication, refuse and name the candidates. Never pick one.
- Inside an attached editor, the attached host's exact publication is the
  natural target. It must not be resolved again from the path.
- From an ordinary shell, the project is inferred from the launch directory.
  A missing launch directory, or one outside any project, gives a clear error
  and stops nothing.
- The protected-state checks and `--force` semantics stay the same as for an
  explicit selector.
- When this works, remove the entry from the Windows section of the user guide.
