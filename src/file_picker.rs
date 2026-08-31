// SPDX-License-Identifier: MPL-2.0

//! Native fuzzy file discovery and presentation-neutral picker state.
//!
//! The scanner deliberately uses only the standard library and Runyte's
//! existing regular-expression engine. It never invokes `git`, `fd`, `find`,
//! or a fuzzy-finder process. Background workers emit immutable batches; the
//! editor remains the sole owner of picker state.

use std::{
    fs,
    io::{self, BufRead, BufReader, Read},
    num::NonZero,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use regex::Regex;
use tokio::sync::mpsc::{Receiver, Sender, channel};

const SCAN_BATCH: usize = 1024;
const PREVIEW_BYTES: u64 = 64 * 1024;
const PREVIEW_CONTEXT_BEFORE: usize = 4;
const PREVIEW_CONTEXT_LINES: usize = PREVIEW_CONTEXT_BEFORE * 2 + 1;
const GREP_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// How many ranking candidates a picker will hold.
///
/// This is a budget on the rank pass a keystroke pays for, not on how much of
/// a project was searched: content search filters in the scanner, so reaching
/// this means a query matches more than 50,000 lines, which a longer query
/// resolves. Ranking is `O(candidates × query × line)` and measurably linear
/// in the count, so the number is where a full budget still re-ranks in about
/// the time a person reads as immediate. On a 147,000-line project every query
/// past a single character comes in complete under it.
pub const CONTENT_ENTRY_LIMIT: usize = 50_000;
const GREP_LINE_CHARACTERS: usize = 512;
const PREVIEW_DIRECTORY_ENTRIES: usize = 512;
/// How long a content scan waits, cancellably, before it reads anything.
///
/// Content search filters in the scanner, so every edit to the query starts a
/// new scan. Settling first means a fast typist starts one project read
/// rather than one per character: each intermediate scan is cancelled while
/// it is still asleep.
/// The candidate count below which a rank pass is not worth dividing.
///
/// Splitting costs a thread spawn per chunk, so a picker holding a handful of
/// paths ranks them on the thread it is already on. Chosen well above where
/// spawning stops being visible against the work it replaces.
const PARALLEL_RANK_CHUNK: usize = 2_048;
const CONTENT_SCAN_SETTLE: Duration = Duration::from_millis(60);
const CONTENT_SCAN_SETTLE_STEP: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilePickerKind {
    Files,
    Contents,
}

impl FilePickerKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Contents => "Fuzzy grep",
        }
    }
}

/// A candidate path discovered by the walker, tagged with the file-type bit
/// already known from `DirEntry::file_type`, so downstream code never needs
/// a second `fs::metadata` call to tell files and directories apart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanEntry {
    pub path: PathBuf,
    pub is_dir: bool,
}

impl ScanEntry {
    pub fn file(path: PathBuf) -> Self {
        Self {
            path,
            is_dir: false,
        }
    }

    pub fn directory(path: PathBuf) -> Self {
        Self { path, is_dir: true }
    }
}

/// One file, held once however many of its lines matched.
///
/// A project's matches cluster: at a full budget on Runyte's own repository
/// 50,000 candidates come from 207 files. Giving every line its own `PathBuf`
/// and its own rendered `relative` spent 2.4MB on paths where the distinct
/// paths are 14KB, and rebuilt `relative` per line through `strip_prefix` and
/// a component walk. Both are done once here, per file.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PickerFile {
    path: PathBuf,
    relative: String,
    is_dir: bool,
}

/// One ranking candidate: a path, or one matching line of one.
///
/// Carries an index into the picker's file table rather than a path. Its
/// character count is computed once at admission because the score sort uses
/// it as a tiebreaker many times while the candidate text remains immutable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntry {
    file: u32,
    row: Option<usize>,
    column: usize,
    text: Option<String>,
    candidate_characters: usize,
}

/// An entry with its file resolved, which is how anything outside the picker
/// reads one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EntryView<'a> {
    pub path: &'a Path,
    pub relative: &'a str,
    pub row: Option<usize>,
    pub column: usize,
    pub text: Option<&'a str>,
    pub is_dir: bool,
}

impl EntryView<'_> {
    fn candidate(&self) -> &str {
        self.text.unwrap_or(self.relative)
    }

    pub fn label(&self) -> String {
        if let Some(row) = self.row {
            format!("{}:{}", self.relative, row + 1)
        } else if self.is_dir {
            format!("{}/", self.relative)
        } else {
            self.relative.to_owned()
        }
    }

    pub fn match_positions_in_label(&self, positions: &[usize]) -> Vec<usize> {
        if self.text.is_some() {
            Vec::new()
        } else {
            positions.to_vec()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerTarget {
    pub path: PathBuf,
    pub row: Option<usize>,
    pub column: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FuzzyMatch {
    pub entry: usize,
    pub score: i64,
    /// Character indices in the path or content text being ranked.
    pub positions: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FilePreview {
    Text(Vec<String>),
    Snippet(FilePreviewSnippet),
    Binary,
    Directory(Vec<String>),
    Unreadable(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePreviewSnippet {
    pub lines: Vec<String>,
    pub start_row: usize,
    pub focus_row: usize,
    pub emphasis: Vec<usize>,
}

impl FilePreviewSnippet {
    pub fn display_lines(&self) -> Vec<String> {
        let line_digits = (self.start_row + self.lines.len()).max(1).to_string().len();
        self.lines
            .iter()
            .enumerate()
            .map(|(offset, line)| {
                let row = self.start_row + offset;
                let marker = if row == self.focus_row { '›' } else { ' ' };
                format!("{marker} {:>line_digits$} │ {line}", row + 1)
            })
            .collect()
    }
}

/// Whether the emphasized candidate characters are direct, meaning each term
/// of the query landed on a contiguous span of its own.
///
/// The scorer returns one position per query character, in query order, so a
/// run of consecutive positions is a stretch that matched as itself. A query
/// is direct when it produced no more runs than it has terms: one word landing
/// on one span, or three words on three. More runs than terms means a gap
/// opened inside a term, which is the fuzzy subsequence the secondary colour
/// is for.
///
/// Fewer runs than terms is not a lesser match but a better one — terms that
/// happen to sit next to each other in the candidate merge into a single run,
/// which is the tightest a multi-word query can land.
pub fn is_direct_match(positions: &[usize], query: &str) -> bool {
    if positions.is_empty() {
        return false;
    }
    let runs = 1 + positions
        .windows(2)
        .filter(|pair| pair[1] != pair[0] + 1)
        .count();
    runs <= query.split_whitespace().count().max(1)
}

impl FilePreview {
    pub fn from_text(text: &str) -> Self {
        Self::from_lines(text.lines().map(str::to_owned))
    }

    pub fn from_lines(lines: impl Iterator<Item = String>) -> Self {
        let mut lines = lines
            .take(512)
            .map(|line| line.chars().take(512).collect::<String>())
            .collect::<Vec<_>>();
        if lines.is_empty() {
            lines.push(String::new());
        }
        Self::Text(lines)
    }

    pub fn snippet_from_text(text: &str, focus_row: usize, emphasis: Vec<usize>) -> Self {
        Self::snippet_from_lines(text.lines().map(str::to_owned), focus_row, emphasis)
    }

    pub fn snippet_from_lines(
        lines: impl Iterator<Item = String>,
        focus_row: usize,
        emphasis: Vec<usize>,
    ) -> Self {
        let start_row = focus_row.saturating_sub(PREVIEW_CONTEXT_BEFORE);
        let lines = lines
            .skip(start_row)
            .take(PREVIEW_CONTEXT_LINES)
            .map(|line| line.chars().take(512).collect::<String>())
            .collect::<Vec<_>>();
        if lines.is_empty() {
            return Self::Unreadable("the matching line is no longer present".to_owned());
        }
        Self::Snippet(FilePreviewSnippet {
            lines,
            start_row,
            focus_row,
            emphasis,
        })
    }

    pub fn snippet_from_path(path: &Path, focus_row: usize, emphasis: Vec<usize>) -> Self {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => return Self::Unreadable(error.to_string()),
        };
        if metadata.len() > GREP_FILE_BYTES {
            return Self::Unreadable("file is now too large for a content preview".to_owned());
        }
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) => return Self::Unreadable(error.to_string()),
        };
        let start_row = focus_row.saturating_sub(PREVIEW_CONTEXT_BEFORE);
        match BufReader::new(file)
            .lines()
            .skip(start_row)
            .take(PREVIEW_CONTEXT_LINES)
            .collect::<io::Result<Vec<_>>>()
        {
            Ok(lines) if lines.is_empty() => {
                Self::Unreadable("the matching line is no longer present".to_owned())
            }
            Ok(lines) => Self::Snippet(FilePreviewSnippet {
                lines: lines
                    .into_iter()
                    .map(|line| line.chars().take(512).collect())
                    .collect(),
                start_row,
                focus_row,
                emphasis,
            }),
            Err(error) => Self::Unreadable(error.to_string()),
        }
    }

    pub fn from_path(path: &Path) -> Self {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => return Self::Unreadable(error.to_string()),
        };
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) => return Self::Unreadable(error.to_string()),
        };
        let mut bytes = Vec::with_capacity(metadata.len().min(PREVIEW_BYTES) as usize);
        if let Err(error) = file.by_ref().take(PREVIEW_BYTES).read_to_end(&mut bytes) {
            return Self::Unreadable(error.to_string());
        }
        let complete = metadata.len() <= PREVIEW_BYTES;
        if crate::external_open::is_binary(&bytes, complete) {
            return Self::Binary;
        }
        match std::str::from_utf8(&bytes) {
            Ok(text) => Self::from_text(text),
            Err(error) if !complete && error.error_len().is_none() => {
                let text = std::str::from_utf8(&bytes[..error.valid_up_to()])
                    .expect("the UTF-8 validator named a valid prefix");
                Self::from_text(text)
            }
            Err(error) => Self::Unreadable(error.to_string()),
        }
    }

    /// A one-level listing of `path`'s files and subdirectories, bounded to
    /// `PREVIEW_DIRECTORY_ENTRIES` names.
    ///
    /// Deliberately does not reuse `fs_plan::DirectorySnapshot`: that
    /// snapshot calls `fs::symlink_metadata` on every entry to carry the
    /// fingerprint the explorer's apply step needs, which is unbounded work
    /// this preview cannot afford on a directory with hundreds of thousands
    /// of entries. This walk only inspects entry type, and only for the
    /// entries it keeps.
    pub fn from_directory(path: &Path, show_hidden: bool) -> Self {
        let read_dir = match fs::read_dir(path) {
            Ok(read_dir) => read_dir,
            Err(error) => return Self::Unreadable(error.to_string()),
        };
        let mut kept = Vec::new();
        let mut omitted = 0usize;
        for entry in read_dir {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            let Some(text) = name.to_str() else { continue };
            if !show_hidden && text.starts_with('.') {
                continue;
            }
            if kept.len() < PREVIEW_DIRECTORY_ENTRIES {
                let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
                kept.push((text.to_owned(), is_dir));
            } else {
                omitted += 1;
            }
        }
        kept.sort_unstable();
        let mut names = kept
            .into_iter()
            .map(|(name, is_dir)| if is_dir { format!("{name}/") } else { name })
            .collect::<Vec<_>>();
        if omitted > 0 {
            names.push(format!(
                "… {omitted} more entr{} not shown",
                if omitted == 1 { "y" } else { "ies" }
            ));
        }
        Self::Directory(names)
    }
}

#[derive(Clone, Debug)]
pub struct FilePicker {
    pub scan_id: u64,
    pub root: PathBuf,
    pub kind: FilePickerKind,
    /// Every distinct file the entries below refer to, each held once.
    files: Vec<PickerFile>,
    pub entries: Vec<FileEntry>,
    pub matches: Vec<FuzzyMatch>,
    pub query: String,
    /// The query the entries on hand were collected for.
    ///
    /// Only content search sets this to anything but the empty string: its
    /// scanner filters, so its entry list answers one query rather than the
    /// whole project. The file picker collects paths unconditionally, and an
    /// empty scan query correctly says every path is still on hand.
    scan_query: String,
    pub query_cursor: usize,
    pub selected: usize,
    pub loading: bool,
    pub skipped: usize,
    pub limited: bool,
    pub error: Option<String>,
    pub show_preview: bool,
    pub preview: Option<FilePreview>,
    selection_user_owned: bool,
    /// Whether the most recent full `rank` pass restricted `matches` to
    /// directories. Narrowing from the existing `matches` is only valid
    /// while this stays unchanged across a query edit: the moment a
    /// trailing-slash query stops applying, entries it excluded (plain
    /// files) must be recovered from the full entry list, not from the
    /// already-narrowed set that no longer contains them.
    directory_only: bool,
    /// Project finders treat `file`, `buffer`, and `terminal` as soft name
    /// preferences. Directory-scoped pickers still match those words
    /// literally.
    unified_finder: bool,
}

impl FilePicker {
    pub fn new(scan_id: u64, root: PathBuf) -> Self {
        Self::with_kind(scan_id, root, FilePickerKind::Files)
    }

    pub fn grep(scan_id: u64, root: PathBuf) -> Self {
        Self::with_kind(scan_id, root, FilePickerKind::Contents)
    }

    fn with_kind(scan_id: u64, root: PathBuf, kind: FilePickerKind) -> Self {
        Self {
            scan_id,
            root,
            kind,
            files: Vec::new(),
            entries: Vec::new(),
            matches: Vec::new(),
            query: String::new(),
            scan_query: String::new(),
            query_cursor: 0,
            selected: 0,
            loading: true,
            skipped: 0,
            limited: false,
            error: None,
            show_preview: true,
            preview: None,
            selection_user_owned: false,
            directory_only: false,
            unified_finder: false,
        }
    }

    pub fn enable_unified_finder(&mut self) {
        self.unified_finder = true;
        self.rank(true, false);
    }

    /// Changes the project finder's filesystem engine while retaining the
    /// query, cursor, and preview preference held by the overlay.
    pub fn switch_kind(&mut self, scan_id: u64, kind: FilePickerKind) {
        self.scan_id = scan_id;
        self.kind = kind;
        self.files.clear();
        self.entries.clear();
        self.matches.clear();
        self.scan_query = if kind == FilePickerKind::Contents {
            self.query.clone()
        } else {
            String::new()
        };
        self.selected = 0;
        self.loading = true;
        self.skipped = 0;
        self.limited = false;
        self.error = None;
        self.preview = None;
        self.selection_user_owned = false;
        self.directory_only = false;
    }

    pub fn add_paths(&mut self, paths: Vec<ScanEntry>) {
        let selected = self
            .selection_user_owned
            .then(|| self.selected_target())
            .flatten();
        let first_new = self.entries.len();
        for entry in paths {
            let file = self.intern(entry.path, entry.is_dir);
            let candidate_characters = self.files[file as usize].relative.chars().count();
            self.entries.push(FileEntry {
                file,
                row: None,
                column: 0,
                text: None,
                candidate_characters,
            });
        }
        self.rank_new_entries(first_new, selected);
    }

    pub fn add_content(&mut self, files: Vec<FileHits>) {
        let selected = self.selection_user_owned.then(|| self.selected_target());
        let first_new = self.entries.len();
        let mut available = CONTENT_ENTRY_LIMIT.saturating_sub(first_new);
        for mut hits in files {
            // The budget counts lines, not files, so one very large file
            // cannot be admitted whole past the ceiling.
            if hits.truncate(available) {
                self.limited = true;
            }
            if hits.is_empty() {
                break;
            }
            available -= hits.lines.len();
            let file = self.intern(hits.path, false);
            self.entries.extend(hits.lines.into_iter().map(|line| {
                let candidate_characters = line.text.chars().count();
                FileEntry {
                    file,
                    row: Some(line.row),
                    column: line.column,
                    text: Some(line.text),
                    candidate_characters,
                }
            }));
        }
        self.rank_new_entries(first_new, selected.flatten());
    }

    /// The file table index for `path`, adding it if this is its first line.
    ///
    /// Batches arrive per file and a restart clears the table, so the file
    /// wanted is almost always the one just used; a scan of a few hundred rows
    /// on the rare miss is cheaper than a map that has to be maintained.
    fn intern(&mut self, path: PathBuf, is_dir: bool) -> u32 {
        if let Some(index) = self.files.iter().rposition(|file| file.path == path) {
            return index as u32;
        }
        self.files.push(PickerFile {
            relative: path_text(path.strip_prefix(&self.root).unwrap_or(&path)),
            path,
            is_dir,
        });
        self.files.len() as u32 - 1
    }

    /// An entry with its file resolved.
    pub fn view(&self, entry: usize) -> Option<EntryView<'_>> {
        self.entries.get(entry).map(|entry| self.resolve(entry))
    }

    /// Every entry, in the order they were collected.
    pub fn views(&self) -> impl Iterator<Item = EntryView<'_>> {
        self.entries.iter().map(|entry| self.resolve(entry))
    }

    /// The ranked entries, best first.
    pub fn ranked(&self) -> impl Iterator<Item = EntryView<'_>> {
        self.matches
            .iter()
            .map(|found| self.resolve(&self.entries[found.entry]))
    }

    /// How many distinct files the entries refer to.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    fn resolve<'a>(&'a self, entry: &'a FileEntry) -> EntryView<'a> {
        resolve_in(&self.files, entry)
    }

    /// What this picker ranks, which decides whether the path heuristics in
    /// the scorer apply. Content results rank line text; the file picker ranks
    /// paths.
    fn candidate_kind(&self) -> FuzzyCandidate {
        match self.kind {
            FilePickerKind::Files => FuzzyCandidate::Path,
            FilePickerKind::Contents => FuzzyCandidate::Line,
        }
    }

    /// Whether the query should be narrowed to directory entries: only the
    /// plain file picker honors a trailing `/`, and the slash itself is not
    /// part of the text handed to the matcher.
    fn directory_only_query(&self) -> (bool, String) {
        let query = if self.unified_finder && self.kind == FilePickerKind::Files {
            crate::finder::finder_matching_query(&self.query)
        } else {
            self.query.clone()
        };
        if self.kind == FilePickerKind::Files && query.ends_with('/') {
            (true, query.trim_end_matches('/').to_owned())
        } else {
            (false, query)
        }
    }

    fn rank_new_entries(&mut self, first_new: usize, selected: Option<PickerTarget>) {
        let (directory_only, query) = self.directory_only_query();
        let new = (first_new..self.entries.len()).collect::<Vec<_>>();
        self.matches.extend(rank_entries(
            &self.files,
            &self.entries,
            &new,
            &query,
            directory_only,
            self.candidate_kind(),
        ));
        self.sort_matches();
        self.selected = 0;
        if let Some(selected) = selected
            && let Some(index) = self.matches.iter().position(|found| {
                let entry = &self.entries[found.entry];
                let entry = self.resolve(entry);
                entry.path == selected.path && entry.row == selected.row
            })
        {
            self.selected = index;
        }
    }

    /// Rebinds the picker to a fresh content scan of the current query.
    ///
    /// The query, its cursor, and the preview toggle are what the person is
    /// holding on screen, so they survive; everything the previous query's
    /// scan produced does not, because those entries answered a different
    /// question and cannot be narrowed into this one.
    pub fn restart_content_scan(&mut self, scan_id: u64) {
        self.scan_id = scan_id;
        self.scan_query = self.query.clone();
        self.files.clear();
        self.entries.clear();
        self.matches.clear();
        self.selected = 0;
        self.selection_user_owned = false;
        self.loading = true;
        self.skipped = 0;
        self.limited = false;
        self.error = None;
        self.preview = None;
        self.directory_only = false;
    }

    /// Whether the entries on hand can still answer the current query.
    ///
    /// A content scan keeps the lines its own query matched, so appending to
    /// that query can only narrow the set: those entries stay authoritative
    /// and the narrowing happens in memory, with no second walk of the
    /// project. Two things break that. A query that is no longer an extension
    /// of the scanned one — anything deleted — asks about lines the scan
    /// discarded. And a scan that hit `CONTENT_ENTRY_LIMIT` stopped early, so
    /// the project holds matches it never reached; that is the case where
    /// narrowing in memory silently loses results, and where the person sees
    /// a match appear only once the file holding it is open.
    ///
    /// Restarting for a query the current scan already ran would loop, so a
    /// query equal to the scanned one is always answered by what is on hand.
    pub fn content_rescan_needed(&self) -> bool {
        self.kind == FilePickerKind::Contents
            && self.query != self.scan_query
            && (self.limited || !self.query.starts_with(&self.scan_query))
    }

    pub fn finish(&mut self, skipped: usize, limited: bool) {
        self.loading = false;
        self.skipped = skipped;
        self.limited |= limited;
    }

    pub fn fail(&mut self, message: String) {
        self.loading = false;
        self.error = Some(message);
    }

    pub fn selected_match(&self) -> Option<&FuzzyMatch> {
        self.matches
            .get(self.selected.min(self.matches.len().saturating_sub(1)))
    }

    pub fn selected_entry(&self) -> Option<EntryView<'_>> {
        self.selected_match()
            .and_then(|found| self.view(found.entry))
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.selected_entry().map(|entry| entry.path)
    }

    pub fn selected_target(&self) -> Option<PickerTarget> {
        let found = self.selected_match()?;
        let entry = self.view(found.entry)?;
        Some(PickerTarget {
            path: entry.path.to_path_buf(),
            row: entry.row,
            column: entry.column
                + entry
                    .row
                    .and_then(|_| found.positions.first().copied())
                    .unwrap_or(0),
        })
    }

    pub fn insert_query(&mut self, character: char) {
        let byte = char_to_byte(&self.query, self.query_cursor);
        let appended = byte == self.query.len();
        self.query.insert(byte, character);
        self.query_cursor += 1;
        self.rank(true, appended);
    }

    pub fn insert_query_text(&mut self, text: &str) {
        let byte = char_to_byte(&self.query, self.query_cursor);
        let appended = byte == self.query.len();
        self.query.insert_str(byte, text);
        self.query_cursor += text.chars().count();
        self.rank(true, appended);
    }

    pub fn backspace_query(&mut self) {
        if self.query_cursor == 0 {
            return;
        }
        let from = char_to_byte(&self.query, self.query_cursor - 1);
        let to = char_to_byte(&self.query, self.query_cursor);
        self.query.replace_range(from..to, "");
        self.query_cursor -= 1;
        self.rank(true, false);
    }

    pub fn delete_query(&mut self) {
        if self.query_cursor >= self.query.chars().count() {
            return;
        }
        let from = char_to_byte(&self.query, self.query_cursor);
        let to = char_to_byte(&self.query, self.query_cursor + 1);
        self.query.replace_range(from..to, "");
        self.rank(true, false);
    }

    pub fn delete_query_word(&mut self) {
        if self.query_cursor == 0 {
            return;
        }
        let chars = self.query.chars().collect::<Vec<_>>();
        let mut from = self.query_cursor;
        while from > 0 && chars[from - 1].is_whitespace() {
            from -= 1;
        }
        while from > 0 && !chars[from - 1].is_whitespace() {
            from -= 1;
        }
        let start = char_to_byte(&self.query, from);
        let end = char_to_byte(&self.query, self.query_cursor);
        self.query.replace_range(start..end, "");
        self.query_cursor = from;
        self.rank(true, false);
    }

    pub fn delete_query_start(&mut self) {
        let end = char_to_byte(&self.query, self.query_cursor);
        self.query.replace_range(..end, "");
        self.query_cursor = 0;
        self.rank(true, false);
    }

    pub fn delete_query_end(&mut self) {
        if self.query_cursor >= self.query.chars().count() {
            return;
        }
        let start = char_to_byte(&self.query, self.query_cursor);
        self.query.truncate(start);
        self.rank(true, false);
    }

    pub fn query_left(&mut self) {
        self.query_cursor = self.query_cursor.saturating_sub(1);
    }

    pub fn query_right(&mut self) {
        self.query_cursor = (self.query_cursor + 1).min(self.query.chars().count());
    }

    pub fn down(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
            self.selection_user_owned = true;
        }
    }

    pub fn up(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.matches.len() - 1) % self.matches.len();
            self.selection_user_owned = true;
        }
    }

    pub fn page_down(&mut self, amount: usize) {
        self.selected = (self.selected + amount.max(1)).min(self.matches.len().saturating_sub(1));
        self.selection_user_owned = !self.matches.is_empty();
    }

    pub fn page_up(&mut self, amount: usize) {
        self.selected = self.selected.saturating_sub(amount.max(1));
        self.selection_user_owned = !self.matches.is_empty();
    }

    pub fn first(&mut self) {
        self.selected = 0;
        self.selection_user_owned = !self.matches.is_empty();
    }

    pub fn last(&mut self) {
        self.selected = self.matches.len().saturating_sub(1);
        self.selection_user_owned = !self.matches.is_empty();
    }

    /// Re-ranks, optionally reusing the previous result set.
    ///
    /// `narrow_existing` claims that the new query cannot match anything the
    /// old one did not, which is what makes it safe to look only at what is
    /// already in `matches`. Only growing the query at its end earns that
    /// claim. Typing into the middle does not: it rewrites a term rather than
    /// extending it, and a term matched literally can widen when it changes —
    /// `ab cd` becoming `a b cd` starts matching `a_x_b_cd`, and inserting
    /// into the term without splitting it does the same, so this is not about
    /// whitespace but about where the caret was.
    fn rank(&mut self, reset_selection: bool, narrow_existing: bool) {
        let (directory_only, query) = self.directory_only_query();
        // Narrowing from `matches` only sees candidates the last full pass
        // kept. If that pass excluded plain files (directory-only mode) and
        // this one no longer does, those files must be recovered from every
        // entry, not just the directories `matches` still holds.
        let narrow_existing = narrow_existing && directory_only == self.directory_only;
        let candidates = if narrow_existing {
            self.matches
                .iter()
                .map(|found| found.entry)
                .collect::<Vec<_>>()
        } else {
            (0..self.entries.len()).collect()
        };
        self.matches = rank_entries(
            &self.files,
            &self.entries,
            &candidates,
            &query,
            directory_only,
            self.candidate_kind(),
        );
        self.sort_matches();
        self.directory_only = directory_only;
        if reset_selection {
            self.selected = 0;
            self.selection_user_owned = false;
        } else {
            self.selected = self.selected.min(self.matches.len().saturating_sub(1));
        }
    }

    fn sort_matches(&mut self) {
        let (files, entries) = (&self.files, &self.entries);
        let view = |found: &FuzzyMatch| resolve_in(files, &entries[found.entry]);
        let query_is_empty = self.directory_only_query().1.is_empty();
        if query_is_empty {
            self.matches.sort_by(|left, right| {
                let (left, right) = (view(left), view(right));
                (left.relative, left.row).cmp(&(right.relative, right.row))
            });
        } else {
            self.matches.sort_by(|left, right| {
                right
                    .score
                    .cmp(&left.score)
                    .then_with(|| {
                        entries[left.entry]
                            .candidate_characters
                            .cmp(&entries[right.entry].candidate_characters)
                    })
                    .then_with(|| {
                        let (left, right) = (view(left), view(right));
                        (left.relative, left.row).cmp(&(right.relative, right.row))
                    })
            });
        }
    }
}

/// Scores `indices` into `entries` against `query`, keeping candidate order.
///
/// A keystroke in a picker is this pass, and it is what a candidate budget is
/// really a budget on: scoring is `O(candidates × query × line)` and every
/// candidate is independent of every other. Above `PARALLEL_RANK_CHUNK` the
/// work is divided across the machine's cores, which is where the headroom to
/// hold more candidates comes from.
///
/// Dividing it cannot change the outcome. Chunks are joined in candidate
/// order, so the sequence handed to the sort is the one a single thread would
/// have produced, and the sort's tiebreaks reach a unique path and row and so
/// order it totally either way.
/// An entry read against the file table it indexes into.
///
/// Free rather than a method so callers can hold the table and the entries as
/// separate borrows, which is what lets `sort_matches` sort one field of the
/// picker while reading two others.
fn resolve_in<'a>(files: &'a [PickerFile], entry: &'a FileEntry) -> EntryView<'a> {
    let file = &files[entry.file as usize];
    EntryView {
        path: &file.path,
        relative: &file.relative,
        row: entry.row,
        column: entry.column,
        text: entry.text.as_deref(),
        is_dir: file.is_dir,
    }
}

fn rank_entries(
    files: &[PickerFile],
    entries: &[FileEntry],
    indices: &[usize],
    query: &str,
    directory_only: bool,
    kind: FuzzyCandidate,
) -> Vec<FuzzyMatch> {
    let rank_chunk = |chunk: &[usize]| {
        let mut matcher = FuzzyMatcher::for_candidate(query, kind);
        let mut matches = Vec::new();
        for entry in chunk.iter().copied() {
            let candidate = resolve_in(files, &entries[entry]);
            if directory_only && !candidate.is_dir {
                continue;
            }
            if let Some((score, positions)) = matcher.score(candidate.candidate()) {
                matches.push(FuzzyMatch {
                    entry,
                    score,
                    positions,
                });
            }
        }
        matches
    };
    let chunks = indices.len().div_ceil(PARALLEL_RANK_CHUNK);
    if chunks <= 1 {
        return rank_chunk(indices);
    }
    static CORES: OnceLock<usize> = OnceLock::new();
    let cores = *CORES.get_or_init(|| thread::available_parallelism().map_or(1, NonZero::get));
    let threads = cores.min(chunks);
    if threads <= 1 {
        return rank_chunk(indices);
    }
    thread::scope(|scope| {
        indices
            .chunks(indices.len().div_ceil(threads))
            .map(|chunk| scope.spawn(|| rank_chunk(chunk)))
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|worker| worker.join().expect("a ranking chunk cannot panic"))
            .collect()
    })
}

fn char_to_byte(text: &str, character: usize) -> usize {
    text.char_indices()
        .nth(character)
        .map_or(text.len(), |(byte, _)| byte)
}

/// Whether `query` occurs in `candidate` as an ordered subsequence, under the
/// same smart-case rule the scorer uses.
///
/// This decides exactly what `fuzzy_match` decides — the scorer reaches a
/// final state precisely when such a subsequence exists — but in one linear
/// pass that allocates nothing. That makes it the filter the content scanner
/// can afford to run over every line of a project, and the guard `fuzzy_match`
/// takes before building a dynamic-programming table it could never fill.
pub fn matches_fuzzy(query: &str, candidate: &str) -> bool {
    let case_sensitive = query.chars().any(char::is_uppercase);
    let mut terms = query.split_whitespace();
    let Some(first) = terms.next() else {
        return true;
    };
    if terms.next().is_none() {
        return subsequence(first, candidate, case_sensitive);
    }
    // Two or more terms are a different question: each has to be there as
    // itself. Nothing is allocated to ask it — `split_whitespace` borrows, and
    // the walk is one pass per term over what is left of the candidate.
    let mut rest = candidate;
    for term in query.split_whitespace() {
        let Some(end) = find_term(rest, term.chars(), case_sensitive) else {
            return false;
        };
        rest = &rest[end..];
    }
    true
}

/// Whether `query` occurs in `candidate` as an ordered subsequence.
fn subsequence(query: &str, candidate: &str, case_sensitive: bool) -> bool {
    let mut wanted = query.chars().peekable();
    for character in candidate.chars() {
        let Some(next) = wanted.peek().copied() else {
            return true;
        };
        if characters_match(character, next, case_sensitive) {
            wanted.next();
        }
    }
    wanted.peek().is_none()
}

/// The byte index just past the first literal occurrence of `term`.
///
/// Generic over how the term is spelled so the two callers can each use what
/// they already hold — `matches_fuzzy` the borrowed `&str` slices
/// `split_whitespace` hands it, and `FuzzyMatcher` its prepared `[char]` terms
/// — without either allocating to ask the question.
fn find_term<T>(candidate: &str, term: T, case_sensitive: bool) -> Option<usize>
where
    T: IntoIterator<Item = char> + Clone,
{
    if term.clone().into_iter().next().is_none() {
        return Some(0);
    }
    for (start, _) in candidate.char_indices() {
        let mut have = candidate[start..].chars();
        let mut end = start;
        let found = term.clone().into_iter().all(|wanted| match have.next() {
            Some(character) if characters_match(character, wanted, case_sensitive) => {
                end += character.len_utf8();
                true
            }
            _ => false,
        });
        if found {
            return Some(end);
        }
    }
    None
}

/// Whether `term` occurs whole at `start`.
fn term_at(candidate: &[char], start: usize, term: &[char], case_sensitive: bool) -> bool {
    start + term.len() <= candidate.len()
        && term.iter().enumerate().all(|(offset, wanted)| {
            characters_match(candidate[start + offset], *wanted, case_sensitive)
        })
}

/// The first character index at or after `from` where `term` occurs whole.
fn term_start(
    candidate: &[char],
    from: usize,
    term: &[char],
    case_sensitive: bool,
) -> Option<usize> {
    (from..=candidate.len().checked_sub(term.len())?)
        .find(|start| term_at(candidate, *start, term, case_sensitive))
}

/// The last character index at which `term` occurs whole and ends by `limit`.
fn last_term_start(
    candidate: &[char],
    limit: usize,
    term: &[char],
    case_sensitive: bool,
) -> Option<usize> {
    (0..=limit.checked_sub(term.len())?)
        .rev()
        .find(|start| term_at(candidate, *start, term, case_sensitive))
}

/// One character compared under the smart-case rule.
///
/// The ASCII branch carries the cost of content search rather than shaving it:
/// this runs once per character of every line in a project, and
/// `char::to_lowercase` builds an iterator per call. It agrees with the
/// general branch by construction, because an ASCII character's lowercase is
/// its ASCII lowercase; anything outside ASCII, where a single character can
/// fold to several, still takes the full comparison.
fn characters_match(candidate: char, wanted: char, case_sensitive: bool) -> bool {
    if case_sensitive {
        candidate == wanted
    } else if candidate.is_ascii() && wanted.is_ascii() {
        candidate.eq_ignore_ascii_case(&wanted)
    } else {
        candidate.to_lowercase().eq(wanted.to_lowercase())
    }
}

/// What a candidate is, which decides whether the path heuristics apply.
///
/// Two of the scoring rules only mean something for a path: the characters
/// after the last `/` are the basename and are worth more, and a candidate is
/// penalized per separator so a shallow path outranks a deep one. Applied to a
/// line of text they misfire badly — a commit line ending in
/// `(origin/main, origin/HEAD)` had its basename start put past the match, so
/// an exact match earlier in it scored 220 where the same match in its sibling
/// lines scored 407, dropping it from the top of the list to rank 717.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FuzzyCandidate {
    /// A path. The basename outweighs the directories that lead to it.
    Path,
    /// A line of text, in which `/` is an ordinary character.
    Line,
}

/// A query prepared once and scored against many candidates.
///
/// Ranking is where a keystroke in a picker is spent: one scoring pass over
/// the whole candidate list. Scoring used to allocate a dynamic-programming
/// table, two score rows, a prefix row and a character vector per candidate —
/// about `3 × query + 9` allocations each — which at a full candidate budget
/// was most of what the pass cost, and is what held the budget down. Here
/// everything derived from the query is computed once, and every buffer whose
/// size depends only on the query and the candidate in hand is reused across
/// candidates, leaving one allocation per scored candidate: the positions the
/// caller keeps.
pub struct FuzzyMatcher {
    kind: FuzzyCandidate,
    /// The whitespace-separated terms of the query.
    ///
    /// One term is the query itself and is matched as a fuzzy subsequence
    /// through `query` below, which is what a single word has always meant.
    /// Two or more are matched as themselves, in order, because that is what
    /// someone typing three words means by them.
    terms: Vec<Vec<char>>,
    query: Vec<char>,
    /// The query under the same case folding `comparable` applied, held for
    /// the whole-basename and basename-prefix bonuses.
    comparable_query: String,
    case_sensitive: bool,
    candidate: Vec<char>,
    lowered: String,
    previous: Vec<i64>,
    current: Vec<i64>,
    prefix: Vec<Option<(i64, usize)>>,
    /// `query × candidate` predecessors, row-major, grown to the widest
    /// candidate seen so far and never shrunk.
    parents: Vec<Option<usize>>,
}

/// The score of a state no alignment can reach. Divided down from the floor so
/// that adding a penalty to it cannot wrap.
const IMPOSSIBLE: i64 = i64::MIN / 4;

impl FuzzyMatcher {
    /// A matcher for paths.
    pub fn new(query: &str) -> Self {
        Self::for_candidate(query, FuzzyCandidate::Path)
    }

    /// A matcher for lines of text, which leaves the path heuristics out.
    pub fn for_lines(query: &str) -> Self {
        Self::for_candidate(query, FuzzyCandidate::Line)
    }

    pub fn for_candidate(query: &str, kind: FuzzyCandidate) -> Self {
        let case_sensitive = query.chars().any(char::is_uppercase);
        let terms = query
            .split_whitespace()
            .map(|term| term.chars().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        // Whitespace separates rather than matches, so a lone term is the
        // query: `abc ` asks exactly what `abc` asks, and the subsequence
        // scored below is the term, not the spaces around it.
        let query = match terms.as_slice() {
            [only] => only.clone(),
            _ => query.trim().chars().collect(),
        };
        let comparable_query = query.iter().collect::<String>();
        Self {
            kind,
            terms,
            comparable_query: if case_sensitive {
                comparable_query
            } else {
                comparable_query.to_lowercase()
            },
            query,
            case_sensitive,
            candidate: Vec::new(),
            lowered: String::new(),
            previous: Vec::new(),
            current: Vec::new(),
            prefix: Vec::new(),
            parents: Vec::new(),
        }
    }

    /// Whether `candidate` answers the query.
    ///
    /// Decides exactly what `matches_fuzzy` decides for the same query, and
    /// exactly what `score` will accept, so the scanner's filter, the picker's
    /// narrowing, and the ranker never disagree about what a match is.
    pub fn matches(&self, candidate: &str) -> bool {
        if self.terms.len() < 2 {
            let mut wanted = self.query.iter().copied().peekable();
            for character in candidate.chars() {
                let Some(next) = wanted.peek().copied() else {
                    return true;
                };
                if characters_match(character, next, self.case_sensitive) {
                    wanted.next();
                }
            }
            return wanted.peek().is_none();
        }
        let mut rest = candidate;
        for term in &self.terms {
            let Some(end) = find_term(rest, term.iter().copied(), self.case_sensitive) else {
                return false;
            };
            rest = &rest[end..];
        }
        true
    }

    /// Scores an ordered subsequence match and returns the candidate character
    /// indices that should be highlighted.
    pub fn score(&mut self, candidate: &str) -> Option<(i64, Vec<usize>)> {
        if self.query.is_empty() {
            return Some((0, Vec::new()));
        }
        if !self.matches(candidate) {
            return None;
        }

        // A line has no basename: the whole of it is compared to the query,
        // and every position is inside it, so the bonus becomes a constant
        // that cannot reorder one candidate against another.
        let (basename_start, basename) = match candidate.rfind('/') {
            Some(byte) if self.kind == FuzzyCandidate::Path => {
                let start = byte + '/'.len_utf8();
                (candidate[..start].chars().count(), &candidate[start..])
            }
            _ => (0, candidate),
        };
        // ASCII lowercasing is Unicode lowercasing for ASCII, and a line of
        // source is nearly always ASCII, so the common candidate folds into a
        // reused buffer rather than a fresh String.
        let owned;
        let comparable_basename = if self.case_sensitive {
            basename
        } else if basename.is_ascii() {
            self.lowered.clear();
            self.lowered.extend(
                basename
                    .chars()
                    .map(|character| character.to_ascii_lowercase()),
            );
            self.lowered.as_str()
        } else {
            owned = basename.to_lowercase();
            owned.as_str()
        };
        let base_score = if comparable_basename == self.comparable_query {
            10_000
        } else if comparable_basename.starts_with(&self.comparable_query) {
            5_000
        } else {
            0
        };

        let Self {
            terms,
            query,
            case_sensitive,
            candidate: candidate_chars,
            previous,
            current,
            prefix,
            parents,
            ..
        } = self;
        let case_sensitive = *case_sensitive;
        candidate_chars.clear();
        candidate_chars.extend(candidate.chars());
        let width = candidate_chars.len();

        let character_score = |wanted: char, position: usize| {
            let mut score = 10;
            if position >= basename_start {
                score += 30;
            }
            if position == 0
                || matches!(candidate_chars[position - 1], '/' | '_' | '-' | '.' | ' ')
                || candidate_chars[position].is_uppercase()
                    && candidate_chars[position - 1].is_lowercase()
            {
                score += 24;
            }
            if candidate_chars[position] == wanted {
                score += 2;
            }
            score
        };
        // Terms are contiguous by construction, so with two or more of them
        // there is no alignment to search: the only choice is which occurrence
        // of each term to take. `latest` says how far right each term may sit
        // and still leave room for the ones after it, so any occurrence up to
        // it can be preferred on score alone without stranding a later term.
        let (alignment_score, positions) = if terms.len() > 1 {
            let mut latest = vec![0; terms.len()];
            let mut limit = width;
            for (index, term) in terms.iter().enumerate().rev() {
                let start = last_term_start(candidate_chars, limit, term, case_sensitive)
                    .expect("the membership test found every term");
                latest[index] = start;
                limit = start;
            }
            let mut positions = Vec::with_capacity(terms.iter().map(Vec::len).sum());
            let mut alignment_score = 0;
            let mut cursor = 0;
            for (index, term) in terms.iter().enumerate() {
                let mut best: Option<(i64, usize)> = None;
                let mut from = cursor;
                while let Some(start) = term_start(candidate_chars, from, term, case_sensitive) {
                    if start > latest[index] {
                        break;
                    }
                    // The same per-character rules the alignment uses, plus
                    // the adjacency it would have paid for a run this long, so
                    // one term and one word score on the same scale.
                    let score = term
                        .iter()
                        .enumerate()
                        .map(|(offset, wanted)| character_score(*wanted, start + offset))
                        .sum::<i64>()
                        + 28 * (term.len() as i64 - 1);
                    if best.is_none_or(|(best, _)| score > best) {
                        best = Some((score, start));
                    }
                    from = start + 1;
                }
                let (score, start) = best.expect("a term is reachable up to its latest start");
                alignment_score += score;
                positions.extend(start..start + term.len());
                cursor = start + term.len();
            }
            (alignment_score, positions)
        } else {
            score_one_term(
                query,
                candidate_chars,
                case_sensitive,
                width,
                previous,
                current,
                prefix,
                parents,
                character_score,
            )?
        };

        let mut score = base_score + alignment_score;
        if self.kind == FuzzyCandidate::Path {
            score -= candidate_chars
                .iter()
                .filter(|character| **character == '/')
                .count() as i64
                * 3;
        }
        score -= width.saturating_sub(query.len()).min(256) as i64 / 8;
        Some((score, positions))
    }
}

/// The single-word alignment: the query as one fuzzy ordered subsequence.
///
/// Dynamic programming chooses the globally best alignment instead of greedily
/// committing each character. Gap penalties saturate after 32 characters, so
/// each state needs at most 31 nearby predecessors plus a prefix maximum for
/// every older one: O(query × candidate × 32), with the multiplier bounded
/// independently of candidate length.
#[allow(clippy::too_many_arguments)]
fn score_one_term(
    query: &[char],
    candidate_chars: &[char],
    case_sensitive: bool,
    width: usize,
    previous: &mut Vec<i64>,
    current: &mut Vec<i64>,
    prefix: &mut Vec<Option<(i64, usize)>>,
    parents: &mut Vec<Option<usize>>,
    character_score: impl Fn(char, usize) -> i64,
) -> Option<(i64, Vec<usize>)> {
    if query.len() > width {
        return None;
    }
    parents.clear();
    parents.resize(query.len() * width, None);
    previous.clear();
    previous.resize(width, IMPOSSIBLE);
    for (position, character) in candidate_chars.iter().copied().enumerate() {
        if characters_match(character, query[0], case_sensitive) {
            previous[position] = character_score(query[0], position);
        }
    }
    for query_index in 1..query.len() {
        prefix.clear();
        prefix.reserve(width);
        let mut best_prefix: Option<(i64, usize)> = None;
        for (position, score) in previous.iter().copied().enumerate() {
            if score != IMPOSSIBLE
                && best_prefix.is_none_or(|(best, best_position)| {
                    score > best || score == best && position < best_position
                })
            {
                best_prefix = Some((score, position));
            }
            prefix.push(best_prefix);
        }

        current.clear();
        current.resize(width, IMPOSSIBLE);
        for (position, character) in candidate_chars.iter().copied().enumerate() {
            if !characters_match(character, query[query_index], case_sensitive) {
                continue;
            }
            let mut transition: Option<(i64, usize)> = None;
            let mut consider = |score: i64, parent: usize| {
                if score != IMPOSSIBLE
                    && transition.is_none_or(|(best, best_parent)| {
                        score > best || score == best && parent < best_parent
                    })
                {
                    transition = Some((score, parent));
                }
            };
            if position > 0 && previous[position - 1] != IMPOSSIBLE {
                consider(previous[position - 1] + 28, position - 1);
            }
            if position >= 2 {
                for (parent, previous_score) in previous
                    .iter()
                    .copied()
                    .enumerate()
                    .take(position - 1)
                    .skip(position.saturating_sub(32))
                {
                    let gap = position - parent - 1;
                    if previous_score != IMPOSSIBLE {
                        consider(previous_score - gap as i64, parent);
                    }
                }
            }
            if position >= 33
                && let Some((score, parent)) = prefix[position - 33]
            {
                consider(score - 32, parent);
            }
            if let Some((score, parent)) = transition {
                current[position] = score + character_score(query[query_index], position);
                parents[query_index * width + position] = Some(parent);
            }
        }
        std::mem::swap(previous, current);
    }

    let (alignment_score, mut position) = previous
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, score)| *score != IMPOSSIBLE)
        .map(|(position, score)| (score, position))
        .max_by(
            |(left_score, left_position), (right_score, right_position)| {
                left_score
                    .cmp(right_score)
                    .then_with(|| right_position.cmp(left_position))
            },
        )?;
    let mut positions = vec![0; query.len()];
    positions[query.len() - 1] = position;
    for query_index in (1..query.len()).rev() {
        position = parents[query_index * width + position]
            .expect("a reachable fuzzy state has a predecessor");
        positions[query_index - 1] = position;
    }
    Some((alignment_score, positions))
}

/// Scores one candidate against one query.
///
/// The convenience over `FuzzyMatcher` for callers with a single candidate to
/// rank. Anything ranking a list should build one matcher and reuse it.
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<(i64, Vec<usize>)> {
    FuzzyMatcher::new(query).score(candidate)
}

#[derive(Clone, Debug)]
pub enum FilePickerEvent {
    Files {
        scan_id: u64,
        paths: Vec<ScanEntry>,
    },
    Content {
        scan_id: u64,
        entries: Vec<FileHits>,
    },
    Finished {
        scan_id: u64,
        skipped: usize,
        limited: bool,
    },
    Failed {
        scan_id: u64,
        message: String,
    },
}

/// One matching line, named only by where it sits in its file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineHit {
    pub row: usize,
    pub column: usize,
    pub text: String,
}

/// Every line of one file that the query matched.
///
/// Grouping at the boundary is what lets the picker hold the path once: the
/// scanner already knows which file it is reading, so nothing downstream has
/// to rediscover it per line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileHits {
    pub path: PathBuf,
    pub lines: Vec<LineHit>,
}

impl FileHits {
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Keeps at most `limit` of the lines, reporting whether any were dropped.
    pub(crate) fn truncate(&mut self, limit: usize) -> bool {
        let dropped = self.lines.len() > limit;
        self.lines.truncate(limit);
        dropped
    }
}

#[derive(Clone, Debug)]
pub struct FileScanner {
    active: Arc<AtomicU64>,
    events: Sender<FilePickerEvent>,
}

impl FileScanner {
    pub fn scan(
        &self,
        scan_id: u64,
        root: PathBuf,
        ignore_root: PathBuf,
        state_root: PathBuf,
        show_hidden: bool,
    ) {
        self.active.store(scan_id, Ordering::Release);
        let active = self.active.clone();
        let events = self.events.clone();
        let failure_events = self.events.clone();
        if let Err(error) = thread::Builder::new()
            .name("runyte-file-scan".to_owned())
            .spawn(move || {
                let result = scan_with(
                    &root,
                    &ignore_root,
                    &state_root,
                    show_hidden,
                    true,
                    || active.load(Ordering::Acquire) != scan_id,
                    |paths| {
                        events
                            .blocking_send(FilePickerEvent::Files { scan_id, paths })
                            .is_ok()
                    },
                );
                if active.load(Ordering::Acquire) != scan_id {
                    return;
                }
                match result {
                    Ok(skipped) => {
                        let _ = events.blocking_send(FilePickerEvent::Finished {
                            scan_id,
                            skipped,
                            limited: false,
                        });
                    }
                    Err(error) => {
                        let _ = events.blocking_send(FilePickerEvent::Failed {
                            scan_id,
                            message: error.to_string(),
                        });
                    }
                }
            })
        {
            let _ = failure_events.try_send(FilePickerEvent::Failed {
                scan_id,
                message: format!("failed to start file scanner: {error}"),
            });
        }
    }

    /// Starts a content scan that keeps only the lines `query` matches.
    ///
    /// Each edit to the query is a new scan under a new id; the old one is
    /// cancelled by the store below, so its late batches are dropped by the
    /// picker's id guard rather than mixed into the new query's results.
    pub fn scan_content(
        &self,
        scan_id: u64,
        root: PathBuf,
        ignore_root: PathBuf,
        state_root: PathBuf,
        show_hidden: bool,
        query: String,
    ) {
        self.active.store(scan_id, Ordering::Release);
        let active = self.active.clone();
        let events = self.events.clone();
        let failure_events = self.events.clone();
        if let Err(error) = thread::Builder::new()
            .name("runyte-content-scan".to_owned())
            .spawn(move || {
                let mut settled = Duration::ZERO;
                while settled < CONTENT_SCAN_SETTLE {
                    if active.load(Ordering::Acquire) != scan_id {
                        return;
                    }
                    thread::sleep(CONTENT_SCAN_SETTLE_STEP);
                    settled += CONTENT_SCAN_SETTLE_STEP;
                }
                let mut emitted = 0;
                let mut limited = false;
                let result = scan_with(
                    &root,
                    &ignore_root,
                    &state_root,
                    show_hidden,
                    false,
                    || active.load(Ordering::Acquire) != scan_id,
                    |paths| {
                        let mut entries = Vec::<FileHits>::new();
                        let mut batch = 0;
                        for path in paths {
                            if active.load(Ordering::Acquire) != scan_id {
                                return false;
                            }
                            let Some(mut hits) = content_entries(&path.path, &query) else {
                                continue;
                            };
                            if hits.truncate(CONTENT_ENTRY_LIMIT - emitted - batch) {
                                limited = true;
                            }
                            batch += hits.len();
                            if !hits.is_empty() {
                                entries.push(hits);
                            }
                            if limited {
                                break;
                            }
                        }
                        emitted += batch;
                        (entries.is_empty()
                            || events
                                .blocking_send(FilePickerEvent::Content { scan_id, entries })
                                .is_ok())
                            && !limited
                    },
                );
                if active.load(Ordering::Acquire) != scan_id {
                    return;
                }
                match result {
                    Ok(skipped) => {
                        let _ = events.blocking_send(FilePickerEvent::Finished {
                            scan_id,
                            skipped,
                            limited,
                        });
                    }
                    Err(error) => {
                        let _ = events.blocking_send(FilePickerEvent::Failed {
                            scan_id,
                            message: error.to_string(),
                        });
                    }
                }
            })
        {
            let _ = failure_events.try_send(FilePickerEvent::Failed {
                scan_id,
                message: format!("failed to start content scanner: {error}"),
            });
        }
    }

    pub fn cancel(&self, scan_id: u64) {
        let _ = self
            .active
            .compare_exchange(scan_id, 0, Ordering::AcqRel, Ordering::Acquire);
    }
}

pub fn scanner() -> (FileScanner, Receiver<FilePickerEvent>) {
    let (events, receiver) = channel(16);
    (
        FileScanner {
            active: Arc::new(AtomicU64::new(0)),
            events,
        },
        receiver,
    )
}

/// Synchronous seam used by isolated tests and by non-TUI embedders.
pub fn scan_files(
    root: &Path,
    ignore_root: &Path,
    state_root: &Path,
    show_hidden: bool,
) -> Result<(Vec<ScanEntry>, usize)> {
    let mut paths = Vec::new();
    let skipped = scan_with(
        root,
        ignore_root,
        state_root,
        show_hidden,
        true,
        || false,
        |batch| {
            paths.extend(batch);
            true
        },
    )?;
    Ok((paths, skipped))
}

/// Synchronous content scan used by isolated tests and non-TUI embedders.
///
/// Only the lines `query` matches are collected, so the returned `limited`
/// flag means the project holds more than `CONTENT_ENTRY_LIMIT` matches for
/// that query rather than more than that many lines.
pub fn scan_content(
    root: &Path,
    ignore_root: &Path,
    state_root: &Path,
    show_hidden: bool,
    query: &str,
) -> Result<(Vec<FileHits>, usize, bool)> {
    let mut files = Vec::new();
    let mut lines = 0;
    let mut limited = false;
    let skipped = scan_with(
        root,
        ignore_root,
        state_root,
        show_hidden,
        false,
        || false,
        |paths| {
            for path in paths {
                let Some(mut hits) = content_entries(&path.path, query) else {
                    continue;
                };
                if hits.truncate(CONTENT_ENTRY_LIMIT - lines) {
                    limited = true;
                }
                lines += hits.len();
                if !hits.is_empty() {
                    files.push(hits);
                }
                if limited {
                    return false;
                }
            }
            true
        },
    )?;
    Ok((files, skipped, limited))
}

fn content_entries(path: &Path, query: &str) -> Option<FileHits> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > GREP_FILE_BYTES {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let lines = line_hits(&text, query);
    (!lines.is_empty()).then(|| FileHits {
        path: path.to_path_buf(),
        lines,
    })
}

/// Bounded matching lines from authoritative live text, holding only the lines
/// `query` can match.
///
/// Filtering here rather than in the picker is what makes the candidate
/// ceiling a ceiling on results instead of on how much of a project was read:
/// a line the query cannot match never becomes an entry, so the budget is
/// spent on matches wherever in the project they live.
pub fn line_hits(text: &str, query: &str) -> Vec<LineHit> {
    text.lines()
        .enumerate()
        .filter_map(|(row, line)| line_hit(line, query).map(|hit| LineHit { row, ..hit }))
        .take(CONTENT_ENTRY_LIMIT)
        .collect()
}

/// Matches one decoded row for the incremental live-resource scanner.
///
/// `row` is left at zero for the caller to replace with the source coordinate.
/// Keeping this transform shared prevents file, buffer, and terminal content
/// from disagreeing about trimming, truncation, or fuzzy matching.
pub fn line_hit(line: &str, query: &str) -> Option<LineHit> {
    let without_trailing = line.trim_end();
    let trimmed = without_trailing.trim_start();
    // The ranked candidate is the truncated line, so the filter has to read
    // the same text: a query matched against the tail of a very long line
    // would produce an entry nothing later can highlight.
    let text = match trimmed.char_indices().nth(GREP_LINE_CHARACTERS) {
        Some((byte, _)) => &trimmed[..byte],
        None => trimmed,
    };
    (!text.is_empty() && matches_fuzzy(query, text)).then(|| LineHit {
        row: 0,
        column: without_trailing
            .chars()
            .take_while(|character| character.is_whitespace())
            .count(),
        text: text.to_owned(),
    })
}

fn scan_with(
    root: &Path,
    ignore_root: &Path,
    state_root: &Path,
    show_hidden: bool,
    include_dirs: bool,
    mut cancelled: impl FnMut() -> bool,
    mut emit: impl FnMut(Vec<ScanEntry>) -> bool,
) -> Result<usize> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve picker root {}", root.display()))?;
    anyhow::ensure!(
        root.is_dir(),
        "picker root {} is not a directory",
        root.display()
    );
    let state_root = state_root
        .canonicalize()
        .unwrap_or_else(|_| state_root.to_path_buf());
    anyhow::ensure!(
        !root.starts_with(&state_root) && !contains_reserved_component(&root),
        "picker root {} is inside reserved Runyte or Git state",
        root.display()
    );
    let ignore_root = ignore_root.canonicalize().unwrap_or_else(|_| root.clone());
    let ignore_root = if root.starts_with(&ignore_root) {
        ignore_root
    } else {
        root.clone()
    };
    let root_relative = root
        .strip_prefix(&ignore_root)
        .expect("the effective ignore root contains the picker root")
        .to_path_buf();
    let mut skipped = 0;
    let mut inherited = Vec::<IgnoreRule>::new();
    if !root_relative.as_os_str().is_empty() {
        let mut ancestor = ignore_root.clone();
        let mut ancestor_relative = PathBuf::new();
        for component in root_relative.components() {
            if cancelled() {
                return Ok(skipped);
            }
            read_ignore_files(&ancestor, &ancestor_relative, &mut inherited, &mut skipped);
            let Component::Normal(component) = component else {
                continue;
            };
            let next_relative = ancestor_relative.join(component);
            if ignored(&inherited, &next_relative, true) {
                return Ok(skipped);
            }
            ancestor.push(component);
            ancestor_relative = next_relative;
        }
    }
    let mut pending = vec![(root.clone(), root_relative, inherited)];
    let mut batch = Vec::with_capacity(SCAN_BATCH);
    while let Some((directory, relative_directory, mut rules)) = pending.pop() {
        if cancelled() {
            return Ok(skipped);
        }
        read_ignore_files(&directory, &relative_directory, &mut rules, &mut skipped);
        if cancelled() {
            return Ok(skipped);
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if directory == root => {
                return Err(error).with_context(|| format!("failed to read {}", root.display()));
            }
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let mut readable_entries = Vec::new();
        for entry in entries {
            if cancelled() {
                return Ok(skipped);
            }
            match entry {
                Ok(entry) => readable_entries.push(entry),
                Err(_) => skipped += 1,
            }
        }
        readable_entries.sort_by_key(fs::DirEntry::file_name);
        for entry in readable_entries.into_iter().rev() {
            if cancelled() {
                return Ok(skipped);
            }
            let name = entry.file_name();
            let name_text = name.to_string_lossy();
            let path = entry.path();
            let relative = relative_directory.join(&name);
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            if file_type.is_symlink() {
                continue;
            }
            let is_directory = file_type.is_dir();
            if name_text == ".git"
                || name_text == ".runyte"
                || path == state_root
                || (!show_hidden && name_text.starts_with('.'))
                || ignored(&rules, &relative, is_directory)
            {
                continue;
            }
            if is_directory {
                if include_dirs {
                    batch.push(ScanEntry::directory(path.clone()));
                    if batch.len() == SCAN_BATCH && !emit(std::mem::take(&mut batch)) {
                        return Ok(skipped);
                    }
                }
                pending.push((path, relative, rules.clone()));
            } else if file_type.is_file() {
                batch.push(ScanEntry::file(path));
                if batch.len() == SCAN_BATCH && !emit(std::mem::take(&mut batch)) {
                    return Ok(skipped);
                }
            }
        }
    }
    if !batch.is_empty() && !emit(batch) {
        return Ok(skipped);
    }
    Ok(skipped)
}

fn contains_reserved_component(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(component, Component::Normal(name) if name == ".git" || name == ".runyte")
    })
}

fn read_ignore_files(
    directory: &Path,
    relative_directory: &Path,
    rules: &mut Vec<IgnoreRule>,
    skipped: &mut usize,
) {
    for name in [".gitignore", ".ignore"] {
        let path = directory.join(name);
        match fs::read_to_string(&path) {
            Ok(contents) => rules.extend(parse_ignore(relative_directory, &contents)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => *skipped += 1,
        }
    }
}

#[derive(Clone, Debug)]
struct IgnoreRule {
    base: PathBuf,
    negated: bool,
    directory_only: bool,
    basename_only: bool,
    matcher: Regex,
}

fn parse_ignore(base: &Path, contents: &str) -> Vec<IgnoreRule> {
    contents
        .lines()
        .filter_map(|line| {
            let mut pattern = strip_unescaped_trailing_spaces(line.trim_end_matches('\r'));
            if pattern.is_empty() || pattern.starts_with('#') {
                return None;
            }
            if let Some(rest) = pattern.strip_prefix("\\#") {
                pattern = format!("#{rest}");
            }
            let negated = pattern.starts_with('!');
            if negated {
                pattern.remove(0);
            } else if let Some(rest) = pattern.strip_prefix("\\!") {
                pattern = format!("!{rest}");
            }
            let directory_only = pattern.ends_with('/');
            if directory_only {
                pattern.pop();
            }
            let anchored = pattern.starts_with('/');
            if anchored {
                pattern.remove(0);
            }
            if pattern.is_empty() {
                return None;
            }
            let basename_only = !anchored && !pattern.contains('/');
            let expression = if basename_only {
                format!("^{}$", glob_expression(&pattern))
            } else {
                format!("^{}(?:/.*)?$", glob_expression(&pattern))
            };
            Regex::new(&expression).ok().map(|matcher| IgnoreRule {
                base: base.to_path_buf(),
                negated,
                directory_only,
                basename_only,
                matcher,
            })
        })
        .collect()
}

fn strip_unescaped_trailing_spaces(line: &str) -> String {
    let mut pattern = line.to_owned();
    while pattern.ends_with(' ') {
        let preceding_backslashes = pattern[..pattern.len() - 1]
            .chars()
            .rev()
            .take_while(|character| *character == '\\')
            .count();
        if preceding_backslashes % 2 == 1 {
            break;
        }
        pattern.pop();
    }
    pattern
}

fn ignored(rules: &[IgnoreRule], relative: &Path, is_directory: bool) -> bool {
    let mut ignored = false;
    for rule in rules {
        let Ok(scoped) = relative.strip_prefix(&rule.base) else {
            continue;
        };
        let scoped = path_text(scoped);
        let target = if rule.basename_only {
            scoped.rsplit('/').next().unwrap_or(&scoped)
        } else {
            scoped.as_str()
        };
        if (!rule.directory_only || is_directory) && rule.matcher.is_match(target) {
            ignored = !rule.negated;
        }
    }
    ignored
}

fn path_text(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(component) => Some(component.to_string_lossy()),
            Component::ParentDir => Some("..".into()),
            Component::CurDir => None,
            Component::RootDir | Component::Prefix(_) => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn glob_expression(pattern: &str) -> String {
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut expression = String::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '*' if chars.get(index + 1) == Some(&'*') => {
                index += 1;
                if chars.get(index + 1) == Some(&'/') {
                    index += 1;
                    expression.push_str("(?:.*/)?");
                } else {
                    expression.push_str(".*");
                }
            }
            '*' => expression.push_str("[^/]*"),
            '?' => expression.push_str("[^/]"),
            '[' => {
                let start = index;
                while index + 1 < chars.len() && chars[index + 1] != ']' {
                    index += 1;
                }
                if index + 1 < chars.len() {
                    index += 1;
                    let mut class = chars[start + 1..index].iter().copied();
                    expression.push('[');
                    if class.clone().next() == Some('!') {
                        expression.push('^');
                        class.next();
                    }
                    for character in class {
                        if matches!(character, '\\' | ']') {
                            expression.push('\\');
                        }
                        expression.push(character);
                    }
                    expression.push(']');
                } else {
                    expression.push_str("\\[");
                }
            }
            '\\' if index + 1 < chars.len() => {
                index += 1;
                expression.push_str(&regex::escape(&chars[index].to_string()));
            }
            character => expression.push_str(&regex::escape(&character.to_string())),
        }
        index += 1;
    }
    expression
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;

    fn temporary(name: &str) -> PathBuf {
        std::env::temp_dir()
            .canonicalize()
            .unwrap_or_else(|_| std::env::temp_dir())
            .join(format!(
                "runyte-file-picker-{name}-{}-{}",
                std::process::id(),
                std::thread::current().name().unwrap_or("test")
            ))
    }

    #[test]
    fn basename_and_boundary_matches_beat_deep_incidental_matches() {
        let basename = fuzzy_match("picker", "src/picker.rs").unwrap().0;
        let incidental = fuzzy_match("picker", "src/ui/file_picker_row.rs")
            .unwrap()
            .0;
        assert!(basename > incidental);
        assert!(
            fuzzy_match("fp", "file_picker.rs").unwrap().0
                > fuzzy_match("fp", "soft_wrap.rs").unwrap().0
        );
        assert_eq!(
            fuzzy_match("picker", "p/archive/picker.rs").unwrap().1,
            vec![10, 11, 12, 13, 14, 15],
            "a stray early letter must not steal the compact basename match"
        );
    }

    #[test]
    fn matching_is_smart_case_and_reports_character_positions() {
        assert_eq!(fuzzy_match("fr", "FileReader.rs").unwrap().1, vec![0, 4]);
        assert!(fuzzy_match("FR", "FileReader.rs").is_some());
        assert!(fuzzy_match("FR", "file_reader.rs").is_none());
        assert!(fuzzy_match("界面", "src/界面.rs").is_some());
        assert_eq!(
            fuzzy_match("abc", "ab/x/bc").unwrap().1,
            vec![0, 5, 6],
            "matching must choose the globally best boundary-rich alignment"
        );

        let long_candidate = "a".repeat(1_024);
        let long_query = "a".repeat(32);
        assert_eq!(
            fuzzy_match(&long_query, &long_candidate).unwrap().1,
            (0..32).collect::<Vec<_>>()
        );
        assert!(is_direct_match(
            &fuzzy_match("reader", "FileReader.rs").unwrap().1,
            "reader"
        ));
        assert!(!is_direct_match(
            &fuzzy_match("fr", "FileReader.rs").unwrap().1,
            "fr"
        ));
        assert!(!is_direct_match(&[], ""));
    }

    #[test]
    fn streamed_results_follow_new_top_scores_until_the_user_navigates() {
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::new(1, root.clone());
        picker.insert_query_text("picker");
        picker.add_paths(vec![ScanEntry::file(root.join("archive/picker_notes.md"))]);
        picker.add_paths(vec![ScanEntry::file(root.join("picker"))]);
        assert_eq!(picker.selected_entry().unwrap().relative, "picker");

        picker.down();
        assert_eq!(
            picker.selected_entry().unwrap().relative,
            "archive/picker_notes.md"
        );
        picker.add_paths(vec![ScanEntry::file(root.join("picker.rs"))]);
        assert_eq!(
            picker.selected_entry().unwrap().relative,
            "archive/picker_notes.md",
            "an explicit selection remains stable while results stream"
        );
    }

    #[test]
    fn nested_ignore_files_negation_hidden_files_and_symlinks_are_respected() {
        let root = temporary("ignore");
        let workspace = root.join(".runyte");
        fs::create_dir_all(root.join("src/generated")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(
            root.join(".gitignore"),
            "target/\n*.log\nsrc/generated/**\nfile[!0].tmp\n",
        )
        .unwrap();
        fs::write(root.join(".ignore"), "!keep.log\n").unwrap();
        fs::write(root.join("src/.gitignore"), "*.tmp\n!keep.tmp\n").unwrap();
        for path in [
            "src/main.rs",
            "src/drop.tmp",
            "src/keep.tmp",
            "src/generated/code.rs",
            "drop.log",
            "keep.log",
            "file0.tmp",
            "file1.tmp",
            ".secret",
            "target/build",
            ".runyte/state",
        ] {
            if let Some(parent) = Path::new(path).parent() {
                fs::create_dir_all(root.join(parent)).unwrap();
            }
            fs::write(root.join(path), "x").unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("src/main.rs"), root.join("link.rs")).unwrap();

        let (paths, skipped) = scan_files(&root, &root, &workspace, false).unwrap();
        let relative = paths
            .iter()
            .map(|entry| {
                (
                    entry.path.strip_prefix(&root).unwrap().to_path_buf(),
                    entry.is_dir,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(skipped, 0);
        assert!(relative.contains(&(PathBuf::from("src/main.rs"), false)));
        assert!(relative.contains(&(PathBuf::from("src/keep.tmp"), false)));
        assert!(relative.contains(&(PathBuf::from("keep.log"), false)));
        assert!(relative.contains(&(PathBuf::from("file0.tmp"), false)));
        assert!(
            relative.contains(&(PathBuf::from("src"), true)),
            "directories are candidates too"
        );
        assert!(
            !relative
                .iter()
                .any(|(path, _)| path == Path::new("src/drop.tmp"))
        );
        assert!(
            !relative
                .iter()
                .any(|(path, _)| path == Path::new("src/generated/code.rs"))
        );
        assert!(
            !relative
                .iter()
                .any(|(path, _)| path == Path::new("file1.tmp"))
        );
        assert!(
            !relative
                .iter()
                .any(|(path, _)| path == Path::new(".secret"))
        );
        assert!(!relative.iter().any(|(path, _)| path == Path::new("target")));
        assert!(
            !relative
                .iter()
                .any(|(path, _)| path == Path::new("target/build"))
        );
        assert!(
            relative.contains(&(PathBuf::from("src/generated"), true)),
            "a `dir/**` content-only rule ignores the directory's contents, not the \
             directory entry itself, matching real gitignore semantics"
        );
        assert!(
            !relative
                .iter()
                .any(|(path, _)| path == Path::new(".runyte/state"))
        );
        #[cfg(unix)]
        assert!(
            !relative
                .iter()
                .any(|(path, _)| path == Path::new("link.rs"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_scans_inherit_ancestor_ignores_and_reserved_roots_are_rejected() {
        let project = temporary("ancestor-ignore");
        let root = project.join("src");
        let workspace = project.join(".runyte");
        fs::create_dir_all(root.join("generated")).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(project.join(".git/objects")).unwrap();
        fs::write(project.join(".gitignore"), "src/generated/\n").unwrap();
        fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("generated/secret.rs"), "secret\n").unwrap();
        fs::write(workspace.join("journal"), "private\n").unwrap();

        let (paths, _) = scan_files(&root, &project, &workspace, true).unwrap();
        assert_eq!(paths, vec![ScanEntry::file(root.join("main.rs"))]);
        assert!(scan_files(&workspace, &project, &workspace, true).is_err());
        assert!(scan_files(&project.join(".git/objects"), &project, &workspace, true).is_err());
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn cancellation_is_observed_during_directory_traversal() {
        let root = temporary("cancel");
        for index in 0..32 {
            fs::create_dir_all(root.join(format!("empty-{index}"))).unwrap();
        }
        let workspace = root.join(".runyte");
        let mut checks = 0;
        let mut emissions = 0;
        scan_with(
            &root,
            &root,
            &workspace,
            true,
            true,
            || {
                checks += 1;
                checks > 4
            },
            |_| {
                emissions += 1;
                true
            },
        )
        .unwrap();
        assert_eq!(emissions, 0);
        assert_eq!(checks, 5);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignore_patterns_preserve_escaped_trailing_spaces() {
        let rules = parse_ignore(Path::new(""), "name\\ \ndrop   \n");
        assert!(ignored(&rules, Path::new("name "), false));
        assert!(ignored(&rules, Path::new("drop"), false));
        assert!(!ignored(&rules, Path::new("name"), false));
    }

    #[test]
    fn picker_ranks_and_edits_a_unicode_query() {
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::new(1, root.clone());
        picker.add_paths(vec![
            ScanEntry::file(root.join("src/picker.rs")),
            ScanEntry::file(root.join("src/ui.rs")),
        ]);
        picker.insert_query_text("p界");
        picker.backspace_query();
        assert_eq!(picker.query, "p");
        assert_eq!(picker.selected_entry().unwrap().relative, "src/picker.rs");
    }

    #[test]
    fn equal_scores_prefer_fewer_unicode_characters() {
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::grep(1, root.clone());
        picker.add_content(vec![FileHits {
            path: root.join("content.txt"),
            lines: vec![
                LineHit {
                    row: 0,
                    column: 0,
                    text: "abx".to_owned(),
                },
                LineHit {
                    row: 1,
                    column: 0,
                    text: "界x".to_owned(),
                },
            ],
        }]);
        picker.query = "x".to_owned();
        picker.matches = vec![
            FuzzyMatch {
                entry: 0,
                score: 10,
                positions: vec![2],
            },
            FuzzyMatch {
                entry: 1,
                score: 10,
                positions: vec![1],
            },
        ];

        picker.sort_matches();

        assert_eq!(
            picker
                .ranked()
                .map(|entry| entry.text.unwrap())
                .collect::<Vec<_>>(),
            ["界x", "abx"],
            "candidate length is measured in characters, not UTF-8 bytes"
        );
    }

    #[test]
    fn trailing_slash_narrows_matches_to_directories() {
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::new(1, root.clone());
        picker.add_paths(vec![
            ScanEntry::directory(root.join("src")),
            ScanEntry::file(root.join("src.rs")),
            ScanEntry::directory(root.join("scripts")),
        ]);

        picker.insert_query_text("s");
        assert_eq!(
            picker.matches.len(),
            3,
            "the bare query matches every s* entry"
        );

        picker.insert_query('/');
        let mut relative = picker
            .matches
            .iter()
            .map(|found| picker.view(found.entry).unwrap().relative)
            .collect::<Vec<_>>();
        relative.sort_unstable();
        assert_eq!(
            relative,
            vec!["scripts", "src"],
            "a trailing slash keeps only directories, matched without the slash"
        );

        picker.backspace_query();
        assert_eq!(
            picker.matches.len(),
            3,
            "removing the trailing slash restores files to the results"
        );
    }

    #[test]
    fn typing_past_a_trailing_slash_recovers_files_excluded_while_directory_only() {
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::new(1, root.clone());
        picker.add_paths(vec![
            ScanEntry::directory(root.join("src")),
            ScanEntry::file(root.join("src/main.rs")),
        ]);

        for character in "src".chars() {
            picker.insert_query(character);
        }
        assert_eq!(
            picker.matches.len(),
            2,
            "src matches both the directory and the file"
        );

        picker.insert_query('/');
        assert_eq!(
            picker.matches.len(),
            1,
            "the trailing slash narrows matches to the directory alone"
        );

        // Typing past the slash exits directory-only mode; narrowing from the
        // now-directory-only `matches` must not permanently drop the file.
        picker.insert_query('m');
        let relative = picker
            .matches
            .iter()
            .map(|found| picker.view(found.entry).unwrap().relative)
            .collect::<Vec<_>>();
        assert_eq!(
            relative,
            vec!["src/main.rs"],
            "typing past the slash must recover files excluded while directory-only"
        );
    }

    #[test]
    fn ranking_scales_across_a_large_streamed_inventory() {
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::new(1, root.clone());
        picker.add_paths(
            (0..10_000)
                .map(|index| {
                    ScanEntry::file(root.join(format!("src/module-{index}/file_picker_{index}.rs")))
                })
                .collect(),
        );
        let started = std::time::Instant::now();
        picker.insert_query_text("fp");
        assert_eq!(picker.matches.len(), 10_000);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "ranking 10,000 short paths should remain interactive"
        );
    }

    #[test]
    fn background_scanner_streams_files_and_a_terminal_event() {
        let root = temporary("background");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        let workspace = root.join(".runyte");
        let (scanner, mut receiver) = scanner();
        scanner.scan(7, root.clone(), root.clone(), workspace, false);

        let mut paths = Vec::new();
        loop {
            match receiver.blocking_recv().unwrap() {
                FilePickerEvent::Files {
                    scan_id,
                    paths: batch,
                } => {
                    assert_eq!(scan_id, 7);
                    paths.extend(batch);
                }
                FilePickerEvent::Finished {
                    scan_id,
                    skipped,
                    limited,
                } => {
                    assert_eq!((scan_id, skipped, limited), (7, 0, false));
                    break;
                }
                FilePickerEvent::Content { .. } => panic!("file scan emitted content"),
                FilePickerEvent::Failed { message, .. } => panic!("scan failed: {message}"),
            }
        }
        assert_eq!(
            paths,
            vec![
                ScanEntry::directory(root.join("src")),
                ScanEntry::file(root.join("src/main.rs")),
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fuzzy_grep_ranks_line_contents_and_keeps_a_jump_target() {
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::grep(1, root.clone());
        picker.add_content(vec![
            FileHits {
                path: root.join("src/main.rs"),
                lines: vec![LineHit {
                    row: 7,
                    column: 0,
                    text: "workspace scanner".to_owned(),
                }],
            },
            FileHits {
                path: root.join("src/app.rs"),
                lines: vec![LineHit {
                    row: 12,
                    column: 0,
                    text: "wide separate cursor".to_owned(),
                }],
            },
        ]);
        picker.insert_query_text("wscan");

        assert_eq!(picker.matches.len(), 1);
        assert_eq!(picker.selected_entry().unwrap().label(), "src/main.rs:8");
        assert!(
            picker
                .selected_entry()
                .unwrap()
                .match_positions_in_label(&picker.matches[0].positions)
                .is_empty(),
            "content matches must not be painted onto their path-only rows"
        );
        assert_eq!(
            picker.selected_target(),
            Some(PickerTarget {
                path: root.join("src/main.rs"),
                row: Some(7),
                column: 0,
            })
        );
    }

    #[test]
    fn content_scan_uses_file_picker_ignore_and_text_boundaries() {
        let root = temporary("content");
        let workspace = root.join(".runyte");
        fs::create_dir_all(root.join("ignored")).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        fs::write(root.join("visible.txt"), "  alpha\n\nbeta\n").unwrap();
        fs::write(root.join("ignored/hidden.txt"), "secret\n").unwrap();
        fs::write(root.join("binary.bin"), [0xff, 0x00]).unwrap();

        let (entries, skipped, limited) =
            scan_content(&root, &root, &workspace, false, "").unwrap();
        assert_eq!(skipped, 0);
        assert!(!limited);
        assert_eq!(
            entries
                .iter()
                .flat_map(|hits| {
                    let path = hits.path.strip_prefix(&root).unwrap();
                    hits.lines
                        .iter()
                        .map(move |line| (path, line.row, line.column, line.text.as_str()))
                })
                .collect::<Vec<_>>(),
            [
                (Path::new("visible.txt"), 0, 2, "alpha"),
                (Path::new("visible.txt"), 2, 0, "beta")
            ]
        );
        assert_eq!(
            entries.len(),
            1,
            "both lines of one file arrive as one group holding one path"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_query_of_several_words_asks_for_each_of_them() {
        // A space separates rather than matches. One word stays the fuzzy
        // subsequence it has always been; two or more each have to be there as
        // themselves, in the order they were typed.
        assert!(matches_fuzzy("cntnt", "content_entries_from_text"));
        assert!(matches_fuzzy(
            "content entries",
            "content_entries_from_text"
        ));
        assert!(
            !matches_fuzzy("cntnt entries", "content_entries_from_text"),
            "a term of a several-word query is not itself fuzzy"
        );
        assert!(
            !matches_fuzzy("entries content", "content_entries_from_text"),
            "terms are wanted in the order they were typed"
        );
        assert!(
            !matches_fuzzy("content missing", "content_entries_from_text"),
            "every term has to be present, not just one"
        );

        // Whitespace around a lone word is separation, not something to match,
        // so it asks exactly what the bare word asks.
        assert_eq!(
            matches_fuzzy("  cntnt  ", "content_entries_from_text"),
            matches_fuzzy("cntnt", "content_entries_from_text")
        );

        // Smart case reads the whole query: one capital makes every term
        // case-sensitive.
        assert!(matches_fuzzy("content text", "content_entries_from_text"));
        assert!(!matches_fuzzy("Content text", "content_entries_from_text"));
        assert!(matches_fuzzy("Content text", "Content_entries_from_text"));
    }

    #[test]
    fn several_words_score_where_each_of_them_landed() {
        let mut lines = FuzzyMatcher::for_lines("pub score");
        let (_, positions) = lines
            .score("pub fn score(&mut self) -> Option<i64>")
            .expect("both terms are present");
        assert_eq!(positions, [0, 1, 2, 7, 8, 9, 10, 11], "one run a term");
        assert!(
            is_direct_match(&positions, "pub score"),
            "terms that each landed whole are a direct match"
        );

        // Terms that end up adjacent are the tightest a several-word query can
        // land, so they stay direct even though they read as one run.
        let mut joined = FuzzyMatcher::for_lines("content entries");
        let (_, positions) = joined
            .score("let contententries = 1;")
            .expect("the terms are present back to back");
        assert_eq!(positions, (4..18).collect::<Vec<_>>());
        assert!(is_direct_match(&positions, "content entries"));

        // A gap inside a single word is still the fuzzy subsequence the
        // secondary colour is for.
        let mut gapped = FuzzyMatcher::for_lines("pbscre");
        let (_, positions) = gapped
            .score("pub fn score(&mut self)")
            .expect("a subsequence");
        assert!(!is_direct_match(&positions, "pbscre"));

        // Where a term appears more than once, the occurrence chosen is the
        // one that scores best, and never one that strands a later term.
        let mut repeated = FuzzyMatcher::for_lines("ab ab");
        let (_, positions) = repeated
            .score("ab ab")
            .expect("two occurrences serve two terms");
        assert_eq!(positions, [0, 1, 3, 4]);

        // A term-mode query may be longer than the candidate, because its
        // spaces are not characters the candidate has to hold.
        let mut spaced = FuzzyMatcher::for_lines("aa bb");
        assert!(spaced.score("aabb").is_some());
    }

    #[test]
    fn splitting_a_term_reconsiders_paths_the_whole_term_excluded() {
        // Narrowing from the previous result set is only sound while the edit
        // could not widen it. Typing at the end cannot, which is the case
        // `a_longer_query_can_only_narrow_what_a_shorter_one_matched` covers.
        // Typing into the middle can: splitting `ab` into `a b` replaces one
        // literal with two that a path may satisfy far apart, and the path
        // holding them was thrown out by the term it never contained.
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::new(1, root.clone());
        picker.add_paths(vec![
            ScanEntry::file(root.join("a_x_b_cd")),
            ScanEntry::file(root.join("ab_cd")),
        ]);
        picker.insert_query_text("ab cd");
        assert_eq!(
            picker
                .ranked()
                .map(|entry| entry.relative)
                .collect::<Vec<_>>(),
            ["ab_cd"],
            "`a_x_b_cd` holds no literal `ab`"
        );

        // Put the caret between `a` and `b` and split the term.
        picker.query_cursor = 1;
        picker.insert_query(' ');
        assert_eq!(picker.query, "a b cd");
        let mut found = picker
            .ranked()
            .map(|entry| entry.relative)
            .collect::<Vec<_>>();
        found.sort_unstable();
        assert_eq!(
            found,
            ["a_x_b_cd", "ab_cd"],
            "a path the old term excluded has to be reconsidered, not stay hidden"
        );

        // Not a whitespace question. Growing a term from the middle rewrites
        // the literal just as splitting it does, and widens the same way.
        let mut picker = FilePicker::new(2, root.clone());
        picker.add_paths(vec![
            ScanEntry::file(root.join("aXb_cd")),
            ScanEntry::file(root.join("ab_cd")),
        ]);
        picker.insert_query_text("ab cd");
        assert_eq!(
            picker
                .ranked()
                .map(|entry| entry.relative)
                .collect::<Vec<_>>(),
            ["ab_cd"]
        );
        picker.query_cursor = 1;
        picker.insert_query('X');
        assert_eq!(picker.query, "aXb cd");
        assert_eq!(
            picker
                .ranked()
                .map(|entry| entry.relative)
                .collect::<Vec<_>>(),
            ["aXb_cd"],
            "the path the rewritten term now matches was excluded by the old one"
        );

        // Growing the query at its end still narrows from what is on hand,
        // which is the case that has to stay cheap.
        let mut picker = FilePicker::new(3, root.clone());
        picker.add_paths(vec![
            ScanEntry::file(root.join("ab_cd")),
            ScanEntry::file(root.join("ab_ce")),
        ]);
        picker.insert_query_text("ab c");
        assert_eq!(picker.matches.len(), 2);
        picker.insert_query('d');
        assert_eq!(
            picker
                .ranked()
                .map(|entry| entry.relative)
                .collect::<Vec<_>>(),
            ["ab_cd"]
        );
    }

    #[test]
    fn a_longer_query_can_only_narrow_what_a_shorter_one_matched() {
        // `content_rescan_needed` reuses the entries on hand whenever the
        // query merely grew, which is only sound while every match of the
        // longer query is a match of the shorter. Crossing from one term to
        // two has to preserve that, because it is where the rule changes.
        let candidates = [
            "content_entries_from_text",
            "let contententries = 1;",
            "fn rank_entries(entries: &[FileEntry])",
            "the picker holds its entries",
            "nothing relevant at all",
            "Content Entries From Text",
        ];
        let full = "content entries from";
        for cut in 1..=full.len() {
            let (shorter, longer) = (&full[..cut - 1], &full[..cut]);
            for candidate in candidates {
                assert!(
                    !matches_fuzzy(longer, candidate) || matches_fuzzy(shorter, candidate),
                    "{longer:?} matched {candidate:?} where its prefix {shorter:?} did not"
                );
            }
        }
    }

    #[test]
    fn a_slash_later_in_a_line_does_not_decide_where_a_match_ranks() {
        // Two rules in the scorer are about paths: the characters after the
        // last `/` are the basename and score 30 more each, and a candidate
        // loses 3 a separator so a shallow path beats a deep one. Ranking line
        // text by them made one commit line an outlier among its own siblings
        // purely because it ended in `(origin/main, origin/HEAD)` — the
        // basename start moved past the match, so a match earlier in the line
        // scored 220 where the identical match in every neighbouring line
        // scored 407, and it fell from the middle of the list to rank 717.
        //
        // The two lines differ only in `-` against `/`, which are both word
        // boundaries, so under line ranking nothing at all should separate
        // them.
        let dashes = "26c3b7bb133c  2026-08-18  Merge branch (origin-main, origin-HEAD)";
        let slashes = "26c3b7bb133c  2026-08-18  Merge branch (origin/main, origin/HEAD)";
        let mut lines = FuzzyMatcher::for_lines("branch");
        assert_eq!(
            lines.score(dashes).map(|(score, _)| score),
            lines.score(slashes).map(|(score, _)| score),
            "a separator later in a line must not move the match before it"
        );

        // A path matcher keeps both rules, because for a path they are the
        // whole point: the basename is what someone is naming.
        let mut paths = FuzzyMatcher::new("picker");
        let basename = paths
            .score("src/picker.rs")
            .expect("the basename matches")
            .0;
        let deep = paths
            .score("src/ui/file_picker_row.rs")
            .expect("the path matches")
            .0;
        assert!(
            basename > deep,
            "path ranking still has to prefer a basename match: {basename} against {deep}"
        );
    }

    #[test]
    fn the_subsequence_filter_decides_exactly_what_the_scorer_decides() {
        // The scanner keeps a line only when `matches_fuzzy` accepts it, and
        // the picker then ranks that line with `fuzzy_match`. A disagreement
        // between the two would either hide a match the scorer would have
        // ranked or collect a line the scorer cannot rank at all, so the two
        // answers are checked against each other rather than separately.
        let candidates = [
            "",
            "aaa",
            "Cargo.toml",
            "src/file_picker.rs",
            "fn compute_the_thing(input: usize) -> usize",
            "\u{391}\u{392}\u{393} \u{3b1}\u{3b2}\u{3b3}",
            "stra\u{df}e",
            "KELVIN \u{212a}",
        ];
        let queries = [
            "",
            "a",
            "aaaa",
            "cti",
            "Cti",
            "CTI",
            "toml",
            "TOML",
            "\u{3b1}\u{3b2}",
            "\u{391}\u{392}",
            "ss",
            "\u{df}",
            "k",
            "K",
            "zzz",
            "fn compute_the_thing(input: usize) -> usize",
        ];
        for candidate in candidates {
            for query in queries {
                assert_eq!(
                    matches_fuzzy(query, candidate),
                    fuzzy_match(query, candidate).is_some(),
                    "the filter and the scorer disagreed on {query:?} in {candidate:?}"
                );
            }
        }

        // The same has to hold once a space turns the query into terms, where
        // the filter and the scorer search for occurrences separately.
        let terms = [
            "fn compute",
            "compute fn",
            "compute the thing",
            "fn  compute",
            " compute ",
            "aa aa",
            "toml Cargo",
            "Cargo toml",
        ];
        for candidate in candidates {
            for query in terms {
                assert_eq!(
                    matches_fuzzy(query, candidate),
                    fuzzy_match(query, candidate).is_some(),
                    "the filter and the scorer disagreed on {query:?} in {candidate:?}"
                );
            }
        }
    }

    #[test]
    fn a_content_scan_reaches_a_match_far_past_the_candidate_limit() {
        // The ceiling is on results, not on how far into a project the walk
        // got. The needle here sits behind several times `CONTENT_ENTRY_LIMIT`
        // lines of ordinary code, in the last file written: that is the shape
        // that used to make a match visible only once its file was open.
        let root = temporary("content-past-the-limit");
        fs::create_dir_all(&root).unwrap();
        let workspace = root.join(".runyte");
        // Sized from the budget rather than to a fixed number, so raising it
        // keeps the fixture on the far side of it.
        let per_file = CONTENT_ENTRY_LIMIT / 4;
        let filler = (0..per_file)
            .map(|line| format!("let value_{line} = compute(input, {line});\n"))
            .collect::<String>();
        for file in 0..8 {
            fs::write(root.join(format!("file{file}.rs")), &filler).unwrap();
        }
        fs::write(
            root.join("zzz_last.rs"),
            format!("{filler}call_the_marked_thing();\n"),
        )
        .unwrap();

        let (entries, skipped, limited) =
            scan_content(&root, &root, &workspace, false, "markedthing").unwrap();
        assert_eq!(skipped, 0);
        assert!(
            !limited,
            "a scan that filters as it walks reports truncation only when the matches truncate"
        );
        assert_eq!(
            entries
                .iter()
                .flat_map(|hits| {
                    let path = hits.path.strip_prefix(&root).unwrap();
                    hits.lines
                        .iter()
                        .map(move |line| (path, line.text.as_str()))
                })
                .collect::<Vec<_>>(),
            [(Path::new("zzz_last.rs"), "call_the_marked_thing();")]
        );

        let (entries, _, limited) = scan_content(&root, &root, &workspace, false, "").unwrap();
        assert!(
            limited,
            "an unfiltered scan of twice the budget still fills the budget"
        );
        assert_eq!(
            entries.iter().map(FileHits::len).sum::<usize>(),
            CONTENT_ENTRY_LIMIT,
            "the budget counts matching lines, not the files they came from"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_file_is_held_once_however_many_of_its_lines_match() {
        // Matches cluster: a full budget on Runyte's own repository comes from
        // a couple of hundred files. Giving every line its own path spent as
        // much memory on paths as on the matching text itself, and rebuilt the
        // displayed relative path per line.
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::grep(1, root.clone());
        let hits = |name: &str, rows: std::ops::Range<usize>| FileHits {
            path: root.join(name),
            lines: rows
                .map(|row| LineHit {
                    row,
                    column: 0,
                    text: format!("let value_{row} = compute(input);"),
                })
                .collect(),
        };
        picker.add_content(vec![hits("src/app.rs", 0..400), hits("src/ui.rs", 0..100)]);
        assert_eq!(picker.entries.len(), 500);
        assert_eq!(picker.file_count(), 2);

        // A later batch for a file already on hand joins it rather than
        // starting a second row, so a scan that streams one file across
        // batches still holds its path once.
        picker.add_content(vec![hits("src/app.rs", 400..450)]);
        assert_eq!(picker.entries.len(), 550);
        assert_eq!(picker.file_count(), 2);

        // Every entry still resolves to the right file and line.
        let view = picker.view(540).unwrap();
        assert_eq!(view.path, root.join("src/app.rs"));
        assert_eq!(view.relative, "src/app.rs");
        assert_eq!(view.row, Some(440));
        assert_eq!(view.label(), "src/app.rs:441");

        // Restarting for a new query drops the table with the entries, so a
        // file that no longer matches is not held for the rest of the session.
        picker.restart_content_scan(2);
        assert_eq!(picker.file_count(), 0);
        assert!(picker.entries.is_empty());
    }

    #[test]
    fn the_candidate_budget_counts_lines_rather_than_files() {
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::grep(1, root.clone());
        picker.add_content(vec![FileHits {
            path: root.join("huge.rs"),
            lines: (0..CONTENT_ENTRY_LIMIT + 10)
                .map(|row| LineHit {
                    row,
                    column: 0,
                    text: "matching line".to_owned(),
                })
                .collect(),
        }]);
        assert_eq!(picker.entries.len(), CONTENT_ENTRY_LIMIT);
        assert!(
            picker.limited,
            "one file past the budget has to report truncation, not be admitted whole"
        );
        assert_eq!(picker.file_count(), 1);
    }

    #[test]
    fn content_entries_are_narrowed_in_memory_only_while_the_scan_was_complete() {
        let mut picker = FilePicker::grep(1, PathBuf::from("/project"));
        picker.restart_content_scan(1);
        picker.finish(0, false);

        // A complete scan of the empty query holds every line in the project,
        // so every longer query is answered from what is already on hand.
        picker.insert_query('a');
        assert!(!picker.content_rescan_needed());
        picker.insert_query('b');
        assert!(!picker.content_rescan_needed());

        // The same scan truncated stopped somewhere inside the project, so the
        // lines it never reached have to be looked at.
        picker.finish(0, true);
        assert!(picker.content_rescan_needed());

        picker.restart_content_scan(2);
        picker.finish(0, false);
        assert!(
            !picker.content_rescan_needed(),
            "a query the scan just ran must never ask for another one"
        );
        picker.insert_query('c');
        assert!(!picker.content_rescan_needed());
        picker.backspace_query();
        assert!(
            !picker.content_rescan_needed(),
            "deleting back to the scanned query lands on the scanned set"
        );
        picker.backspace_query();
        assert!(
            picker.content_rescan_needed(),
            "a query the scan filtered away asks about lines it discarded"
        );

        let mut files = FilePicker::new(1, PathBuf::from("/project"));
        files.finish(0, true);
        files.insert_query('a');
        assert!(
            !files.content_rescan_needed(),
            "path scanning collects every path whatever the query is"
        );
    }

    #[test]
    fn background_content_scanner_streams_lines_and_finishes() {
        let root = temporary("background-content");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("notes.txt"), "first line\nsecond line\n").unwrap();
        let workspace = root.join(".runyte");
        let (scanner, mut receiver) = scanner();
        scanner.scan_content(
            11,
            root.clone(),
            root.clone(),
            workspace,
            false,
            String::new(),
        );

        let mut entries = Vec::new();
        loop {
            match receiver.blocking_recv().unwrap() {
                FilePickerEvent::Content {
                    scan_id,
                    entries: batch,
                } => {
                    assert_eq!(scan_id, 11);
                    entries.extend(batch);
                }
                FilePickerEvent::Finished {
                    scan_id,
                    skipped,
                    limited,
                } => {
                    assert_eq!((scan_id, skipped, limited), (11, 0, false));
                    break;
                }
                FilePickerEvent::Files { .. } => panic!("content scan emitted file paths"),
                FilePickerEvent::Failed { message, .. } => panic!("scan failed: {message}"),
            }
        }
        assert_eq!(
            entries
                .iter()
                .flat_map(|hits| hits.lines.iter().map(|line| line.text.as_str()))
                .collect::<Vec<_>>(),
            ["first line", "second line"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_restarted_content_scan_replaces_the_one_it_cancelled() {
        // Every edit to a content query is a new scan, so the scanner has to
        // do two things: filter by the query it was given, and stay silent
        // once a later scan has taken over. Silence is the load-bearing half —
        // a cancelled scan that still emitted would mix a stale query's lines
        // into the results of the current one.
        let root = temporary("restarted-content");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("notes.txt"), "alpha line\nbeta line\n").unwrap();
        let workspace = root.join(".runyte");
        let (scanner, mut receiver) = scanner();

        scanner.scan_content(
            11,
            root.clone(),
            root.clone(),
            workspace.clone(),
            false,
            "alpha".to_owned(),
        );
        scanner.scan_content(
            12,
            root.clone(),
            root.clone(),
            workspace,
            false,
            "beta".to_owned(),
        );

        let mut entries = Vec::new();
        loop {
            match receiver.blocking_recv().unwrap() {
                FilePickerEvent::Content {
                    scan_id,
                    entries: batch,
                } => {
                    assert_eq!(scan_id, 12, "the cancelled scan must emit nothing");
                    entries.extend(batch);
                }
                FilePickerEvent::Finished { scan_id, .. } => {
                    assert_eq!(scan_id, 12, "the cancelled scan must not report an end");
                    break;
                }
                FilePickerEvent::Files { .. } => panic!("content scan emitted file paths"),
                FilePickerEvent::Failed { message, .. } => panic!("scan failed: {message}"),
            }
        }
        assert_eq!(
            entries
                .iter()
                .flat_map(|hits| hits.lines.iter().map(|line| line.text.as_str()))
                .collect::<Vec<_>>(),
            ["beta line"],
            "the surviving scan keeps only the lines its own query matches"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_preview_lists_files_and_subdirectories() {
        let root = temporary("directory-preview");
        fs::create_dir_all(root.join("subdir")).unwrap();
        fs::write(root.join("note.txt"), "hello").unwrap();
        fs::write(root.join(".hidden"), "secret").unwrap();

        let FilePreview::Directory(lines) = FilePreview::from_directory(&root, false) else {
            panic!("directory preview expected");
        };
        assert_eq!(lines, vec!["note.txt".to_owned(), "subdir/".to_owned()]);

        let FilePreview::Directory(lines) = FilePreview::from_directory(&root, true) else {
            panic!("directory preview expected");
        };
        assert_eq!(
            lines,
            vec![
                ".hidden".to_owned(),
                "note.txt".to_owned(),
                "subdir/".to_owned()
            ]
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_preview_bounds_large_listings_and_reports_the_omitted_count() {
        let root = temporary("large-directory-preview");
        fs::create_dir_all(&root).unwrap();
        let entry_count = PREVIEW_DIRECTORY_ENTRIES + 50;
        for index in 0..entry_count {
            fs::write(root.join(format!("file-{index:05}.txt")), "").unwrap();
        }

        let FilePreview::Directory(lines) = FilePreview::from_directory(&root, false) else {
            panic!("directory preview expected");
        };
        assert_eq!(
            lines.len(),
            PREVIEW_DIRECTORY_ENTRIES + 1,
            "the listing is capped, plus one summary line for what was omitted"
        );
        assert_eq!(
            lines.last().unwrap(),
            "… 50 more entries not shown",
            "the omitted count is exact, not just a lower bound"
        );
        assert!(
            lines[..PREVIEW_DIRECTORY_ENTRIES].is_sorted(),
            "the kept prefix is still presented in sorted order"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn previews_bound_text_and_classify_binary_without_rejecting_large_text() {
        let root = temporary("preview");
        fs::create_dir_all(&root).unwrap();
        let text = root.join("text.rs");
        let binary = root.join("image.bin");
        let large = root.join("large.txt");
        let split_unicode = root.join("split-unicode.txt");
        fs::write(&text, "first\nsecond\n").unwrap();
        fs::write(&binary, [0, 1, 2, 3]).unwrap();
        fs::write(
            &split_unicode,
            format!("{}界", "a".repeat(PREVIEW_BYTES as usize - 1)),
        )
        .unwrap();
        fs::write(&large, vec![b'a'; 5 * 1024 * 1024]).unwrap();

        assert!(matches!(
            FilePreview::from_path(&text),
            FilePreview::Text(_)
        ));
        assert!(matches!(
            FilePreview::from_path(&split_unicode),
            FilePreview::Text(_)
        ));
        assert_eq!(FilePreview::from_path(&binary), FilePreview::Binary);
        let FilePreview::Text(lines) = FilePreview::from_path(&large) else {
            panic!("large text should still have a bounded preview");
        };
        assert_eq!(lines[0].len(), 512);

        let generated = (0..).map(|index| {
            assert!(index < 512, "preview consumed beyond its line bound");
            "line".to_owned()
        });
        let FilePreview::Text(lines) = FilePreview::from_lines(generated) else {
            panic!("generated text preview expected");
        };
        assert_eq!(lines.len(), 512);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn content_preview_centers_context_and_preserves_match_emphasis() {
        let text = (0..20)
            .map(|row| format!("line {}", row + 1))
            .collect::<Vec<_>>()
            .join("\n");
        let FilePreview::Snippet(snippet) = FilePreview::snippet_from_text(&text, 12, vec![0, 3])
        else {
            panic!("content snippet expected");
        };

        assert_eq!(snippet.start_row, 8);
        assert_eq!(snippet.focus_row, 12);
        assert_eq!(snippet.lines.len(), 9);
        assert_eq!(snippet.lines[0], "line 9");
        assert_eq!(snippet.lines[4], "line 13");
        assert_eq!(snippet.lines[8], "line 17");
        assert_eq!(snippet.emphasis, vec![0, 3]);
        assert_eq!(snippet.display_lines()[4], "› 13 │ line 13");
        assert!(
            snippet
                .display_lines()
                .iter()
                .all(|line| !line.contains("line 1 ")),
            "the preview must not begin at the head of a distant match"
        );
    }
}
