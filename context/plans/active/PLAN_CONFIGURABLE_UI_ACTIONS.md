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
- Terminal checkpoint validation: full root suite (four test threads), root and
  desktop Clippy, desktop tests, formatting and guide links passed.
- Program action menus now use their own shared UI map, preserving wildcard
  modifiers and nested prompt ownership. Effective hints are used in snapshots
  and standalone rendering. Tests cover all default modifier masks, alias
  replacement/removal, actual navigation/cancellation, persisted selection and
  persistence failure retention using temporary cache storage. Astra approved;
  all 19 program-related tests and root Clippy passed. Buffer action menus stay
  in their existing list dispatch position pending shared stale-selection and
  list-owner migration.
- Command, text, path and program prompts now resolve shared UI actions.
  Literal text retains ownership; prompt actions do not inherit leader cancel.
  Separate deletion actions preserve the original completion-selection reset
  differences, and defaults retain Control-before-Alt precedence and per-owner
  navigation/completion behavior. Snapshot and standalone prompt hints use the
  effective map. Fresh configured acceptance retains native handoff authority;
  inherited modified Enter, repeats and replay retain their prior limits.
  Covering overlays, jump labels and macro replay cannot grant prompt authority.
  Astra approved the implementation and Windows fixture. All 4,105 active
  library tests passed (16 ignored) before the final owner-predicate follow-up;
  all 68 focused prompt tests and root Clippy passed after it. New app tests
  inspect captured plugin requests across fresh/repeat/replay. A Windows host
  regression additionally exercises actual handoff success/refusal; it requires
  Windows CI/runtime, unavailable in this Linux environment.
- File pickers and the project finder now resolve shared UI actions before
  held-ranking publication and query editing. Separate backspace actions retain
  the former Ctrl-h publication behavior. Empty-query leader cancellation uses
  the authored map underneath the temporary leader overlay, including removed
  defaults; nonempty queries restore that map. Effective snapshot and standalone
  hints follow owner bindings. Astra found no blocking issue. All 4,112 active
  library tests passed (16 ignored), including navigation, acceptance, both split
  axes, query ownership and publication timing; root Clippy, formatting and guide
  validation passed.
- Lists expose `list-accept`, `list-open-actions`, and `list-backspace` through
  their existing scoped registry. Explicit compatibility rows preserve every
  legacy modifier mask; exact overrides retain sibling aliases. Direct workflow
  dispatch preserves report no-ops and owner cleanup. Leader resolution follows
  effective Escape, consumes tombstones, and drives truthful hints. Stale-row
  guards retain nested-owner precedence. LSP presented-frame checks and Windows
  plugin handoff classification use active ownership and configured provenance.
  Astra reviewed and cleared the implementation after regressions were added for
  covering confirmations, nested stale selection, named leaders and fresh versus
  repeated/replayed acceptance. Full library validation passed 4,118 tests with
  only the command-inventory count assertion failing; that count was updated and
  its test passed separately, as did an additional report-remapping regression.
  Formatting, guide validation and root Clippy passed. Nested menus and their
  separate cancellation/confirmation policies remain the next checkpoint.
- Nested buffer, terminal and session menus now own configurable UI actions.
  Back (legacy Tab) is distinct from Cancel (Escape/Ctrl-c/leader), preserving
  stale-selection refusal for Back while permitting cancellation. Navigation
  disarms captured terminal/session confirmations; removed keys remain inert.
  Armed acceptance uses the active owner at the presented-frame gate, including
  hidden-live terminal Close. Effective hints and force-kill messages combine
  surviving dismissal actions. Session-menu literal control characters remain
  reserved for the existing text prehandler. Astra cleared the implementation;
  all 30 focused menu tests, root and desktop Clippy, desktop tests, formatting
  and guide validation passed. Tests include target preservation, all modifier
  defaults, remap/removal, stale rows, arming/disarming, covering confirmations,
  hidden-live terminal closure and effective dismissal text. Plugin-manager
  return remains a separate transparent prehandler for the next checkpoint.
- Plugin-manager return now has a configurable transparent UI owner. A typed
  list resolution retains return-versus-binding ownership and the exact registry
  key used for authority. Removing return exposes scoped physical-key behavior;
  removing its conditional leader restores text without reviving an Escape
  alias. Dispatch, acceptance classification, snapshots and standalone hints
  share that resolution. Astra cleared the change; all 11 plugin-manager tests,
  root Clippy, formatting and guide validation passed. A complete root suite is
  running across the accumulated list/menu checkpoints.
- Direct session-directory and session-inventory views now have independent UI
  owners. Directory browsing remains separate from workspace acceptance; report
  navigation retains its ordinary picker behavior. Literal query input, captured
  destinations/incarnations, cancellation restoration and stale-response guards
  are preserved. Both renderers use effective direct-view hints, including report
  return keys after reload. Astra found no blocker. All 122 session-related tests
  passed. The complete root run reached the key-hints integration suite and found
  an Open-with title clipping regression from expanded prompt aliases; compact
  effective labels fix it and the exact integration regression now passes. Final
  full-suite validation remains pending.
- Simple reload/discard/rebase confirmations and stash confirmations now resolve
  shared actions using raw input and the existing conditional leader policy.
  Stash has a separate owner to preserve its inert Ctrl-c default. Presented
  approval checks resolve effective acceptance before the Escape exemption;
  existing repeat behavior and target revalidation are unchanged. Body, status
  and compact footer hints read effective bindings. Astra identified conflicting
  display/input order for stacked decisions; concrete dialog selection now follows
  dispatch, with stacked stash/buffer and armed-terminal/buffer regressions.
  Astra cleared production changes; all 41 confirmation tests passed, including
  remap/removal, stale disk/repository captures, actual discard/rebase effects,
  dynamic hints and stacked owner behavior. Text and filesystem confirmations
  remain separate pending checkpoints.
- Exact-text branch/worktree confirmations now share configurable acceptance,
  cancellation and grapheme editing actions. Raw literal text, wildcard named-key
  defaults, revision invalidation on ineffective editing intent, conditional
  leader cancellation and required Typed-versus-Enter authorization remain
  unchanged. Dynamic body/status/footer hints include typed forms. Astra found
  no production blocker; the extracted terminal fixture retains its original
  Unix-only boundary. All 45 focused confirmation tests and root Clippy passed;
  formatting and guide validation passed. New regressions exercise grapheme
  edits in all three forms, exact-name refusal, remapped checkout and captured
  branch/worktree mutation requests with both authorization requirements.
- Filesystem review and tree destination/delete dialogs now resolve shared UI
  actions while preserving raw structural choices. Plain P alone requests
  permanent deletion; tree digits retain captured pane order and unique-digit
  auto-selection; y/Y/n/N retain their original modifier policy. Configured
  cancellation can use compound keys. Effective leader cancellation precedes
  structural choices in dispatch, approval gates and hints. Active destination
  input is not an approval; covered generic dialogs no longer obscure tree/FS
  decisions. Plugin-owned plans still record their captured Trash/Permanent
  decision for host execution. Astra found no production blocker. All 36
  filesystem and 84 tree-related tests passed, along with root Clippy, formatting
  and guide validation. Regressions cover default modifier masks, remaps, captured
  targets, actual operations, masked-choice hints, plugin ownership/readiness and
  stacked decisions.
- Accumulated checkpoint validation: the complete root suite passed (4,154
  active library tests, 16 ignored, plus all integration/doc suites), followed
  by desktop Clippy and desktop/native/preview tests. Logs are outside tracked
  context. Context grant and plugin-native input owners remain the next stage.
- Context permission review now exposes choice navigation and its six approved
  scrolling actions. Defaults still inherit the narrow editor-motion registry;
  replacements suppress and consume inherited aliases without exposing structural
  permission choices. Repeat/replay and presented/full-review guards recognize
  semantic acceptance. Hints follow effective motion and structural ownership.
  Astra found no correctness or authority blocker. All 61 context-related tests
  passed; additional compiled-map modifier and structural-reservation checks
  passed, along with root Clippy, formatting and guide validation.
- Provider reload and best-effort overwrite now have shared action owners with
  exact raw default modifiers, unconditional leader cancellation, and their
  differing Enter/leader precedence preserved. Semantic repeat and presented
  guards preserve captured intent/action identity and repair only the retained
  provider owner. Body and snapshot hints use effective actions. Astra caught
  a pre-sync authority leak in the first implementation; provider acceptance
  no longer grants generic plugin-command authority. A stale remapped key can
  still reach its ordinary editor command without gaining Windows handoff
  permission. All 143 provider-related tests passed, including modifier masks,
  removal, repeats, stale competing dialogs, reoffers, actual request authority,
  and host lifecycle regressions. Root Clippy, formatting and guide checks pass.
- Plugin forms, pickers, confirmations, choice fields and path completion now
  resolve separate shared owners. Literal text, exact default modifier policies,
  scalar picker deletion versus grapheme form editing, and validation revision
  semantics remain intact. Space choice toggling is configurable. Completion
  retains Tab versus conditional Enter repeat behavior. Final dispatch resolves
  its completion route once and checks explicit physical freshness; presented
  admission conservatively includes any masked base submission. This closes the
  mutable-filesystem routing races identified by Astra. No authority is lent to
  stale-surface fallback commands. Layered hints and native validation feedback
  use effective actions. Astra cleared the final source; all 178 plugin-related
  tests, root Clippy, formatting and guide checks passed. UI owner input remains
  bounded (64 owners, 32 aliases per action).
