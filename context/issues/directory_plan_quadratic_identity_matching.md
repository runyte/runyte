Building a filesystem plan from an editable directory projection repeatedly
scans the entire snapshot and desired row list. `FsPlan::build_with_limits`
looks up every desired identity with the linear `DirectorySnapshot::entry`,
then filters the complete desired list once per original snapshot entry.
Even an unchanged directory therefore requires quadratic identity matching
before any filesystem operation is planned. The ordinary editable explorer
does not impose a small entry limit, so large directories can stall the
editor while saving or previewing their changes.

Index the snapshot identities and group desired rows by identity once while
retaining the current order-dependent rules: an unchanged original pathname
is preferred as the primary row, otherwise the first occurrence is primary;
remaining occurrences are copies. Unknown identities must still fail, omitted
identities must still produce deletion, and duplicate target rejection and
source validation must remain intact.

A reproduction builds a no-change plan for snapshots with increasing numbers
of ordinary files and measures only planning after the snapshot is read.
Regression coverage should also exercise reordered rows, duplicate identities,
renames, deletions and stale identities. Keep timing assertions out of tests.
