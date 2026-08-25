# Review terminal parser and emulator

Conduct a focused hardening review of terminal escape parsing, emulation, the
cell grid, scrollback, and terminal key encoding. Treat child-process output as
untrusted input. This is a proactive review rather than evidence of a known
defect; make changes only for confirmed problems. Fix every confirmed problem
that is safely within this category; do not stop after reporting findings.

The primary scope is `src/terminal/parser.rs`, `emulator.rs`, `grid.rs`, and
`keys.rs`, plus `tests/terminal.rs`. Check incomplete, malformed, nested, and
oversized escape sequences; invalid UTF-8; control-string termination;
parameter arithmetic; cursor and region bounds; alternate screens; modes;
resizing; scrollback bounds; colors and attributes; combining and wide
characters; grapheme replacement; mouse and paste modes; and behavior under
large or deliberately pathological streams. Consult the terminal compatibility
reference before changing behavior.

Add deterministic parser and emulator regression tests for every confirmed
defect, including chunked-input variants where stream boundaries matter.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
