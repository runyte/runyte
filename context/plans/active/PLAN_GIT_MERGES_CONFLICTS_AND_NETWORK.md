# Git merges conflicts and network view

Status: active. Approved for implementation on 2026-09-30.

Created: 2026-09-30. Source baseline: `47d955c`.

## Purpose and decisions

Extend Runyte's Git interface with reviewed branch merges, explicit fetching,
conflict resolution, and a navigable commit graph. Keep these workflows inside
the existing asynchronous Git service and ordinary editor navigation.

Every merge request opens the merge management overlay, including clean merges
and results with no changed files or conflicts. Cleanliness never bypasses
review. Approval of a non-fast-forward merge applies it without committing;
creating the merge commit requires a separate explicit step. A fast-forward
also requires overlay approval and creates no commit. An already-up-to-date
result remains inspectable in the overlay and changes nothing.

Fetching targets one branch. `Tab f` fetches the selected remote branch or the
selected local branch's configured upstream. Whole-remote fetching and remote
branch discovery are outside this plan.

The initial scope is two-head merging with Git's ordinary merge machinery.
Squash merges, strategy selection, history rewriting, and a full rebase
sequencer interface are outside this change. Existing pull and push behavior
remains available.

## Commands and surfaces

| Surface | Binding | Proposed command and behavior |
| --- | --- | --- |
| Branch list | `Tab m` | `git-merge-branch`: review merging the selected branch into the current branch |
| Branch list | `Tab f` | `git-fetch-branch`: fetch the selected remote branch, or the selected local branch's upstream |
| Global Git namespace | `Space g c` | `git-conflicts`: open `[git conflicts]` |
| Global Git namespace | `Space g n` | `git-network`: open `[git network]` |
| Conflict list | `Enter` | Open the selected conflict in its file and focus the first unresolved region |
| Conflict list | `Tab c` | `git-merge-continue`: review the index and open the merge commit message |
| Conflict list | `Tab A` | `git-merge-abort`: review and confirm aborting the active merge |
| Network buffer | `Enter` | Open the selected commit through existing commit-detail navigation |
| Network buffer | `Ctrl-n` / `Ctrl-p` | Next/previous graph page |

Provide colon commands for each action, with applicability and disabled reasons
in the registry. Add merge continue/abort to the status buffer as well, so a
clean pending merge is manageable without conflicts. Continue requires no
unmerged entries. Ordinary letter motions, search, copying, and selections
retain their meanings in generated buffers; contextual actions use `Tab`.

All bindings, aliases, help, hints, and footer labels must derive from the same
registry, including the new overlay scopes. The literal spellings in this plan
are defaults and remain subject to effective-scope validation.

## Reviewed branch merging

### Opening and reviewing

`Tab m` captures the selected local or cached remote branch and the current
local branch. A source checked out in another worktree is valid: its committed
tip is read, and that other working tree is untouched. A remote source is
explicitly labelled as cached; merging never implicitly fetches.

Refuse the current branch, detached or unborn destination HEAD, an existing
merge/rebase/cherry-pick/revert operation, a dirty index or working tree
(including untracked files), and unsaved repository file buffers. This follows
the existing conservative checkout policy and avoids implicit stashing. A
source already contained in the destination opens the management overlay with
“Already up to date,” zero changed files, zero conflicts, and approval disabled.
Cancel remains selected. Unrelated histories are refused in this scope.

Prepare the review asynchronously and always open the management overlay for
a valid merge request, even when the change or conflict list is empty. Show
explicit “No changed files” and “No conflicts” states where applicable; a clean
merge with new ancestry but identical resulting text still requires approval
and a separate commit step. The overlay states direction, captured
tips, predicted result, and whether approval finishes a fast-forward, leaves
a clean merge awaiting a commit, or starts a conflicted merge:

```text
Merge feature/search into main
Current: main 8f32c91       Other: feature/search 74e290a
Result: merge commit required · 3 changed files · 1 conflict

  M  src/search.rs       +42 -10
  A  tests/search.rs    +31  -0
  !  src/keymap.rs      content conflict

  Enter: inspect selected change
  [A]pprove merge                         > [C]ancel merge
```

Counts compare the current tip with the simulated result, rather than comparing
the two branch tips. Conflicted-file counts are labelled provisional or omitted
where a meaningful merged count does not exist. Binary changes, mode changes,
renames, deletions, and non-file conflict notices have explicit rows.

The footer stays visible while the body scrolls. Initial focus is Cancel.
Up/Down traverse file rows and footer actions; Left/Right switch between footer
actions. Enter inspects a file or activates the focused action. `a`/`A` approves
and `c`/`C` cancels from the review root. Escape, `Ctrl-c`, and the effective
leader dismiss through the cancellation path. Reopening or recomputing a review
resets focus to Cancel.

Enter on a file opens a detail page inside the same overlay. Clean files show
the proposed patch against current; conflicts show labelled Base, Current,
Other, and provisional Result where available. Support scrolling and a visible
Back action. Escape returns to the review with its previous selection and
scroll position. Approval shortcuts are inactive in details; the user returns
to the root before approving. This prevents a detail-navigation key from
accidentally accepting a merge.

Very small terminals use a stacked layout and retain a reachable footer.
Truncated file details say so. An incomplete overview or conflict inventory
must never offer approval as if it were a complete review.

The existing live-terminal acknowledgment rule still applies. If any live
terminal session belongs to this workspace, the root overlay names that fact
and requires the exact destination branch in an input row before approval
becomes enabled. While that row owns text input, letters including `a` and `c`
are text, not shortcuts. Cancellation remains available. This preserves the
current UI contract for changing files beneath running terminal jobs.

### Preview and application contract

Use the modern `git merge-tree --write-tree` interface with machine-delimited
output and captured commit IDs. It computes a tree without changing the index
or working tree, but writes objects to Git's object store. Parse its explicit
conflict output and exit status; conflict markers alone cannot identify all
conflicts. [Git merge-tree documentation](https://git-scm.com/docs/git-merge-tree)

Probe the required capability lazily and cache it per Git executable. If it is
unavailable, disable reviewed merging with an actionable reason; never simulate
by merging and resetting the user's working tree. Capability acceptance must
cover the exact options and output shape used, not just a version comparison.

Before implementing the UI, verify preview/application equivalence in fixtures
for rename detection, multiple merge bases, attributes, custom merge drivers,
and relevant configuration. Preview uses the same supported strategy and
options as application. Unsupported configurations must produce a clear refusal
instead of an apparently exact preview. Custom merge drivers may execute during
preview; account for their cancellation and observable side effects. Do not
claim that preview is a sandbox or runs no repository-configured tools.

A prepared merge holds the repository/worktree identity, destination symbolic
ref and OID, source full ref and OID, index fingerprint, clean disk baseline,
relevant editor buffer revisions, operation state, and effective merge settings.
Approval queues that exact plan once. Under existing repository ordering,
recheck these values immediately before applying; any mismatch invalidates the
review and requires a fresh preview. In particular, a moved source ref must not
silently substitute a different commit, and a switched destination must never
receive an old approval.

Use an explicit fast-forward policy and no autostash. A true merge always stops
before committing, including a clean or textually unchanged result. Continue
then uses Runyte's commit message buffer, and only explicitly saving that message
creates the commit. Git's `--no-commit` does not stop a fast-forward, so the
overlay must describe these two outcomes separately: an approved fast-forward
moves the branch to an existing commit; it never creates a merge commit.
[Git merge documentation](https://git-scm.com/docs/git-merge)

Retain normal hooks and signing behavior; do not bypass them for convenience.
Make rerere behavior explicit and verify that cached resolutions cannot silently
stage results the review did not represent. Reconcile actual Git state after
every subprocess outcome. Conflicts are a successful transition to resolution,
not an opaque failed-command notification. Actual conflict entries supersede
preview estimates. Reload clean affected buffers, preserve dirty buffers, and
refresh status, branch drift, comparisons, and gutter bases.

Before approval, Cancel dismisses the plan without changing HEAD, index, or
working files. After approval, cancellation is a subprocess request, not an
undo promise: inspect the resulting repository and expose recovery. Never use
an automatic hard reset. The process-wide repository lock does not exclude an
external CLI or a different host process; retain Git's own locking and report
stale or uncertain outcomes rather than claiming full cross-process atomicity.

Closing a review, switching workspace, detaching, or superseding its originating
request invalidates unconsumed approval. Late results cannot open a review in
an unrelated view. An already submitted mutation belongs to its originating
workspace host and reconciles there, without changing a newly active workspace.

## Fetching one branch

`Tab f` on a remote row fetches that row's exact server ref into its corresponding
remote-tracking ref. On a local row it fetches the configured upstream, even if
that upstream has a different branch name. A local branch without a remote
upstream gets a clear disabled reason explaining that an upstream must be
configured first. Local `.`
upstreams are not network fetch targets. Fetch does not move local branch tips,
switch branches, merge, rebase, or require a clean working tree.

Resolve remote names and refspecs from structured configuration; never split
`origin/feature` at the first slash to infer identity. Validate options and
refspec destinations at the execution boundary, including configured negative
and unusual mappings. Limit writes to the intended remote-tracking namespace;
unsupported mappings fail clearly instead of updating a local branch. Updating
a remote-tracking ref after an upstream force-push is allowed and reported.
This action never force-pushes or force-updates a local branch.

Build an explicit single-branch refspec and constrain opportunistic ref updates
so the action cannot expand into a whole-remote fetch through configuration.
Do not add implicit pruning, tag fetching, or tag overwrites. Explain when a
configured mapping excludes the selected branch; do not silently rewrite remote
configuration. Test that other remote-tracking refs and tags remain unchanged.
Branches not yet known locally require discovery outside this workflow; there
is no `Tab F` whole-remote action in this scope.
[Git fetch documentation](https://git-scm.com/docs/git-fetch)

Reuse bounded asynchronous network execution, credential-helper support,
noninteractive failure behavior, progress, and cancellation. On success refresh
branch rows and drift while retaining the selected full ref. On interruption or
failure re-read refs because part of the fetch may already have completed.
Opening the branch list and `Space g r` remain local reads.

## Conflict resolution

### Conflict list and file editing

`[git conflicts]` is a read-only special buffer listing unresolved paths and
their conflict kinds, sourced from unmerged index entries. Its header names the
active operation and the available Current/Other identities. It is useful for a
merge started in Runyte or the CLI. Empty state distinguishes “No conflicts”
from “Merge ready to commit.” Saving a generated list never writes a file.

Enter opens the ordinary working file with a conflict navigation context. The
file remains editable using Runyte's normal selections, transactions, undo,
save, search, and splits. A conflict-specific `Tab` group offers:

| Action | Default binding | Effect |
| --- | --- | --- |
| Next / previous conflict | `Tab n` / `Tab p` | Move between unresolved regions, then files |
| Keep current region | `Tab o` | Replace the selected conflict region with Current |
| Take other region | `Tab t` | Replace the selected conflict region with Other |
| Inspect sides | `Tab d` | Read-only Base/Current/Other inspection with return navigation |
| Mark file resolved | `Tab r` | Stage this saved, reviewed file or deletion |
| Return to conflict list | `Tab l` | Restore the selected path and viewport |

Use named identities in action descriptions: during a merge, Current is the
destination at merge start and Other is the incoming source. Never infer stage
meaning solely from a marker label. If conflicts belong to a rebase or other
operation, show operation-specific stage descriptions; do not misleadingly call
the rebased branch “Current.” Generic inspection and manual resolution may work,
but merge continue/abort must be disabled for non-merge operations.

Keep/take affects one recognized region, preserves surrounding edits, and is one
undoable transaction. Custom rewriting is ordinary editing. Recognize standard,
diff3, and zdiff3 marker layouts and configured marker widths; invalidate region
identities after edits and refuse ambiguous/malformed regions rather than
guessing. Marker discovery assists text navigation; index entries remain the
authority for whether a path is unresolved.

Saving does not mark a file resolved. `Tab r` requires saved buffer contents,
matching disk/index/stage identities, and no recognized unresolved regions.
Show that it stages the entire file, including manual edits. Marker-like text
that is intentional needs an explicit reviewed override rather than an absolute
ban. Stage only the reviewed path or related structural conflict group, then
verify that its unmerged entries are gone. Do not fall back to staging everything.
Undo after staging makes the file dirty again; it does not secretly unstage it.

Provide separately labelled whole-file Keep current / Take other actions for
binary and non-region choices. These review replacement/deletion, check dirty
buffers, and never silently discard manual resolution work. Modify/delete,
add/add, rename/rename, rename/delete, mode, symlink, file/directory, and submodule
conflicts must all appear. Offer only valid side choices. Where a structural
conflict cannot safely be expressed as a single-file action, show related paths
and an explanation, support manual file management, and validate the entire
group before marking resolved. No automatic submodule merge is promised.

Refresh preserves selection by path and region identity when possible. External
resolution removes completed rows. Edits during an asynchronous read invalidate
its result. Closing a file or the conflict list never aborts a merge or discards
resolution work.

### Completing and aborting

Continue refreshes operation state and the index, refuses remaining unmerged
entries and unsaved resolution buffers, and shows everything that will be
committed. Seed the existing commit message buffer from the active merge message
and preserve merge parents. Guard saving that message against a changed HEAD,
merge identity, or index so stale review cannot commit a different resolution.
A failed hook/signature keeps both message and merge available for correction.
Cancelling message editing leaves the pending merge intact.

Abort is a separate confirmation with Cancel selected by default. It explains
that resolution work may be discarded, refuses unsaved affected buffers until
handled, rechecks the active merge identity, then uses Git's merge-abort flow.
Refresh from actual state on failure; never substitute a hard reset. Test
post-merge edits and clean pending merges as well as conflicts. Fast-forwards
have no active merge to abort; no “Abort” action is advertised for them.

## Commit network buffer

Open `[git network]` through `Space g n`. Default scope is commits reachable
from local branches, cached remote branches, tags, and HEAD; opening it performs
no network access. A scope action can restrict it to the current branch or a
selected ref. Display the scope in buffer metadata.

Proposed narrow, one-commit-per-row layout:

```text
8f32c91  AB  ●─╮  [HEAD → main] Merge search improvements
74e290a  CD  │ ●  [feature/search, origin/feature/search] Add search tests
04d8a22  AB  ● │  Clarify status messages
91ac270  CD  ●─╯  Extract matcher
0eb7613  EF  ●    [v0.3.5] Release preparation
```

Hashes occupy the leftmost column, followed by author initials, graph lanes,
ref labels, and subject. Initials use Unicode graphemes and a stable fallback;
the commit detail supplies the full author identity. Hashes are muted, HEAD is
distinct, and lane colors come from theme roles. Node/edge shapes and labels
carry meaning without color. Provide ASCII fallback glyphs. Long decorations
and subjects remain reachable by horizontal scrolling; graph rows do not wrap.

Enter reuses `OpenGitCommit` with the row's full OID, including merge commits.
Returning restores the graph page, commit, and scroll position. Search, copying,
normal movement, splits, help, and special-buffer retention behave like other
generated buffers. Keep graph geometry coherent with document row identities;
searching or selecting text must not turn a drawn connector into another commit.

Read structured commit and parent records in topological order, then compute
lane geometry in a pure module. Do not parse ANSI output from `git log --graph`:
Git may emit connector-only lines, which conflicts with the requested one-row
contract. Topological ordering keeps parents below their children.
[Git log documentation](https://git-scm.com/docs/git-log)

Use a dedicated graph cursor, not the current single-boundary log cursor.
Capture root OIDs and ref labels for each graph generation; page continuation
retains traversal and lane state. Ref changes mark the generation stale and
explicit refresh captures new roots. A refresh retains the selected OID where
it remains reachable. Backward navigation restores identical lanes. Bound
page size, retained pages, total traversal, lane count, and protocol payloads;
show continuation or an explicit graph-limit state rather than inventing edges
or silently dropping parents. Shallow boundaries are visible.

A focused prototype must prove routing for merges with several parents,
crossings, disconnected roots, and page boundaries using exactly one commit
row. If the compact layout cannot show an edge unambiguously, provide a labelled
overflow indicator and parent inspection; never draw a false topology. Numerical
budgets are fixed and tested in that prototype before production integration.

## Implementation ownership

Keep Git invocation and parsing in `src/git/`. Extend `GitProvider`, typed
operations/responses in `src/git/service.rs`, and `GitCliProvider` through
focused merge, fetch, conflict, and network modules. Keep pure conflict-region
parsing and graph layout independent of UI and subprocess execution.

Add focused app coordinators alongside `src/app/git_comparison.rs` for merge
review, conflict navigation, and network state instead of growing
`src/app/git_workflows.rs` into every feature. Reuse comparison return context
and complete-file rendering where their ownership contracts fit. Every text
resolution goes through the transaction layer.

Add `GitConflicts` and `GitNetwork` buffer kinds, semantic row identities,
generated-view refresh handling, retention, help scopes, and navigation. Extend
`src/snapshot.rs`, private `src/protocol/` DTOs and version, and TUI rendering for
merge review pages, focus, footer, and graph spans. Bundled frontends render
owned snapshots without reading live Git state. No public plugin protocol
extension is needed.

Reuse workspace-host ownership and process-wide Git ordering; inspect shared
common-directory versus worktree-specific state carefully. Git merge/index
state belongs to the selected worktree, while refs and objects may be shared.
Long work remains off input/render paths. Reads are bounded and revision-tagged;
mutations are ordered and cannot be duplicated by repeated approval keys.

Conflict reads and graph generation run on demand and through existing Git
refresh events. Do not add an idle polling loop or graph work to startup.
Measure readiness/idle changes against `context/reference/startup-performance.md`
if scheduling or startup paths change.

## Delivery sequence and acceptance

1. **Establish backend contracts.** Add operation-state inspection, merge
   capability probing, typed previews, conflict inventory, and guarded mutation
   results. Prove no working-tree/index changes from preview and agreement with
   actual merge fixtures before building approval UI.
2. **Ship fetching.** Add the single-branch action, exact destination handling,
   refresh preservation, cancellation, and help. This slice is independent of
   merge/conflict UI.
3. **Build conflict resolution.** Deliver the conflict list, transactional region
   choices, manual editing, guarded whole-file/group resolution, merge message
   continuation, and confirmed abort. Exercise merges started by the CLI first.
4. **Connect reviewed merging.** Add nested review details, footer navigation,
   default cancellation, terminal acknowledgment, stale-plan refusal, and all
   post-approval outcomes. Merge approval is not complete until resolution and
   abort are available.
5. **Build the network view.** Prove compact graph routing and bounded paging,
   then integrate buffer actions, snapshots, theming, and commit return context.
6. **Complete documentation and platform acceptance.** Update the user guide,
   Git command tables, README feature summary, Helix keymap register, and UI
   vocabulary together. Replace the blanket recommendation to use Lazygit for
   conflicts with the actual supported workflow and remaining limits.

Each implementation slice includes tests at its behavior boundary. Proposed
test homes are `tests/git_provider.rs`, `tests/git_parsing.rs`, new focused
`src/git/tests/` modules, `src/app/tests/` Git workflow tests, snapshot/protocol
round trips, and existing TUI/PTY acceptance suites.

Required scenarios include:

- Preview and application: fast-forward, already contained, clean merge,
  content and structural conflicts, multiple merge bases, no common ancestor,
  unsupported Git/configuration, output limits, unusual paths, SHA-256 object
  IDs, moved refs, changed index/disk/buffers, locks, hook failure, cancellation,
  and custom-driver/rerere behavior.
- Interaction: the overlay opens for clean, conflicted, textually unchanged,
  fast-forward, and already-up-to-date results; approval of a true merge never
  creates a commit; Enter initially cancels; both cases of approval/cancellation
  keys; arrows and footer focus; detail return; small terminals; remapped keys;
  terminal acknowledgment; duplicate input; late responses; detach, workspace
  switch, and standalone/attached parity.
- Fetch: local upstream with a different name, no upstream, no configured
  remote, absent selected upstream, multiple remotes, slash-containing remote names,
  unusual refspecs, force-updated/deleted upstreams, network/authentication
  errors, option-shaped arguments, interruption after partial progress, and
  proof that only the selected branch is fetched while other refs/tags stay put.
- Resolution: multiple files/regions, manual edits, undo, every marker style,
  literal marker text, dirty-buffer refusal, stage/disk races, binary and
  structural choices, external resolution, correct merge parents, cancelled
  messages, failing hooks, and abort preserving unrelated work or refusing
  safely when preservation cannot be established.
- Network: linear and branching histories, two-parent and octopus merges,
  criss-cross ancestry, identical timestamps, disconnected roots, Unicode
  authors/subjects, many labels/lanes, detached/unborn HEAD, shallow history,
  page boundaries, stable back navigation, changed refs, limits, and commit
  detail return position.

Use temporary repositories and local bare remotes for deterministic tests.
Subprocess fixtures isolate `XDG_CONFIG_HOME` and explicit configuration;
hook/driver fixtures link the checked-in `src/fixtures/stand-in` and use behavior
data files. Never execute a test-written program or write test state into the
repository's runtime/configuration directories.

Before handing off Rust changes run `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, and canonical
`cargo llvm-cov --locked --workspace`. Keep the 89% coverage floor on Linux and
macOS and complete native Windows build/lint/test acceptance. Include measured
large-repository interaction and idle checks; report platforms actually tested.

The merge-review and single-branch-fetch decisions are settled and implementation
is approved. Retain the architectural rationale under `completed/` after delivery.
