# Key bindings cannot be changed in the configuration

Every binding is compiled in. A reader who wants `Space g c` instead of
`Space g l`, or who cannot press Space comfortably, has no way to say so.

## Observed behavior

- `src/keymap.rs` is the single declarative source of truth.
  `built_in_bindings()` is a static list, `build_keymap` turns it into two
  `LazyLock` statics, and `keymap_for(fast_pane_keys)` returns one of them as
  `&'static Keymap`. `App` holds `keymap: &'static Keymap`.
- `Keymap::new` already rejects a duplicate sequence and returns
  `DuplicateBinding`; `build_keymap` treats that as a programming error, and
  `every_mode_sequence_is_unique_and_described` in `tests/keymap.rs` pins the
  invariant for the built-ins.
- There is no leader concept. Space is spelled literally as `Key::char(' ')`
  at the head of every application sequence, and `Ctrl-w` likewise heads the
  window sequences.
- `src/config.rs` loads `config.yaml`. Malformed YAML and values rejected by
  `validate_settings` abort startup with a context error. `themes` is the
  existing precedent for a user-defined map inside the same file.
- Key spellings are printed by `KeyStroke::label()` and normalized by
  `KeyStroke::canonical_for_binding`; nothing parses a key from text today.

## Expected behavior

- `config.yaml` can rebind sequences in the `Space` and `Ctrl-w` namespaces.
- A whole prefix can move, so that `Space g` → `Space G` carries every
  descendant and the namespace label with it.
- The leader key can be changed. Space is **the leader key**, and a
  replacement may be another key or a modified key such as `Ctrl-x`.
- Colon commands cannot be rebound; they keep their typed names, which are
  also the stable identities a configuration refers to.
- A changed binding is reflected everywhere the registry is read: dispatch,
  the key-hint popup, help, and the alias column of a hint row. No surface may
  describe a default that is no longer in force.
- One sequence may never reach two commands. A conflict is detected and the
  reader is told about it on startup.
- The configuration has to be easy to read.

## Deliverable

The next output of this issue is a **plan**, not an implementation. It belongs
under `context/plans/proposed/` and is written by working through the open
decisions below with the person, asking about each doubt as it appears rather
than resolving it silently in code. Implementation begins only once that plan
has been approved and moved to `context/plans/active/`.

The plan has to answer every question in this section, name the files it
touches, and state its migration and failure behavior. The questions are the
agenda for that conversation:

**Shape of the configuration.** Two readable shapes exist. The first states
the rewrite the way the request states it, with the default sequence or prefix
on the left:

```yaml
keys:
  leader: Ctrl-x
  rebind:
    Space g: Space G
    Space g l: Space g c
```

It covers a single binding, a whole prefix, and the leader with one rule, and
reads as the change the person is making. Its weakness is that the left-hand
side is not a stable identity: if a default moves in a later release, an entry
silently stops matching unless the loader reports an unmatched left-hand side,
which it then must.

The second keys the map by command name, which is stable, but cannot express a
prefix move without a second kind of entry:

```yaml
keys:
  leader: Ctrl-x
  bindings:
    git-log: Space g c
```

**Key spelling.** The parser should accept exactly what the editor prints —
`Space`, `Ctrl-w`, `Alt-j`, `Up`, `Tab` — so a reader can copy a hint row back
into the file. `KeyStroke::label()` is the reference, and parsing has to be its
inverse, including the canonicalization `canonical_for_binding` performs.

**Which conflicts are reported, and how.** At least four classes exist: an
unparsable key, an unknown command or unmatched default sequence, two entries
reaching the same sequence, and an entry that collides with a binding outside
the configurable namespaces — choosing `s` as the leader would shadow search.
The report requires the reader to be informed on startup. The recommendation
is that these are reported through the startup notification path, naming the
offending entries, with the built-in binding kept for each rejected entry so
the editor always opens; whole-file YAML errors keep today's hard failure.
Aborting startup for every class, consistent with `validate_settings`, is the
alternative. This is undecided.

**Ownership of the keymap.** `App::keymap` is `&'static Keymap` and the two
built-in keymaps are `LazyLock` statics. A configured keymap has to be built
from the built-ins, the configuration, and the `editor.fast_pane_keys` option,
and owned by the editor. `App::sync_keymap` already repoints the registry through
`keymap_for` wherever `config` moves — a setting preview, a save, a rolled-back
preview — and must rebuild the configured keymap the same way. `set_keymap`
takes a `&'static Keymap` and has the same problem.

**Scope.** The request names the `Space` and `Ctrl-w` namespaces. Undecided:
whether the short aliases those namespaces advertise — `,` for `Space s c`,
`&` for `Space s a`, the finder's short spelling — are configurable; whether a
reader may bind a command that has no default binding; and whether the
restricted `Ctrl-w` set available inside a terminal follows a reconfigured
window prefix.

**What a changed leader does to the surfaces that use bare Space.**
`context/reference/ui-vocabulary.md` records that modal result lists, choice
overlays, action menus, and confirmations take a bare Space as a symmetric
dismissal key, and that an initial bare Space cancels a picker. If the leader
moves, those dismissals either follow it or stay on Space; the register has to
say which.

## Constraints

- Dispatch, help, and key hints must keep reading one registry. A
  configuration that only dispatch knew about would leave help swearing to a
  key that does something else.
- `context/reference/helix-keymap-v1.md` describes the bindings as facts. It
  becomes the register of the **defaults**, and must say so.
- `docs/user-guide.md` gains the configuration syntax in its Configuration
  section, beside the existing settings and themes.
- Runtime state stays out of `context/`; the configuration is the person's
  `config.yaml` and nothing is written back to it by this feature.

## Regression coverage

Cover the loader and the resulting keymap rather than the file format alone: a
single rebound sequence dispatches, and its old spelling no longer does; a
rebound prefix carries its descendants and its namespace label; a changed
leader reaches every application binding; each conflict class is reported with
the offending entry named and leaves the editor usable; and the key-hint popup
and help both show the configured spellings.
