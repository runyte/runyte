# Key remapping

Status: completed 2026-09-03

Created: 2026-09-03

Revised: 2026-09-03 after review

Issue: `context/issues/configurable_key_bindings.md`

## Decision

`config.yaml` gains a `keys` section that remaps the default keymap. It holds
two named prefixes — `leader` and `window` — and a `rebind` map whose left-hand
sides name **default** spellings and whose right-hand sides are absolute target
sequences. One rule covers a single binding, a whole prefix, and an advertised
alias; the two named prefixes move the heads of the application and window
namespaces.

The remapping is resolved once, into an owned keymap that dispatch, help,
hints, the manual, the About page, the tutorial, and every actionable message
all read, so no surface can instruct a reader to press a key that no longer
does the thing.

**This is remapping, not full configurability.** It moves existing defaults
and prefixes. It cannot unbind a command, bind a command that has no default,
or reach most direct single-key bindings. User-facing documentation says "key
remapping" for that reason; the issue keeps its own file name.

### Naming

The V1 surface is deliberately narrow, and the name says so. `docs/user-guide.md`
introduces the section as **Key remapping**, and its first paragraph states the
three things it cannot do, so a reader learns the boundary before the syntax
rather than after a rejected entry.

## Shape of the configuration

```yaml
keys:
  leader: Ctrl-x
  window: Ctrl-a
  rebind:
    Space g: Leader G         # a whole prefix, and everything under it
    Space g l: Leader G c     # one binding
    Ctrl-w x: Window e        # the window namespace
    Space e: Space            # a literal Space, now that the leader has left
    ",": F12                  # an advertised alias
```

The left-hand side is the spelling Runyte ships with, so the file reads as the
change the person is making. It is not a stable identity — a default that
moves in a later release silently stops matching — so the loader **must**
report an unmatched left-hand side. That report is the price of this shape and
is not optional.

The alternative shape keyed on command name (`git-log: Space G c`) was
rejected: it is stable, but a prefix move has no spelling in it, so a reader
moving one namespace would restate every descendant by hand.

### What the sides mean

- **Left**: always a default spelling, always with a literal `Space` and a
  literal `Ctrl-w`. It is matched against the built-in registry, never against
  an already-rewritten keymap. `Leader` and `Window` are **rejected** on the
  left, with a message naming the default spelling to write instead; allowing
  them would give one left-hand side two spellings and invite a pair of
  entries that disagree.
- **Right**: an absolute sequence. `Leader` and `Window` are tokens standing
  for the configured prefixes, so moving a prefix does not force restating
  every target that sits under it.
- `Space` and `Ctrl-w` on the right are **literal keys**, not the prefixes. An
  earlier draft overloaded `Space` on the right to mean the leader; that made
  an "absolute" target not absolute and made a literal Space impossible to
  reach. The tokens replace it. `Space e: Space` is therefore a legal way to
  put the explorer on the bare space bar once the leader has moved away — and
  it is correctly rejected while the leader is still Space, because the leader
  prefix already holds that key.

### Composition

Entries resolve simultaneously against default spellings, and the **longest
matching left-hand side wins**. Given the entries above, `Space g d` becomes
`Ctrl-x G d` by the prefix entry, while `Space g l` is governed by its own more
specific entry. Order in the file does not affect the result. The loader
collects `rebind` into a `BTreeMap<String, String>` — no new dependency, and a
deterministic report order.

A descendant may land outside its own moved prefix, splitting a namespace.
That is legal and silent: the two entries that produce it are explicit, and a
supported configuration must not manufacture a warning on every launch. The
user guide calls out the composition rule and shows the split case so the
result is not surprising.

`Config` captures `keys` as a raw `serde_yaml::Value`; the configured-keymap
loader then requires a mapping containing only `leader`, `window`, and
`rebind`, with string values and a string-to-string `rebind` mapping. This
keeps a structurally malformed but syntactically valid `keys` section on the
same non-fatal reporting path as bad key spellings. An unknown member or wrong
value shape rejects the whole section with one error rather than silently
ignoring a likely typo or partially applying an uncertain file. YAML syntax
errors and deserialization errors elsewhere in `Config` keep their existing
hard-failure behavior.

## Key spelling

`KeyStroke::parse` is added to `src/input.rs` as the inverse of
`KeyStroke::label()`, defined over **logical reported characters** rather than
physical key combinations.

`canonical_for_binding` (`src/input.rs:138`) removes Shift from a character
key but explicitly does not case-fold, so `Shift-g` and `G` are *not* the same
identity: the first canonicalizes to `Char('g')`, the second is `Char('G')`. A
terminal reports the shifted character, and which character a physical Shift
produces is keyboard-layout dependent, badly so for punctuation. The syntax
therefore follows what the terminal reports:

- An upper-case binding is spelled `G`. **`Shift-` on a character token is
  rejected**, with a message naming the character to write instead. Shift
  remains legal on a non-character token, where it is what the terminal
  reports — `Shift-Backspace` is an existing binding (`src/keymap.rs:1922`).
- Modifiers are stripped greedily from the front by name (`Ctrl-`, `Alt-`,
  `Super-`, `Shift-`, `Hyper-`, `Meta-`); whatever remains is the code. This
  parses the awkward spellings correctly: `-` is the code `Char('-')`, and
  `Ctrl--` is `Ctrl` plus that code. No `KeyCode` label begins with a modifier
  name followed by a dash, so the greedy split is unambiguous.
- A sequence is the space-separated form `KeySequence`'s `Display` already
  prints, which is why the space bar is spelled with the word `Space`.

Round-tripping is pinned over **canonical binding values**, not over every raw
modifier combination: for each `KeyCode` variant and each legal modifier set,
`parse(label(k)) == k.canonical_for_binding()`. The rejected spellings get
their own cases — `Shift-g`, and the accepted `-`, `Ctrl--`, `Space`, `F5`,
`Ctrl-Alt-x`.

## Resolution

A new module `src/keymap/configured.rs` owns the loader; `src/keymap.rs` keeps
the registry types and the built-ins.

1. **Parse.** `leader` and `window` become one `KeyStroke` each; each `rebind`
   entry becomes a pair of `KeySequence`s. An unparsable key rejects that rule
   alone. An unmodified character is not a legal `window`: unlike explicit
   bindings, ordinary text insertion is implicit and therefore invisible to
   the keymap validator, so accepting `window: a` would make `a` impossible to
   type in Insert/Replace mode as well as taking it from a terminal child.
2. **Admit.** A left-hand side must exist in the built-in registry as an exact
   binding sequence, an advertised alias, or a proper prefix of at least one
   binding sequence. It must also be in scope: headed by `Space` or `Ctrl-w`,
   or be an advertised alias. A bare `Space` or `Ctrl-w` left-hand side is
   rejected in favour of `leader` and `window`.
3. **Rewrite.** For a default sequence `s`, first find the longest admitted
   left-hand side `p` with `s.starts_with(p)`.
   - When one matches, expand `to(p)`, replacing only explicit `Leader` and
     `Window` tokens with their effective keys, then append
     `s[p.len()..]`. A literal leading `Space` or `Ctrl-w` in `to(p)` is never
     substituted. This is what makes `Space e: Space` reach the bare space bar.
   - When none matches, copy `s` and replace its leading default `Space` or
     `Ctrl-w`, if present, with the corresponding effective named prefix.

   The same function applies to `Binding::sequence`, `Binding::alias`, and
   `BindingNamespace::sequence` — which is what carries a namespace's label
   with it when its prefix moves. Provenance records separately which
   `rebind`, `leader`, and `window` rules contributed; a literal key on a
   right-hand side does not acquire named-prefix provenance.
4. **Validate** the finished keymap (below). Every violation is attributed to
   the rules that produced it; those rules are rolled back and steps 3-4 repeat.
5. **Build.** `Keymap::with_namespaces(...).with_context_actions(...)`, then
   install the effective named prefixes and the default-to-effective spelling
   map collected during rewriting. `DuplicateBinding` is retained as an
   assertion that step 4 did its job, not as the mechanism: it reports one
   collision, and a reader with a broken file deserves all of them.

An alias is a real binding *and* an advertisement on another binding — `,` is
registered at `src/keymap.rs:965` and advertised on `Space s c` at
`src/keymap.rs:1218`. Applying one rewrite function to both fields keeps them
in step by construction, and the existing
`aliases_reach_the_command_they_are_advertised_on` invariant fails if they
drift. `ContextAction` mnemonics are local to an open menu, never dispatch, and
are not rewritten.

### Validation

Checking for duplicate `(mode, scope, sequence)` tuples is **not sufficient**;
it misses three classes that the registry's own invariants already forbid.
`src/keymap/validate.rs` holds one validator, run over every `Mode` and every
`BindingScope::ALL` entry, checking the **effective** lookup that
`bindings_for_scope` defines rather than the raw binding list. It accepts
candidate binding, namespace, and action slices rather than a constructed
`Keymap`: duplicates are one of the failures it has to report, while
`Keymap::with_namespaces` refuses to construct a value that contains one. The
validator therefore reproduces the effective scoped view internally; building
the `Keymap` remains the final assertion after validation succeeds.

- **No duplicate effective sequence.** The tuple check, but computed after
  scoped bindings have hidden the globals they hide.
- **No `ExactAndPrefix`.** `src/input_grammar.rs:1060` groups it with `Prefix`
  and waits for more input, so a sequence that is both executable and a prefix
  is a command the reader cannot run. `Space g: Space f` would do exactly this
  to the Finder.
- **No global/scoped shadowing.** `bindings_for_scope` (`src/keymap.rs:665`)
  drops a global whose sequence a scoped binding holds, so `leader: q` would
  be swallowed by Help's own `q` without ever producing a duplicate tuple.
- **Namespace integrity.** Every namespace unique, reachable, and not itself
  executable — the property `every_namespace_is_unique_reachable_and_not_an_exact_binding`
  (`src/keymap.rs:2427`) already pins for the built-ins.
- **Alias integrity.** Every advertised alias still reaches the command it is
  advertised on.

The rule is **zero violations**. The validator runs against the built-in parts
first and must return an empty result; every configured candidate must return
an empty result too. A future deliberately tolerated exception must be a
narrow, named allowlist entry with its own rationale and regression test, not a
general comparison that lets one defect hide another. `src/keymap.rs:2414`
already asserts no shadowing and
`built_in_bindings_have_no_exact_prefix_ambiguity` asserts no ambiguity, but
the existing tests are weaker than this validator because the ambiguity test
only checks each binding in its own `binding.scope` and never checks a global
binding under an active buffer scope.

**Landing gate.** The validator is written and run against the built-ins as
the *first* commit, before any configuration exists. If it surfaces a
pre-existing violation in a scope the current tests never examined, that is a
registry bug to fix or record on its own terms; it must not be discovered
midway through the loader. `tests/keymap.rs` and the inline invariants then
delegate to the shared validator, so the built-ins and a configured keymap
answer to one definition of a valid keymap.

### Attribution and rollback

Each rewritten sequence carries provenance: the `rebind` entry that matched it,
if any, and the named-prefix rule that moved its head, if any. When a violation
is found, the rules that produced the offending sequences are rolled back in
increasing order of blast radius, re-resolving after each step:

1. the `rebind` entries involved;
2. `window`, if it still participates;
3. `leader`, last, because it moves every application binding.

A named-prefix rollback is reported prominently, since it reverts everything a
reader wrote. The loop is bounded by the number of rules and terminates at
worst with the built-in keymap intact.

Rejecting a rule means re-resolving as though that rule were absent. A more
general admitted prefix rule may still rewrite the same default binding; the
guarantee is not that every rejected leaf returns to its literal built-in
spelling, but that no rejected rule contributes to the result.

### Bounds

Configuration is untrusted input and the fixpoint re-resolves, so the loader
is bounded: at most **256** `rebind` entries, at most **8** keys per sequence,
at most **32** items rendered in a report before it collapses to a count, and
at most `rules + 2` resolution passes. The validator indexes each
`(mode, scope)` into a `HashMap` rather than scanning pairwise, so a pass is
linear in the binding count instead of quadratic; exceeding a bound rejects the
section with one message rather than being resolved slowly.

## The window prefix and terminal routing

`window` moves the window prefix **everywhere, including inside a live
terminal**, and terminal routing follows it.

The prefix cannot be moved by halves. The terminal window grammar is not the
six terminal-scoped bindings alone: `Ctrl-w w/x/h/j/k/l` are *global Insert*
bindings (`src/keymap.rs:1777-1787`) and only `v/s/f/z` are `terminal_insert`
(`src/keymap.rs:1789-1794`). Meanwhile `src/app/input.rs:1235` admits the
literal `KeyStroke::ctrl('w')` and sends everything else to the child. Moving
the registry without moving routing would leave a configured prefix delivered
to the child, an old `Ctrl-w` holding part of its former command set, and hints
describing neither.

So `key != KeyStroke::ctrl('w')` at `src/app/input.rs:1235` becomes a
comparison against `self.keymap.window_prefix()`, read from the active
resolved keymap rather than from the raw configuration. The comment above it
— which already names "the registered `Ctrl-w` window prefix" — is restated in
terms of the configured prefix.

The consequences are stated plainly in the user guide, because they cut both
ways:

- `Ctrl-w` is handed back to the child process. In a shell that is readline's
  delete-word, which Runyte currently takes.
- The configured prefix is taken from the child instead. A reader who chooses
  `Ctrl-a` loses it inside a terminal, which is the key tmux and readline's
  beginning-of-line both want.

`window` must resolve to a **single** key, since routing can only admit one
first key; a multi-key target is rejected with that reason. It also cannot be
an unmodified character key, including `Space`: the Insert/Replace text path
is not represented by a binding, so the validator could not otherwise report
that the prefix makes that character untypeable. Modified characters and
named non-character keys remain legal, with the terminal trade described
above. The single-key requirement also holds for `leader`.

`Ctrl-\` and `is_terminal_normal_key` are untouched: they are the escape from
terminal input, not part of the window grammar.

## The leader and bare Space

`src/app/input.rs:1148` turns a bare Space into an Escape for a modal overlay,
and its comment gives the reason: "Space opens most application surfaces, so
the same bare key closes a modal overlay that already owns input."

That reason is about the leader, not the space bar, so **the dismissal follows
the effective leader in the active resolved keymap**. With `leader: Ctrl-x`,
`Ctrl-x` dismisses a modal result list, a choice overlay, an action menu, and a
confirmation, cancels a picker from an empty query, and Space becomes ordinary
input everywhere — including surfaces that today cannot take a space because
it would dismiss them. The comparison becomes one against
`self.keymap.leader()`. Exact-text confirmations remain the exception they
already are. Reading both prefixes from the keymap matters when a named-prefix
rule is rejected, or when it is accepted in one fast-pane variant and rejected
in the other: routing must follow what dispatch actually uses, not what the
file attempted to configure.

`context/reference/ui-vocabulary.md` states the rule in terms of the leader at
lines 96 and 232, naming the default once so a reader who has changed nothing
still recognises the key.

## Live surfaces

Every surface that **instructs** a reader must print the live spelling. Four
hold hard-coded keys today:

| Surface | Literal `Space` | Literal `Ctrl-w` |
| --- | --- | --- |
| `src/help.rs` contextual overviews | 19 | in prose |
| `src/manual.rs` general manual | 30 | — |
| `src/about.rs` first steps | 10 | — |
| `src/tutorial.rs` lessons | 8 | 3 |

Plus the actionable messages: the startup help text (`src/app.rs:2995`), the
file-change status (`src/app.rs:2840`, `src/app.rs:927`), and the macro and Git
messages that name a key.

The tutorial is the worst case and the reason this cannot be deferred. Lesson 8
tells the reader "the complete sequence is Space s c" while the lesson advances
on the *command*, so a moved leader leaves them unable to finish the lesson at
all. Lessons 9 and 10 do the same with `Ctrl-w`. The tutorial already
interpolates spellings — `hints.line_start()` at `src/tutorial.rs:104` — but
`MotionHints` is a const table keyed on a tutorial preference, not a registry
read. The interpolation shape exists; the resolution source is what is missing.

### The shared renderer

`src/key_spelling.rs` is new and neutral: given a template and a `&Keymap`, it
returns resolved text plus the ranges it substituted. The keymap carries the
effective named prefixes and a map from every remappable default binding,
advertised alias, and namespace spelling to its live spelling. Resolution does
not guess from registry order, namespace description, or command role. Markers
use an explicit kind rather than treating every pair of braces as one:

- `{key:git-log}` — a command, by the stable name it already has, rendered in
  its primary spelling;
- `{key:swap-window:compatibility}` — a specific spelling, reusing the
  `BindingRole` the registry already carries, which is what `src/help.rs:126`
  needs to keep saying "Ctrl-w x is the compatibility spelling" truthfully;
- `{binding:Space s c}` — the binding identified by that exact default
  spelling, used when one command has more than one binding with the same
  `BindingRole`. The direct `,` binding and namespaced `Space s c` binding are
  both primary today, so a command-and-role lookup alone cannot choose the
  spelling the prose means;
- `{prefix:Space g}` — the prefix whose default spelling is `Space g`, rendered
  in its configured spelling. `{prefix:Space}` and `{prefix:Ctrl-w}` read the
  two effective named prefixes from the same keymap metadata used by overlay
  dismissal and terminal routing;
- `{literal-key:Space}` — an intentional physical-key reference that does not
  follow remapping, needed for statements such as a terminal child receiving
  the space bar.

Only `key:`, `binding:`, `prefix:`, and `literal-key:` open a marker. Other
braces are ordinary prose, so the regular-expression notation `{n,m}` in the
general manual remains untouched. Doubling the opening brace escapes these
marker prefixes if a document ever needs to show their syntax literally.

A `key:` lookup must find exactly one implemented global binding with the
requested command and role. Ambiguity is an error, not registry-order
selection; prose that needs one of two same-role spellings uses `binding:`.
`binding:` must name an exact default binding or advertised alias, while
`prefix:` must name a default namespace. All three resolve through metadata
recorded while the configured keymap is built.

Only authored static templates are parsed. A path, command output, error, or
other runtime value is inserted as an opaque segment after marker resolution;
it is never formatted into a string and then passed back through the marker
parser, because user-controlled text may itself contain `{key:...}`. Surfaces
also resolve before measuring, wrapping, centring, or aligning their text. In
particular, About computes its key-column width from the live spellings rather
than from the default table.

Help, the manual, About, the tutorial, and the actionable messages all call it.
In `src/help.rs` the substituted ranges are marked `HelpRole::KeyBinding` as
they are written, which removes most of the authored colouring list at
`src/help.rs:456-476`: it shrinks to what is genuinely not one command's
spelling — `Shift-click`, `Ctrl-u/Ctrl-d`, `h/j/k/l`, `NORMAL`, `Escape`. The
comment at `src/help.rs:430-441` explaining why the document is not searched
for raw binding text stays true, extended to say why markers are exempt: a
marker's range is known when it is written, so nothing is searched.

Prose reaches the output through `writeln!(out, "{paragraph}\n")` as a value
rather than a format string, so braces in the data are inert today; the marker
pass is the only thing that will read them.

### The invariant that makes this stick

Converting strings by hand does not stay converted. Two tests carry the part
that can be checked mechanically without mistaking punctuation for a binding:

1. **No stale namespace literal.** Every registered template in `help.rs`,
   `manual.rs`, `about.rs`, `tutorial.rs`, and the actionable-message inventory
   is parsed. After marker spans are removed, the test searches the remaining
   text, longest sequence first and on token boundaries, for every built-in
   remappable sequence headed by `Space` or `Ctrl-w`; any match fails.
   Intentional physical references use `literal-key:`. Single-character advertised aliases
   such as `,` cannot be distinguished mechanically from prose punctuation;
   today they appear only in generated registry rows, and any future prose that
   names one must use a `key:` marker with the appropriate role.
2. **Every marker resolves.** Each `key:`, `binding:`, and `prefix:` marker in
   the complete registered inventory resolves unambiguously against both
   built-in keymap variants; `literal-key:` parses as one key. A typo cannot
   leave marker syntax in a help page or lesson. A configured sentinel map that
   moves both named prefixes, one namespace, and every advertised alias then
   renders the inventory, while targeted assertions pin the live spellings on
   each surface.

Spellings outside the remappable set stay literal and stay correct by
construction: `Ctrl-u/Ctrl-d`, `h/j/k/l`, `x/X`, and the `MotionHints`
spellings `gh`/`0`/`ge`/`G` are direct global bindings this feature cannot move.

Static external documentation — `README.md`, `docs/user-guide.md` — keeps
default spellings and says at the head of its keymap material that these are
the defaults, which a `keys` section may move.

## Reporting

Every `keys` error is non-fatal; rejected rules contribute nothing to the
re-resolved map, so the editor always opens and can be used to fix the file. A
remaining broader rule may still carry the affected binding with its prefix.
This follows the grammar registry, which already degrades into a startup
notification at `src/app.rs:2995-3016` and `src/app.rs:3182`, rather than
`validate_settings`, which aborts.

`keys` is therefore deliberately **not** validated by `validate_settings`, with
a comment saying why: a mistyped key would otherwise lock the reader out of the
editor that fixes it, a trade `tab_width` never has to make. Capturing that one
field as raw YAML also prevents a wrong `keys` shape from failing `Config`
deserialization before the loader can report it. A syntactically invalid YAML
file, or an invalid value elsewhere in `Config`, keeps today's hard failure.

| Class | Outcome |
| --- | --- |
| Unknown member or wrong value shape in `keys` | Whole section rejected, one message |
| Unparsable key | Rule rejected |
| `Shift-` on a character token | Rule rejected, naming the character to write |
| `Leader`/`Window` on the left, or bare `Space`/`Ctrl-w` on the left | Rule rejected, naming `leader`/`window` |
| Multi-key `leader` or `window` | Rule rejected |
| Unmodified character `window`, including `Space` | Rule rejected, explaining that it would consume ordinary text input |
| Left side matches no default sequence, alias, or prefix | Entry rejected |
| Left side outside the namespaces and not an alias | Entry rejected |
| Any validator violation | Contributing rules rolled back by blast radius |
| A bound exceeded | Whole section rejected, one message |
| A descendant landing outside its own moved prefix | Kept silently; composition is documented |

**Rejections** produce an `Error` notification titled `Key bindings`, listing
the rejected section or each rule in its file spelling with its reason, plus a
short
`│ N key binding entries rejected` suffix on the startup status line built the
way `registry_failure_summary` builds its own.

Legal configurations produce no notification. In particular, a split
namespace and a moved window prefix are explicit supported choices documented
in the user guide, not conditions that warn again on every standalone launch.

## Ownership of the keymap

`App::keymap` becomes `Arc<Keymap>` and the two built-in `LazyLock` statics
become `LazyLock<Arc<Keymap>>`. `App::keymap()` still returns `&Keymap`, so
every reader of the registry is unchanged. `set_keymap` takes `Arc<Keymap>`; it
has two callers, both tests (`src/app/tests/search_and_pickers.rs:523`,
`tests/key_hints.rs:734`).

`Keymap` also owns the non-dispatch metadata that must agree with those
bindings:

```rust
leader: KeyStroke,
window: KeyStroke,
default_spellings: HashMap<KeySequence, KeySequence>, // default -> effective
```

The built-in constructor records `Space`, `Ctrl-w`, and identity entries for
every binding sequence, advertised alias, and `BindingNamespace`. The
configured builder preserves each default sequence while recording its
rewritten sequence, then records the effective `leader` and `window` after
rollback. `Keymap::leader`, `Keymap::window_prefix`, and
`Keymap::spelling_for_default` are the only readers. This gives
`key_spelling.rs` stable default identities without adding a second positional
relationship between the built-in and configured vectors. A `Keymap` made
directly by tests receives the default named prefixes and identity mappings
for its own bindings, aliases, and namespaces unless the test explicitly
installs other metadata.

**Both `fast_pane_keys` variants are compiled and diagnosed at startup** when a
`keys` section is present. They cannot share a diagnosis: `with_fast_pane_keys`
lets `Ctrl-h/j/k/l` shadow the bindings they collide with, so a rule can be
valid in one variant and rejected in the other. An earlier draft compiled the
inactive variant lazily, which meant a settings preview could silently activate
a keymap whose rejections were never reported. Compiling both is bounded work
paid only by a reader who configured something, and the report names the
variant when the two differ.

```rust
keymap: Arc<Keymap>,
configured_keymaps: Option<[Arc<Keymap>; 2]>,   // indexed by fast_pane_keys
```

With no `keys` section, `configured_keymaps` is `None`, nothing is compiled,
and `sync_keymap` clones an `Arc` out of the static exactly as `keymap_for`
selects one today. **Startup cost for an unconfigured reader is unchanged.**
`sync_keymap` only ever selects; it never rebuilds, which matters because it
runs on every settings preview, save, and rollback
(`src/app/settings_workflows.rs:450`, `:497`, `:556`).

Compilation lives in `App::new_with_boundaries`, where `keymap_for` is called
at `src/app.rs:3015`, so the rejection list joins `registry_errors` on the path
that already builds `status` and `startup_notification`, and `Config` stays a
value that does not need the registry loaded.

## Files touched

- `src/input.rs` — `KeyStroke::parse`, `KeyCode::parse`, `Modifiers::parse`.
- `src/keymap/validate.rs` — new: the shared effective-keymap validator.
- `src/keymap/configured.rs` — new: parse, admit, rewrite, attribute, report.
- `src/key_spelling.rs` — new: marker resolution shared by every teaching
  surface.
- `src/config.rs` — raw optional `keys` value; explicitly absent from
  `validate_settings`, with a comment saying why structural and semantic
  parsing belong to the non-fatal keymap loader.
- `src/keymap.rs` — `LazyLock<Arc<Keymap>>`, `keymap_for` returning an `Arc`,
  effective named-prefix and default-to-live namespace metadata, built-ins
  reachable to the loader, and inline invariants delegating to the validator.
- `src/app.rs` — `keymap: Arc<Keymap>`, `configured_keymaps`, startup
  compilation, rejections folded into `status` and
  `startup_notification`, and the actionable messages at `:927` and `:2840`.
- `src/app/input.rs` — `set_keymap`, `sync_keymap`, the bare-Space dismissal at
  `:1148`, and terminal routing at `:1235`.
- `src/help.rs`, `src/manual.rs`, `src/about.rs`, `src/tutorial.rs` — templates
  instead of literal keys; the shrunken colouring list in `help.rs`.
- `docs/user-guide.md` — a **Key remapping** section: what it cannot do, the
  two shapes of entry, the tokens, the window-prefix text-input restriction,
  the terminal trade, and the reported classes.
- `context/reference/helix-keymap-v1.md` — a statement at the head that it
  registers the **defaults**, which a `keys` section may move.
- `context/reference/ui-vocabulary.md` — dismissal in terms of the leader.
- `context/reference/terminal-compatibility-v1.md` — the configured window
  prefix and what it takes from, and hands back to, the child.
- `context/reference/startup-performance.md` — the unconfigured path unchanged,
  and the configured path's one-time cost for two variants.
- `AGENTS.md` — Architecture lines for the three new modules.

## Migration and failure behavior

Nothing to migrate. `keys` is absent from every existing `config.yaml`, and its
absence preserves current behavior: no configured keymap is compiled, the
static default variant is selected with one `Arc` clone, its prefix metadata is
the identity map, and terminal routing reads `Ctrl-w` from
`Keymap::window_prefix()`.

Nothing is written back to `config.yaml`. `keys` has no setting in the
`[config]` buffer, is not patched by the lossless setting write, and no
in-editor action can change a binding.

Failure behavior is the table above: section-level rejection for a malformed
shape, rule-level rejection and re-resolution for semantic errors, one `Error`
notification and a status-line suffix when anything was rejected, and no
notification for legal compositions. Invalid YAML and invalid non-`keys`
configuration retain the existing hard failure.

## Sequencing

1. `src/keymap/validate.rs` and its adoption by the existing invariants, run
   against the built-ins. **Landing gate**: the baseline must be clean before
   anything below is written.
2. `KeyStroke::parse` and its round-trip tests.
3. The identity prefix metadata on `Keymap`, `src/key_spelling.rs`, and the
   conversion of all four teaching surfaces plus the actionable messages, with
   the template-inventory tests. This is independent of configured loading and
   valuable on its own: the built-in map initially records identity prefix
   mappings, while the change removes hand-maintained duplication that is
   already drifting.
4. Raw `keys` capture, structural parsing, the loader, attribution, bounds,
   and reporting.
5. `Arc<Keymap>` ownership, both-variant compilation, and `sync_keymap`.
6. Terminal routing and the reference updates.

## Regression coverage

Covering the resulting keymap, not the file format. New `tests/keys_config.rs`
unless noted:

- A single rebound sequence dispatches; its default spelling reaches nothing.
- A rebound prefix carries every descendant *and* its namespace label, checked
  through `Keymap::namespaces` and a hint row.
- A changed leader reaches every application binding, and `Space` is ordinary
  input in a buffer.
- A changed leader dismisses a modal overlay and cancels a picker from an empty
  query; `Space` does neither.
- `Space e: Space` binds the explorer to the bare space bar when the leader has
  moved, and is rejected when it has not.
- A rebound alias moves both the real binding and the advertisement, checked
  through the hint row's alias column.
- A changed `window` moves the window grammar in Modal, Insert, and terminal
  scopes together, reaches the editor from inside a live terminal, and delivers
  literal `Ctrl-w` to the child.
- Each validator class, each naming the offending rule and leaving the editor
  usable: duplicate effective sequence; `ExactAndPrefix` via `Space g: Space f`
  making the Finder unreachable; scoped shadowing via `leader: q` against
  Help's `q`; a namespace made executable; a broken alias.
- Each parse or prefix rejection: unparsable key, `Shift-g`, `Leader` on the
  left, a multi-key `leader`, and an unmodified character `window`; a modified
  character window prefix is accepted.
- A non-mapping `keys`, an unknown member, and a non-string `rebind` member
  each reject the section without preventing startup; an invalid non-`keys`
  setting retains the existing hard failure.
- Rollback precedence: a violation that a `rebind` rollback resolves does not
  revert `leader`; one that only a `leader` rollback resolves does, and says so.
- A rule valid under `fast_pane_keys: false` and rejected under `true` is
  reported at startup naming the variant, before any settings preview.
- Bounds: 257 entries, a 9-key sequence.
- The hint popup and help both print configured spellings for a rebound prefix.
- `KeyStroke::label()` round-trips through `parse` for every `KeyCode` variant
  and legal modifier set, plus `-`, `Ctrl--`, `Space`, `F5` (`src/input.rs`).
- No registered template contains an unmarked instructional `Space` or
  `Ctrl-w` sequence; intentional physical-key references use `literal-key:`,
  every marker resolves, `{n,m}` remains literal prose, and the sentinel
  remapping reaches every teaching surface (`src/key_spelling.rs`).

Tests point `app.config_path` at a temporary file through `note_loaded_config`
per the storage rules, and write no `config.yaml` outside a temporary
directory.

The `cargo llvm-cov --locked --workspace` total must stay at or above the floor
in `context/reference/test-coverage.md`. The loader's rejection paths are the
least-exercised code this adds, which is why each class above is a named test
rather than one test with several assertions.

## Known limitations

- A left-hand side is a default spelling, not a stable identity. A default that
  moves in a later release stops matching, and the reader learns from the
  unmatched-entry report rather than from a silently dead file.
- A command with no default binding cannot be reached, a bound command cannot
  be unbound, and bindings outside the two namespaces and the advertised
  aliases cannot be moved. This is remapping, not full configurability.
- A configured window prefix is taken from terminal child processes, in
  exchange for handing `Ctrl-w` back to them.
