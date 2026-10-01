// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::git::network::{LocalBranch, LocalBranches, NetworkMembership};

impl GitCliProvider {
    pub(super) fn read_network_local_branches(
        &self,
        repository: &Repository,
    ) -> Result<LocalBranches> {
        let output = self.run_read_bounded(
            repository.workdir(),
            &[
                "for-each-ref".to_owned(),
                format!("--count={}", MAX_NETWORK_ROOTS + 1),
                "--sort=refname".into(),
                "--format=%(objectname)%00%(refname)%00%(HEAD)%00%(committerdate:unix)".into(),
                "refs/heads/".into(),
            ],
            MAX_NETWORK_BYTES,
        )?;
        let text = std::str::from_utf8(&output)
            .map_err(|_| malformed("non-UTF-8 local branch inventory"))?;
        let mut tips = Vec::new();
        for line in text.lines() {
            let fields = line.split('\0').collect::<Vec<_>>();
            if fields.len() != 4 || !valid_object_id(fields[0]) {
                return Err(malformed("invalid local branch record"));
            }
            let name = fields[1]
                .strip_prefix("refs/heads/")
                .filter(|name| !name.is_empty() && !name.chars().any(char::is_control))
                .ok_or_else(|| malformed("invalid local branch name"))?;
            tips.push(LocalBranch {
                name: name.into(),
                oid: fields[0].into(),
                current: fields[2] == "*",
                committer_time: fields[3]
                    .parse()
                    .map_err(|_| malformed("invalid branch tip timestamp"))?,
            });
        }
        let limited = tips.len() > MAX_NETWORK_ROOTS;
        tips.truncate(MAX_NETWORK_ROOTS);
        tips.sort_by(|a, b| {
            b.current
                .cmp(&a.current)
                .then_with(|| b.committer_time.cmp(&a.committer_time))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(LocalBranches { tips, limited })
    }

    pub(super) fn read_network_membership(
        &self,
        repository: &Repository,
        branches: LocalBranches,
    ) -> Result<NetworkMembership> {
        if branches.limited || branches.tips.is_empty() {
            // An incomplete inventory cannot establish priority or uniqueness.
            let limited = branches.limited;
            return Ok(NetworkMembership::capture(branches, &[], limited));
        }
        let mut args = vec![
            "--no-replace-objects".to_owned(),
            "rev-list".into(),
            "--topo-order".into(),
            "--parents".into(),
            format!("--max-count={}", MAX_NETWORK_COMMITS + 1),
        ];
        args.extend(branches.tips.iter().map(|branch| branch.oid.clone()));
        args.push("--".into());
        let output = match self.run_read_bounded(repository.workdir(), &args, MAX_NETWORK_BYTES) {
            Ok(output) => output,
            Err(GitError::TooLarge { .. }) => {
                return Ok(NetworkMembership::capture(branches, &[], true));
            }
            Err(error) => return Err(error),
        };
        let text =
            std::str::from_utf8(&output).map_err(|_| malformed("invalid containment ancestry"))?;
        let mut ancestry = Vec::new();
        for line in text.lines() {
            let mut ids = line.split_whitespace();
            let oid = ids
                .next()
                .ok_or_else(|| malformed("missing containment commit"))?;
            let parents = ids.collect::<Vec<_>>();
            if !valid_object_id(oid) || parents.iter().any(|oid| !valid_object_id(oid)) {
                return Err(malformed("invalid containment object ID"));
            }
            ancestry.push((oid, parents));
        }
        let limited = ancestry.len() > MAX_NETWORK_COMMITS;
        ancestry.truncate(MAX_NETWORK_COMMITS);
        Ok(NetworkMembership::capture(branches, &ancestry, limited))
    }
}
