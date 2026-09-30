// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeMap, ffi::OsString, io::Read};

use super::GitCliProvider;
use crate::git::{
    GitError, Repository, Result,
    history::{parse_log, valid_object_id},
    network::{
        GraphLanes, MAX_NETWORK_BYTES, MAX_NETWORK_COMMITS, MAX_NETWORK_ROOTS, NETWORK_PAGE_SIZE,
        NetworkCursor, NetworkPage, NetworkRequest, NetworkRoot, NetworkScope,
    },
};

fn malformed(detail: &str) -> GitError {
    GitError::Malformed {
        command: "git network".into(),
        detail: detail.into(),
    }
}

impl GitCliProvider {
    pub(super) fn read_network_roots(
        &self,
        repository: &Repository,
        scope: &NetworkScope,
    ) -> Result<(Vec<NetworkRoot>, bool)> {
        let mut roots = BTreeMap::<String, (Vec<String>, Vec<String>)>::new();
        let mut limited = false;
        if !matches!(scope, NetworkScope::Head) {
            if let NetworkScope::Ref(reference) = scope
                && (!reference.starts_with("refs/") || reference.contains(['\0', '\n']))
            {
                return Err(malformed("network scope requires a full ref identity"));
            }
            let mut args = vec!["for-each-ref".to_owned(), format!("--count={}", MAX_NETWORK_ROOTS + 1), "--sort=refname".into(), "--format=%(objectname)%00%(*objectname)%00%(objecttype)%00%(*objecttype)%00%(refname)".into()];
            match scope {
                NetworkScope::All => args.extend([
                    "refs/heads/".into(),
                    "refs/remotes/".into(),
                    "refs/tags/".into(),
                ]),
                NetworkScope::Ref(reference) => args.push(reference.clone()),
                NetworkScope::Head => unreachable!(),
            }
            let output = self.run_read_bounded(repository.workdir(), &args, MAX_NETWORK_BYTES)?;
            let lines = output
                .split(|&byte| byte == b'\n')
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>();
            limited = lines.len() > MAX_NETWORK_ROOTS;
            for line in lines.into_iter().take(MAX_NETWORK_ROOTS) {
                let fields = line.split(|&byte| byte == 0).collect::<Vec<_>>();
                if fields.len() != 5 {
                    return Err(malformed("root record does not contain five fields"));
                }
                let reference = std::str::from_utf8(fields[4])
                    .map_err(|_| malformed("non-UTF-8 ref identity"))?;
                if let NetworkScope::Ref(selected) = scope
                    && reference != selected
                {
                    continue;
                }
                let oid = if fields[2] == b"commit" {
                    fields[0]
                } else if fields[3] == b"commit" {
                    fields[1]
                } else {
                    continue;
                };
                let oid =
                    std::str::from_utf8(oid).map_err(|_| malformed("invalid root object ID"))?;
                if !valid_object_id(oid) {
                    return Err(malformed("root is not a full object ID"));
                }
                let label = reference
                    .strip_prefix("refs/heads/")
                    .or_else(|| reference.strip_prefix("refs/remotes/"))
                    .or_else(|| reference.strip_prefix("refs/tags/"))
                    .unwrap_or(reference)
                    .to_owned();
                let root = roots.entry(oid.into()).or_default();
                root.0.push(label);
                root.1.push(reference.to_owned());
            }
        }
        let head_oid = match self.run_read_bounded(
            repository.workdir(),
            &["rev-parse", "--verify", "--quiet", "HEAD"],
            128,
        ) {
            Ok(bytes) => Some(
                String::from_utf8(bytes)
                    .map_err(|_| malformed("invalid HEAD identity"))?
                    .trim_end_matches('\n')
                    .to_owned(),
            ),
            Err(GitError::Failed { code: Some(1), .. }) => None,
            Err(error) => return Err(error),
        };
        if let Some(oid) = head_oid
            && (!matches!(scope, NetworkScope::Ref(_)) || roots.contains_key(&oid))
        {
            let labels = &mut roots.entry(oid).or_default().0;
            let head = self.run_read_bounded(
                repository.workdir(),
                &["symbolic-ref", "--quiet", "--short", "HEAD"],
                4096,
            );
            let label = match head {
                Ok(bytes) => format!(
                    "HEAD → {}",
                    String::from_utf8(bytes)
                        .map_err(|_| malformed("invalid symbolic HEAD"))?
                        .trim_end_matches('\n')
                ),
                Err(GitError::Failed { code: Some(1), .. }) => "HEAD (detached)".into(),
                Err(error) => return Err(error),
            };
            labels.insert(0, label);
        }
        Ok((
            roots
                .into_iter()
                .map(|(oid, (labels, references))| NetworkRoot {
                    oid,
                    labels,
                    references,
                })
                .collect(),
            limited,
        ))
    }

    pub(super) fn read_network_page(
        &self,
        repository: &Repository,
        request: &NetworkRequest,
    ) -> Result<NetworkPage> {
        let (current, roots_limited) = self.read_network_roots(repository, &request.scope)?;
        if request.cursor.is_none()
            && matches!(request.scope, NetworkScope::Ref(_))
            && current.is_empty()
        {
            return Err(malformed("selected ref no longer names a commit"));
        }
        let shallow_path = self.run_read_bounded(
            repository.workdir(),
            &["rev-parse", "--git-path", "shallow"],
            4096,
        )?;
        let shallow_path = shallow_path.strip_suffix(b"\n").unwrap_or(&shallow_path);
        let shallow_path = repository.workdir().join(
            crate::git::status::path_from_bytes(shallow_path)
                .map_err(|_| malformed("invalid shallow metadata path"))?,
        );
        let mut shallow = String::new();
        match std::fs::File::open(&shallow_path) {
            Ok(file) => {
                file.take(MAX_NETWORK_BYTES as u64 + 1)
                    .read_to_string(&mut shallow)
                    .map_err(|_| malformed("cannot read shallow boundaries"))?;
                if shallow.len() > MAX_NETWORK_BYTES {
                    return Err(malformed("shallow boundary inventory exceeds limit"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(malformed("cannot open shallow boundary inventory")),
        }
        let shallow_fingerprint = crate::hash::sha256_hex(shallow.as_bytes());
        if let Some(cursor) = request.cursor.as_ref()
            && cursor.shallow_fingerprint != shallow_fingerprint
        {
            return Err(malformed(
                "shallow boundaries changed; refresh the commit network",
            ));
        }
        let cursor = request.cursor.clone().unwrap_or_else(|| NetworkCursor {
            roots: current.clone(),
            offset: 0,
            lanes: GraphLanes::default(),
            roots_limited,
            shallow_fingerprint: shallow_fingerprint.clone(),
        });
        if cursor.offset >= MAX_NETWORK_COMMITS
            || !cursor.offset.is_multiple_of(NETWORK_PAGE_SIZE)
            || cursor.roots.len() > MAX_NETWORK_ROOTS + 1
            || cursor.lanes.pending.len() > crate::git::network::MAX_NETWORK_LANES
            || cursor.roots.iter().any(|root| !valid_object_id(&root.oid))
            || cursor
                .lanes
                .pending
                .iter()
                .flatten()
                .any(|oid| !valid_object_id(oid))
        {
            return Err(malformed(
                "network cursor exceeds its bounds or contains invalid identities",
            ));
        }
        if cursor.roots.is_empty() {
            return Ok(NetworkPage {
                rows: vec![],
                next: None,
                roots: vec![],
                stale: current != cursor.roots,
                limited: false,
                roots_limited: false,
            });
        }
        let mut args = vec![
            OsString::from("--no-replace-objects"),
            OsString::from("log"),
            "-z".into(),
            "--topo-order".into(),
            "--abbrev=7".into(),
            format!("--skip={}", cursor.offset).into(),
            format!("--max-count={}", NETWORK_PAGE_SIZE + 1).into(),
            "--date=format:%Y-%m-%d %H:%M".into(),
            "--format=%H%x00%h%x00%P%x00%an%x00%at%x00%as%x00%ad%x00%s%x00".into(),
        ];
        args.extend(cursor.roots.iter().map(|root| OsString::from(&root.oid)));
        args.push("--".into());
        let output = self.run_read_bounded(repository.workdir(), &args, MAX_NETWORK_BYTES)?;
        let fields = output.split(|&byte| byte == 0).collect::<Vec<_>>();
        let mut commits = parse_log(&output)?;
        for (index, commit) in commits.iter_mut().enumerate() {
            let truncated = String::from_utf8_lossy(fields[index * 9 + 7])
                .chars()
                .count()
                > 4096;
            commit.subject = commit
                .subject
                .chars()
                .flat_map(|character| {
                    if character.is_control() {
                        character.escape_default().collect::<Vec<_>>()
                    } else {
                        vec![character]
                    }
                })
                .collect();
            if truncated {
                commit.subject.push_str(" [subject truncated]");
            }
        }
        let has_more = commits.len() > NETWORK_PAGE_SIZE;
        commits.truncate(NETWORK_PAGE_SIZE);
        let mut lanes = cursor.lanes;
        let rows = commits
            .into_iter()
            .map(|mut commit| {
                commit.decorations = cursor
                    .roots
                    .iter()
                    .find(|root| root.oid == commit.oid)
                    .map_or_else(Vec::new, |root| root.labels.clone());
                let boundary = shallow.lines().any(|oid| oid == commit.oid);
                lanes.row(commit, boundary)
            })
            .collect::<Vec<_>>();
        let offset = cursor.offset + rows.len();
        let limited = cursor.roots_limited || (has_more && offset >= MAX_NETWORK_COMMITS);
        let next = (has_more && offset < MAX_NETWORK_COMMITS).then(|| NetworkCursor {
            roots: cursor.roots.clone(),
            offset,
            lanes,
            roots_limited: cursor.roots_limited,
            shallow_fingerprint,
        });
        Ok(NetworkPage {
            rows,
            next,
            stale: current != cursor.roots || roots_limited != cursor.roots_limited,
            roots: cursor.roots,
            limited,
            roots_limited: cursor.roots_limited,
        })
    }
}
