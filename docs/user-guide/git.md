## Git

Git support needs `git` on `PATH` and a project inside a Git working tree.
Otherwise there is simply no gutter and no branch in the status line, and the
`Space g` namespace is dimmed.

### Git keys

| Key | Action |
| --- | --- |
| `Space g g` | Open the changed-file list |
| `Space g d` | Open the active file's unstaged diff |
| `Space g D` | Compare the active file's complete Git versions side by side |
| `Space g b` | List local and cached remote branches and check one out locally |
| `Space g w` | Open the repository worktree list |
| `Space g l` | Open paged commit history |
| `Space g n` | Open the cached commit network |
| `Space g f` | Fuzzy-search commits in a hash/title list with author, date, and full-message preview |
| `Space g B` | Open live-buffer attribution for the whole file |
| `Space g t` | Open the bounded stash list |
| `Space g r` | Re-read branch, changed files, and changed lines from Git |

Every Git view uses `Tab` for its actions; see
[Contextual actions](the-screen.md#contextual-actions-tab). The `:git-…` commands are listed
under [Commands](commands.md#commands).

**A typical commit:**

1. `Space g g` — open the changed-file list.
2. Select rows, then `Tab s` — stage them.
3. `Tab c` — write the message in the buffer that opens.
4. `:w` — commit.
5. `Tab P` — push.

### Status line and gutter

**Status line.** `main ↑1 +2 ~3 -1 ?4 !1` reads as: branch `main`, one commit
ahead of upstream, two added, three modified, one deleted, four untracked, and
one conflicted file. Each file is counted once, under the most consequential
thing that happened to it.

**Gutter.** Tracked files get a change mark between the line number and its
separator:

| Mark | Meaning |
| --- | --- |
| `+` | Added line |
| `~` | Modified line |
| `-` | Lines were removed here (on the surviving row that closed the gap) |

- Marks compare against the **index**, not `HEAD`, so staging a change removes
  its marks.
- A mark appears only on a logical line's first screen row; soft-wrap
  continuations use the cell for their `↪` arrow.
- Fold triangles share the cell. Only when a folded line is itself changed
  does the gutter add a second cell so both `▸` and the mark show.
- Files with no unconflicted index entry — untracked, mid-merge, or binary —
  show no change column, rather than one claiming every line is new.

**How marks stay current.** Git is asked for a file's staged text once, when
it opens and after relevant Git changes while it is visible. Everything else
is diffed in memory, so marks follow typing without a process per keystroke.

- Large files are compared on a background worker: a changed gutter clears
  briefly, then shows marks for the newest completed revision.
- Hidden buffers reload their staged text only when a pane shows their gutter.

### Automatic refresh

Filesystem changes to the worktree, index, `HEAD`, refs, packed refs, stashes,
and linked-worktree metadata trigger one debounced background refresh.
`git.refresh_interval_seconds` bounds how often (see
[Automatic Git refresh](configuration.md#automatic-git-refresh)). `:git-refresh` or
`Space g r` reconciles immediately.

**A refresh waits while you work**, because it rewrites a Git view's text and
moves the cursor. It is held back:

- until you pause briefly;
- while a prompt is open, including `s` and `/` queries;
- while a Git view holds a deliberate selection, such as the matches `s`
  leaves.

Nothing is dropped: the refresh runs as soon as you stop, the prompt closes,
or the selection collapses. A selection in an ordinary file defers nothing,
because a refresh changes its gutter, not its text.

**The cursor keeps its place.** Its row is matched by identity — the commit,
path, branch, or hunk it was on — and falls back to the nearest surviving
row. The column is clamped to the end of whatever row it lands on.

### Changed files, staging, and committing

`Space g g` opens the changed-file list: files grouped by whether a commit
would take them, one per line. A selection over several rows is a selection of
files, and one key acts on all of them.

```text
# main ↑1 · 2 staged · 1 not staged · 1 untracked · +105 -28

Staged
  M  +82  -12  src/app.rs
  A  +20   -0  src/git/stats.rs

Not staged
  M   +3  -16  README.md

Untracked
  ?    ·    ·  logo.png
```

| Key | Action in the list |
| --- | --- |
| `Tab s` / `Tab u` | Stage / unstage every file the selection covers |
| `Tab S` | Stage every unstaged and untracked file in the list |
| `Enter` | Show this row's diff |
| `Tab d` | Compare this row's complete versions in a temporary split |
| `Tab o` | Open the file on this line |
| `Tab D` | Discard the selected files' changes |
| `Tab c` | Write a message and commit what is staged |
| `Tab i` | Review everything staged for the next commit |
| `Tab p` / `Tab P` | Pull / push the branch this working tree is on |
| `Space g r` | Re-read everything from Git |

#### Reading the changed-file list

**The first row** names the branch, its drift from upstream, and how many
files are in each section. It counts sections rather than per-file states, so
it never calls a file "modified" above a heading that calls it staged. The
status line's compact `~1` answers a different question: what changed, rather
than where it sits.

**Line counts** show what each change costs, added then removed, in one
aligned column; the first row totals them.

- They use the theme's `change_added` and `change_removed`, the same colours
  as the gutter and diffs. Built-in themes use green and red; custom themes
  can change either.
- They are Git's own `--numstat` counts of the same two trees the row comes
  from. A file staged and then edited again is counted once on each of its
  rows, and the total is the sum of what is shown.
- An untracked file counts every line as added.
- A change that cannot be counted — a binary file, an untracked symlink, a
  file over a megabyte, or a whole untracked directory Git collapsed into one
  row — shows `·` in both columns and is left out of the total. A list with no
  countable rows omits the columns.
- Counts are read only while the list is open.

**A file in two sections** (staged, then edited again) is not a duplicate:
those are two changes, staged separately. `Enter` on a staged row shows what a
commit would take; on an unstaged row, what it would not. A selection covering
both acts on the file once.

**After staging**, the caret follows the file into its new section, so `Tab u`
undoes what you just did.

#### Staging

- `Tab s` records each file **as written on disk**. When a buffer has unsaved
  changes, an INFO notification says so.
- Staging moves the gutter's base, so marks for staged lines disappear at
  once.
- For a rename, the displayed destination is what you open or diff, while
  staging and unstaging act on both paths, so a move is never split across the
  index.
- The changed-file list reads files on disk, not buffers — the one place the
  gutter and the diff views can disagree. The diff header says so when a
  buffer has unsaved changes.

**Hunks.** In a per-file diff, `Tab s` stages the exact hunk under the cursor
and `Tab u` unstages the exact staged hunk. The request carries the hunk bytes
plus repository, HEAD, index, file, and live-buffer preconditions; Git checks
the patch, and any stale precondition changes nothing.

**Lines.** `:git-stage-lines` stages a deliberately narrow slice from a saved,
clean source buffer: one contiguous selection containing every added or
modified new line in one hunk.

- Refused: dirty buffers, deletion-only choices, multiple or partial hunks,
  binary files, conflicts, renames, and untracked files.
- A refused partial action never becomes whole-file staging.
- Use Lazygit for finer patch surgery or advanced history work. Native merge
  review and conflict resolution are described below.

#### Committing

`Tab c` opens a commit message buffer with the template Git would give an
external editor: an empty first line, then commented instructions and the
files to be recorded.

| To | Do |
| --- | --- |
| Commit | Write the buffer: `:w` or `:wq` (`:wq` does not exit Runyte here) |
| Cancel an unchanged message | `:c` |
| Cancel an edited message | `:c!` |

- The index is refreshed first, so staging done outside the editor is
  included.
- A commit takes **the index** — exactly what the Staged section shows.
- Comment lines are not part of the message, so a message line cannot start
  with `#`.
- Cancelling commits nothing and leaves the index as it was.
- A refused commit (an unset identity, a rejecting hook) keeps the message in
  the buffer to fix and retry.
- Afterwards the pane returns to where you started.
- Hooks run in the background commit, so a slow `pre-commit` leaves the editor
  responsive. A signing key that wants a terminal prompt fails rather than
  asking.

#### Discarding

`Tab D` restores the selected files to `HEAD`, throwing away both staged and
unstaged changes.

- It is the only Git action here that cannot be undone: the content was never
  a commit, so no reflog brings it back. It asks first and names what it will
  take.
- A file with unwritten buffer edits is refused, so Git never overwrites text
  that exists only in the editor. Clean buffers reload afterwards.
- Discarding a staged addition removes its clean open buffer with the file.
  Discarding a rename restores the original path and removes the destination.
- Untracked files are refused. Discarding one could only mean deleting it,
  which Git keeps behind `clean` and Runyte keeps in the explorer, where
  deletion is a confirmed plan that goes to the trash.

### Diffs

| Key | Opens |
| --- | --- |
| `Space g d` | The active file's unstaged patch |
| `Space g D` | Two complete, aligned versions: index on the left, working tree on the right |
| `Tab i` | Everything staged for the next commit |

- In the changed-file list, `Space g d` and `Space g D` follow the selected
  row, so a staged row compares `HEAD` with the index.
- A missing side of an added or removed file is shown as hatched filler.
- Patches are coloured by line: added and removed lines in the gutter colours,
  hunk positions in the accent colour, headings muted. The same colouring
  applies to all generated patch views.

**The side-by-side pair is temporary.** Closing either pane removes the split
and returns the surviving pane to the buffer active before the comparison.
`:diff-off` (`:do`) does the same from either side. If that buffer has since
been closed, the pane opens the workspace explorer.

### Branches

`Space g b` opens one read-only list, Local first and Remote second:

```text
Local
  feature [↑1] [worktree: /home/me/project-feature]
* main    [↑2 ↓1] [worktree: /home/me/project]
  spike

Remote
  origin/feature   [tracked by: feature]
  origin/main      [tracked by: main]
  origin/review/42 [not tracked locally]
```

| Key | Action in the list |
| --- | --- |
| `Enter` | Check out this local or remote branch locally |
| `Tab n` | Start a new branch here and switch to it |
| `Tab w` | Create a worktree for this branch; attach in mux mode |
| `Tab d` | Compare committed tips with this branch |
| `Tab D` | Delete this local branch, with its worktree and session, after a confirmation |
| `Tab f` | Fetch this cached remote branch or this local branch's upstream |
| `Tab m` | Review merging this selected branch into the current branch |
| `Tab p` | Fast-forward the current local branch onto what it tracks |
| `Tab P` | Publish this local branch to what it tracks |

#### Reading the branch list

- `*` marks the current local branch.
- `[worktree: ...]` names the local path of each registered worktree that has
  the branch checked out.
- **Drift** is shown in a dimmed column of its own:

  | Marker | Meaning |
  | --- | --- |
  | `↑N` | N commits the local branch has that its upstream lacks |
  | `↓N` | N commits the upstream has that the local branch lacks |
  | `[=]` | In step |
  | `[gone]` | The upstream ref no longer exists |
  | (nothing) | The branch tracks nothing |

- **Remote rows** are remote-tracking refs Git has already cached; opening the
  list and `Space g r` do not fetch.
- A symbolic default such as `origin/HEAD` is not a row. Remote names
  containing `/` are kept exact.
- Each remote row names every local branch whose configured upstream is that
  exact ref. Equal names or equal tips do not count as tracking; a remote row
  with no tracker says `[not tracked locally]`.

#### Checking out

- **A local row:** Enter checks it out.
- **A remote row:** Enter checks out its one local tracking branch. With
  several, a picker opens. With none, Runyte creates and checks out a tracking
  branch named after the part after the remote: `origin/feature/auth`
  suggests `feature/auth`. If that name already exists, an editable name
  prompt opens instead of reusing the unrelated branch.

**Refusals and confirmation:**

- A checkout (and `Tab n`) is refused while the index or working tree has
  staged, unstaged, or untracked changes, or an open file buffer in the
  repository has unsaved edits.
- If this workspace owns any live terminal session — visible, hidden, or in
  another pane — Runyte asks you to type the exact target branch name. That
  acknowledges the terminal job keeps its directory while Git replaces files
  under it. Escape or `Ctrl-c` leaves the checkout unchanged. Exited terminals
  need no confirmation.
- After a checkout, open files that still exist are reloaded from the new
  branch, and the Git status and gutter bases are refreshed.
- Opening another worktree with `Space g w` is an attachment switch, not a
  checkout, and does not ask.

#### Creating and deleting branches

**`Tab n`** asks for a name, creates the branch at the selected one, and
switches to it. The checkout refusals are reported before the name is asked
for.

**`Tab D`** first reviews the exact branch tip.

- If the tip is kept by the configured upstream or another local branch, Enter
  confirms, and the confirmation names those branches.
- Otherwise you must type the exact branch name.
- Upstream reachability uses the locally cached remote-tracking ref, and the
  confirmation says so. Fetch first if it must include newer remote state.
- The final step rechecks the tip and the refs keeping it, so a branch changed
  after review is not deleted.

**A branch checked out in a worktree** is not refused. A worktree means nothing
without its branch, and a session means nothing without its worktree, so
deleting the branch offers to remove all three. The confirmation names every
level and always requires the exact branch name:

```text
Delete branch enh/render-space.
This also:
  · stops and forgets session 5 (runyte-enh-render-space)
  · removes worktree /home/me/code/runyte-enh-render-space
Type enh/render-space exactly to continue.
Escape keeps it.
```

- It runs bottom-up: the session stops, the worktree is removed, then the
  branch is deleted. A failure at any level stops there.
- The checkout's own refusals — a dirty working tree, a lock, a session with
  unsaved buffers — are reported before you are asked anything.
- A branch checked out in more than one worktree, or at the current Runyte
  root, is still refused before review.

#### Creating a worktree from a branch

`Tab w` asks for a destination and creates a checkout of the selected local
branch.

- A branch already checked out here or in another worktree is refused without
  forcing a duplicate; the message names the existing path and points to
  `Space g w`.
- On an untracked remote row, it creates the suggested tracking branch
  directly in the new worktree. A remote with several trackers asks which to
  use.
- In mux mode the new worktree is attached at once. In ide mode
  you stay in the current workspace.

### Worktrees

`Space g w` (`:git-worktrees`) lists every checkout registered with the
repository: linked, detached, locked, prunable, and bare.

| Key | Action in the worktree list |
| --- | --- |
| `Enter` | Attach to this root's persistent session, starting it if necessary |
| `Tab n` | Name a new branch at this checkout's tip and create its worktree; attach in mux mode |
| `Tab d` | Compare committed tips with this worktree |
| `Tab D` | Remove this worktree and its session after confirmation; keep its branch |
| `Space g r` | Re-read the registered worktrees |

- `*` marks the current root. Unavailable states are written on their rows.
- Paths are the identity, including paths that are not valid UTF-8 (displayed
  with replacement characters).

**Opening another root** never retargets this workspace's buffers or language
servers.

| Mode | Enter | `Tab n` |
| --- | --- | --- |
| Mux | Detaches the TUI and attaches to the destination's host (starting it if needed). The old host keeps its buffers and terminals. | Creates the worktree and attaches at once |
| Ide | Explains that attaching needs mux mode | Creates the worktree and stays in the current workspace |

#### Removing a worktree

`Tab D` removes one ordinary worktree at a time and never deletes its branch.

**Refused before confirmation:**

- staged, unstaged, or untracked files;
- a running session on it with unsaved file buffers or unavailable health;
- the current Runyte root, and locked, bare, missing, or otherwise unavailable
  worktrees.

**What the confirmation asks for:**

| Situation | Confirm with |
| --- | --- |
| A clean session runs on it | Typed text — the branch name, or the displayed path for a detached checkout (the session goes too) |
| Its branch has commits ahead of the cached upstream, or the upstream is gone | The exact branch name |
| An unretained detached checkout | Its displayed path |
| Otherwise | Enter |

```text
Remove worktree /home/me/code/runyte-enh-render-space.
This also stops and forgets session 5 (runyte-enh-render-space).
Branch enh/render-space will remain.
Type enh/render-space exactly to continue.
Escape keeps it.
```

**Order of events:**

1. Git status, worktree identity, upstream state, and session health are
   checked again.
2. The session is stopped — the host owns the directory and its runtime state.
   A failed stop leaves the worktree standing.
3. Git removes the directory. If Git refuses, the branch and history record are
   left alone.
4. The workspace's history record is forgotten, so nothing keeps a claim on
   the digit it used. This happens whether or not a host was running.

Stopping the session is part of removal, not a `session` command, so it
happens in ide mode too: an ide-mode editor cannot attach to a
session, but it can still find and stop one running on the worktree.

### Committed comparisons

`Tab d` in the branch or worktree list opens a read-only **committed
comparison**: the current tip on the left, the selected local branch, cached
remote branch, or worktree HEAD on the right.

- It compares the contents at the two tips directly. Uncommitted changes,
  staged changes, and unsaved buffers are excluded.
- Nothing is fetched or checked out.
- Detached worktrees are labelled with their commit ID. A worktree with no
  available commit cannot be compared.

| Key | Action in the committed comparison list |
| --- | --- |
| `Enter` | Open this file's unified patch |
| `Tab d` | Open this file's complete versions in a temporary split |
| `Space g r` | Capture fresh tips and refresh the list |

**Reading it:**

- The heading names both sides and their captured commit IDs.
- Each row shows the path on each side and added/removed line counts. The
  summary totals and per-file counts use `change_added` and `change_removed`
  (green and red in built-in themes); only the signed counts are coloured.
- Renames show both names; `—` marks an absent side. Narrow panes combine the
  two paths into one column.
- Binary and metadata-only changes are labelled. Empty added or deleted files
  stay visible.
- Identical tips show `No committed differences.`

**Stability:**

- The list, counts, patches, and splits all use the captured commits, even if
  a branch moves while you browse.
- Refresh keeps the selected file when possible.
- Closing a patch returns to the list at the same file and scroll position.
  Closing either split side, or `:diff-off`, collapses the pair and restores
  that position too.
- Movement, search, selection, and copying work as usual. Binary files show
  their patch metadata but cannot open as a text split.

### History, search, and blame

#### Commit history

`Space g l` opens history in pages of up to 10,000 commits, newest first in
Git's topological order.

| Key | Action |
| --- | --- |
| Enter | Open the commit's metadata and patch |
| `Ctrl-n` / `Ctrl-p` | Next / previous page |
| `Space g r` | Re-read the view |

- **First line:** current and total pages, the earliest and latest author
  dates, and a paging reminder, separated by `|` and within 80 characters.
  The reminder is a muted hint, not buffer text: it cannot be selected,
  searched, or copied.
- **Each row:** short object ID, author date and time as `YYYY-MM-DD HH:MM` in
  the commit's timezone, author, and subject. The full ID is kept behind it.
- **Commit detail:** Enter displays `Author-time: YYYY-MM-DD HH:MM` in the
  commit's original timezone, matching the history row's date and time.
- **Refs:** a commit's branches and tags are a muted hint one space past that
  row's own text, not aligned to a shared column, so a long subject never
  pushes a shorter row's hint off a narrow pane.
- **Paging** is on Ctrl chords so `l` and every other motion keep working.
  Pages use boundary objects instead of an unbounded result, so going back
  re-requests the earlier page.
- **Refresh** keeps the caret on the same commit when it is still on the page,
  even if new commits appeared above; otherwise the nearest row. Only the
  first page refreshes automatically; later pages sit behind a commit boundary
  and cannot change.

#### Commit network

`Space g n` opens `[git network]`, a read-only graph of commits reachable from
local branches, cached remote branches, tags, and HEAD. Opening it never fetches.
Each commit occupies one row: a six-character hash, Unicode author initials,
ASCII graph, branch label, captured refs, and commit title. The branch label uses
the same color as the commit's graph path. Paths retain their columns and colors
until they end; empty columns can be reused without moving surviving paths.
The graph does not wrap; horizontal scrolling reaches long names and titles.

`*` marks a commit, `+` marks a parent connection, and `-` and `|` draw the
rectangular paths. A `-|-` crossing passes through an unrelated path without
joining it; its vertical stroke keeps that path's color. Horizontal segments use
the destination path's color. Where several parents share a horizontal segment,
the nearest endpoint supplies its color. Four theme colors repeat as needed, so
color alone does not identify a branch.

Branch labels follow captured branch tips along first-parent paths. Git does not
record the branch on which a commit was originally created; shared history keeps
the surviving displayed path's label. The current symbolic HEAD name is preferred,
then a local branch, then a cached remote branch. Paths without a known name
fall back to local branch containment: `[in dev]` means the commit is reachable
from exactly one local branch, and `[in dev, ...]` means it is reachable from
several. The displayed containing branch is the current local branch when it
contains the commit; otherwise it is the branch with the newest tip commit
(committer timestamp, with branch name breaking ties). This is current
membership, not historical branch ownership. Remote branches and tags do not
count toward this fallback. `[?]` means neither a path name nor local containment
is known. Captured names and membership remain fixed until explicit refresh.

Enter opens full commit detail, including the full object ID, all merge parents,
and `Author-time: YYYY-MM-DD HH:MM` in the author's original timezone. Short hash
collisions do not affect navigation: each row retains its full commit identity.
Closing detail returns to the same page, selection, and viewport. `Ctrl-n` and
`Ctrl-p` move through cached 200-commit pages. The first line names the scope,
page, continuation, and any limit. Tab opens these registered actions:

| Action | Default key | Colon command |
| --- | --- | --- |
| All cached refs and HEAD | `Tab a` | `:git-network-all` |
| Current HEAD only | `Tab h` | `:git-network-head` |
| Choose an exact captured ref | `Tab r` | `:git-network-choose-ref` |
| Next graph page | `Ctrl-n` | `:next-git-network-page` |
| Previous graph page | `Ctrl-p` | `:previous-git-network-page` |

`:git-network-ref refs/heads/main` selects an exact full ref directly.
`:git-network` or `Space r` captures new roots; refreshing keeps the selected
commit while it remains within the bounded traversal. Continuation uses the
original object IDs and labels even if refs move, marking that generation stale.
No remote access or idle graph scan runs.

Containment is captured independently of the selected graph scope, from up to
256 local branches and a topological walk of up to 10,000 commits. An incomplete
branch inventory disables the fallback; commits beyond that ancestry walk also
retain `[?]` when their paths have no name. These bounds, including the 2 MiB
ancestry-output bound, display a separate `local containment limit` notice
rather than guessing a name or claiming a single containing branch from
incomplete evidence. This notice preserves the graph's continuation status;
narrowing the graph scope does not change the containment bounds.

If a parent needs a new lane beyond the 16-lane limit, an `overflow parents`
note lists its abbreviated ID; existing parent lanes remain connected even at
capacity. Enter exposes every full parent ID. Unplaced commits and shallow
boundaries are labelled. Limits are 256 captured refs plus HEAD, 16 lanes,
50 retained pages, and 10,000 commits per generation, with 200 commits per page
and a 2 MiB subprocess-output bound per read. An atomic refresh temporarily
retains at most two bounded generations while locating the selected commit.
Traversal reads immutable ancestry without replacement refs; changing shallow
boundaries requires a fresh generation. Narrow the scope when a graph limit is
reached. Ordinary movement, search, copying, selections, and splits remain
available throughout the network.

#### Searching commits

`Space g f` opens a fuzzy picker over commits reachable from `HEAD`, newest
first.

- It matches subjects, message bodies, the object ID (a prefix is enough),
  the author, and the author date.
- Rows rank as lines, not paths, so `/` is an ordinary character.
- Spaces split the query into terms that must appear in order. The first
  `Space` in an empty filter still closes the picker.
- Rows show the subject, author date, author, and abbreviated ID. The preview
  shows the full message.
- Enter opens the same commit detail as the log and blame.
- Discovery is asynchronous and capped at the newest 5,000 commits; the picker
  says when the cap is reached.

#### Blame

| Command | Shows |
| --- | --- |
| `:git-blame` | Attribution for the primary line, without leaving the file |
| `Space g B` (`:git-blame-file`) | A read-only view aligned to the source rows; Enter opens the row's commit |

- Both send the current in-memory text to Git's porcelain blame, so an unsaved
  line says `uncommitted` instead of borrowing an older attribution.
- A result is discarded if the buffer changes while Git works.
- Inputs over 4 MiB, and whole-file views over 20,000 lines, are refused.
  Untracked or otherwise unblamable files report Git's refusal.
- The full-file view also shows each commit's author date as `YYYY-MM-DD`, in
  the commit's timezone.

### Stashes

`Space g t` opens a bounded, read-only stash list. Rows keep full stash object
identities.

| Key | Action |
| --- | --- |
| `Tab a` | Apply the selected stash, keeping it |
| `Tab D` | Drop it, after confirmation |
| `Space g r` | Refresh, keeping the selected stash |

Creating a stash uses a command that names its scope:

| Command | Effect |
| --- | --- |
| `:git-stash-tracked <name>` | Records the tracked worktree and index, leaving staged changes applied |
| `:git-stash-all <name>` | Records the same tracked state, and clears both worktree and index |
| `:git-stash-untracked <name>` | Additionally includes untracked files |

- Every create, apply, and drop asks for confirmation.
- Create and apply are refused while the repository has unsaved editor
  buffers.
- An apply conflict keeps the stash. Open `Space g c` to inspect unmerged
  stages and review supported saved-file resolutions; unsupported structural
  or submodule cases can still use an external Git tool. Resolving does not
  remove the retained stash.

### Reviewed merges and conflicts

In the branch list (`Space g b`), `Tab m` reviews merging the selected local
or cached remote branch **into the current branch**. Cached remote tips reflect
the last fetch; merge review does not contact the remote. The native overlay
names both branches and captured commit IDs, lists proposed file changes, and
predicts already up to date, fast-forward, clean pending merge, or conflicts.
Already-contained requests still open the review with approval disabled.

Cancel is selected initially. Up/Down visit file rows and footer actions;
Left/Right choose a footer action. Enter inspects a file or activates the
selected action. `a`/`A` approves and `c`/`C` cancels at the root. Escape,
`Ctrl-c`, and the configured leader cancel. The footer remains visible in
narrow terminals. Details contain the proposed patch or labelled Base,
Current, Other, and provisional Result where available. Back returns to the
same root selection and scroll; approval shortcuts are inactive in details.
Long details and overviews remain scrollable with explicit continuation.

Live terminal sessions require typing the exact destination branch before
approval. While that input has focus, letters are text. Unsaved repository
files, concurrent file/index/ref changes, stale previews, unsupported custom
merge drivers or filters, unrelated histories, and unsupported Git versions
are refused. A stale review must be opened again.

Approval applies the reviewed merge with no autostash and no automatic merge
commit. A fast-forward has no pending merge to continue or abort. A successful
non-fast-forward merge remains pending even if clean or textually unchanged;
creating its commit always requires a separate reviewed Continue and explicit
message save. Actual index conflicts replace the simulated conflict inventory.

`Space g c` / `:git-conflicts` opens `[git conflicts]`, a read-only list from
unmerged index stages. It supports merges started in Runyte or with the CLI.
The header identifies the active operation and sides; empty state distinguishes
“No conflicts” from “Merge ready to commit.” Enter opens the ordinary working
file at its first recognized region. Editing, language tools, undo/redo, save,
search, copying, and splits keep their ordinary meanings.

| Conflict action | Default key |
| --- | --- |
| Next / previous unresolved region, then file | `Tab n` / `Tab p` |
| Keep Current / take Other for one region | `Tab o` / `Tab t` |
| Inspect index sides | `Tab d` |
| Review staging the saved complete file or deletion | `Tab r` |
| Explicit review allowing intentional literal marker text | `Tab R` |
| Return to the conflict list | `Tab l` |
| Review the complete Current / Other side, including deletion | `Tab O` / `Tab T` |

The conflict list also offers sides, staging, and complete-side actions, so
absent working paths remain manageable. Region choices are one undoable text
transaction and preserve edits outside the region. Standard, diff3, and
zdiff3 markers use the path's configured marker width; malformed marker blocks
are refused. Saving changes the working file without resolving its index
stages. `Tab r` reviews and stages that exact saved file or deletion; remaining
markers require the explicit `Tab R` override. Undo after staging makes the
working file dirty without undoing the staged resolution.

Binary content is labelled. Related paths for structural conflicts are shown
with an explanation; review each manually saved file or deletion. Unsafe whole
side choices for structural groups and submodules are disabled. In a rebase,
cherry-pick, or other non-merge operation, inspection uses stage identities;
Runyte stages supported file resolutions but continuation/abort use Git.

In the conflict list, `Tab c` continues and `Tab A` aborts. Git status offers
`Tab C` and `Tab A`; ordinary Commit routes an active merge through its reviewed
Continue workflow too. Continue refreshes operation state and the index and
requires all index conflicts resolved. Its approval opens the seeded merge
message; only saving that exact buffer creates the commit. A rejected hook
retains the message and review authority for retry while its captured state
remains valid. Cancelling message editing leaves the merge pending.

Abort is a separate review with Cancel selected. It explains that resolution
work and post-merge edits can be discarded; unsaved repository buffers must be
saved first. Closing a conflict file or list never aborts a merge. Git versions
without the required simulated-merge capability refuse preview safely.

### Fetch, pull, and push

`Tab f` in the branch list fetches one branch. Pull and push use `Tab p` and
`Tab P` in the branch list and the changed-file list. `:git-refresh` re-reads
what is already local.

#### Fetch (`Tab f`)

- A remote row fetches its exact server branch into the selected remote-tracking
  ref. A local row fetches its configured upstream, including a differently
  named or currently missing upstream.
- Configure a remote upstream first for a local branch that has none. A local
  `.` upstream is not a network target. Excluded or ambiguous fetch mappings,
  symbolic destinations, and mappings outside `refs/remotes/` are refused.
- Only the selected remote-tracking ref changes. Other cached branches and tags
  stay in place; no pruning or tag fetching occurs. A force-pushed upstream may
  replace that cached ref, and the retained Git operation notification identifies
  the forced update.
- Local branches, the index, and working files remain intact. Unsaved buffers
  and a dirty working tree do not prevent fetching.
- The branch list refreshes and retains its selected ref, including after
  failure or cancellation. Discovering branches not yet known locally belongs
  in an external Git tool; there is no whole-remote fetch action.

`:git-fetch-branch` performs the same action on the selected branch-list row.

#### Pull (`Tab p`)

| Situation | What happens |
| --- | --- |
| A fast-forward is possible | It happens silently: the branch had no commits of its own to lose |
| Both sides have new commits | It shows the drift and offers to replay your commits on theirs |
| Unsaved buffers in the repository | Refused first, since pulled files are reloaded |
| Dirty worktree | Refused |

The offer reads like:
`main and origin/main have both moved on. Press Enter to replay 2 local commits on top of the 1 on origin/main`.
Escape leaves the branch as it was.

- **The replay is a rebase**, so there is never a merge commit whose message
  nothing here could write. It rewrites the replayed commits; they stay
  reachable from the reflog under their old identities, which is why it asks.
- **On conflict it undoes itself:** the rebase is aborted, the working tree
  keeps what it held, and the refusal says so. Runyte has no surface for
  resolving conflicts, so it never leaves you holding one; finish those in
  your usual Git tool.
- **Afterwards** open files are reloaded and gutter bases refreshed, as after a
  checkout.
- **No autostash.** Uncommitted changes are never stashed, even with
  `merge.autoStash` or `rebase.autoStash`. Git reapplies such a stash
  afterwards, and when that conflicts it still exits successfully — leaving
  conflict markers and a stash to recover, with nothing to roll back. A dirty
  worktree is refused up front instead.
- **Fetch and merge run separately**, not as one `git pull`, so an unreachable
  remote is reported as such. The drift offered for replay is read from
  remote-tracking refs, which are only worth reading once a fetch has
  refreshed them.

#### Push (`Tab P`)

- Publishes to the ref the branch tracks.
- The first time, it sets an upstream: `origin` when it exists, or the only
  remote when there is exactly one.
- Nothing forces. A push rejected because the remote has commits you lack is
  reported as such, naming `Tab p` as the way to catch up.

### Background operation and failures

Git discovery, reads, mutations, hooks, fetch, pull, and push run on a bounded
background service, so editing and rendering continue while they queue or run.

**Progress.** A long mutation temporarily replaces the status row with the
action, its target, elapsed time, a cancellation hint, and a rotating
`- \ | /` bar at the right. Every background service using this surface gets
the same spinner.

**Cancelling.** `:git-cancel` stops the current operation. A cancelled
mutation is reported as uncertain and reconciled at once, because cancelling
is not rollback.

**Network safety.**

- Network operations have a two-minute deadline.
- Nothing can prompt: Git's prompts are off, `ssh` runs in batch mode unless
  you set `GIT_SSH_COMMAND`, and no askpass helper falls back to the terminal.
  An authentication needing a password fails with a message instead of
  hanging behind an invisible prompt.

**Environment safety.**

- Repository- and object-selection variables such as `GIT_DIR` and
  `GIT_INDEX_FILE`, and inherited one-shot Git configuration, are removed from
  every child. Starting Runyte from another Git command cannot retarget it.
- File arguments are always literal: a name resembling Git pathspec syntax
  still names only that file.

**Discovery failures.**

- The status line shows the failure with a `:git-refresh` hint, and
  `retrying discovery` while a retry runs.
- `:git-refresh` makes one new discovery attempt in the background. Further
  invocations are unavailable, with a reason, until it finishes. Nothing retries automatically: launch
  failures, signal exits, permissions, and configuration errors all wait for
  you.
- Each of discovery's three local Git reads has a 30-second deadline and
  bounded output. A signal exit does not tell whether the child reached Git.
- Other Git commands stay unavailable. Their palette reasons and the `git` row
  in `:service-health` keep the last diagnostic during a retry.
- A new failure replaces the diagnostic. Success clears it and either
  refreshes the repository or reports that the project is not in a Git
  repository, which offers no retry.
- Failure notifications stay in the notification history.
- Persistent sessions keep the discovery state and notifications across
  detach and reattach; reattaching does not start another attempt.

<!-- navigation -->
[Contents](README.md) · [Previous: Terminals](terminals.md) · [Next: Language support](language-support.md)
