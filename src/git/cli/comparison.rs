// SPDX-License-Identifier: MPL-2.0

use super::super::{ComparisonTarget, RevisionComparison, RevisionFile, RevisionFileView};
use super::*;
use std::{collections::BTreeMap, ffi::OsStr, path::Path};

impl GitCliProvider {
    fn comparison_read<S: AsRef<OsStr>>(
        &self,
        directory: &Path,
        arguments: &[S],
    ) -> Result<Vec<u8>> {
        self.run_read_bounded(directory, arguments, self.max_output_bytes)
    }

    fn comparison_tip(&self, directory: &Path, reference: &str) -> Result<String> {
        let output = self.comparison_read(
            directory,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{reference}^{{commit}}"),
            ],
        )?;
        let oid = String::from_utf8_lossy(&output).trim().to_owned();
        if !valid_object_id(&oid) {
            return Err(comparison_error("invalid commit identity"));
        }
        Ok(oid)
    }

    pub(super) fn read_revision_comparison(
        &self,
        repository: &Repository,
        target: &ComparisonTarget,
    ) -> Result<RevisionComparison> {
        let left_head = self.status(repository)?.head;
        let (left_reference, left_label) = match left_head {
            Head::Branch(name) => (format!("refs/heads/{name}"), name),
            Head::Detached(oid) => (oid.clone(), format!("detached {}", &oid[..8])),
            Head::Unborn(_) => {
                return Err(comparison_error(
                    "the current branch has no commit to compare",
                ));
            }
        };
        let left_oid = self.comparison_tip(repository.workdir(), &left_reference)?;
        let (right_oid, right_label) = match target {
            ComparisonTarget::Branch { reference, label } => (
                self.comparison_tip(repository.workdir(), reference)?,
                label.clone(),
            ),
            ComparisonTarget::Worktree(path) => {
                let worktree = self
                    .worktrees(repository)?
                    .into_iter()
                    .find(|entry| entry.path == *path)
                    .filter(|entry| !entry.bare && !entry.missing && entry.prunable.is_none())
                    .ok_or_else(|| comparison_error("the selected worktree is unavailable"))?;
                let oid = worktree
                    .head
                    .filter(|oid| valid_object_id(oid))
                    .ok_or_else(|| {
                        comparison_error("the selected worktree has no commit to compare")
                    })?;
                let label = worktree
                    .branch
                    .map(|name| name.strip_prefix("refs/heads/").unwrap_or(&name).to_owned())
                    .unwrap_or_else(|| format!("detached {}", &oid[..8]));
                (oid, label)
            }
        };
        let base = [
            "--literal-pathspecs",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--ignore-submodules=none",
            "--find-renames",
            "-l1000",
        ];
        let mut args = base.to_vec();
        args.extend(["--raw", "--no-abbrev", "-z", &left_oid, &right_oid, "--"]);
        let raw = self.comparison_read(repository.workdir(), &args)?;
        let mut args = base.to_vec();
        args.extend(["--numstat", "-z", &left_oid, &right_oid, "--"]);
        let stats: BTreeMap<_, _> =
            parse_numstat(&self.comparison_read(repository.workdir(), &args)?)
                .map_err(|message| comparison_error(&message))?
                .into_iter()
                .collect();
        let files = parse_files(&raw, &stats)?;
        Ok(RevisionComparison {
            target: target.clone(),
            left_label,
            right_label,
            left_oid,
            right_oid,
            files,
        })
    }

    pub(super) fn read_revision_file(
        &self,
        repository: &Repository,
        comparison: &RevisionComparison,
        file: &RevisionFile,
        split: bool,
    ) -> Result<RevisionFileView> {
        if !valid_object_id(&comparison.left_oid) || !valid_object_id(&comparison.right_oid) {
            return Err(comparison_error("invalid comparison commit identity"));
        }
        if split {
            let content = |path: &Option<std::path::PathBuf>, object: &str, mode: &str| {
                if path.is_none() {
                    return Ok(BaseContent::Absent);
                }
                if !valid_object_id(object) {
                    return Err(comparison_error("invalid file object identity"));
                }
                if mode == "160000" {
                    return Ok(BaseContent::Text(format!("Subproject commit {object}\n")));
                }
                self.object_content(repository, object)
            };
            return Ok(RevisionFileView::Split(FileComparison {
                previous: content(&file.left, &file.left_object, &file.left_mode)?,
                current: content(&file.right, &file.right_object, &file.right_mode)?,
            }));
        }
        let mut args: Vec<&OsStr> = [
            "-c",
            "core.quotePath=true",
            "--literal-pathspecs",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--ignore-submodules=none",
            "--find-renames",
            "-l1000",
            "--src-prefix=a/",
            "--submodule=short",
            "--dst-prefix=b/",
            "--no-relative",
            &comparison.left_oid,
            &comparison.right_oid,
            "--",
        ]
        .into_iter()
        .map(OsStr::new)
        .collect();
        for path in [&file.left, &file.right].into_iter().flatten() {
            args.push(path.as_os_str());
        }
        let patch = self.comparison_read(repository.workdir(), &args)?;
        Ok(RevisionFileView::Patch(selected_patch(&patch, file)?))
    }
}

fn comparison_error(detail: &str) -> GitError {
    GitError::Malformed {
        command: "git diff".into(),
        detail: detail.into(),
    }
}

fn parse_files(
    raw: &[u8],
    stats: &BTreeMap<std::path::PathBuf, LineStats>,
) -> Result<Vec<RevisionFile>> {
    let mut fields = raw
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut files = Vec::new();
    while let Some(header) = fields.next() {
        let header =
            std::str::from_utf8(header).map_err(|_| comparison_error("invalid diff header"))?;
        let parts: Vec<_> = header.trim_start_matches(':').split_whitespace().collect();
        if parts.len() != 5 {
            return Err(comparison_error("invalid raw diff fields"));
        }
        let path = |bytes| {
            super::super::status::path_from_bytes(bytes)
                .map_err(|message| comparison_error(&message))
        };
        let first = path(
            fields
                .next()
                .ok_or_else(|| comparison_error("missing file path"))?,
        )?;
        let status = parts[4].as_bytes()[0];
        let second = if matches!(status, b'R' | b'C') {
            path(
                fields
                    .next()
                    .ok_or_else(|| comparison_error("missing rename path"))?,
            )?
        } else {
            first.clone()
        };
        let count = stats.get(&second).copied();
        files.push(RevisionFile {
            left: (status != b'A').then_some(first),
            right: (status != b'D').then_some(second),
            left_mode: parts[0].into(),
            right_mode: parts[1].into(),
            left_object: parts[2].into(),
            right_object: parts[3].into(),
            stats: count,
        });
    }
    Ok(files)
}

/// Literal pathspecs can also match descendants after a file became a directory.
/// Keep only sections naming the captured pair; a type change can have two.
fn selected_patch(patch: &[u8], file: &RevisionFile) -> Result<String> {
    let left = file
        .left
        .as_deref()
        .or(file.right.as_deref())
        .ok_or_else(|| comparison_error("file has no path"))?;
    let right = file.right.as_deref().unwrap_or(left);
    let header = format!(
        "diff --git {} {}\n",
        patch_path("a/", left),
        patch_path("b/", right)
    );
    let mut selected = Vec::new();
    let mut include = false;
    for line in patch.split_inclusive(|byte| *byte == b'\n') {
        if line.starts_with(b"diff --git ") {
            include = line == header.as_bytes();
        }
        if include {
            selected.extend_from_slice(line);
        }
    }
    if selected.is_empty() {
        return Err(comparison_error(
            "the captured file pair is missing from its patch",
        ));
    }
    Ok(String::from_utf8_lossy(&selected).into_owned())
}

/// Git's core.quotePath C spelling, including raw Unix pathname bytes.
fn patch_path(prefix: &str, path: &Path) -> String {
    use std::fmt::Write;
    let bytes: Vec<_> = prefix
        .bytes()
        .chain(path.as_os_str().as_encoded_bytes().iter().copied())
        .collect();
    if bytes
        .iter()
        .all(|byte| (32..127).contains(byte) && !matches!(byte, b'"' | b'\\'))
    {
        return String::from_utf8(bytes).expect("ASCII path");
    }
    let mut quoted = String::from("\"");
    for byte in bytes {
        match byte {
            b'"' => quoted.push_str("\\\""),
            b'\\' => quoted.push_str("\\\\"),
            7 => quoted.push_str("\\a"),
            8 => quoted.push_str("\\b"),
            b'\t' => quoted.push_str("\\t"),
            b'\n' => quoted.push_str("\\n"),
            11 => quoted.push_str("\\v"),
            12 => quoted.push_str("\\f"),
            b'\r' => quoted.push_str("\\r"),
            32..=126 => quoted.push(char::from(byte)),
            _ => {
                let _ = write!(quoted, "\\{byte:03o}");
            }
        }
    }
    quoted.push('"');
    quoted
}
