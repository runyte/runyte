---
title: "Git network contextual help omits row-label meaning"
status: resolved
reported: 2026-10-02
resolved: 2026-10-02
commit: 9e41bcd
---

## Resolution

Commit `9e41bcd` (Explain Git network labels in contextual help) fixes the missing explanation in `HelpTopic::overview_for`. Its Git network overview described navigation and graph geometry but did not define the branch and containment labels beside commits. The added paragraph explains that `[dev]` names a displayed branch path, while `[in exp]` and `[in exp, ...]` express reachability from one or multiple local branches. It also states that these labels do not identify the branch on which a commit was created. `contextual_help_explains_branch_and_containment_labels` in `src/app/tests/git_network.rs` checks the text reached through `Space ?` from the network view.

Known limitation: The `exp` graph at this fix commit still renders direct ref decorations. Branch-path labels came from `d8eb11a` (Restore rectangular Git network with colored branch labels) on `dev`; containment labels came from `cfc71e1` (Label unnamed Git network paths by local branch containment) on `dev`. Those graph changes are not backported by this documentation fix, so `exp` does not yet generate `[in exp, ...]` rows.

## Report

The Git network opened with `Space g n` can display rows such as:

```text
9fbdb8  KA    | *            [exp] Make x and X only grow their own outer edge
6f25f0  KA    * |            [dev] Record resolution of indentation backspace issue
a95da1  KA    *-+            [dev] Align indentation backspace and add language and file overrides
7d311e  KA    +-*            [exp] Merge branch 'exp' into dev
9a106c  KA    * |            [in exp, ...] Record completed Git interface plan and validation
9bd991  KA    *-|-+          [in exp, ...] Merge reviewed Git keymap integration contracts
997ddc  KA    +-|-*          [in exp, ...] Align keymap integration contracts with merge review scopes
```

Opening `Space ?` from this page did not explain what `[dev]` or `[in exp, ...]` meant. The contextual help should define these annotations so a reader can distinguish a displayed branch path from local-branch containment.
