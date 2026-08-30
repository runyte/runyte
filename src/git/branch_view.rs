// SPDX-License-Identifier: MPL-2.0

//! The local and cached remote branch list, as a projection of Git refs.
//!
//! The same shape as the changed-file list next door: deterministic, holding no
//! state, and pairing each row's text with the thing a key pressed on that row
//! acts on. What it adds is a third piece — the columns the annotations occupy
//! — because upstream drift and checkout paths have to read as notes about the
//! branch rather than as part of its name, and a frontend cannot find those
//! columns again without re-deriving the whole row.

use std::path::Path;

use super::{Branch, BranchList, Divergence, RemoteBranch};

/// One row of the list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchRow {
    pub text: String,
    /// The branch a key pressed on this row acts on. `None` on the placeholder
    /// row an empty repository shows.
    pub branch: Option<Branch>,
    /// The remote-tracking ref this row acts on. Local rows and headings have
    /// none. Kept separate from `branch` because delete, pull, and push are
    /// deliberately local-branch operations.
    pub remote: Option<RemoteBranch>,
    /// Character columns `[start, end)` of the annotations, for a
    /// frontend to set apart from the name. `None` when the row carries none.
    pub annotation: Option<(usize, usize)>,
}

/// Projects local branches into their section of the branch list.
///
/// Names are padded to a common width so the annotations line up in a column
/// of their own; a reader comparing checkout locations or how far two branches
/// have drifted is not hunting for notes at the end of ragged names.
pub fn branch_rows(branches: &BranchList) -> Vec<BranchRow> {
    let mut rows = vec![heading("Local")];
    rows.extend(local_rows(&branches.local));
    rows.push(heading(""));
    rows.push(heading("Remote"));
    rows.extend(remote_rows(&branches.remote));
    rows
}

fn heading(text: impl Into<String>) -> BranchRow {
    BranchRow {
        text: text.into(),
        branch: None,
        remote: None,
        annotation: None,
    }
}

fn local_rows(branches: &[Branch]) -> Vec<BranchRow> {
    if branches.is_empty() {
        return vec![heading("  no local branches")];
    }
    let annotated = branches
        .iter()
        .map(|branch| (branch, annotation(branch)))
        .collect::<Vec<_>>();
    let width = annotated
        .iter()
        .filter(|(_, annotation)| annotation.is_some())
        .map(|(branch, _)| branch.name.chars().count())
        .max()
        .unwrap_or_default();
    annotated
        .into_iter()
        .map(|(branch, annotation)| {
            let marker = if branch.current { '*' } else { ' ' };
            let mut text = format!("{marker} {}", branch.name);
            let columns = annotation.map(|annotation| {
                // Characters rather than display cells: the columns below are
                // what a frontend highlights, and it addresses the row by
                // character offset. A name of wide characters therefore lines
                // up by count rather than by width, which is the same trade the
                // rest of the editor's column arithmetic makes.
                let name = branch.name.chars().count();
                text.push_str(&" ".repeat(width.saturating_sub(name) + 1));
                let start = text.chars().count();
                text.push_str(&annotation);
                (start, start + annotation.chars().count())
            });
            BranchRow {
                text,
                branch: Some(branch.clone()),
                remote: None,
                annotation: columns,
            }
        })
        .collect()
}

fn remote_rows(branches: &[RemoteBranch]) -> Vec<BranchRow> {
    if branches.is_empty() {
        return vec![heading("  no remote branches known")];
    }
    let annotations = branches
        .iter()
        .map(|branch| {
            if branch.tracked_by.is_empty() {
                "[not tracked locally]".to_owned()
            } else {
                format!("[tracked by: {}]", branch.tracked_by.join(", "))
            }
        })
        .collect::<Vec<_>>();
    let width = branches
        .iter()
        .map(|branch| branch.name.chars().count())
        .max()
        .unwrap_or_default();
    branches
        .iter()
        .zip(annotations)
        .map(|(branch, annotation)| {
            let mut text = format!("  {}", branch.name);
            text.push_str(&" ".repeat(width.saturating_sub(branch.name.chars().count()) + 1));
            let start = text.chars().count();
            text.push_str(&annotation);
            BranchRow {
                text,
                branch: None,
                remote: Some(branch.clone()),
                annotation: Some((start, start + annotation.chars().count())),
            }
        })
        .collect()
}

/// What one branch's row says about its upstream and checkouts, if anything.
///
/// A branch with no upstream configured says nothing: there is no second place
/// for it to be compared against, and an empty pair of brackets would suggest
/// otherwise. The three states that do have an upstream are all worth telling
/// apart — drifted, in step, and pointing at a ref that no longer exists.
fn annotation(branch: &Branch) -> Option<String> {
    let mut notes = Vec::new();
    if let Some(upstream) = branch.upstream.as_ref() {
        notes.push(match upstream.divergence {
            None => "[gone]".to_owned(),
            Some(Divergence {
                ahead: 0,
                behind: 0,
            }) => "[=]".to_owned(),
            Some(Divergence { ahead, behind: 0 }) => format!("[↑{ahead}]"),
            Some(Divergence { ahead: 0, behind }) => format!("[↓{behind}]"),
            Some(Divergence { ahead, behind }) => format!("[↑{ahead} ↓{behind}]"),
        });
    }
    notes.extend(
        branch
            .checkouts
            .iter()
            .map(|path| format!("[worktree: {}]", display_path(path))),
    );
    (!notes.is_empty()).then(|| notes.join(" "))
}

/// Makes an operating-system path safe for a buffer whose physical lines are
/// also its actionable rows. Lossy conversion keeps a non-UTF-8 checkout
/// visible, while escaping controls prevents a path from manufacturing rows.
pub(crate) fn display_path(path: &Path) -> String {
    let mut display = String::new();
    for character in path.to_string_lossy().chars() {
        match character {
            '\n' => display.push_str("\\n"),
            '\r' => display.push_str("\\r"),
            '\t' => display.push_str("\\t"),
            character if character.is_control() => display.extend(character.escape_unicode()),
            character => display.push(character),
        }
    }
    display
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Upstream;

    fn tracked(name: &str, current: bool, ahead: usize, behind: usize) -> Branch {
        Branch {
            name: name.to_owned(),
            current,
            checkouts: Vec::new(),
            upstream: Some(Upstream::origin(name, Some(Divergence { ahead, behind }))),
            merged: false,
        }
    }

    fn text(rows: &[BranchRow]) -> Vec<&str> {
        rows.iter().map(|row| row.text.as_str()).collect()
    }

    #[test]
    fn each_direction_of_drift_reads_as_its_own_arrow() {
        let rows = local_rows(&[
            tracked("ahead", false, 2, 0),
            tracked("behind", false, 0, 3),
            tracked("both", false, 2, 3),
            tracked("level", false, 0, 0),
        ]);

        assert_eq!(
            text(&rows),
            vec![
                "  ahead  [↑2]",
                "  behind [↓3]",
                "  both   [↑2 ↓3]",
                "  level  [=]",
            ]
        );
    }

    /// An upstream that is configured and missing is not the same as none at
    /// all, and neither is the same as being in step with one.
    #[test]
    fn a_branch_without_an_upstream_says_nothing_about_one() {
        let mut gone = tracked("stale", false, 0, 0);
        gone.upstream.as_mut().unwrap().divergence = None;
        let rows = local_rows(&[Branch::new("local", true), gone]);

        assert_eq!(text(&rows), vec!["* local", "  stale [gone]"]);
        assert_eq!(rows[0].annotation, None);
    }

    /// The annotation columns are what a frontend colours, so they have to name
    /// the annotation and nothing else — not the padding before it, and not the
    /// name it follows.
    #[test]
    fn the_annotation_columns_cover_the_annotation_alone() {
        let rows = local_rows(&[tracked("feature", false, 1, 0)]);

        let (start, end) = rows[0].annotation.unwrap();
        let columns = rows[0].text.chars().collect::<Vec<_>>();
        assert_eq!(columns[start..end].iter().collect::<String>(), "[↑1]");
    }

    /// Every row carries the branch it acts on, so `D` on a row can never find
    /// a neighbour's branch instead.
    #[test]
    fn rows_carry_the_branch_they_act_on() {
        let rows = local_rows(&[Branch::new("main", true), Branch::new("other", false)]);

        assert_eq!(
            rows.iter()
                .map(|row| row.branch.as_ref().map(|branch| branch.name.as_str()))
                .collect::<Vec<_>>(),
            vec![Some("main"), Some("other")]
        );
        // An empty repository offers a row to read and nothing to act on.
        let empty = local_rows(&[]);
        assert_eq!(text(&empty), vec!["  no local branches"]);
        assert!(empty[0].branch.is_none());
    }

    #[test]
    fn checked_out_branches_show_every_path_without_changing_row_identity() {
        let mut branch = Branch::new("topic", false);
        branch.checkouts = vec!["/repo/topic".into(), "/tmp/topic copy".into()];

        let rows = local_rows(&[branch]);

        assert_eq!(
            text(&rows),
            vec!["  topic [worktree: /repo/topic] [worktree: /tmp/topic copy]"]
        );
        assert_eq!(rows[0].branch.as_ref().unwrap().name, "topic");
        let (start, end) = rows[0].annotation.unwrap();
        assert_eq!(
            rows[0].text.chars().collect::<Vec<_>>()[start..end]
                .iter()
                .collect::<String>(),
            "[worktree: /repo/topic] [worktree: /tmp/topic copy]"
        );
    }

    #[test]
    fn checkout_paths_cannot_manufacture_actionable_rows() {
        let mut branch = Branch::new("topic", false);
        branch.checkouts = vec!["/tmp/line\nbreak\t❯\\folder".into()];

        let rows = local_rows(&[branch]);

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].text,
            "  topic [worktree: /tmp/line\\nbreak\\t❯\\folder]"
        );
    }

    #[test]
    fn local_and_remote_sections_name_every_tracking_relationship() {
        let mut origin_main = RemoteBranch::new("origin", "main");
        origin_main.tracked_by = vec!["main".to_owned(), "release".to_owned()];
        let branches = BranchList {
            local: vec![Branch::new("main", true), Branch::new("release", false)],
            remote: vec![origin_main, RemoteBranch::new("origin", "review/42")],
        };

        let rows = branch_rows(&branches);

        assert_eq!(
            text(&rows),
            vec![
                "Local",
                "* main",
                "  release",
                "",
                "Remote",
                "  origin/main      [tracked by: main, release]",
                "  origin/review/42 [not tracked locally]",
            ]
        );
        assert!(rows[0].branch.is_none() && rows[0].remote.is_none());
        assert_eq!(rows[1].branch.as_ref().unwrap().name, "main");
        assert_eq!(rows[5].remote.as_ref().unwrap().name, "origin/main");
        assert_eq!(rows[6].annotation.map(|(start, _)| start), Some(19));
    }
}
