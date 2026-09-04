// SPDX-License-Identifier: MPL-2.0

//! Native fuzzy file discovery and presentation-neutral picker state.
//!
//! The scanner deliberately uses only the standard library and Runyte's
//! existing regular-expression engine. It never invokes `git`, `fd`, `find`,
//! or a fuzzy-finder process. Background workers emit immutable batches; the
//! editor remains the sole owner of picker state.

use std::{
    collections::HashMap,
    fs,
    io::{self, BufRead, BufReader, Read},
    num::NonZero,
    ops::Range,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc as sync_mpsc,
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use regex::Regex;
use tokio::sync::mpsc::{Receiver, Sender, channel};

const SCAN_BATCH: usize = 128;
/// Files admitted between background result publications.
///
/// The editor sees scanner batches of `SCAN_BATCH`, which bounds admission on
/// its thread. The ranker combines several of those batches before it merges
/// and publishes, avoiding a whole growing-list clone for every 128 paths.
const RANK_PUBLISH_BATCH: usize = 4_096;
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

    /// What this picker's query line reads before anything is typed. Both
    /// renderers ask for it here so an attached client and a standalone
    /// editor cannot invite the reader to do two different things.
    pub const fn query_placeholder(self) -> &'static str {
        match self {
            Self::Files => "type to fuzzy-find",
            Self::Contents => "type to fuzzy-search contents",
        }
    }
}

/// Which ignore files, if any, a scan under a picker root obeys.
///
/// Held by the picker rather than passed once per scan. Tab and a content
/// re-scan restart the walk on the person's behalf, so a scope supplied only
/// at open time would widen or narrow itself the first time either happened.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScanScope {
    /// Read `.gitignore` and `.ignore`, inheriting the rules that apply from
    /// this directory down to the picker root.
    Ignoring { from: PathBuf },
    /// Read no ignore file at all. The reserved names, the workspace state
    /// directory, symlinks, and the hidden-file rule still apply.
    Everything,
}

impl ScanScope {
    /// The ordinary scope: every ignore file from `from` down to the root.
    pub fn ignoring(from: impl Into<PathBuf>) -> Self {
        Self::Ignoring { from: from.into() }
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

/// The corpus a content re-scan replaced, kept readable while the walk that
/// replaces it is still finding its first results.
///
/// Rows already on screen name the scan they were ranked against, so they go
/// on resolving through this until an answer for the new scan arrives. It is
/// one generation deep: a second re-scan retires it.
#[derive(Clone, Debug)]
struct PreviousCorpus {
    scan: u64,
    files: Vec<PickerFile>,
    entries: Vec<FileEntry>,
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
struct FilePreviewKey {
    target: PickerTarget,
    content_match: Option<(usize, Vec<usize>)>,
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
    /// The preview's own lines.
    ///
    /// A snippet contributes the text it previews rather than the numbered
    /// rows it displays, and a preview that has no content to show — a binary
    /// file, or one that could not be read — contributes nothing.
    pub fn lines(&self) -> &[String] {
        match self {
            Self::Text(lines) | Self::Directory(lines) => lines,
            Self::Snippet(snippet) => &snippet.lines,
            Self::Binary | Self::Unreadable(_) => &[],
        }
    }

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

    /// The rows a content snippet shows around a match on `focus_row`.
    ///
    /// A source that can read one row directly, as terminal scrollback can,
    /// reads this range instead of skipping a line iterator. Every content
    /// preview then shows the same context whatever produced its lines.
    pub const fn snippet_rows(focus_row: usize) -> Range<usize> {
        let start_row = focus_row.saturating_sub(PREVIEW_CONTEXT_BEFORE);
        start_row..start_row + PREVIEW_CONTEXT_LINES
    }

    /// A snippet from lines the caller has already positioned at `start_row`.
    pub fn snippet_from_rows(
        lines: impl Iterator<Item = String>,
        start_row: usize,
        focus_row: usize,
        emphasis: Vec<usize>,
    ) -> Self {
        let focus_offset = focus_row.checked_sub(start_row);
        let emphasized_end = emphasis
            .iter()
            .copied()
            .max()
            .map_or(0, |position| position.saturating_add(1));
        let lines = lines
            .enumerate()
            .map(|(offset, line)| {
                // Content candidates omit indentation before applying their
                // 512-character bound. The preview restores that indentation,
                // so its focused line may need to extend past 512 to keep the
                // already-bounded match visible.
                let characters = if Some(offset) == focus_offset {
                    GREP_LINE_CHARACTERS.max(emphasized_end)
                } else {
                    GREP_LINE_CHARACTERS
                };
                line.chars().take(characters).collect::<String>()
            })
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

    pub fn snippet_from_lines(
        lines: impl Iterator<Item = String>,
        focus_row: usize,
        emphasis: Vec<usize>,
    ) -> Self {
        let rows = Self::snippet_rows(focus_row);
        Self::snippet_from_rows(
            lines.skip(rows.start).take(rows.len()),
            rows.start,
            focus_row,
            emphasis,
        )
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
        let rows = Self::snippet_rows(focus_row);
        match BufReader::new(file)
            .lines()
            .skip(rows.start)
            .take(rows.len())
            .collect::<io::Result<Vec<_>>>()
        {
            Ok(lines) => {
                Self::snippet_from_rows(lines.into_iter(), rows.start, focus_row, emphasis)
            }
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
    /// What the walk under `root` obeys. `root` is what the overlay shows;
    /// this is what the scanner reads, and it has to outlive the individual
    /// scan because Tab and a content re-scan start fresh ones.
    pub scope: ScanScope,
    pub kind: FilePickerKind,
    /// Every distinct file the entries below refer to, each held once.
    files: Vec<PickerFile>,
    pub entries: Vec<FileEntry>,
    /// The corpus a content re-scan replaced, still read by the rows on
    /// screen. See [`FilePicker::view_in`].
    previous: Option<PreviousCorpus>,
    pub matches: Vec<FuzzyMatch>,
    pub query: String,
    /// Monotonic identity of the query shown in the prompt.
    ///
    /// Background rankings carry this value back. A result for an older
    /// revision is stale even when it belongs to the same filesystem scan.
    pub query_revision: u64,
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
    /// Whether the background ranker is still answering the displayed query.
    pub ranking: bool,
    /// Whether the answer still owed is the ranker's flush of a finished
    /// scan. A publish the ranker had already sent when the scan finished
    /// answers only the candidates it held then, so it cannot be the one
    /// that lets the rows be read.
    final_rank_pending: bool,
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
    path_files: HashMap<PathBuf, u32>,
    preview_request: Option<(u64, FilePreviewKey)>,
    next_preview_request_id: u64,
}

impl FilePicker {
    pub fn new(scan_id: u64, root: PathBuf, scope: ScanScope) -> Self {
        Self::with_kind(scan_id, root, scope, FilePickerKind::Files)
    }

    pub fn grep(scan_id: u64, root: PathBuf, scope: ScanScope) -> Self {
        Self::with_kind(scan_id, root, scope, FilePickerKind::Contents)
    }

    fn with_kind(scan_id: u64, root: PathBuf, scope: ScanScope, kind: FilePickerKind) -> Self {
        Self {
            scan_id,
            root,
            scope,
            kind,
            files: Vec::new(),
            entries: Vec::new(),
            previous: None,
            matches: Vec::new(),
            query: String::new(),
            query_revision: 0,
            scan_query: String::new(),
            query_cursor: 0,
            selected: 0,
            loading: true,
            ranking: false,
            final_rank_pending: false,
            skipped: 0,
            limited: false,
            error: None,
            show_preview: true,
            preview: None,
            selection_user_owned: false,
            directory_only: false,
            unified_finder: false,
            path_files: HashMap::new(),
            preview_request: None,
            next_preview_request_id: 1,
        }
    }

    /// How this picker's scope reads in a title, or `None` for the ordinary
    /// project one, which needs no saying.
    ///
    /// Three keys open the finder over three scopes, so every surface that
    /// draws it has to name the one in front of the reader. It lives here so
    /// that the drawn title and an attached client's snapshot cannot disagree
    /// about which finder is open.
    pub fn scope_label(&self, project_root: &Path) -> Option<String> {
        match &self.scope {
            ScanScope::Ignoring { .. } => None,
            ScanScope::Everything if self.root == project_root => Some("all files".to_owned()),
            ScanScope::Everything => Some(self.root.display().to_string()),
        }
    }

    pub fn enable_unified_finder(&mut self) {
        self.unified_finder = true;
        self.rank(true, false);
    }

    /// Changes the project finder's filesystem engine while retaining the
    /// query, cursor, and preview preference held by the overlay.
    pub(crate) fn switch_kind(
        &mut self,
        scan_id: u64,
        kind: FilePickerKind,
    ) -> DiscardedPickerCorpus {
        let mut discarded = self.take_corpus();
        // A mode switch replaces the rows as well as the table under them,
        // so the corpus a content re-scan kept readable has no reader left.
        if let Some(previous) = self.previous.take() {
            discarded.files = previous.files;
            discarded.entries = previous.entries;
        }
        self.scan_id = scan_id;
        self.kind = kind;
        self.scan_query = if kind == FilePickerKind::Contents {
            self.query.clone()
        } else {
            String::new()
        };
        self.selected = 0;
        self.loading = true;
        self.final_rank_pending = false;
        self.skipped = 0;
        self.limited = false;
        self.error = None;
        self.selection_user_owned = false;
        self.directory_only = false;
        discarded
    }

    pub fn add_paths(&mut self, paths: Vec<ScanEntry>) {
        let candidates = self.add_paths_unranked(paths);
        let selected = self
            .selection_user_owned
            .then(|| self.selected_target())
            .flatten();
        let first_new = candidates
            .first()
            .map_or(self.entries.len(), |entry| entry.entry);
        self.rank_new_entries(first_new, selected);
    }

    /// Admits a filesystem batch without doing any ranking on the editor
    /// thread. The returned lightweight candidates are sent to the background
    /// ranker; their indices are exactly the entries just admitted here.
    pub(crate) fn add_paths_unranked(&mut self, paths: Vec<ScanEntry>) -> Vec<FileRankCandidate> {
        let mut candidates = Vec::with_capacity(paths.len());
        for entry in paths {
            let path = entry.path;
            let is_dir = entry.is_dir;
            let relative = path_text(path.strip_prefix(&self.root).unwrap_or(&path));
            let file = self.files.len() as u32;
            self.files.push(PickerFile {
                path: path.clone(),
                relative: relative.clone(),
                is_dir,
            });
            self.path_files.insert(path.clone(), file);
            let candidate_characters = relative.chars().count();
            let entry = self.entries.len();
            self.entries.push(FileEntry {
                file,
                row: None,
                column: 0,
                text: None,
                candidate_characters,
            });
            candidates.push(FileRankCandidate {
                entry,
                path,
                relative: relative.clone(),
                text: relative,
                row: None,
                candidate_characters,
                is_dir,
            });
        }
        candidates
    }

    pub fn add_content(&mut self, files: Vec<FileHits>) {
        let candidates = self.add_content_unranked(files);
        let selected = self.selection_user_owned.then(|| self.selected_target());
        let first_new = candidates
            .first()
            .map_or(self.entries.len(), |entry| entry.entry);
        self.rank_new_entries(first_new, selected.flatten());
    }

    pub(crate) fn add_content_unranked(&mut self, files: Vec<FileHits>) -> Vec<FileRankCandidate> {
        let mut candidates = Vec::new();
        let mut available = CONTENT_ENTRY_LIMIT.saturating_sub(self.entries.len());
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
            let path = self.files[file as usize].path.clone();
            let relative = self.files[file as usize].relative.clone();
            for line in hits.lines {
                let candidate_characters = line.text.chars().count();
                let entry = self.entries.len();
                candidates.push(FileRankCandidate {
                    entry,
                    path: path.clone(),
                    relative: relative.clone(),
                    text: line.text.clone(),
                    row: Some(line.row),
                    candidate_characters,
                    is_dir: false,
                });
                self.entries.push(FileEntry {
                    file,
                    row: Some(line.row),
                    column: line.column,
                    text: Some(line.text),
                    candidate_characters,
                });
            }
        }
        candidates
    }

    /// The file table index for `path`, adding it if this is its first line.
    /// Dense files can span many scanner batches, so the path index keeps each
    /// admission independent of the number of files already seen.
    fn intern(&mut self, path: PathBuf, is_dir: bool) -> u32 {
        if let Some(index) = self.path_files.get(&path) {
            return *index;
        }
        self.files.push(PickerFile {
            relative: path_text(path.strip_prefix(&self.root).unwrap_or(&path)),
            path: path.clone(),
            is_dir,
        });
        let index = self.files.len() as u32 - 1;
        self.path_files.insert(path, index);
        index
    }

    /// An entry with its file resolved.
    pub fn view(&self, entry: usize) -> Option<EntryView<'_>> {
        self.entries.get(entry).map(|entry| self.resolve(entry))
    }

    /// Reads an entry ranked against `scan`, which may be the corpus a
    /// content re-scan replaced.
    ///
    /// A row names the scan its index belongs to, so this is what lets rows
    /// stay on screen across a re-scan instead of blanking until the new walk
    /// has results. `None` once that corpus is gone, which is the same answer
    /// the scan-id guard has always given.
    pub fn view_in(&self, scan: u64, entry: usize) -> Option<EntryView<'_>> {
        if scan == self.scan_id {
            return self.view(entry);
        }
        let previous = self.previous.as_ref().filter(|held| held.scan == scan)?;
        previous
            .entries
            .get(entry)
            .map(|entry| resolve_in(&previous.files, entry))
    }

    /// Retires the replaced corpus once nothing reads it any more.
    pub(crate) fn forget_previous_corpus(&mut self) -> Option<impl Send + 'static> {
        self.previous
            .take()
            .map(|previous| (previous.files, previous.entries))
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

    pub(crate) fn background_rank_request(
        &self,
        finder: Option<crate::finder::FinderFileRankContext>,
    ) -> FileRankRequest {
        let (directory_only, query) = self.directory_only_query();
        FileRankRequest {
            scan_id: self.scan_id,
            query_revision: self.query_revision,
            query,
            directory_only,
            kind: self.kind,
            finder,
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
    pub(crate) fn restart_content_scan(&mut self, scan_id: u64) -> DiscardedPickerCorpus {
        let mut discarded = self.take_corpus();
        // The rows on screen were ranked against the table just taken, so it
        // stays readable under its own scan id until this walk answers for
        // the new one. Only one generation is kept: a reader who has typed
        // past two re-scans is no longer looking at the first one's rows.
        let previous = PreviousCorpus {
            scan: self.scan_id,
            files: std::mem::take(&mut discarded.files),
            entries: std::mem::take(&mut discarded.entries),
        };
        if let Some(older) = self.previous.replace(previous) {
            discarded.files = older.files;
            discarded.entries = older.entries;
        }
        self.scan_id = scan_id;
        self.scan_query = self.query.clone();
        self.selected = 0;
        self.selection_user_owned = false;
        self.loading = true;
        self.final_rank_pending = false;
        self.skipped = 0;
        self.limited = false;
        self.error = None;
        self.directory_only = false;
        discarded
    }

    fn take_corpus(&mut self) -> DiscardedPickerCorpus {
        DiscardedPickerCorpus {
            files: std::mem::take(&mut self.files),
            entries: std::mem::take(&mut self.entries),
            matches: std::mem::take(&mut self.matches),
            path_files: std::mem::take(&mut self.path_files),
            preview: self.preview.take(),
            scan_query: std::mem::take(&mut self.scan_query),
            error: self.error.take(),
            preview_request: self.preview_request.take(),
        }
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
        self.ranking = false;
        self.final_rank_pending = false;
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
        if self.ranking {
            return None;
        }
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

    pub(crate) fn insert_query_unranked(&mut self, character: char) {
        let byte = char_to_byte(&self.query, self.query_cursor);
        self.query.insert(byte, character);
        self.query_cursor += 1;
        self.note_query_changed();
    }

    pub(crate) fn insert_query_text_unranked(&mut self, text: &str) {
        let byte = char_to_byte(&self.query, self.query_cursor);
        self.query.insert_str(byte, text);
        self.query_cursor += text.chars().count();
        self.note_query_changed();
    }

    fn note_query_changed(&mut self) {
        self.query_revision = self.query_revision.wrapping_add(1);
        self.ranking = true;
        // A new query supersedes the flush a finished scan asked for: the
        // rank it starts covers every candidate the scan produced, and the
        // answer to the old revision will be discarded as stale. Holding the
        // gate open for a flush nothing will match again would leave the rows
        // inert for the rest of the picker's life.
        self.final_rank_pending = false;
        self.selected = 0;
        self.selection_user_owned = false;
        self.preview = None;
        self.preview_request = None;
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

    pub(crate) fn backspace_query_unranked(&mut self) {
        if self.query_cursor == 0 {
            return;
        }
        let from = char_to_byte(&self.query, self.query_cursor - 1);
        let to = char_to_byte(&self.query, self.query_cursor);
        self.query.replace_range(from..to, "");
        self.query_cursor -= 1;
        self.note_query_changed();
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

    pub(crate) fn delete_query_unranked(&mut self) {
        if self.query_cursor >= self.query.chars().count() {
            return;
        }
        let from = char_to_byte(&self.query, self.query_cursor);
        let to = char_to_byte(&self.query, self.query_cursor + 1);
        self.query.replace_range(from..to, "");
        self.note_query_changed();
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

    pub(crate) fn delete_query_word_unranked(&mut self) {
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
        self.note_query_changed();
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

    pub(crate) fn delete_query_end_unranked(&mut self) {
        if self.query_cursor >= self.query.chars().count() {
            return;
        }
        let start = char_to_byte(&self.query, self.query_cursor);
        self.query.truncate(start);
        self.note_query_changed();
    }

    pub(crate) fn apply_background_matches(
        &mut self,
        matches: Vec<FuzzyMatch>,
        positions: &[Option<usize>],
        complete: bool,
        flushed: bool,
    ) -> Vec<FuzzyMatch> {
        let selected = self
            .selection_user_owned
            .then(|| self.selected_match().map(|found| found.entry))
            .flatten();
        let old = std::mem::replace(&mut self.matches, matches);
        self.selected = selected
            .and_then(|entry| positions.get(entry).copied().flatten())
            .unwrap_or(0);
        if flushed {
            self.final_rank_pending = false;
        }
        if complete && !self.final_rank_pending {
            self.ranking = false;
        }
        old
    }

    /// Whether the answer still owed is that flush, which is the only one
    /// that can let the rows be read.
    pub(crate) fn awaiting_final_rank(&self) -> bool {
        self.final_rank_pending
    }

    /// Holds the rows inert until the ranker answers the flush that a
    /// finished scan asks for.
    pub(crate) fn begin_final_rank(&mut self) {
        self.ranking = true;
        self.final_rank_pending = true;
    }

    pub(crate) fn preview_request_id(&self) -> Option<u64> {
        self.preview_request.as_ref().map(|(request, _)| *request)
    }

    pub(crate) fn clear_preview_request(&mut self) {
        self.preview_request = None;
    }

    /// Starts a preview unless the exact destination and match spans are
    /// already in flight.
    ///
    /// A content query may keep the same file, row, and first matched column
    /// while an added term changes the rest of the emphasized positions. The
    /// spans therefore belong to preview identity just as much as the target;
    /// comparing only the target can leave an older, partial highlight in
    /// place after the current ranking arrives.
    pub(crate) fn begin_preview_request(
        &mut self,
        target: PickerTarget,
        content_match: Option<&(usize, Vec<usize>)>,
    ) -> Option<u64> {
        let key = FilePreviewKey {
            target,
            content_match: content_match.cloned(),
        };
        if self
            .preview_request
            .as_ref()
            .is_some_and(|(_, pending)| pending == &key)
        {
            return None;
        }
        let request = self.next_preview_request_id;
        self.next_preview_request_id = self.next_preview_request_id.wrapping_add(1).max(1);
        self.preview_request = Some((request, key));
        self.preview = None;
        Some(request)
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
/// Whitespace separates rather than matches. Each term is as loose as a lone
/// word and the terms are wanted in the order they were typed, which together
/// is one ordered subsequence of the terms run together — so `ab cd` accepts
/// what `abcd` accepts, and differs from it only in how the two stretches
/// score.
///
/// This decides exactly what `fuzzy_match` decides — the scorer reaches a
/// final state precisely when such a subsequence exists — but in one linear
/// pass that allocates nothing. That makes it the filter the content scanner
/// can afford to run over every line of a project, and the guard `fuzzy_match`
/// takes before building a dynamic-programming table it could never fill.
pub fn matches_fuzzy(query: &str, candidate: &str) -> bool {
    let case_sensitive = query.chars().any(char::is_uppercase);
    subsequence(
        query.split_whitespace().flat_map(str::chars),
        candidate,
        case_sensitive,
    )
}

/// Whether `query` occurs in `candidate` as an ordered subsequence.
///
/// Generic over how the query is spelled so the two callers can each use what
/// they already hold — `matches_fuzzy` the borrowed `&str` slices
/// `split_whitespace` hands it, and `FuzzyMatcher` its prepared characters —
/// without either allocating to ask the question.
///
/// Feeding it the terms one after another is what makes several words one
/// question: each term is consumed from where the last one ended, so the terms
/// are ordered and cannot overlap, and each is itself as loose as a lone word.
fn subsequence<I>(query: I, candidate: &str, case_sensitive: bool) -> bool
where
    I: IntoIterator<Item = char>,
{
    let mut characters = candidate.chars();
    'wanted: for wanted in query {
        for character in characters.by_ref() {
            if characters_match(character, wanted, case_sensitive) {
                continue 'wanted;
            }
        }
        return false;
    }
    true
}

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
    /// The characters the alignment searches for: the whitespace-separated
    /// terms of the query run together.
    ///
    /// Each term is matched as its own fuzzy subsequence and the terms are
    /// wanted in the order they were typed. Matching them that way is the same
    /// question as matching them run together, because an ordered subsequence
    /// of `ab` followed by one of `cd` is an ordered subsequence of `abcd`. So
    /// which candidates match is decided here, and the whitespace survives in
    /// `boundaries` below, which decides how they score.
    query: Vec<char>,
    /// Whether each character of `query` opens a term.
    ///
    /// The distance between two terms is not a gap. Whitespace is what someone
    /// types to say that two stretches are separate, so the alignment charges
    /// nothing for how far apart they land and pays nothing for their landing
    /// adjacent, where the same distance inside one term is penalized.
    boundaries: Vec<bool>,
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
        // Whitespace separates rather than matches, so neither the spaces
        // around a lone term nor those between several are characters a
        // candidate has to hold: `abc ` asks exactly what `abc` asks.
        let mut characters = Vec::new();
        let mut boundaries = Vec::new();
        for term in query.split_whitespace() {
            for (offset, character) in term.chars().enumerate() {
                characters.push(character);
                boundaries.push(offset == 0);
            }
        }
        // The basename bonuses compare a whole name against what was typed, so
        // they keep the query as typed, spaces and all. A file really named
        // `release notes.md` is what `release notes` should land on.
        let comparable_query = query.trim().to_owned();
        let query = characters;
        Self {
            kind,
            boundaries,
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
        subsequence(self.query.iter().copied(), candidate, self.case_sensitive)
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
            query,
            boundaries,
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
        let (alignment_score, positions) = score_one_term(
            query,
            boundaries,
            candidate_chars,
            case_sensitive,
            width,
            previous,
            current,
            prefix,
            parents,
            character_score,
        )?;

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

/// The alignment: the query as one fuzzy ordered subsequence.
///
/// Dynamic programming chooses the globally best alignment instead of greedily
/// committing each character. Gap penalties saturate after 32 characters, so
/// each state needs at most 31 nearby predecessors plus a prefix maximum for
/// every older one: O(query × candidate × 32), with the multiplier bounded
/// independently of candidate length.
///
/// `boundaries` marks the characters that open a term. A term boundary is the
/// one transition that costs nothing at any distance, because the whitespace
/// the person typed there says the two stretches are separate. Everywhere else
/// distance is a gap and is charged for.
#[allow(clippy::too_many_arguments)]
fn score_one_term(
    query: &[char],
    boundaries: &[bool],
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
            if boundaries[query_index] {
                // Opening a term: any predecessor will do and none is
                // preferred, so the best one anywhere earlier is taken at no
                // charge. Terms that land adjacent are not rewarded for it
                // either — the space said they were separate.
                if position > 0
                    && let Some((score, parent)) = prefix[position - 1]
                {
                    consider(score, parent);
                }
            } else {
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
    Ranked {
        scan_id: u64,
        query_revision: u64,
        matches: Vec<FuzzyMatch>,
        match_positions: Vec<Option<usize>>,
        finder_matches: Option<Vec<crate::finder::FinderMatch>>,
        finder_revision: Option<u64>,
        finder_positions: HashMap<crate::finder::FinderMatchSource, usize>,
        /// Whether this publish is the one the flush of a finished scan
        /// asked for, and so covers every candidate that scan produced.
        flushed: bool,
    },
    Preview {
        scan_id: u64,
        query_revision: u64,
        request_id: u64,
        preview: FilePreview,
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

#[derive(Clone, Debug)]
pub(crate) struct FileRankCandidate {
    entry: usize,
    path: PathBuf,
    relative: String,
    text: String,
    row: Option<usize>,
    candidate_characters: usize,
    is_dir: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct FileRankRequest {
    pub scan_id: u64,
    pub query_revision: u64,
    pub query: String,
    pub directory_only: bool,
    pub kind: FilePickerKind,
    pub finder: Option<crate::finder::FinderFileRankContext>,
}

#[derive(Debug)]
pub(crate) struct DiscardedPickerCorpus {
    files: Vec<PickerFile>,
    entries: Vec<FileEntry>,
    matches: Vec<FuzzyMatch>,
    path_files: HashMap<PathBuf, u32>,
    preview: Option<FilePreview>,
    scan_query: String,
    error: Option<String>,
    preview_request: Option<(u64, FilePreviewKey)>,
}

impl DiscardedPickerCorpus {
    fn discard(self) {
        let Self {
            files,
            entries,
            matches,
            path_files,
            preview,
            scan_query,
            error,
            preview_request,
        } = self;
        drop((
            files,
            entries,
            matches,
            path_files,
            preview,
            scan_query,
            error,
            preview_request,
        ));
    }
}

#[derive(Debug)]
pub(crate) struct FilePreviewRequest {
    pub scan_id: u64,
    pub query_revision: u64,
    pub request_id: u64,
    pub path: PathBuf,
    pub is_dir: bool,
    pub content_match: Option<(usize, Vec<usize>)>,
    pub show_hidden: bool,
}

#[derive(Debug)]
struct FilePreviewMailbox {
    pending: Mutex<Option<FilePreviewRequest>>,
    wake: sync_mpsc::SyncSender<()>,
}

impl FilePreviewMailbox {
    fn request(&self, request: FilePreviewRequest) {
        *self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(request);
        let _ = self.wake.try_send(());
    }
}

#[derive(Debug)]
struct DiscardedFileRankResult {
    matches: Vec<FuzzyMatch>,
    finder_matches: Vec<crate::finder::FinderMatch>,
    match_positions: Vec<Option<usize>>,
    finder_positions: HashMap<crate::finder::FinderMatchSource, usize>,
}

struct WorkerGarbage(Box<dyn Send>);

impl std::fmt::Debug for WorkerGarbage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WorkerGarbage(..)")
    }
}

#[derive(Debug, Default)]
struct PendingFileRankWork {
    reset: Option<(u64, FilePickerKind)>,
    add_scan_id: u64,
    candidates: Vec<FileRankCandidate>,
    query: Option<(u64, FileRankRequest)>,
    finder: Option<(u64, u64, Option<crate::finder::FinderFileRankContext>)>,
    flush: Option<u64>,
    discarded: Vec<DiscardedFileRankResult>,
    discarded_corpora: Vec<DiscardedPickerCorpus>,
    discarded_candidates: Vec<Vec<FileRankCandidate>>,
    discarded_queries: Vec<FileRankRequest>,
    discarded_finders: Vec<Option<crate::finder::FinderFileRankContext>>,
    garbage: Vec<WorkerGarbage>,
    close: bool,
}

#[derive(Debug)]
struct FileRankMailbox {
    pending: Mutex<PendingFileRankWork>,
    wake: sync_mpsc::SyncSender<()>,
}

impl FileRankMailbox {
    fn notify(&self) {
        let _ = self.wake.try_send(());
    }

    fn reset(&self, scan_id: u64, kind: FilePickerKind) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.reset = Some((scan_id, kind));
        // A newly opened picker supersedes a close that the worker has not
        // consumed yet. Keeping `close` set would make the worker discard this
        // reset and every coalesced candidate/query behind it.
        pending.close = false;
        pending.add_scan_id = scan_id;
        let candidates = std::mem::take(&mut pending.candidates);
        if !candidates.is_empty() {
            pending.discarded_candidates.push(candidates);
        }
        if let Some((_, query)) = pending.query.take() {
            pending.discarded_queries.push(query);
        }
        if let Some((_, _, finder)) = pending.finder.take() {
            pending.discarded_finders.push(finder);
        }
        pending.flush = None;
        drop(pending);
        self.notify();
    }

    fn add(&self, scan_id: u64, candidates: Vec<FileRankCandidate>) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if pending.add_scan_id != scan_id {
            pending.add_scan_id = scan_id;
            let discarded = std::mem::take(&mut pending.candidates);
            if !discarded.is_empty() {
                pending.discarded_candidates.push(discarded);
            }
        }
        pending.candidates.extend(candidates);
        drop(pending);
        self.notify();
    }

    fn query(&self, token: u64, request: FileRankRequest) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((_, discarded)) = pending.query.replace((token, request)) {
            pending.discarded_queries.push(discarded);
        }
        drop(pending);
        self.notify();
    }

    fn finder_context(
        &self,
        scan_id: u64,
        query_revision: u64,
        finder: Option<crate::finder::FinderFileRankContext>,
    ) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((pending_scan, pending_query, pending_finder)) = pending.finder.as_mut()
            && *pending_scan == scan_id
            && *pending_query == query_revision
        {
            match finder {
                Some(finder) => compose_finder_context(pending_finder, finder),
                None => *pending_finder = None,
            }
            drop(pending);
            self.notify();
            return;
        }
        if let Some((_, _, discarded)) = pending.finder.replace((scan_id, query_revision, finder)) {
            pending.discarded_finders.push(discarded);
        }
        drop(pending);
        self.notify();
    }

    fn flush(&self, scan_id: u64) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.flush = Some(scan_id);
        drop(pending);
        self.notify();
    }

    fn discard(&self, result: DiscardedFileRankResult) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.discarded.push(result);
        drop(pending);
        self.notify();
    }

    fn discard_corpus(&self, corpus: DiscardedPickerCorpus) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.discarded_corpora.push(corpus);
        drop(pending);
        self.notify();
    }

    fn discard_owned(&self, value: impl Send + 'static) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.garbage.push(WorkerGarbage(Box::new(value)));
        drop(pending);
        self.notify();
    }

    fn close(&self) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let candidates = std::mem::take(&mut pending.candidates);
        if !candidates.is_empty() {
            pending.discarded_candidates.push(candidates);
        }
        if let Some((_, query)) = pending.query.take() {
            pending.discarded_queries.push(query);
        }
        if let Some((_, _, finder)) = pending.finder.take() {
            pending.discarded_finders.push(finder);
        }
        pending.reset = None;
        pending.flush = None;
        pending.close = true;
        drop(pending);
        self.notify();
    }
}

struct FileRankState {
    scan_id: u64,
    kind: FilePickerKind,
    candidates: Vec<FileRankCandidate>,
    query_revision: u64,
    query: String,
    directory_only: bool,
    matches: Vec<FuzzyMatch>,
    finder: Option<crate::finder::FinderFileRankContext>,
    token: u64,
    ranked_candidates: usize,
}

impl Default for FileRankState {
    fn default() -> Self {
        Self {
            scan_id: 0,
            kind: FilePickerKind::Files,
            candidates: Vec::new(),
            query_revision: 0,
            query: String::new(),
            directory_only: false,
            matches: Vec::new(),
            finder: None,
            token: 0,
            ranked_candidates: 0,
        }
    }
}

fn close_file_rank_state(state: &mut FileRankState) {
    *state = FileRankState::default();
}

fn file_rank_worker(
    mailbox: Arc<FileRankMailbox>,
    wake: sync_mpsc::Receiver<()>,
    events: Sender<FilePickerEvent>,
    active_rank: Arc<AtomicU64>,
) {
    let mut state = FileRankState::default();
    while wake.recv().is_ok() {
        let mut pending = {
            let mut queued = mailbox
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            std::mem::take(&mut *queued)
        };
        for discarded in pending.discarded.drain(..) {
            drop((
                discarded.matches,
                discarded.finder_matches,
                discarded.match_positions,
                discarded.finder_positions,
            ));
        }
        for corpus in pending.discarded_corpora {
            corpus.discard();
        }
        for WorkerGarbage(value) in pending.garbage {
            drop(value);
        }
        drop((
            pending.discarded_candidates,
            pending.discarded_queries,
            pending.discarded_finders,
        ));
        if pending.close {
            close_file_rank_state(&mut state);
            continue;
        }
        if let Some((scan_id, kind)) = pending.reset {
            state = FileRankState {
                scan_id,
                kind,
                ..FileRankState::default()
            };
        }
        if pending.add_scan_id == state.scan_id && !pending.candidates.is_empty() {
            let first = state.candidates.len();
            debug_assert!(
                pending
                    .candidates
                    .iter()
                    .enumerate()
                    .all(|(offset, candidate)| candidate.entry == first + offset)
            );
            state.candidates.append(&mut pending.candidates);
        }

        let mut publish = false;
        if let Some((token, request)) = pending.query
            && request.scan_id == state.scan_id
        {
            state.token = token;
            state.kind = request.kind;
            state.query_revision = request.query_revision;
            state.query = request.query;
            state.directory_only = request.directory_only;
            state.finder = None;
            if let Some(finder) = request.finder {
                apply_finder_context(&mut state.finder, finder);
            }
            let Some(mut matches) = rank_file_candidates(
                &state.candidates,
                &state.query,
                state.directory_only,
                state.kind,
                || active_rank.load(Ordering::Acquire) != token,
            ) else {
                continue;
            };
            if active_rank.load(Ordering::Acquire) != token {
                continue;
            }
            sort_file_matches(&mut matches, &state.candidates, &state.query);
            if active_rank.load(Ordering::Acquire) != token {
                continue;
            }
            state.matches = matches;
            state.ranked_candidates = state.candidates.len();
            publish = true;
        }
        if let Some((scan_id, query_revision, finder)) = pending.finder
            && scan_id == state.scan_id
            && query_revision == state.query_revision
        {
            match finder {
                Some(finder) => apply_finder_context(&mut state.finder, finder),
                None => state.finder = None,
            }
            publish = true;
        }
        let flush = pending.flush == Some(state.scan_id);
        if state.ranked_candidates < state.candidates.len()
            && (flush || state.candidates.len() - state.ranked_candidates >= RANK_PUBLISH_BATCH)
        {
            let first = state.ranked_candidates;
            let mut additions = rank_file_candidates(
                &state.candidates[first..],
                &state.query,
                state.directory_only,
                state.kind,
                || false,
            )
            .unwrap_or_default();
            sort_file_matches(&mut additions, &state.candidates, &state.query);
            merge_file_matches(
                &mut state.matches,
                additions,
                &state.candidates,
                &state.query,
            );
            state.ranked_candidates = state.candidates.len();
            publish = true;
        } else if flush {
            publish = true;
        }
        if !publish {
            continue;
        }
        let (finder_matches, finder_revision, finder_positions) = combined_finder_matches(&state);
        let mut match_positions = vec![None; state.candidates.len()];
        for (position, found) in state.matches.iter().enumerate() {
            match_positions[found.entry] = Some(position);
        }
        let _ = events.blocking_send(FilePickerEvent::Ranked {
            scan_id: state.scan_id,
            query_revision: state.query_revision,
            matches: state.matches.clone(),
            match_positions,
            finder_matches,
            finder_revision,
            finder_positions,
            flushed: flush,
        });
    }
}

fn file_preview_worker(
    mailbox: Arc<FilePreviewMailbox>,
    wake: sync_mpsc::Receiver<()>,
    events: Sender<FilePickerEvent>,
    active: Arc<AtomicU64>,
) {
    while wake.recv().is_ok() {
        let Some(request) = mailbox
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        else {
            continue;
        };
        let FilePreviewRequest {
            scan_id,
            query_revision,
            request_id,
            path,
            is_dir,
            content_match,
            show_hidden,
        } = request;
        let preview = if is_dir {
            FilePreview::from_directory(&path, show_hidden)
        } else if let Some((row, emphasis)) = content_match {
            FilePreview::snippet_from_path(&path, row, emphasis)
        } else {
            FilePreview::from_path(&path)
        };
        if active.load(Ordering::Acquire) == request_id {
            let _ = events.blocking_send(FilePickerEvent::Preview {
                scan_id,
                query_revision,
                request_id,
                preview,
            });
        }
    }
}

fn rank_file_candidates(
    candidates: &[FileRankCandidate],
    query: &str,
    directory_only: bool,
    kind: FilePickerKind,
    mut cancelled: impl FnMut() -> bool,
) -> Option<Vec<FuzzyMatch>> {
    let candidate_kind = match kind {
        FilePickerKind::Files => FuzzyCandidate::Path,
        FilePickerKind::Contents => FuzzyCandidate::Line,
    };
    let mut matcher = FuzzyMatcher::for_candidate(query, candidate_kind);
    let mut matches = Vec::new();
    for (offset, candidate) in candidates.iter().enumerate() {
        if offset % 128 == 0 && cancelled() {
            return None;
        }
        if directory_only && !candidate.is_dir {
            continue;
        }
        if let Some((score, positions)) = matcher.score(&candidate.text) {
            matches.push(FuzzyMatch {
                entry: candidate.entry,
                score,
                positions,
            });
        }
    }
    Some(matches)
}

fn file_match_order(
    left: &FuzzyMatch,
    right: &FuzzyMatch,
    candidates: &[FileRankCandidate],
    query: &str,
) -> std::cmp::Ordering {
    let left_entry = &candidates[left.entry];
    let right_entry = &candidates[right.entry];
    if query.is_empty() {
        return (&left_entry.relative, left_entry.row)
            .cmp(&(&right_entry.relative, right_entry.row));
    }
    right
        .score
        .cmp(&left.score)
        .then_with(|| {
            left_entry
                .candidate_characters
                .cmp(&right_entry.candidate_characters)
        })
        .then_with(|| {
            (&left_entry.relative, left_entry.row).cmp(&(&right_entry.relative, right_entry.row))
        })
}

fn sort_file_matches(matches: &mut [FuzzyMatch], candidates: &[FileRankCandidate], query: &str) {
    matches.sort_by(|left, right| file_match_order(left, right, candidates, query));
}

fn merge_file_matches(
    current: &mut Vec<FuzzyMatch>,
    additions: Vec<FuzzyMatch>,
    candidates: &[FileRankCandidate],
    query: &str,
) {
    if additions.is_empty() {
        return;
    }
    let mut existing = std::mem::take(current).into_iter().peekable();
    let mut incoming = additions.into_iter().peekable();
    let mut merged = Vec::with_capacity(existing.len() + incoming.len());
    while let (Some(left), Some(right)) = (existing.peek(), incoming.peek()) {
        if file_match_order(left, right, candidates, query).is_le() {
            merged.push(existing.next().unwrap());
        } else {
            merged.push(incoming.next().unwrap());
        }
    }
    merged.extend(existing);
    merged.extend(incoming);
    *current = merged;
}

fn combined_finder_matches(
    state: &FileRankState,
) -> (
    Option<Vec<crate::finder::FinderMatch>>,
    Option<u64>,
    HashMap<crate::finder::FinderMatchSource, usize>,
) {
    let Some(finder) = state.finder.as_ref() else {
        return (None, None, HashMap::new());
    };
    let mut matches = state
        .matches
        .iter()
        .filter(|found| {
            !finder
                .suppressed_paths
                .contains(&state.candidates[found.entry].path)
        })
        .map(|found| crate::finder::FinderMatch {
            source: crate::finder::FinderMatchSource::File(found.entry),
            emphasis: found.positions.clone(),
            detail_emphasis: Vec::new(),
            score: found.score,
            type_boost: finder.file_boost,
        })
        .chain(
            finder
                .resource_matches
                .iter()
                .map(crate::finder::resource_to_finder_match),
        )
        .collect::<Vec<_>>();
    if finder.sort {
        matches.sort_by(crate::finder::finder_match_order);
    }
    if state.kind == FilePickerKind::Contents && matches.len() > CONTENT_ENTRY_LIMIT {
        matches.truncate(CONTENT_ENTRY_LIMIT);
    }
    let positions = matches
        .iter()
        .enumerate()
        .map(|(position, found)| (found.source, position))
        .collect();
    (Some(matches), Some(finder.revision), positions)
}

fn apply_finder_context(
    current: &mut Option<crate::finder::FinderFileRankContext>,
    mut incoming: crate::finder::FinderFileRankContext,
) {
    if incoming.replace_resources || current.is_none() {
        if !incoming.sort {
            incoming.resource_matches.sort_by_key(|found| found.item);
        }
        incoming.replace_resources = false;
        incoming.removed_resources.clear();
        incoming.remap_resources = None;
        *current = Some(incoming);
        return;
    }
    apply_finder_delta(current.as_mut().expect("checked as present"), incoming);
}

fn apply_finder_delta(
    current: &mut crate::finder::FinderFileRankContext,
    mut incoming: crate::finder::FinderFileRankContext,
) {
    if let Some(remap) = incoming.remap_resources.take() {
        current.resource_matches.retain_mut(|found| {
            let Some(item) = remap.get(found.item).copied().flatten() else {
                return false;
            };
            found.item = item;
            true
        });
    }
    if !incoming.removed_resources.is_empty() {
        incoming.removed_resources.sort_unstable();
        incoming.removed_resources.dedup();
        current.resource_matches.retain(|found| {
            incoming
                .removed_resources
                .binary_search(&found.item)
                .is_err()
        });
    }
    current
        .resource_matches
        .append(&mut incoming.resource_matches);
    current.revision = incoming.revision;
    current.suppressed_paths = incoming.suppressed_paths;
    current.file_boost = incoming.file_boost;
    current.sort = incoming.sort;
    if !current.sort {
        current.resource_matches.sort_by_key(|found| found.item);
    }
}

fn compose_finder_context(
    current: &mut Option<crate::finder::FinderFileRankContext>,
    mut incoming: crate::finder::FinderFileRankContext,
) {
    if incoming.replace_resources || current.is_none() {
        *current = Some(incoming);
        return;
    }
    let current = current.as_mut().expect("checked as present");
    if current.replace_resources {
        apply_finder_delta(current, incoming);
        current.replace_resources = true;
        return;
    }
    if let Some(next_remap) = incoming.remap_resources.take() {
        for found in &mut current.resource_matches {
            let Some(item) = next_remap.get(found.item).copied().flatten() else {
                found.item = usize::MAX;
                continue;
            };
            found.item = item;
        }
        current
            .resource_matches
            .retain(|found| found.item != usize::MAX);
        current.removed_resources = current
            .removed_resources
            .drain(..)
            .filter_map(|item| next_remap.get(item).copied().flatten())
            .collect();
        current.remap_resources = Some(match current.remap_resources.take() {
            Some(previous) => previous
                .into_iter()
                .map(|item| item.and_then(|item| next_remap.get(item).copied().flatten()))
                .collect(),
            None => next_remap,
        });
    }
    if !incoming.removed_resources.is_empty() {
        incoming.removed_resources.sort_unstable();
        incoming.removed_resources.dedup();
        current.resource_matches.retain(|found| {
            incoming
                .removed_resources
                .binary_search(&found.item)
                .is_err()
        });
    }
    current
        .removed_resources
        .append(&mut incoming.removed_resources);
    current
        .resource_matches
        .append(&mut incoming.resource_matches);
    current.revision = incoming.revision;
    current.suppressed_paths = incoming.suppressed_paths;
    current.file_boost = incoming.file_boost;
    current.sort = incoming.sort;
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
    active_rank: Arc<AtomicU64>,
    next_rank: Arc<AtomicU64>,
    active_preview: Arc<AtomicU64>,
    rank_commands: Arc<OnceLock<Option<Arc<FileRankMailbox>>>>,
    preview_commands: Arc<OnceLock<Option<Arc<FilePreviewMailbox>>>>,
    events: Sender<FilePickerEvent>,
}

impl FileScanner {
    fn rank_mailbox(&self, scan_id: u64) -> Option<&Arc<FileRankMailbox>> {
        self.rank_commands
            .get_or_init(|| {
                let (wake, receiver) = sync_mpsc::sync_channel(1);
                let mailbox = Arc::new(FileRankMailbox {
                    pending: Mutex::new(PendingFileRankWork::default()),
                    wake,
                });
                let events = self.events.clone();
                let rank_events = self.events.clone();
                let rank_cancellation = self.active_rank.clone();
                let worker_mailbox = mailbox.clone();
                match thread::Builder::new()
                    .name("runyte-file-rank".to_owned())
                    .spawn(move || {
                        file_rank_worker(worker_mailbox, receiver, rank_events, rank_cancellation);
                    }) {
                    Ok(_) => Some(mailbox),
                    Err(error) => {
                        let _ = events.try_send(FilePickerEvent::Failed {
                            scan_id,
                            message: format!("failed to start file ranker: {error}"),
                        });
                        None
                    }
                }
            })
            .as_ref()
    }

    fn reset_ranker(&self, scan_id: u64, kind: FilePickerKind) {
        self.active_rank.store(
            self.next_rank.fetch_add(1, Ordering::Relaxed),
            Ordering::Release,
        );
        if let Some(mailbox) = self.rank_mailbox(scan_id) {
            mailbox.reset(scan_id, kind);
        }
    }

    pub(crate) fn add_rank_candidates(&self, scan_id: u64, candidates: Vec<FileRankCandidate>) {
        if !candidates.is_empty()
            && let Some(mailbox) = self.rank_mailbox(scan_id)
        {
            mailbox.add(scan_id, candidates);
        }
    }

    pub(crate) fn rank(&self, request: FileRankRequest) {
        let token = self.next_rank.fetch_add(1, Ordering::Relaxed);
        self.active_rank.store(token, Ordering::Release);
        if let Some(mailbox) = self.rank_mailbox(request.scan_id) {
            mailbox.query(token, request);
        }
    }

    pub(crate) fn update_finder_context(
        &self,
        scan_id: u64,
        query_revision: u64,
        finder: Option<crate::finder::FinderFileRankContext>,
    ) {
        if let Some(mailbox) = self.rank_mailbox(scan_id) {
            mailbox.finder_context(scan_id, query_revision, finder);
        }
    }

    pub(crate) fn flush_rank(&self, scan_id: u64) {
        if let Some(mailbox) = self.rank_mailbox(scan_id) {
            mailbox.flush(scan_id);
        }
    }

    pub(crate) fn discard_rank_result(
        &self,
        matches: Vec<FuzzyMatch>,
        finder_matches: Vec<crate::finder::FinderMatch>,
        match_positions: Vec<Option<usize>>,
        finder_positions: HashMap<crate::finder::FinderMatchSource, usize>,
    ) {
        if let Some(mailbox) = self.rank_commands.get().and_then(Option::as_ref) {
            mailbox.discard(DiscardedFileRankResult {
                matches,
                finder_matches,
                match_positions,
                finder_positions,
            });
        }
    }

    pub(crate) fn discard_picker_corpus(&self, corpus: DiscardedPickerCorpus) {
        if let Some(mailbox) = self.rank_commands.get().and_then(Option::as_ref) {
            mailbox.discard_corpus(corpus);
        } else {
            corpus.discard();
        }
    }

    pub(crate) fn discard_owned(&self, value: impl Send + 'static) {
        if let Some(mailbox) = self.rank_commands.get().and_then(Option::as_ref) {
            mailbox.discard_owned(value);
        } else {
            drop(value);
        }
    }

    pub(crate) fn close_ranker(&self) {
        self.active_rank.store(
            self.next_rank.fetch_add(1, Ordering::Relaxed),
            Ordering::Release,
        );
        if let Some(mailbox) = self.rank_commands.get().and_then(Option::as_ref) {
            mailbox.close();
        }
    }

    pub(crate) fn preview(&self, request: FilePreviewRequest) {
        self.active_preview
            .store(request.request_id, Ordering::Release);
        let commands = self.preview_commands.get_or_init(|| {
            let (wake, receiver) = sync_mpsc::sync_channel(1);
            let mailbox = Arc::new(FilePreviewMailbox {
                pending: Mutex::new(None),
                wake,
            });
            let events = self.events.clone();
            let preview_events = self.events.clone();
            let active = self.active_preview.clone();
            let worker_mailbox = mailbox.clone();
            match thread::Builder::new()
                .name("runyte-file-preview".to_owned())
                .spawn(move || {
                    file_preview_worker(worker_mailbox, receiver, preview_events, active);
                }) {
                Ok(_) => Some(mailbox),
                Err(error) => {
                    let _ = events.try_send(FilePickerEvent::Preview {
                        scan_id: request.scan_id,
                        query_revision: request.query_revision,
                        request_id: request.request_id,
                        preview: FilePreview::Unreadable(format!(
                            "failed to start file previewer: {error}"
                        )),
                    });
                    None
                }
            }
        });
        if let Some(mailbox) = commands {
            mailbox.request(request);
        }
    }

    pub fn scan(
        &self,
        scan_id: u64,
        root: PathBuf,
        scope: ScanScope,
        state_root: PathBuf,
        show_hidden: bool,
    ) {
        self.reset_ranker(scan_id, FilePickerKind::Files);
        self.active.store(scan_id, Ordering::Release);
        let active = self.active.clone();
        let events = self.events.clone();
        let failure_events = self.events.clone();
        if let Err(error) = thread::Builder::new()
            .name("runyte-file-scan".to_owned())
            .spawn(move || {
                let result = scan_with(
                    &root,
                    &scope,
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
        scope: ScanScope,
        state_root: PathBuf,
        show_hidden: bool,
        query: String,
    ) {
        self.reset_ranker(scan_id, FilePickerKind::Contents);
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
                    &scope,
                    &state_root,
                    show_hidden,
                    false,
                    || active.load(Ordering::Acquire) != scan_id,
                    |paths| {
                        let mut entries = Vec::<FileHits>::new();
                        let mut batch = 0usize;
                        let mut admitted = 0usize;
                        for path in paths {
                            if active.load(Ordering::Acquire) != scan_id {
                                return false;
                            }
                            let Some(mut hits) = content_entries(&path.path, &query) else {
                                continue;
                            };
                            if hits.truncate(CONTENT_ENTRY_LIMIT - emitted - admitted) {
                                limited = true;
                            }
                            admitted += hits.len();
                            let hit_path = hits.path;
                            let mut lines = hits.lines.into_iter();
                            loop {
                                let available = SCAN_BATCH - batch;
                                let chunk = lines.by_ref().take(available).collect::<Vec<_>>();
                                if chunk.is_empty() {
                                    break;
                                }
                                batch += chunk.len();
                                entries.push(FileHits {
                                    path: hit_path.clone(),
                                    lines: chunk,
                                });
                                if batch == SCAN_BATCH {
                                    if events
                                        .blocking_send(FilePickerEvent::Content {
                                            scan_id,
                                            entries: std::mem::take(&mut entries),
                                        })
                                        .is_err()
                                    {
                                        return false;
                                    }
                                    batch = 0;
                                }
                            }
                            if limited {
                                break;
                            }
                        }
                        emitted += admitted;
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
        self.active_rank.store(
            self.next_rank.fetch_add(1, Ordering::Relaxed),
            Ordering::Release,
        );
        self.active_preview.store(0, Ordering::Release);
    }
}

pub fn scanner() -> (FileScanner, Receiver<FilePickerEvent>) {
    let (events, receiver) = channel(16);
    let active_rank = Arc::new(AtomicU64::new(0));
    let next_rank = Arc::new(AtomicU64::new(1));
    (
        FileScanner {
            active: Arc::new(AtomicU64::new(0)),
            active_rank,
            next_rank,
            active_preview: Arc::new(AtomicU64::new(0)),
            rank_commands: Arc::new(OnceLock::new()),
            preview_commands: Arc::new(OnceLock::new()),
            events,
        },
        receiver,
    )
}

/// Synchronous seam used by isolated tests and by non-TUI embedders.
pub fn scan_files(
    root: &Path,
    scope: &ScanScope,
    state_root: &Path,
    show_hidden: bool,
) -> Result<(Vec<ScanEntry>, usize)> {
    let mut paths = Vec::new();
    let skipped = scan_with(
        root,
        scope,
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
    scope: &ScanScope,
    state_root: &Path,
    show_hidden: bool,
    query: &str,
) -> Result<(Vec<FileHits>, usize, bool)> {
    let mut files = Vec::new();
    let mut lines = 0;
    let mut limited = false;
    let skipped = scan_with(
        root,
        scope,
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
    let column = without_trailing
        .chars()
        .take_while(|character| character.is_whitespace())
        .count();
    line_hit_from_trimmed(trimmed, query, column)
}

pub(crate) fn line_hit_from_trimmed(trimmed: &str, query: &str, column: usize) -> Option<LineHit> {
    let trimmed = trimmed.trim_end();
    // The ranked candidate is the truncated line, so the filter has to read
    // the same text: a query matched against the tail of a very long line
    // would produce an entry nothing later can highlight.
    let text = match trimmed.char_indices().nth(GREP_LINE_CHARACTERS) {
        Some((byte, _)) => &trimmed[..byte],
        None => trimmed,
    };
    (!text.is_empty() && matches_fuzzy(query, text)).then(|| LineHit {
        row: 0,
        column,
        text: text.to_owned(),
    })
}

fn scan_with(
    root: &Path,
    scope: &ScanScope,
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
    let respect_ignore_files = matches!(scope, ScanScope::Ignoring { .. });
    let mut skipped = 0;
    let mut inherited = Vec::<IgnoreRule>::new();
    // A path is matched against the rules of the directory that stated them,
    // so it is spelled relative to where inheritance began. A scan that reads
    // no rules has nothing to be relative to.
    let mut root_relative = PathBuf::new();
    if let ScanScope::Ignoring { from } = scope {
        let ignore_root = from.canonicalize().unwrap_or_else(|_| root.clone());
        let ignore_root = if root.starts_with(&ignore_root) {
            ignore_root
        } else {
            root.clone()
        };
        root_relative = root
            .strip_prefix(&ignore_root)
            .expect("the effective ignore root contains the picker root")
            .to_path_buf();
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
    }
    let mut pending = vec![(root.clone(), root_relative, inherited)];
    let mut batch = Vec::with_capacity(SCAN_BATCH);
    while let Some((directory, relative_directory, mut rules)) = pending.pop() {
        if cancelled() {
            return Ok(skipped);
        }
        if respect_ignore_files {
            read_ignore_files(&directory, &relative_directory, &mut rules, &mut skipped);
        }
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
                || (respect_ignore_files && ignored(&rules, &relative, is_directory))
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
    use std::{collections::HashSet, fs, path::Path};

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
        let mut picker = FilePicker::new(1, root.clone(), ScanScope::ignoring(&root));
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

        let (paths, skipped) =
            scan_files(&root, &ScanScope::ignoring(&root), &workspace, false).unwrap();
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

    /// `ScanScope::Everything` drops the ignore files and nothing else. The
    /// reserved names, the workspace state directory, symlinks, and the
    /// hidden-file rule are separate filters and all still apply, so the same
    /// fixture gains exactly the entries its `.gitignore` and `.ignore` had
    /// been excluding.
    #[test]
    fn an_unfiltered_scan_drops_the_ignore_files_and_no_other_filter() {
        let root = temporary("unfiltered");
        let workspace = root.join(".runyte");
        fs::create_dir_all(root.join("src/generated")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(
            root.join(".gitignore"),
            "target/\n*.log\nsrc/generated/**\n",
        )
        .unwrap();
        fs::write(root.join("src/.gitignore"), "*.tmp\n").unwrap();
        for path in [
            "src/main.rs",
            "src/drop.tmp",
            "src/generated/code.rs",
            "drop.log",
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

        let (paths, skipped) =
            scan_files(&root, &ScanScope::Everything, &workspace, false).unwrap();
        let relative = paths
            .iter()
            .map(|entry| entry.path.strip_prefix(&root).unwrap().to_path_buf())
            .collect::<Vec<_>>();
        assert_eq!(skipped, 0);
        for found in [
            "src/main.rs",
            "src/drop.tmp",
            "src/generated/code.rs",
            "drop.log",
            "target",
            "target/build",
        ] {
            assert!(
                relative.iter().any(|path| path == Path::new(found)),
                "an unfiltered scan keeps {found}: {relative:?}"
            );
        }
        for omitted in [".secret", ".runyte/state", ".gitignore"] {
            assert!(
                !relative.iter().any(|path| path == Path::new(omitted)),
                "{omitted} is excluded by a filter the scope does not touch: {relative:?}"
            );
        }
        #[cfg(unix)]
        assert!(!relative.iter().any(|path| path == Path::new("link.rs")));
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

        let (paths, _) =
            scan_files(&root, &ScanScope::ignoring(&project), &workspace, true).unwrap();
        assert_eq!(paths, vec![ScanEntry::file(root.join("main.rs"))]);
        assert!(scan_files(&workspace, &ScanScope::ignoring(&project), &workspace, true).is_err());
        assert!(
            scan_files(
                &project.join(".git/objects"),
                &ScanScope::ignoring(&project),
                &workspace,
                true
            )
            .is_err()
        );
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
            &ScanScope::ignoring(&root),
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
        let mut picker = FilePicker::new(1, root.clone(), ScanScope::ignoring(&root));
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
    fn editing_the_query_releases_the_wait_for_a_finished_scan_flush() {
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::new(1, root.clone(), ScanScope::ignoring(&root));
        picker.add_paths(vec![ScanEntry::file(root.join("alpha.rs"))]);
        let matches = picker.matches.clone();
        picker.begin_final_rank();
        picker.insert_query_unranked('a');

        // The flush answers the revision that is now stale, so nothing will
        // ever carry it. The rank this edit starts is complete on its own.
        let positions = vec![Some(0)];
        picker.apply_background_matches(matches, &positions, true, false);

        assert!(!picker.ranking);
        assert!(picker.selected_target().is_some());
    }

    #[test]
    fn deferred_query_edit_never_ranks_the_candidate_table() {
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::new(1, root.clone(), ScanScope::ignoring(&root));
        picker.add_paths(vec![ScanEntry::file(root.join("alpha.rs"))]);
        assert_eq!(picker.matches.len(), 1);

        picker.insert_query_unranked('z');

        assert_eq!(picker.query, "z");
        assert_eq!(picker.query_revision, 1);
        assert!(picker.ranking);
        assert_eq!(
            picker.matches.len(),
            1,
            "the old immutable result stays visible until the worker answers"
        );
    }

    #[test]
    fn background_ranker_answers_the_latest_query_revision() {
        let (scanner, mut events) = scanner();
        let root = PathBuf::from("/project");
        scanner.reset_ranker(7, FilePickerKind::Files);
        let mut picker = FilePicker::new(7, root.clone(), ScanScope::ignoring(&root));
        let candidates = picker.add_paths_unranked(vec![
            ScanEntry::file(root.join("alpha.rs")),
            ScanEntry::file(root.join("beta.rs")),
        ]);
        scanner.add_rank_candidates(7, candidates);
        picker.insert_query_unranked('b');
        scanner.rank(picker.background_rank_request(None));

        let ranked = loop {
            let event = events.blocking_recv().unwrap();
            if let FilePickerEvent::Ranked {
                scan_id: 7,
                query_revision: 1,
                matches,
                ..
            } = event
            {
                break matches;
            }
        };
        assert_eq!(ranked.len(), 1);
        assert_eq!(picker.view(ranked[0].entry).unwrap().relative, "beta.rs");
    }

    #[test]
    fn blocked_ranker_coalesces_scan_batches_and_keeps_only_the_latest_query() {
        let (wake, receiver) = sync_mpsc::sync_channel(1);
        let mailbox = FileRankMailbox {
            pending: Mutex::new(PendingFileRankWork::default()),
            wake,
        };
        let root = PathBuf::from("/project");
        for entry in 0..1_000 {
            let relative = format!("file-{entry}.rs");
            mailbox.add(
                9,
                vec![FileRankCandidate {
                    entry,
                    path: root.join(&relative),
                    text: relative.clone(),
                    relative,
                    row: None,
                    candidate_characters: 12,
                    is_dir: false,
                }],
            );
        }
        for query_revision in 1..=20 {
            mailbox.query(
                query_revision,
                FileRankRequest {
                    scan_id: 9,
                    query_revision,
                    query: format!("query-{query_revision}"),
                    directory_only: false,
                    kind: FilePickerKind::Files,
                    finder: None,
                },
            );
        }

        let pending = mailbox.pending.lock().unwrap();
        assert_eq!(pending.candidates.len(), 1_000);
        assert_eq!(pending.query.as_ref().unwrap().1.query_revision, 20);
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_err(), "only one wake may queue");
    }

    #[test]
    fn rank_mailbox_retires_replaced_payloads_for_worker_side_drop() {
        let (wake, _receiver) = sync_mpsc::sync_channel(1);
        let mailbox = FileRankMailbox {
            pending: Mutex::new(PendingFileRankWork::default()),
            wake,
        };
        let request = |revision| FileRankRequest {
            scan_id: 3,
            query_revision: revision,
            query: "x".repeat(8_192),
            directory_only: false,
            kind: FilePickerKind::Files,
            finder: None,
        };
        mailbox.add(
            3,
            vec![FileRankCandidate {
                entry: 0,
                path: PathBuf::from("/project/large"),
                relative: "large".to_owned(),
                text: "large".to_owned(),
                row: None,
                candidate_characters: 5,
                is_dir: false,
            }],
        );
        mailbox.query(1, request(1));
        mailbox.query(2, request(2));
        mailbox.reset(4, FilePickerKind::Contents);

        let pending = mailbox.pending.lock().unwrap();
        assert_eq!(pending.discarded_queries.len(), 2);
        assert_eq!(pending.discarded_candidates.len(), 1);
        assert!(pending.query.is_none());
        assert!(pending.candidates.is_empty());
    }

    #[test]
    fn reset_supersedes_a_pending_ranker_close() {
        let (wake, _receiver) = sync_mpsc::sync_channel(1);
        let mailbox = FileRankMailbox {
            pending: Mutex::new(PendingFileRankWork::default()),
            wake,
        };
        mailbox.close();
        mailbox.reset(12, FilePickerKind::Files);
        mailbox.add(
            12,
            vec![FileRankCandidate {
                entry: 0,
                path: PathBuf::from("/project/reopened.rs"),
                relative: "reopened.rs".to_owned(),
                text: "reopened.rs".to_owned(),
                row: None,
                candidate_characters: 11,
                is_dir: false,
            }],
        );
        mailbox.query(
            4,
            FileRankRequest {
                scan_id: 12,
                query_revision: 1,
                query: "reopen".to_owned(),
                directory_only: false,
                kind: FilePickerKind::Files,
                finder: None,
            },
        );

        let pending = mailbox.pending.lock().unwrap();
        assert!(!pending.close);
        assert_eq!(pending.reset, Some((12, FilePickerKind::Files)));
        assert_eq!(pending.candidates.len(), 1);
        assert_eq!(pending.query.as_ref().unwrap().1.query_revision, 1);
    }

    #[test]
    fn worker_finder_context_applies_resource_remaps_and_deltas() {
        let context = |revision, items: &[usize], replace_resources, remap_resources| {
            crate::finder::FinderFileRankContext {
                revision,
                resource_matches: items
                    .iter()
                    .map(|item| crate::finder::ResourceMatch {
                        item: *item,
                        emphasis: vec![*item],
                        detail_emphasis: Vec::new(),
                        score: *item as i64,
                        type_boost: false,
                    })
                    .collect(),
                replace_resources,
                removed_resources: Vec::new(),
                remap_resources,
                suppressed_paths: Arc::new(HashSet::new()),
                file_boost: false,
                sort: true,
            }
        };
        let mut current = None;
        apply_finder_context(&mut current, context(1, &[0, 1], true, None));
        apply_finder_context(
            &mut current,
            context(2, &[1], false, Some(vec![None, Some(0)])),
        );

        let current = current.unwrap();
        assert_eq!(current.revision, 2);
        assert_eq!(
            current
                .resource_matches
                .iter()
                .map(|found| found.item)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn worker_composes_multiple_empty_query_resource_replacements_in_source_order() {
        let resource = |item, score| crate::finder::ResourceMatch {
            item,
            emphasis: Vec::new(),
            detail_emphasis: Vec::new(),
            score,
            type_boost: false,
        };
        let context =
            |revision, matches, removed, replace_resources| crate::finder::FinderFileRankContext {
                revision,
                resource_matches: matches,
                replace_resources,
                removed_resources: removed,
                remap_resources: None,
                suppressed_paths: Arc::new(HashSet::new()),
                file_boost: false,
                sort: false,
            };
        let mut materialized = None;
        apply_finder_context(
            &mut materialized,
            context(
                1,
                vec![resource(0, 0), resource(1, 1), resource(2, 2)],
                Vec::new(),
                true,
            ),
        );
        let mut pending = Some(context(2, vec![resource(1, 20)], vec![1], false));
        compose_finder_context(
            &mut pending,
            context(3, vec![resource(2, 30)], vec![2], false),
        );
        apply_finder_context(&mut materialized, pending.unwrap());

        let materialized = materialized.unwrap();
        assert_eq!(
            materialized
                .resource_matches
                .iter()
                .map(|found| (found.item, found.score))
                .collect::<Vec<_>>(),
            vec![(0, 0), (1, 20), (2, 30)]
        );
    }

    #[test]
    fn worker_composition_drops_superseded_pending_additions() {
        let resource = |item, score| crate::finder::ResourceMatch {
            item,
            emphasis: Vec::new(),
            detail_emphasis: Vec::new(),
            score,
            type_boost: false,
        };
        let context =
            |revision, matches, removed, replace_resources| crate::finder::FinderFileRankContext {
                revision,
                resource_matches: matches,
                replace_resources,
                removed_resources: removed,
                remap_resources: None,
                suppressed_paths: Arc::new(HashSet::new()),
                file_boost: false,
                sort: false,
            };
        let seeded = || {
            let mut materialized = None;
            apply_finder_context(
                &mut materialized,
                context(1, vec![resource(0, 0), resource(1, 1)], Vec::new(), true),
            );
            materialized
        };

        let mut pending = Some(context(2, vec![resource(2, 20)], Vec::new(), false));
        compose_finder_context(&mut pending, context(3, Vec::new(), vec![2], false));
        let mut materialized = seeded();
        apply_finder_context(&mut materialized, pending.unwrap());
        assert_eq!(
            materialized
                .unwrap()
                .resource_matches
                .iter()
                .map(|found| found.item)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );

        let mut pending = Some(context(2, vec![resource(1, 20)], Vec::new(), false));
        compose_finder_context(
            &mut pending,
            context(3, vec![resource(1, 30)], vec![1], false),
        );
        let mut materialized = seeded();
        apply_finder_context(&mut materialized, pending.unwrap());
        assert_eq!(
            materialized
                .unwrap()
                .resource_matches
                .iter()
                .map(|found| (found.item, found.score))
                .collect::<Vec<_>>(),
            vec![(0, 0), (1, 30)]
        );
    }

    #[test]
    fn attached_picker_garbage_is_destroyed_on_the_rank_worker() {
        struct DropReporter(sync_mpsc::Sender<thread::ThreadId>);

        impl Drop for DropReporter {
            fn drop(&mut self) {
                let _ = self.0.send(thread::current().id());
            }
        }

        let editor_thread = thread::current().id();
        let (scanner, _events) = scanner();
        scanner.reset_ranker(1, FilePickerKind::Files);
        let (dropped, observed) = sync_mpsc::channel();
        scanner.discard_owned(DropReporter(dropped));

        let worker_thread = observed
            .recv_timeout(Duration::from_secs(1))
            .expect("rank worker should dispose retired picker ownership");
        assert_ne!(worker_thread, editor_thread);
    }

    #[test]
    fn closing_the_ranker_releases_its_retained_candidate_capacity() {
        let mut state = FileRankState {
            candidates: Vec::with_capacity(50_000),
            matches: Vec::with_capacity(50_000),
            ..FileRankState::default()
        };
        state.candidates.push(FileRankCandidate {
            entry: 0,
            path: PathBuf::from("/project/large"),
            relative: "large".to_owned(),
            text: "large".to_owned(),
            row: None,
            candidate_characters: 5,
            is_dir: false,
        });

        close_file_rank_state(&mut state);

        assert_eq!(state.candidates.capacity(), 0);
        assert_eq!(state.matches.capacity(), 0);
    }

    #[test]
    fn preview_mailbox_keeps_one_wake_and_only_the_latest_target() {
        let (wake, receiver) = sync_mpsc::sync_channel(1);
        let mailbox = FilePreviewMailbox {
            pending: Mutex::new(None),
            wake,
        };
        for request_id in 1..=100 {
            mailbox.request(FilePreviewRequest {
                scan_id: 1,
                query_revision: 2,
                request_id,
                path: PathBuf::from(format!("/project/{request_id}")),
                is_dir: false,
                content_match: None,
                show_hidden: false,
            });
        }

        assert_eq!(
            mailbox.pending.lock().unwrap().as_ref().unwrap().request_id,
            100
        );
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn preview_identity_includes_every_content_match_position() {
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::grep(1, root.clone(), ScanScope::ignoring(&root));
        let target = PickerTarget {
            path: root.join("imports.py"),
            row: Some(8),
            column: 0,
        };
        let partial = (8, vec![0, 1, 2, 3, 4, 5, 11, 12, 13]);
        let complete = (8, vec![0, 1, 2, 3, 4, 5, 11, 12, 13, 14]);

        assert_eq!(
            picker.begin_preview_request(target.clone(), Some(&partial)),
            Some(1)
        );
        picker.preview = Some(FilePreview::Text(vec!["stale".to_owned()]));
        assert_eq!(
            picker.begin_preview_request(target.clone(), Some(&complete)),
            Some(2),
            "the unchanged target must not hide corrected match spans"
        );
        assert!(picker.preview.is_none());
        assert_eq!(
            picker.begin_preview_request(target, Some(&complete)),
            None,
            "an identical preview request is still coalesced"
        );
    }

    #[test]
    fn constructing_the_scanner_does_not_start_an_idle_ranker() {
        let (scanner, _events) = scanner();
        assert!(scanner.rank_commands.get().is_none());
        assert!(scanner.preview_commands.get().is_none());
    }

    #[test]
    fn equal_scores_prefer_fewer_unicode_characters() {
        let root = PathBuf::from("/project");
        let mut picker = FilePicker::grep(1, root.clone(), ScanScope::ignoring(&root));
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
        let mut picker = FilePicker::new(1, root.clone(), ScanScope::ignoring(&root));
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
        let mut picker = FilePicker::new(1, root.clone(), ScanScope::ignoring(&root));
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
        let mut picker = FilePicker::new(1, root.clone(), ScanScope::ignoring(&root));
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
        scanner.scan(
            7,
            root.clone(),
            ScanScope::ignoring(&root),
            workspace,
            false,
        );

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
                FilePickerEvent::Ranked { .. } | FilePickerEvent::Preview { .. } => continue,
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
        let mut picker = FilePicker::grep(1, root.clone(), ScanScope::ignoring(&root));
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
            scan_content(&root, &ScanScope::ignoring(&root), &workspace, false, "").unwrap();
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
        // A space separates rather than matches. Every term is the fuzzy
        // subsequence a lone word has always been, and the terms are wanted in
        // the order they were typed.
        assert!(matches_fuzzy("cntnt", "content_entries_from_text"));
        assert!(matches_fuzzy(
            "content entries",
            "content_entries_from_text"
        ));
        assert!(
            matches_fuzzy("cntnt entries", "content_entries_from_text"),
            "a term of a several-word query is as loose as a lone word"
        );
        assert!(
            matches_fuzzy("kmap validate", "src/keymap/validate.rs"),
            "an abbreviation narrowed by a second word still finds its file"
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

        // Ordered terms that are each as loose as a lone word ask the same
        // question as the terms run together, so where a space falls decides
        // how a candidate scores rather than whether it matches at all.
        for candidate in [
            "content_entries_from_text",
            "cone_ties",
            "c_o_n_t_e_n_t_r_i_e_s",
            "entries_content",
        ] {
            assert_eq!(
                matches_fuzzy("content entries", candidate),
                matches_fuzzy("contententries", candidate),
                "a split term accepts what the joined term accepts: {candidate}"
            );
        }

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
    fn a_space_frees_the_distance_between_terms_but_not_inside_one() {
        // The whitespace someone types says the two stretches are separate, so
        // the alignment charges nothing for how far apart they land. The same
        // distance inside a single term is a gap and is paid for. This is the
        // whole of what a space still decides, now that it no longer decides
        // which candidates match.
        let candidate = "ab________cd";
        let (split, _) = FuzzyMatcher::for_lines("ab cd")
            .score(candidate)
            .expect("the terms are present in order");
        let (joined, _) = FuzzyMatcher::for_lines("abcd")
            .score(candidate)
            .expect("the same characters are present in order");
        assert!(
            split > joined,
            "a term boundary is free where a gap is not: {split} vs {joined}"
        );

        // Nothing is paid for terms landing adjacent either. On a candidate
        // where both queries align on the same characters, so that every
        // position-dependent bonus and the length penalty are identical, the
        // only difference left is the one adjacency bonus the boundary
        // declines to pay.
        let (joined, _) = FuzzyMatcher::for_lines("abcd")
            .score("zabcdz")
            .expect("a match");
        let (split, _) = FuzzyMatcher::for_lines("ab cd")
            .score("zabcdz")
            .expect("a match");
        assert_eq!(
            joined - split,
            28,
            "exactly the adjacency across the boundary"
        );
    }

    #[test]
    fn each_term_of_a_several_word_query_is_itself_fuzzy() {
        // The reported case: an abbreviation that no candidate holds as a
        // contiguous run, narrowed by a second word.
        let mut matcher = FuzzyMatcher::new("kmap validate");
        assert!(matcher.score("src/keymap/validate.rs").is_some());
        assert!(matcher.score("src/keymap/configured.rs").is_none());

        // The terms are still wanted in the order they were typed, which is
        // where this deliberately parts from fzf.
        let mut reversed = FuzzyMatcher::new("validate kmap");
        assert!(
            reversed.score("src/keymap/validate.rs").is_none(),
            "terms are wanted in the order they were typed"
        );

        // A term that is fuzzy rather than whole is what the secondary
        // emphasis colour is for, and stays distinguishable from one that
        // landed as itself.
        let (_, scattered) = FuzzyMatcher::new("kmap validate")
            .score("src/keymap/validate.rs")
            .expect("a match");
        assert!(!is_direct_match(&scattered, "kmap validate"));
        let (_, whole) = FuzzyMatcher::new("keymap validate")
            .score("src/keymap/validate.rs")
            .expect("a match");
        assert!(is_direct_match(&whole, "keymap validate"));
    }

    #[test]
    fn inserting_into_the_query_can_only_narrow_what_matched() {
        // Narrowing from the previous result set is sound while the edit could
        // not widen it. Every insertion leaves the earlier characters in place
        // and in order, so the terms run together only grow, and a candidate
        // that did not hold the shorter run cannot hold the longer one. Typing
        // at the end is the case
        // `a_longer_query_can_only_narrow_what_a_shorter_one_matched` covers;
        // these are the edits that rewrite a term from the middle.
        let root = PathBuf::from("/project");
        let rows = |picker: &FilePicker| {
            let mut found = picker
                .ranked()
                .map(|entry| entry.relative.to_owned())
                .collect::<Vec<_>>();
            found.sort_unstable();
            found
        };

        // Splitting a term is the edit that used to widen, back when each term
        // had to be present as a contiguous literal and `a_x_b_cd` was thrown
        // out by an `ab` it never held. A term is now as loose as a lone word,
        // so it was never excluded, and the space that splits it is separation
        // rather than a character to hold.
        let mut picker = FilePicker::new(1, root.clone(), ScanScope::ignoring(&root));
        picker.add_paths(vec![
            ScanEntry::file(root.join("a_x_b_cd")),
            ScanEntry::file(root.join("ab_cd")),
        ]);
        picker.insert_query_text("ab cd");
        let before = rows(&picker);
        assert_eq!(before, ["a_x_b_cd", "ab_cd"]);

        // Put the caret between `a` and `b` and split the term.
        picker.query_cursor = 1;
        picker.insert_query(' ');
        assert_eq!(picker.query, "a b cd");
        assert_eq!(
            rows(&picker),
            before,
            "where a space falls decides how a candidate scores, not whether it matches"
        );

        // Growing a term from the middle is a different edit: it adds a
        // character the run has to hold, so it narrows like any other.
        let mut picker = FilePicker::new(2, root.clone(), ScanScope::ignoring(&root));
        picker.add_paths(vec![
            ScanEntry::file(root.join("aXb_cd")),
            ScanEntry::file(root.join("ab_cd")),
        ]);
        picker.insert_query_text("ab cd");
        assert_eq!(rows(&picker), ["aXb_cd", "ab_cd"]);
        picker.query_cursor = 1;
        picker.insert_query('X');
        assert_eq!(picker.query, "aXb cd");
        assert_eq!(
            rows(&picker),
            ["aXb_cd"],
            "the path without the added character drops out"
        );

        // Growing the query at its end still narrows from what is on hand,
        // which is the case that has to stay cheap.
        let mut picker = FilePicker::new(3, root.clone(), ScanScope::ignoring(&root));
        picker.add_paths(vec![
            ScanEntry::file(root.join("ab_cd")),
            ScanEntry::file(root.join("ab_ce")),
        ]);
        picker.insert_query_text("ab c");
        assert_eq!(picker.matches.len(), 2);
        picker.insert_query('d');
        assert_eq!(rows(&picker), ["ab_cd"]);
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

        let (entries, skipped, limited) = scan_content(
            &root,
            &ScanScope::ignoring(&root),
            &workspace,
            false,
            "markedthing",
        )
        .unwrap();
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

        let (entries, _, limited) =
            scan_content(&root, &ScanScope::ignoring(&root), &workspace, false, "").unwrap();
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
        let mut picker = FilePicker::grep(1, root.clone(), ScanScope::ignoring(&root));
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
        let mut picker = FilePicker::grep(1, root.clone(), ScanScope::ignoring(&root));
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
        let mut picker = FilePicker::grep(
            1,
            PathBuf::from("/project"),
            ScanScope::ignoring(Path::new("/project")),
        );
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

        let mut files = FilePicker::new(
            1,
            PathBuf::from("/project"),
            ScanScope::ignoring(Path::new("/project")),
        );
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
            ScanScope::ignoring(&root),
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
                FilePickerEvent::Ranked { .. } | FilePickerEvent::Preview { .. } => continue,
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
    fn one_dense_file_reaches_the_editor_in_bounded_batches() {
        let root = temporary("bounded-background-content");
        fs::create_dir_all(&root).unwrap();
        let lines = (0..SCAN_BATCH * 3 + 7)
            .map(|row| format!("matching line {row}\n"))
            .collect::<String>();
        fs::write(root.join("dense.txt"), lines).unwrap();
        let (scanner, mut receiver) = scanner();
        scanner.scan_content(
            17,
            root.clone(),
            ScanScope::ignoring(&root),
            root.join(".runyte"),
            false,
            "matching".to_owned(),
        );

        let mut seen = 0usize;
        loop {
            match receiver.blocking_recv().unwrap() {
                FilePickerEvent::Content {
                    scan_id: 17,
                    entries,
                } => {
                    let batch = entries.iter().map(FileHits::len).sum::<usize>();
                    assert!(batch <= SCAN_BATCH);
                    seen += batch;
                }
                FilePickerEvent::Finished { scan_id: 17, .. } => break,
                FilePickerEvent::Ranked { .. } | FilePickerEvent::Preview { .. } => continue,
                FilePickerEvent::Failed { message, .. } => panic!("scan failed: {message}"),
                event => panic!("unexpected event: {event:?}"),
            }
        }
        assert_eq!(seen, SCAN_BATCH * 3 + 7);
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
            ScanScope::ignoring(&root),
            workspace.clone(),
            false,
            "alpha".to_owned(),
        );
        scanner.scan_content(
            12,
            root.clone(),
            ScanScope::ignoring(&root),
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
                FilePickerEvent::Ranked { .. } | FilePickerEvent::Preview { .. } => continue,
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

    #[test]
    fn content_preview_retains_a_match_beyond_the_indented_line_bound() {
        let line = format!("{}{}needle", " ".repeat(20), "x".repeat(500));
        let emphasis = (520..526).collect::<Vec<_>>();
        let FilePreview::Snippet(snippet) =
            FilePreview::snippet_from_text(&line, 0, emphasis.clone())
        else {
            panic!("content snippet expected");
        };

        assert_eq!(snippet.emphasis, emphasis);
        assert_eq!(snippet.lines[0].chars().count(), 526);
        assert_eq!(
            snippet
                .emphasis
                .iter()
                .map(|position| snippet.lines[0].chars().nth(*position).unwrap())
                .collect::<String>(),
            "needle"
        );
    }
}
