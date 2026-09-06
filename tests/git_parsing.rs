// SPDX-License-Identifier: MPL-2.0

//! Git's machine-readable output, read straight by the parsers that own it.
//!
//! `tests/git_provider.rs` proves the provider against a real repository, so
//! it can only ever exercise what the installed Git chooses to write. The
//! refusals below are the other half: output that is truncated, mislabelled,
//! not UTF-8, or past a bound Runyte sets for itself. A parser reached only
//! through a real child would have those branches described but never run,
//! and the whole point of them is that a future Git, a wrapper script, or a
//! corrupted repository is not allowed to become a wrong answer.
//!
//! The workspace-facts reader uses fabricated `.git` layouts in a temporary
//! directory. Worktree parsing also checks whether the reported paths exist.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use runyte::git::{
    GitError, MAX_BLAME_LINES, MAX_STASH_ENTRIES, Repository, parse_blame, parse_commit_search,
    parse_hunks, parse_log, parse_stashes, parse_worktree_porcelain, read_workspace_git_facts,
    select_lines,
};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let number = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "runyte-git-parsing-{label}-{}-{number}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// Writes one file, creating the directories above it.
    fn write(&self, relative: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Joins NUL-delimited fields the way `-z` output arrives.
fn record(fields: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for field in fields {
        bytes.extend_from_slice(field);
        bytes.push(0);
    }
    bytes
}

fn detail(error: &GitError) -> String {
    error.to_string()
}

const OID: &[u8] = b"1234567890abcdef1234567890abcdef12345678";

/// The eight `git log -z` fields, in order, for one ordinary commit.
fn log_fields() -> Vec<Vec<u8>> {
    vec![
        OID.to_vec(),
        b"1234567".to_vec(),
        Vec::new(),
        b"Author".to_vec(),
        b"1700000000".to_vec(),
        b"2023-11-14".to_vec(),
        b"Subject".to_vec(),
        Vec::new(),
    ]
}

/// Builds a log document from `log_fields` with one field replaced.
fn log_with(index: usize, value: &[u8]) -> Vec<u8> {
    let mut fields = log_fields();
    fields[index] = value.to_vec();
    record(&fields.iter().map(Vec::as_slice).collect::<Vec<_>>())
}

#[test]
fn a_log_record_whose_abbreviated_id_is_not_hexadecimal_is_refused() {
    // The full object id is checked against its own alphabet, and the short
    // one has to be checked separately: it is what every row is labelled with
    // and what a later request is resolved from.
    let error = parse_log(&log_with(1, b"not-hex")).unwrap_err();
    assert!(
        detail(&error).contains("abbreviated object id is invalid"),
        "{error}"
    );
    assert!(parse_log(&log_with(1, b"")).is_err(), "an empty short id");
    assert!(parse_log(&log_with(0, b"1234")).is_err(), "a truncated id");
}

#[test]
fn a_log_record_whose_author_time_is_not_an_integer_is_refused() {
    let error = parse_log(&log_with(4, b"yesterday")).unwrap_err();
    assert!(
        detail(&error).contains("author time is not an integer"),
        "{error}"
    );
}

#[test]
fn a_log_field_that_is_not_utf8_is_refused_rather_than_replaced() {
    // Author names and subjects are read leniently, because a commit written
    // in an unknown encoding is still worth showing. The identities and the
    // timestamp are not: a lossy replacement there would be a value that
    // resolves to nothing.
    let error = parse_log(&log_with(0, b"\xff\xfe")).unwrap_err();
    assert!(detail(&error).contains("is not UTF-8"), "{error}");

    let error = parse_log(&log_with(4, b"17000\xff0000")).unwrap_err();
    assert!(
        detail(&error).contains("author time is not UTF-8"),
        "{error}"
    );

    let lenient = parse_log(&log_with(3, b"Auth\xffor")).unwrap();
    assert_eq!(lenient[0].author, "Auth\u{fffd}or");
}

#[test]
fn a_commit_search_record_missing_its_message_is_refused() {
    // Commit search asks for the log's eight fields plus the full message.
    // Eight fields alone parse perfectly well as a log record, so the count
    // is the only thing that can catch a query that lost its ninth field.
    let error = parse_commit_search(&log_with(7, b"HEAD -> main")).unwrap_err();
    assert!(
        detail(&error).contains("commit-search record does not contain nine fields"),
        "{error}"
    );

    let mut fields = log_fields();
    fields.push(b"Subject\n\nThe body.\n".to_vec());
    let searched = parse_commit_search(&record(
        &fields.iter().map(Vec::as_slice).collect::<Vec<_>>(),
    ))
    .unwrap();
    assert_eq!(searched[0].message, "Subject\n\nThe body.\n");
    assert!(searched[0].haystack().contains("2023-11-14"));
}

#[test]
fn a_stash_list_accepts_its_ceiling_and_refuses_one_more_entry() {
    let mut output = Vec::new();
    for index in 0..MAX_STASH_ENTRIES {
        output.extend_from_slice(&record(&[
            OID,
            format!("stash@{{{index}}}").as_bytes(),
            b"WIP on main",
        ]));
    }

    assert_eq!(parse_stashes(&output).unwrap().len(), MAX_STASH_ENTRIES);
    output.extend_from_slice(&record(&[
        OID,
        format!("stash@{{{MAX_STASH_ENTRIES}}}").as_bytes(),
        b"WIP on main",
    ]));
    let error = parse_stashes(&output).unwrap_err();
    assert!(
        matches!(error, GitError::TooLarge { limit, .. } if limit == MAX_STASH_ENTRIES),
        "{error}"
    );
}

#[test]
fn a_stash_entry_that_cannot_be_selected_again_is_refused() {
    // The selector is what a later apply or drop is addressed by, so one that
    // is not `stash@{n}` is not an entry a person can act on.
    let error = parse_stashes(&record(&[OID, b"stash-0", b"WIP on main"])).unwrap_err();
    assert!(
        detail(&error).contains("stash selector is invalid"),
        "{error}"
    );

    let entries = parse_stashes(&record(&[OID, b"stash@{0}", b"WIP on main"])).unwrap();
    assert_eq!(entries[0].selector, "stash@{0}");
}

/// One `--line-porcelain` record: the header, the fields, then the content.
fn blame_record(oid: &[u8], line: usize, tz: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(oid);
    bytes.extend_from_slice(format!(" {line} {line} 1\n").as_bytes());
    bytes.extend_from_slice(b"author Author\nauthor-time 1700000000\n");
    bytes.extend_from_slice(format!("author-tz {tz}\n").as_bytes());
    bytes.extend_from_slice(b"summary Subject\n\tcontent\n");
    bytes
}

#[test]
fn a_blame_timezone_that_is_not_an_offset_leaves_the_date_absent() {
    // The date is computed from the timestamp in the commit's own timezone,
    // so an unreadable offset has no correct answer. Dropping the date keeps
    // the rest of the attribution rather than shifting it by a day.
    for tz in ["", "+1", "Z0000", "+aa00"] {
        let lines = parse_blame(&blame_record(OID, 1, tz)).unwrap();
        assert_eq!(lines[0].author_date, None, "timezone `{tz}`");
        assert_eq!(lines[0].author_time, Some(1_700_000_000));
        assert_eq!(lines[0].author, "Author");
    }

    let lines = parse_blame(&blame_record(OID, 1, "+0100")).unwrap();
    assert_eq!(lines[0].author_date.as_deref(), Some("2023-11-14"));
}

#[test]
fn blame_accepts_its_line_ceiling_and_refuses_one_more_line() {
    let mut output = Vec::new();
    for line in 1..=MAX_BLAME_LINES {
        output.extend_from_slice(&blame_record(OID, line, "+0000"));
    }

    assert_eq!(parse_blame(&output).unwrap().len(), MAX_BLAME_LINES);
    output.extend_from_slice(&blame_record(OID, MAX_BLAME_LINES + 1, "+0000"));
    let error = parse_blame(&output).unwrap_err();
    assert!(
        matches!(&error, GitError::Failed { command, stderr, .. }
            if command == "git blame"
                && stderr == &format!("blame output is limited to {MAX_BLAME_LINES} lines")),
        "the refusal states the ceiling: {error}"
    );
}

fn worktree_repository() -> Repository {
    Repository::new("/repository")
}

#[test]
fn worktree_records_end_at_the_next_path_without_a_blank_field() {
    // Git separates records with an empty field, but the last one before the
    // end of the stream has no separator after it, and a wrapper that drops
    // them entirely still describes distinct checkouts. A path field is
    // therefore what starts a record, whatever came before it.
    let output = record(&[
        b"worktree /repository",
        b"HEAD 1234567890abcdef1234567890abcdef12345678",
        b"branch refs/heads/main",
        b"worktree /repository/linked",
        b"HEAD 1234567890abcdef1234567890abcdef12345678",
        b"detached",
    ]);

    let worktrees = parse_worktree_porcelain(&worktree_repository(), &output).unwrap();
    assert_eq!(worktrees.len(), 2);
    assert_eq!(worktrees[0].path, PathBuf::from("/repository"));
    assert_eq!(worktrees[0].branch.as_deref(), Some("refs/heads/main"));
    assert!(!worktrees[0].detached);
    assert_eq!(worktrees[1].path, PathBuf::from("/repository/linked"));
    assert!(worktrees[1].detached);
    assert_eq!(worktrees[1].branch, None);
}

#[test]
fn a_worktree_field_arriving_before_its_path_is_refused() {
    let error = parse_worktree_porcelain(
        &worktree_repository(),
        &record(&[b"HEAD 1234567890abcdef1234567890abcdef12345678"]),
    )
    .unwrap_err();
    assert!(
        detail(&error).contains("worktree record has fields before its path"),
        "{error}"
    );
}

#[test]
fn a_worktree_listing_with_no_records_is_refused() {
    // Git always lists at least the main checkout, so an empty answer means
    // the command did not do what was asked rather than that no checkout
    // exists. Returning an empty list would read as "every worktree is gone".
    for output in [b"".as_slice(), b"\0\0".as_slice()] {
        let error = parse_worktree_porcelain(&worktree_repository(), output).unwrap_err();
        assert!(
            detail(&error).contains("Git returned no worktree records"),
            "{error}"
        );
    }
}

#[test]
fn a_lock_or_prune_marker_without_a_reason_is_still_locked_or_prunable() {
    let output = record(&[b"worktree /repository/linked", b"locked", b"prunable"]);

    let worktrees = parse_worktree_porcelain(&worktree_repository(), &output).unwrap();
    assert_eq!(worktrees[0].locked.as_deref(), Some(""));
    assert_eq!(worktrees[0].prunable.as_deref(), Some(""));

    let with_reasons = parse_worktree_porcelain(
        &worktree_repository(),
        &record(&[
            b"worktree /repository/linked",
            b"locked being rebased",
            b"prunable gitdir file points to non-existent location",
        ]),
    )
    .unwrap();
    assert_eq!(with_reasons[0].locked.as_deref(), Some("being rebased"));
    assert_eq!(
        with_reasons[0].prunable.as_deref(),
        Some("gitdir file points to non-existent location")
    );
}

#[test]
fn a_workspace_reaches_its_git_directory_through_a_relative_gitfile() {
    // `git worktree add` writes an absolute gitdir, but the file is a path
    // like any other and a repository moved as a whole may carry a relative
    // one. Resolving it against the workspace is what keeps the row readable
    // after such a move.
    let scratch = TempDir::new("relative-gitfile");
    scratch.write(".git", "gitdir: .git-private\n");
    scratch.write(".git-private/HEAD", "ref: refs/heads/enh/moved\n");
    scratch.write(".git-private/commondir", "../shared.git\n");
    scratch.write(
        "shared.git/config",
        "[remote \"origin\"]\n\turl = https://example.invalid/project.git\n",
    );

    let facts = read_workspace_git_facts(scratch.path()).expect("the workspace has facts");
    assert_eq!(facts.branch.as_deref(), Some("enh/moved"));
    assert_eq!(facts.worktree.as_deref(), Some(scratch.path()));
    assert_eq!(
        facts.remote.as_deref(),
        Some("https://example.invalid/project.git")
    );
}

#[test]
fn a_commondir_naming_the_git_directory_itself_is_a_main_checkout() {
    // `.` is the usual spelling, and an empty file says nothing at all. Both
    // are a repository stating that it shares its objects with nobody, so
    // neither may label the workspace as a linked worktree.
    for commondir in [".", "", "  \n"] {
        let scratch = TempDir::new("own-commondir");
        scratch.write(".git/HEAD", "ref: refs/heads/main\n");
        scratch.write(".git/commondir", commondir);

        let facts = read_workspace_git_facts(scratch.path()).expect("the workspace has facts");
        assert_eq!(facts.worktree, None, "commondir `{commondir}`");
        assert_eq!(facts.branch.as_deref(), Some("main"));
    }
}

#[test]
fn origin_is_preferred_over_an_earlier_remote_and_non_remote_urls() {
    let scratch = TempDir::new("origin-preferred");
    scratch.write(".git/HEAD", "ref: refs/heads/main\n");
    scratch.write(
        ".git/config",
        concat!(
            "# a comment naming url = not-a-remote\n",
            "[core]\n",
            "\turl = https://example.invalid/core.git\n",
            "[remote \"upstream\"]\n",
            "\tfetch = +refs/heads/*:refs/remotes/upstream/*\n",
            "\turl = https://example.invalid/upstream.git\n",
            "[remote \"origin\"]\n",
            "\turl = https://example.invalid/origin.git\n",
        ),
    );

    let facts = read_workspace_git_facts(scratch.path()).expect("the workspace has facts");
    assert_eq!(
        facts.remote.as_deref(),
        Some("https://example.invalid/origin.git")
    );
}

#[test]
fn without_an_origin_the_first_remote_that_names_a_url_is_taken() {
    // A hand-edited config may use the one-token `[remote.name]` spelling,
    // and a remote may be declared with no URL at all. Neither is what Git
    // writes, and both have to read as the remote they describe rather than
    // as no remote.
    let scratch = TempDir::new("first-remote");
    scratch.write(".git/HEAD", "ref: refs/heads/main\n");
    scratch.write(
        ".git/config",
        concat!(
            "[remote \"empty\"]\n",
            "\turl =\n",
            "[remote.backup]\n",
            "\turl = \"https://example.invalid/backup.git\"\n",
            "[remote \"later\"]\n",
            "\turl = https://example.invalid/later.git\n",
        ),
    );

    let facts = read_workspace_git_facts(scratch.path()).expect("the workspace has facts");
    assert_eq!(
        facts.remote.as_deref(),
        Some("https://example.invalid/backup.git")
    );
}

#[test]
fn an_oversized_gitfile_is_not_the_file_this_reader_is_looking_for() {
    // The `.git` link holds one short line. Something far larger under that
    // name is not a link Runyte can trust, so the workspace reads as having
    // no Git facts rather than as having whatever the first line said.
    let scratch = TempDir::new("oversized-gitfile");
    let mut contents = b"gitdir: .git-private\n".to_vec();
    contents.resize(8 * 1024, b'x');
    scratch.write(".git", contents);
    scratch.write(".git-private/HEAD", "ref: refs/heads/main\n");

    assert_eq!(read_workspace_git_facts(scratch.path()), None);
}

const FILE_HEADER: &str = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n";

#[test]
fn a_hunk_body_line_with_an_unknown_prefix_is_refused() {
    // Every line inside a hunk is a context, addition, deletion, or the
    // no-newline marker. Anything else means the byte ranges this parser
    // counted are not the ranges Git would apply.
    let patch = format!("{FILE_HEADER}@@ -1,2 +1,2 @@\n-old\n+new\n?strange\n");
    let error = parse_hunks(patch.as_bytes()).unwrap_err();
    assert!(
        detail(&error).contains("hunk contains an unknown line prefix"),
        "{error}"
    );
}

#[test]
fn a_hunk_header_whose_range_starts_at_zero_with_content_is_refused() {
    // Line zero exists only as the empty range a pure insertion or deletion
    // is anchored at. A count alongside it describes lines that cannot be
    // addressed.
    let patch = format!("{FILE_HEADER}@@ -0,1 +1 @@\n-old\n+new\n");
    let error = parse_hunks(patch.as_bytes()).unwrap_err();
    assert!(detail(&error).contains("invalid hunk range"), "{error}");
}

#[test]
fn an_empty_or_inverted_selection_is_not_a_line_range() {
    let patch = format!("{FILE_HEADER}@@ -1 +1 @@\n-old\n+new\n");
    let hunk = parse_hunks(patch.as_bytes()).unwrap().remove(0);

    for (first, last) in [(0, 1), (0, 0), (3, 2)] {
        let error = select_lines(&hunk, first, last).unwrap_err();
        assert!(
            detail(&error).contains("selected line range is invalid"),
            "{first}..{last}: {error}"
        );
    }
}

#[test]
fn selecting_only_some_of_a_hunks_new_lines_is_refused_rather_than_split() {
    // Splitting a hunk would mean rewriting its ranges and counts, which is
    // patch surgery rather than staging. The refusal names the whole-hunk
    // action that does work.
    let patch = format!("{FILE_HEADER}@@ -1 +1,2 @@\n-old\n+first\n+second\n");
    let hunk = parse_hunks(patch.as_bytes()).unwrap().remove(0);
    assert_eq!(hunk.new_lines, [1, 2]);

    let error = select_lines(&hunk, 1, 1).unwrap_err();
    assert!(
        detail(&error).contains("requires every changed new line in one hunk"),
        "{error}"
    );
    assert_eq!(select_lines(&hunk, 1, 2).unwrap(), hunk.patch);
}

#[test]
fn a_patch_carrying_binary_content_is_refused_before_it_is_read_as_text() {
    for patch in [
        b"diff --git a/a.png b/a.png\nGIT binary patch\nliteral 4\n".to_vec(),
        {
            let mut bytes = b"diff --git a/a.bin b/a.bin\n@@ -1 +1 @@\n-old\n+".to_vec();
            bytes.extend_from_slice(&[0, b'\n']);
            bytes
        },
    ] {
        let error = parse_hunks(&patch).unwrap_err();
        assert!(
            detail(&error).contains("binary patches cannot be partially staged"),
            "{error}"
        );
    }
}
