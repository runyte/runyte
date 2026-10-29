<!-- SPDX-License-Identifier: MPL-2.0 -->
# Configurable UI actions

Tracking issue: [#46](https://github.com/runyte/runyte/issues/46).
Implementation branch: `feat/configurable-ui-actions`.
Status: approved implementation; independent Astra High design and code reviews.

## Contract

Preserve default behavior, input precedence, exact modifier semantics, physical
approval requirements, captured targets, and drag ownership. Make actions
configurable wherever they execute, including UI and frontend-owned actions.
Structural option identifiers, numeric counts, jump labels, character operands,
and literal text remain input to an interaction rather than action bindings.
Classify by purpose, not chord length. Accept, cancel, back, movement, editing,
pane manipulation, and view controls are actions even when bound to one key.

Existing `keys.bind`, `rebind`, `leader`, `window`, and `defaults` retain their
meaning. New configuration is additive; absent new configuration must preserve
the existing keyboard and pointer behavior. Invalid entries produce bounded,
non-fatal diagnostics and retain usable defaults. All teaching surfaces must
read the effective registry. Do not change unrelated menu options or features.

## Architecture and decisions

Configuration shape: `keys.pointer.<action>` and
`keys.ui.<owner>.<action>` replace an action's binding list; omitted actions
inherit defaults, and `[]` removes bindings. Compile replacements simultaneously
so swaps work. Validate final effective maps and roll back the affected owner
atomically on conflict. `keys.scopes.<scope>.<mode>` will expose existing
registry-backed editor scopes. Existing `keys.defaults` keeps its old meaning.
Fragments merge pointer by action and UI by owner/action, replacing lists whole.
Configured strokes match exact modifiers; legacy defaults preserve their authored
matching policies. Multi-key UI actions require explicit prefix/text fallback
and owner-transition rules before admission.

- Extend the shared keymap with explicit owning UI scopes and semantic actions.
  Resolve only against the current input owner. An overlay must not trigger a
  global edit merely because it has no matching overlay action.
- Share navigation/editing action identities, while keeping execution within
  the surface that owns its state. Do not simulate old physical key events to
  execute a new binding: that can trigger structural options or authorization.
- Preserve raw input for text, structural choices, macros, and terminal child
  processes. Canonicalize keyboard bindings only at lookup; pointer modifier
  matching must remain exact unless an existing gesture explicitly allows more.
- Keep action sequences and key sequences distinct. UI sequence state belongs
  to its owner and resets when that owner closes or changes; literal input must
  not become unreachable through an accidental prefix.
- Introduce typed pointer action bindings. Resolve a drag's action at pointer
  down and retain the captured owner/action until release. Preserve geometry,
  shared-border precedence, and frontend-specific availability.
- Configurable scoped actions must preserve existing scope inheritance and
  validate conflicts. Structural option identifiers win within their owner;
  reject bindings that cannot be reached there rather than advertising them.
- Native window/preview actions must use effective bindings. Bundled protocol
  changes remain private and bounded; no new external plugin/context authority.
- Acceptance authorization is evaluated from the resolved action and captured
  owner at every gate, including physical input, repeated events and presented
  frame checks. Remapping acceptance must neither confer new authority nor leave
  an old physical key with latent approval authority.
- Configuration fragments, export/reload, help, hints, legends, and diagnostic
  limits are part of each configuration feature, not follow-up polish.

## Delivery stages

Each stage adds default-behavior and remapping regressions, receives independent
review, and records remaining work. Stages are implementation checkpoints, not
separate reductions of the issue's acceptance criteria.

1. [x] Inspect repository state, search existing issues, create tracking issue
   and implementation branch.
2. [ ] Inventory input owners and formalize configuration/precedence contracts.
3. [ ] Add typed configurable pointer actions, starting with pane move/extend;
   retain border and active-drag ownership. Complete remaining pointer actions.
4. [ ] Expose registry-backed special-view and list scopes with shared actions.
5. [ ] Convert prompt, picker, completion and menu navigation/editing, preserving
   structural option identifiers and literal paste/IME behavior.
6. [ ] Convert reusable confirmation actions and approval classification together.
7. [ ] Resolve terminal editor actions and native window/preview controls through
   effective bindings, preserving PTY forwarding and frontend ownership.
8. [ ] Convert action-owned grammar controls (cancel/back/context actions/hint
   navigation), retaining counts, operands and jump-label identifiers.
9. [ ] Complete documentation and direct-input audit; run full validation and
   independent final review. Publish reviewable branch/PR linked to #46.

## Input-owner inventory

| Owner | Existing source | Required work |
| --- | --- | --- |
| Ordinary editor and pending grammar | `src/input_grammar.rs`, `src/keymap/` | Preserve existing actions; migrate action-owned reservations |
| Pane pointer gestures | `src/app/pane_drag.rs`, `src/app/input.rs` | Typed action bindings; captured drag semantics |
| Editor pointer selection, scroll, links | `src/app/input.rs` | Registry lookup with existing hit-test precedence |
| Command/search/path prompts | `src/app/input.rs`, `prompt_editing.rs` | Shared editing/navigation actions |
| Finder/resource pickers | `src/app/input.rs` | Navigation, acceptance, split actions |
| Completion, lists, action menus | `src/app/language_workflows.rs` | Shared actions; structural menu choices retained |
| Special buffer scopes | `src/keymap.rs::BindingScope` | Configurable per-scope actions and hints |
| Session inventory/directory chooser | `src/app/workspace_workflows.rs` | Navigation and accept/cancel with captured identity |
| Tree destination/delete | `src/app/directory_tree.rs` | Separate option identifiers from actions |
| Merge review and exact-text confirmations | `src/app/git_merge_review.rs`, `input.rs` | Preserve text versus choice ownership |
| Context, plugin and LSP approvals | `context_access.rs`, `plugin_*`, host input admission | Resolve physical acceptance consistently |
| Integrated terminal | `src/app/input.rs`, terminal workflows | Configurable editor escape/clipboard; literal child input |
| Native controls and document preview | `crates/runyte-native/src/` | Effective map, transport, hints, pointer ownership |

## Verification

For each migrated interaction cover defaults, remap, removal, aliases, wrong
modifiers, nested owners, dismissal and stale-owner behavior where applicable.
Assert state/output, not merely registry lookup. Establish default regression
cases before changing dispatch. Pointer tests cover complete down/drag/up traces,
modifier changes mid-drag, borders and terminal content. Approval tests include
repeated input, unpresented/stale frames, synthetic input and old/new accept keys.
Storage/process fixtures use temporary roots and checked-in stand-ins.

Required final checks: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, and desktop clippy/tests
for `runyte-native`, `runyte-desktop`, `runyte-preview`. Canonical coverage is
`cargo llvm-cov --locked --package runyte` (89% floor); desktop coverage retains
its separate floor. Run guide-link checks. Record unavailable platform or real
terminal/window acceptance rather than claiming sandbox tests prove them.

## Review and progress

- Initial Astra review identified raw-Enter approval classification as a
  cross-cutting migration requirement; included above before implementation.
- First pointer slice implements `pane-move` and `pane-extend` with exact
  modifier masks and `LeftDrag`. Other buttons are explicitly rejected until
  initiating-button ownership is generalized. Effective gestures travel in
  bounded private frame protocol version 83 and force full-frame publication
  when changed. Remaining pointer actions are still pending.
- Astra approved the plan with simultaneous replacement/rollback, exact matcher,
  input-owner, and effective frontend-map refinements incorporated above.
- Pane review fixes: capture the painted map with native physical input;
  separate host gesture compatibility from geometry compatibility; consume
  rejected gesture tails; resolve configured pane gestures before native link
  clicks; bound malformed configuration diagnostics. Astra approved the revised
  source. Default and remapped pane traces, PTY tail ownership, host reload,
  protocol publication, and configuration round-trip regressions are included.
- Completion is the first migrated UI owner (`keys.ui.completion`). Shared
  next/previous/accept/cancel actions coexist with explicit cancel-or-continue
  and dismiss-and-continue actions. Continuation forwards the actual physical
  key once to the underlying editor grammar, never a synthesized old key.
  Default matcher expansion is cached; replacement removes every old alias and
  legacy wildcard. Both snapshot and standalone completion hints use the map.
  Astra approved the source; remaining UI owners are not yet migrated.
- Scoped configuration design: retain key-to-action/null maps. Null requires
  exact suppression metadata shared by lookup, validation and discovery, rather
  than deleting a local row and revealing an inherited binding. User-authored
  exact shadowing needs explicit provenance. Child scopes outrank parents;
  conflicting inherited parent overrides must be diagnosed. Include List in
  shared validation, preserve merge-review isolation and directory-tree prefix
  masking, and retain metadata through runtime plugin map rebuilding.

- Scoped checkpoint adds exact suppression/provenance and a shared effective
  resolver used by lookup, validation and discovery. The inherited scope graph
  is validated atomically: invalid `keys.scopes` rolls back that whole section,
  independently of `bind`, UI and pointer configuration. This deliberately
  avoids order-dependent recovery between multiple parents.
- Scoped assignments initially accept one existing action name or null; direct
  list and merge executors do not support action lists/arguments. Raw owner
  handlers remain pending migration: terminal Insert/Replace, context Tab,
  list accept/edit/context actions, overlay leader dismissal, and merge input
  editing. Unsupported modes/keys are rejected rather than falsely advertised.
  Nested action menus remain separate UI owners. Stage 4 is not complete yet.
- Validation for the published pane/completion checkpoint: root and desktop
  Clippy/tests passed, guide links passed, root canonical coverage 92.21%
  (89% floor). Later scoped work requires another validation pass.
- Astra approved the scoped checkpoint after reservations for unmigrated owner
  handlers, structural digits, cross-mode alias filtering, and nonallocating
  namespace checks. Root and desktop full tests and Clippy passed; keymap tests
  cover inherited removals/conflicts, plugin rebuilds, fragments/export, scoped
  hints and action reference. App tests exercise actual scoped movement and list
  navigation/closing. Faster help exposed a fixture's implicit dependency on
  publication pacing; the fixture now explicitly clears its pacing timestamp,
  following existing model tests. Production pacing is unchanged.
- Context action, path popup, and path action owners now resolve shared UI
  actions before their structural choice identifiers. Their defaults retain
  legacy modifier matching and leader dismissal; replacing cancellation also
  replaces the inherited leader. Effective snapshot hints follow the same map.
  Astra approved the source. Regression coverage includes every modifier mask,
  owner-local rollback, structural reservations, nested dismissal, actual app
  transitions, and leader keys that mask only some default aliases. Root and
  desktop Clippy/tests passed. One unrelated temporary file-lock fixture failed
  during the initial parallel root run, passed isolated, and the full suite
  passed with four test threads; production lock behavior was not changed.
- Terminal Insert now shares a cached first-key routing decision with help.
  Scoped assignments admit terminal-safe factory actions and claim their keys;
  default persistent navigation remains conditional, while an explicit scoped
  session action reports unavailable outside persistent mode. Exact nulls also
  suppress legacy capture-only policies without removing surviving prefixes.
  Terminal Replace remains unavailable. Astra approved the source after tests
  excluded ordinary Ctrl-j text editing from the factory admission inventory.
  Regression coverage compares all default modifier combinations, actual child
  input forwarding, remapped mode changes, prefix cancellation/execution,
  standalone unavailable actions, plugin rebuilds and effective help rows.
