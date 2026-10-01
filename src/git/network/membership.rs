// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeMap;

use super::{ContainingBranch, GraphRow};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LocalBranch {
    pub name: String,
    pub oid: String,
    pub current: bool,
    pub committer_time: i64,
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct LocalBranches {
    /// Current branch first, then newest tip, then name to break ties.
    pub tips: Vec<LocalBranch>,
    pub limited: bool,
}

impl LocalBranches {
    pub(crate) fn fingerprint(&self) -> String {
        let mut inventory = format!("{}\0", self.limited);
        for tip in &self.tips {
            inventory.push_str(&format!("{}\0{}\0{}\0", tip.name, tip.oid, tip.current));
        }
        crate::hash::sha256_hex(inventory.as_bytes())
    }
}

/// Captured local containment, independent of displayed paths and graph scope.
/// Private fields prevent unbounded construction; cursors share this through Arc.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct NetworkMembership {
    pub(crate) branches: LocalBranches,
    /// The two best distinct branches suffice for both preference and plurality.
    commits: BTreeMap<String, [usize; 2]>,
    pub(crate) limited: bool,
}

impl NetworkMembership {
    pub(crate) fn capture(
        branches: LocalBranches,
        ancestry: &[(&str, Vec<&str>)],
        limited: bool,
    ) -> Self {
        let mut pending = BTreeMap::<&str, [usize; 2]>::new();
        for (index, branch) in branches.tips.iter().enumerate() {
            merge(
                pending.entry(&branch.oid).or_insert([usize::MAX; 2]),
                [index, usize::MAX],
            );
        }
        let mut commits = BTreeMap::new();
        // Topological order guarantees every child contributes before its parent.
        // Even a bounded prefix therefore has complete evidence for emitted rows.
        for (oid, parents) in ancestry {
            let Some(membership) = pending.remove(oid) else {
                continue;
            };
            commits.insert((*oid).to_owned(), membership);
            for parent in parents {
                merge(pending.entry(parent).or_insert([usize::MAX; 2]), membership);
            }
        }
        Self {
            branches,
            commits,
            limited,
        }
    }

    pub(crate) fn label_row(&self, row: &mut GraphRow) {
        if row.branch.is_none()
            && let Some([first, second]) = self.commits.get(&row.commit.oid)
        {
            row.containing_branch = Some(ContainingBranch {
                name: self.branches.tips[*first].name.clone(),
                multiple: *second != usize::MAX,
            });
        }
    }
}

fn merge(into: &mut [usize; 2], from: [usize; 2]) {
    for candidate in from {
        if candidate < into[0] {
            into[1] = into[0];
            into[0] = candidate;
        } else if candidate != into[0] && candidate < into[1] {
            into[1] = candidate;
        }
    }
}
