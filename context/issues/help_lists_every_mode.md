# Help omits Insert, Replace and Command mode bindings

`Space ?` opens a help window for the current view. Its generated key tables
come from the keymap registry for one mode only.
`render_document_with_descriptions` in `src/help.rs` fixes it:

```rust
let mode = Mode::Normal;
```

Normal and Select bind the same sequences, so that one table covers both.
Insert, Replace and Command mode bindings are not listed anywhere in the help
window, and the Text topic's overview only describes Normal and Select.

Insert and Replace mode bindings missing from help include:

| Key | Command |
| --- | --- |
| `Tab` | `insert-tab`: insert spaces to the next configured tab stop |
| `Shift-Tab` | `insert-literal-tab`: insert a literal tab character |
| `Ctrl-x` | request language-server completions |
| `Ctrl-u` / `Ctrl-k` | delete to line start / end |
| `Alt-Backspace` / `Alt-Delete` | delete the previous / next word |
| `Ctrl-c` | toggle comments on the caret lines |
| `Ctrl-v` / `Alt-v` | paste the system clipboard, including images |
| `Ctrl-w` then a pane suffix | move to another pane without leaving Insert |

`docs/user-guide.md` lists all of these in its Insert and Replace modes table.
`user_guide_covers_every_direct_editing_binding` in `src/keymap.rs` keeps that
table complete, but nothing does the same for help.

## Indent keys

`<` (`unindent`) and `>` (`indent`) are Normal and Select mode bindings. In
the help for a text buffer they appear under "Direct keys", "Letters and
punctuation", spelled as the bare characters `<` and `>`. Searching the help
for `Shift-<` or `Shift->`, which is how the keys are pressed on most
layouts, finds nothing. In a read-only view, `hides_a_refusal` leaves them
out, because they could only produce a refusal there. It is not yet known
which of these two explains the report that help does not list them. Check
this while fixing the issue.

## Expected behavior

`Space ?` gives a complete list of key bindings for every mode: Normal/Select,
Insert, Replace and Command. Each mode has its own heading. Every key
dispatch can run in the current view must appear in exactly one of these
places:

- a generated table
- a prefix row whose hint popup teaches the rest
- a section that explicitly marks the key as shared across modes

Constraints:

- Tables stay generated from the same keymap registry that dispatch and key
  hints read. Do not hand-write a second key list.
- Scope filtering stays: a view lists its own scoped keys and the global keys
  that apply to it.
- Insert and Replace mostly share bindings. Show shared keys once and list
  what only one of them does separately, rather than printing two
  near-identical tables.
- Read-only views may still leave out keys that could only produce a refusal.
  If a view omits a whole mode (for example, Insert in a view that cannot be
  edited), say so in one line rather than leaving the reader to guess.
- A shifted punctuation key should be findable by how it is pressed. Adding
  its `Shift-` spelling beside the character is one option. Choose between
  that and other presentations while fixing this.
- Terminal Insert, where keys go to the child program, already has its own
  prose in the Terminal topic. Keep it consistent with any new Insert table.

## Coverage

Add a test to `src/help.rs`, alongside the existing help tests, that walks
every default single-key and prefix binding for every mode and scope. It
should assert that each one either appears in that scope's rendered help
document or is deliberately hidden as a refusal. It should also cover
configured keymaps, as the existing marker tests do. Update the
help-window section of `docs/user-guide.md` to describe the new sections.
