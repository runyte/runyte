Branch checkout from the branch list is allowed while the current workspace
owns a live terminal session. A terminal job is not terminated by the checkout,
but it keeps running in the same working directory while Git replaces files
under it. A watcher, build, test run, or other long-running job can therefore
observe content from two branches or write into the checkout while it is being
changed.

To reproduce:

1. Open a repository in Runyte.
2. Start a terminal with `Space t n` and run a long-lived job in the repository.
3. Leave the terminal visible in another pane or hide it without ending the
   terminal session.
4. Open the local branch list with `Space g b`, select another branch, and press
   `Enter`.

When the repository and Runyte's file buffers are otherwise clean, the branch
checkout proceeds. The presence of the live terminal session is not currently
a checkout precondition.

Switching branches while the workspace owns any live terminal session should
require an additional safety boundary. The checkout may be refused until the
terminal job exits, or Runyte may require typed confirmation that includes the
exact target branch name. A simple Enter confirmation is not sufficient because
the branch-list action itself already uses Enter. The chosen behavior should
cover live terminal sessions whether they are visible, hidden, or shown in
another pane, and should have behavior-boundary tests for both refusal or
confirmation and the checkout after the terminal exits.

This issue is limited to changing the branch of the current Git worktree through
the branch list. Opening another worktree with `Space g w` and `Enter` is not the
same operation: in persistent mode it switches the TUI attachment while the old
workspace host retains its buffers and terminal sessions. Worktree attachment
switching should remain unchanged.

Implementation is intentionally deferred while adjacent branch-confirmation
work is in progress on `enh/confirm-branch-delete`; the safety interaction can
be designed and implemented alongside that work.
