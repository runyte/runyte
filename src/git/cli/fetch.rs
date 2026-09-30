// SPDX-License-Identifier: MPL-2.0

//! Exact, configuration-resolved single-branch fetches.

use super::super::FetchBranchTarget;
use super::{GitCliProvider, GitError, Repository, Result};

#[derive(Debug, Eq, PartialEq)]
struct FetchDestination {
    remote: String,
    source: String,
    destination: String,
}

impl GitCliProvider {
    pub(super) fn fetch_selected_branch(
        &self,
        repository: &Repository,
        target: &FetchBranchTarget,
    ) -> Result<String> {
        let destination = self.resolve_fetch_destination(repository, target)?;
        for reference in [&destination.source, &destination.destination] {
            self.run_text(repository.workdir(), &["check-ref-format", reference])?;
        }
        match self.run_text(
            repository.workdir(),
            &["symbolic-ref", "-q", &destination.destination],
        ) {
            Ok(_) => {
                return Err(fetch_error(
                    "the fetch destination is symbolic; it must be an ordinary remote-tracking ref",
                ));
            }
            Err(GitError::Failed { code: Some(1), .. }) => {}
            Err(error) => return Err(error),
        }
        let refspec = format!("+{}:{}", destination.source, destination.destination);
        let report = self.run_network(
            repository.workdir(),
            &[
                "fetch",
                "--refmap=",
                "--no-tags",
                "--no-prune",
                "--no-prune-tags",
                "--no-recurse-submodules",
                "--no-write-fetch-head",
                "--no-auto-maintenance",
                "--",
                &destination.remote,
                &refspec,
            ],
        )?;
        let name = destination
            .destination
            .strip_prefix("refs/remotes/")
            .unwrap();
        Ok(if report.trim().is_empty() {
            format!("Fetched {name}")
        } else {
            format!("Fetched {name}\n{}", report.trim())
        })
    }

    fn fetch_config(&self, repository: &Repository, key: &str) -> Result<Vec<String>> {
        match self.run_text(
            repository.workdir(),
            &["config", "--null", "--get-all", key],
        ) {
            Ok(value) => Ok(value.split_terminator('\0').map(str::to_owned).collect()),
            Err(GitError::Failed { code: Some(1), .. }) => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    fn resolve_fetch_destination(
        &self,
        repository: &Repository,
        target: &FetchBranchTarget,
    ) -> Result<FetchDestination> {
        let remotes = self.run_text(repository.workdir(), &["remote"])?;
        let (wanted_remote, wanted_source, wanted_destination) = match target {
            FetchBranchTarget::LocalBranch(branch) => {
                self.run_text(
                    repository.workdir(),
                    &["check-ref-format", &format!("refs/heads/{branch}")],
                )?;
                // Do not mistake a gone upstream for missing configuration.
                let remote = self.fetch_config(repository, &format!("branch.{branch}.remote"))?;
                let source = self.fetch_config(repository, &format!("branch.{branch}.merge"))?;
                if remote.len() != 1 || source.len() != 1 {
                    return Err(fetch_error(
                        "configure exactly one remote upstream for this local branch first",
                    ));
                }
                (Some(remote[0].clone()), Some(source[0].clone()), None)
            }
            FetchBranchTarget::RemoteTrackingRef(reference) => {
                if !reference.starts_with("refs/remotes/") {
                    return Err(fetch_error(
                        "fetch destination must be a remote-tracking ref",
                    ));
                }
                (None, None, Some(reference.clone()))
            }
        };
        if let Some(remote) = &wanted_remote {
            validate_remote(remote)?;
            if !remotes.lines().any(|name| name == remote) {
                return Err(fetch_error(
                    "the upstream's configured remote no longer exists",
                ));
            }
        }
        let mut matches = Vec::new();
        for remote in remotes.lines().filter(|name| {
            wanted_remote
                .as_deref()
                .is_none_or(|wanted| wanted == *name)
        }) {
            let mappings = self.fetch_config(repository, &format!("remote.{remote}.fetch"))?;
            for mapping in &mappings {
                let Some((source, destination)) = positive_mapping(mapping)? else {
                    continue;
                };
                let pair = match (&wanted_source, &wanted_destination) {
                    (Some(wanted), None) => map_reference(source, destination, wanted)?
                        .map(|destination| (wanted.clone(), destination)),
                    (None, Some(wanted)) => map_reference(destination, source, wanted)?
                        .map(|source| (source, wanted.clone())),
                    _ => unreachable!(),
                };
                let Some((source, destination)) = pair else {
                    continue;
                };
                validate_remote(remote)?;
                if !source.starts_with("refs/heads/") || !destination.starts_with("refs/remotes/") {
                    return Err(fetch_error(
                        "single-branch fetching supports only server branches mapped into refs/remotes/",
                    ));
                }
                for excluded in mappings
                    .iter()
                    .filter_map(|mapping| mapping.strip_prefix('^'))
                {
                    if pattern_capture(excluded, &source)?.is_some() {
                        return Err(fetch_error(
                            "the configured negative fetch mapping excludes this branch",
                        ));
                    }
                }
                let candidate = FetchDestination {
                    remote: remote.to_owned(),
                    source,
                    destination,
                };
                if !matches.contains(&candidate) {
                    matches.push(candidate);
                }
            }
        }
        if matches.len() != 1 {
            return Err(fetch_error(if matches.is_empty() {
                "no configured fetch mapping identifies this branch; configure its remote upstream first"
            } else {
                "fetch mappings for this branch are ambiguous; configure one exact remote-tracking destination"
            }));
        }
        Ok(matches.remove(0))
    }
}

fn fetch_error(detail: &str) -> GitError {
    GitError::Malformed {
        command: "git fetch".to_owned(),
        detail: detail.to_owned(),
    }
}

fn validate_remote(remote: &str) -> Result<()> {
    if remote == "." {
        return Err(fetch_error(
            "a local '.' upstream is not a network fetch target",
        ));
    }
    if remote.is_empty()
        || remote.starts_with('-')
        || remote.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(fetch_error(
            "the configured remote name is not a safe fetch target",
        ));
    }
    Ok(())
}

fn positive_mapping(mapping: &str) -> Result<Option<(&str, &str)>> {
    if let Some(negative) = mapping.strip_prefix('^') {
        if negative.contains(':')
            || negative.matches('*').count() > 1
            || !negative.starts_with("refs/")
        {
            return Err(fetch_error("unsupported negative fetch mapping"));
        }
        return Ok(None);
    }
    let mapping = mapping.strip_prefix('+').unwrap_or(mapping);
    let Some((source, destination)) = mapping.split_once(':') else {
        // Source-only fetch rules cannot identify a remote-tracking ref.
        return Ok(None);
    };
    if source.matches('*').count() > 1
        || destination.matches('*').count() > 1
        || source.contains('*') != destination.contains('*')
        || destination.contains(':')
    {
        return Err(fetch_error("unsupported configured fetch mapping"));
    }
    Ok(Some((source, destination)))
}

fn pattern_capture<'a>(pattern: &str, reference: &'a str) -> Result<Option<&'a str>> {
    if pattern.matches('*').count() > 1 {
        return Err(fetch_error("unsupported fetch pattern"));
    }
    if let Some((prefix, suffix)) = pattern.split_once('*') {
        Ok(reference
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(suffix)))
    } else {
        Ok((pattern == reference).then_some(""))
    }
}

fn map_reference(from: &str, to: &str, reference: &str) -> Result<Option<String>> {
    Ok(pattern_capture(from, reference)?.map(|capture| to.replace('*', capture)))
}
