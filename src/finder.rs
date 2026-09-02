// SPDX-License-Identifier: MPL-2.0

//! The in-memory half of the project finder.
//!
//! Files keep their asynchronous, ignore-aware scanner in
//! [`crate::file_picker`]. Open buffers and terminals are already editor
//! state, so walking the filesystem for them would be both slower and less
//! truthful. This module ranks those resources and merges their scores with
//! the file scanner without copying the scanner's candidate table.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use crate::{
    file_picker::{
        CONTENT_ENTRY_LIMIT, EntryView, FilePicker, FilePreview, FuzzyMatcher, PickerTarget,
    },
    terminal::{TerminalId, TerminalLineId},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinderMode {
    Names,
    Contents,
}

impl FinderMode {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Names => "Names",
            Self::Contents => "Contents",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceKind {
    Buffer,
    Terminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceTarget {
    Buffer(usize),
    BufferLocation {
        buffer: usize,
        row: usize,
        column: usize,
    },
    Terminal(TerminalId),
    TerminalLocation {
        terminal: TerminalId,
        line_id: TerminalLineId,
        column: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinderTarget {
    File(PickerTarget),
    Resource(ResourceTarget),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceItem {
    pub label: String,
    pub detail: String,
    pub target: ResourceTarget,
    pub kind: ResourceKind,
    pub path: Option<PathBuf>,
    fields: Vec<String>,
}

impl ResourceItem {
    pub fn new(
        label: impl Into<String>,
        detail: impl Into<String>,
        target: ResourceTarget,
        kind: ResourceKind,
        fields: impl IntoIterator<Item = String>,
    ) -> Self {
        let label = label.into();
        let detail = detail.into();
        let mut searchable = vec![label.clone(), detail.clone()];
        searchable.extend(fields);
        searchable.sort();
        searchable.dedup();
        Self {
            label,
            detail,
            target,
            kind,
            path: None,
            fields: searchable,
        }
    }

    /// A content row ranks only its decoded text. Its source label is identity,
    /// not an accidental second content candidate.
    pub fn content(
        label: impl Into<String>,
        text: impl Into<String>,
        target: ResourceTarget,
        kind: ResourceKind,
    ) -> Self {
        let text = text.into();
        Self {
            label: label.into(),
            detail: text.clone(),
            target,
            kind,
            path: None,
            fields: vec![text],
        }
    }

    pub fn with_path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceMatch {
    pub item: usize,
    pub emphasis: Vec<usize>,
    pub detail_emphasis: Vec<usize>,
    pub(crate) score: i64,
    pub(crate) type_boost: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FinderMatchSource {
    File(usize),
    Resource(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinderMatch {
    pub source: FinderMatchSource,
    pub emphasis: Vec<usize>,
    pub detail_emphasis: Vec<usize>,
    pub(crate) score: i64,
    pub(crate) type_boost: bool,
}

/// File rows kept across a content re-query, with the scan they were ranked
/// against so the usual guard still decides whether they may be read.
pub(crate) struct KeptFileRows {
    rows: Vec<FinderMatch>,
    scan: Option<u64>,
}

#[derive(Clone, Debug)]
pub(crate) struct FinderFileRankContext {
    pub revision: u64,
    pub resource_matches: Vec<ResourceMatch>,
    pub replace_resources: bool,
    pub removed_resources: Vec<usize>,
    pub remap_resources: Option<Vec<Option<usize>>>,
    pub suppressed_paths: Arc<HashSet<PathBuf>>,
    pub file_boost: bool,
    pub sort: bool,
}

#[derive(Clone, Debug)]
pub struct ResourceFinder {
    pub mode: FinderMode,
    pub items: Vec<ResourceItem>,
    pub resource_matches: Vec<ResourceMatch>,
    pub matches: Vec<FinderMatch>,
    pub selected: usize,
    pub loading: bool,
    pub limited: bool,
    suppressed_paths: HashSet<PathBuf>,
    file_suppressed_paths: Arc<HashSet<PathBuf>>,
    name_rank: Option<(String, usize)>,
    unpublished_resource_matches: usize,
    file_rank_replace_resources: bool,
    file_rank_removed_resources: Vec<usize>,
    file_rank_remap_resources: Option<Vec<Option<usize>>>,
    terminal_content_items: HashMap<TerminalId, Vec<(TerminalLineId, usize)>>,
    free_content_items: Vec<usize>,
    pending_free_content_items: Vec<usize>,
    active_content_items: usize,
    selection_user_owned: bool,
    claimed_selection: Option<FinderTarget>,
    claimed_match_source: Option<FinderMatchSource>,
    selected_preview: Option<FilePreview>,
    file_rank_revision: u64,
    /// The picker scan whose entry table this finder's file matches index.
    ///
    /// An entry index means nothing on its own. A restarted scan refills
    /// `entries` from scratch, so the same index names a different line, and
    /// a finder that outlives one scan can hold matches that no longer
    /// describe anything. Recording the scan lets every reading of a file
    /// match ask whether the two still agree before it resolves an index.
    file_match_scan: Option<u64>,
}

impl Default for ResourceFinder {
    fn default() -> Self {
        Self::new(FinderMode::Names)
    }
}

impl ResourceFinder {
    pub fn new(mode: FinderMode) -> Self {
        Self {
            mode,
            items: Vec::new(),
            resource_matches: Vec::new(),
            matches: Vec::new(),
            selected: 0,
            loading: false,
            limited: false,
            suppressed_paths: HashSet::new(),
            file_suppressed_paths: Arc::new(HashSet::new()),
            name_rank: None,
            unpublished_resource_matches: 0,
            file_rank_replace_resources: false,
            file_rank_removed_resources: Vec::new(),
            file_rank_remap_resources: None,
            terminal_content_items: HashMap::new(),
            free_content_items: Vec::new(),
            pending_free_content_items: Vec::new(),
            active_content_items: 0,
            selection_user_owned: false,
            claimed_selection: None,
            claimed_match_source: None,
            selected_preview: None,
            file_rank_revision: 0,
            file_match_scan: None,
        }
    }

    /// Moves the potentially large result corpus out in constant time before
    /// an attached finder starts a new query. Name ranking retains its stable
    /// resource identities; content ranking replaces them with new rows.
    pub(crate) fn retire_background_corpus(&mut self, retain_items: bool) -> impl Send + 'static {
        let items = if retain_items {
            Vec::new()
        } else {
            std::mem::take(&mut self.items)
        };
        let resource_matches = std::mem::take(&mut self.resource_matches);
        // A name rank replaces this list wholesale when its answer lands, and
        // until then the reader is still choosing from it, so the rows stay
        // where they are rather than blanking for the length of a round trip.
        // The scan they were ranked against stays named with them: a scan
        // that restarts underneath them is what retires them, and the guard
        // in `file_entry` is what notices.
        let matches = if retain_items {
            Vec::new()
        } else {
            std::mem::take(&mut self.matches)
        };
        let suppressed_paths = if retain_items {
            HashSet::new()
        } else {
            std::mem::take(&mut self.suppressed_paths)
        };
        let file_suppressed_paths = if retain_items {
            Arc::new(HashSet::new())
        } else {
            std::mem::replace(&mut self.file_suppressed_paths, Arc::new(HashSet::new()))
        };
        if !retain_items {
            self.file_match_scan = None;
        }
        let selected_preview = self.selected_preview.take();
        let claimed_selection = self.claimed_selection.take();
        let claimed_match_source = self.claimed_match_source.take();
        let terminal_content_items = std::mem::take(&mut self.terminal_content_items);
        let free_content_items = std::mem::take(&mut self.free_content_items);
        let pending_free_content_items = std::mem::take(&mut self.pending_free_content_items);
        self.active_content_items = 0;
        (
            items,
            resource_matches,
            matches,
            suppressed_paths,
            file_suppressed_paths,
            selected_preview,
            claimed_selection,
            claimed_match_source,
            terminal_content_items,
            free_content_items,
            pending_free_content_items,
        )
    }

    pub fn begin_content_scan(
        &mut self,
        picker: &FilePicker,
        query: &str,
        suppressed_paths: impl IntoIterator<Item = PathBuf>,
    ) {
        self.items.clear();
        self.terminal_content_items.clear();
        self.free_content_items.clear();
        self.pending_free_content_items.clear();
        self.active_content_items = 0;
        self.resource_matches.clear();
        self.suppressed_paths = suppressed_paths.into_iter().collect();
        self.rebuild_file_suppressed_paths();
        self.name_rank = None;
        self.unpublished_resource_matches = 0;
        self.loading = true;
        self.limited = false;
        self.selection_user_owned = false;
        self.claimed_selection = None;
        self.claimed_match_source = None;
        self.selected_preview = None;
        self.merge(picker, query, None);
    }

    pub(crate) fn begin_content_scan_unmerged(
        &mut self,
        query: &str,
        suppressed_paths: Arc<HashSet<PathBuf>>,
    ) {
        self.note_file_rank_change();
        self.file_rank_replace_resources = true;
        self.file_rank_removed_resources.clear();
        self.file_rank_remap_resources = None;
        self.items.clear();
        self.terminal_content_items.clear();
        self.free_content_items.clear();
        self.pending_free_content_items.clear();
        self.active_content_items = 0;
        self.resource_matches.clear();
        self.matches.clear();
        self.file_match_scan = None;
        self.suppressed_paths.clear();
        self.file_suppressed_paths = suppressed_paths;
        self.name_rank = None;
        self.unpublished_resource_matches = 0;
        self.loading = true;
        self.limited = false;
        self.selection_user_owned = false;
        self.claimed_selection = None;
        self.claimed_match_source = None;
        self.selected_preview = None;
        self.rank_resources(query);
    }

    pub fn append_items(
        &mut self,
        items: impl IntoIterator<Item = ResourceItem>,
        picker: &FilePicker,
        query: &str,
    ) {
        let selected = self.preserved_selection(picker);
        let first_new = self.items.len();
        self.items.extend(items);
        self.rebuild_file_suppressed_paths();
        let parsed = ParsedQuery::new(query, self.mode == FinderMode::Names);
        let mut additions = self.items[first_new..]
            .iter()
            .enumerate()
            .filter_map(|(offset, candidate)| {
                parsed
                    .score(candidate)
                    .map(|found| (first_new + offset, found))
            })
            .map(
                |(item, (score, type_boost, emphasis, detail_emphasis))| ResourceMatch {
                    item,
                    emphasis,
                    detail_emphasis,
                    score,
                    type_boost: self.mode == FinderMode::Names && type_boost,
                },
            )
            .collect::<Vec<_>>();
        additions.sort_by(resource_match_order);
        let finder_additions = additions
            .iter()
            .map(resource_to_finder_match)
            .collect::<Vec<_>>();
        merge_sorted_by(&mut self.resource_matches, additions, resource_match_order);
        merge_sorted_by(&mut self.matches, finder_additions, finder_match_order);
        if self.mode == FinderMode::Contents && self.matches.len() > CONTENT_ENTRY_LIMIT {
            self.matches.truncate(CONTENT_ENTRY_LIMIT);
        }
        self.restore_selection(picker, selected.as_ref());
    }

    pub(crate) fn append_items_unmerged(
        &mut self,
        items: impl IntoIterator<Item = ResourceItem>,
        query: &str,
    ) {
        self.note_file_rank_change();
        let first_new = self.items.len();
        self.items.extend(items);
        let parsed = ParsedQuery::new(query, self.mode == FinderMode::Names);
        let mut additions = self.items[first_new..]
            .iter()
            .enumerate()
            .filter_map(|(offset, candidate)| {
                parsed
                    .score(candidate)
                    .map(|found| (first_new + offset, found))
            })
            .map(
                |(item, (score, type_boost, emphasis, detail_emphasis))| ResourceMatch {
                    item,
                    emphasis,
                    detail_emphasis,
                    score,
                    type_boost: self.mode == FinderMode::Names && type_boost,
                },
            )
            .collect::<Vec<_>>();
        additions.sort_by(resource_match_order);
        merge_sorted_by(&mut self.resource_matches, additions, resource_match_order);
    }

    /// Admits one bounded live-content batch without repeatedly merging or
    /// cloning the complete accumulated result set. A `true` return means a
    /// publication-sized immutable snapshot is now worth sending.
    pub(crate) fn append_content_items_unmerged(
        &mut self,
        items: impl IntoIterator<Item = ResourceItem>,
        query: &str,
    ) -> bool {
        let parsed = ParsedQuery::new(query, false);
        let mut additions = Vec::new();
        for candidate in items {
            let item = if let Some(item) = self.free_content_items.pop() {
                self.items[item] = candidate;
                item
            } else {
                let item = self.items.len();
                self.items.push(candidate);
                item
            };
            self.active_content_items += 1;
            if let ResourceTarget::TerminalLocation {
                terminal, line_id, ..
            } = self.items[item].target
            {
                self.terminal_content_items
                    .entry(terminal)
                    .or_default()
                    .push((line_id, item));
            }
            if self.selection_user_owned
                && self.claimed_selection.as_ref()
                    == Some(&FinderTarget::Resource(self.items[item].target))
            {
                self.claimed_match_source = Some(FinderMatchSource::Resource(item));
            }
            if let Some((score, _, emphasis, detail_emphasis)) = parsed.score(&self.items[item]) {
                additions.push(ResourceMatch {
                    item,
                    emphasis,
                    detail_emphasis,
                    score,
                    type_boost: false,
                });
            }
        }
        self.unpublished_resource_matches += additions.len();
        self.resource_matches.extend(additions);
        false
    }

    pub(crate) fn content_item_count(&self) -> usize {
        self.active_content_items
    }

    /// Moves one terminal's ordered content index into a refresh cursor.
    /// The potentially large vector is never copied or traversed here.
    pub(crate) fn take_terminal_content_items(
        &mut self,
        terminal: TerminalId,
    ) -> Vec<(TerminalLineId, usize)> {
        self.terminal_content_items
            .remove(&terminal)
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn terminal_content_index_storage(&self, terminal: TerminalId) -> (usize, usize) {
        self.terminal_content_items
            .get(&terminal)
            .map_or((0, 0), |items| (items.as_ptr() as usize, items.len()))
    }

    #[cfg(test)]
    pub(crate) fn selection_is_user_owned(&self) -> bool {
        self.selection_user_owned
    }

    pub(crate) fn keep_terminal_content_item(
        &mut self,
        terminal: TerminalId,
        line_id: TerminalLineId,
        item: usize,
    ) {
        self.terminal_content_items
            .entry(terminal)
            .or_default()
            .push((line_id, item));
    }

    pub(crate) fn retire_content_item(
        &mut self,
        item: usize,
        preserve_selection: bool,
    ) -> Option<ResourceItem> {
        if item >= self.items.len() {
            return None;
        }
        self.active_content_items = self.active_content_items.saturating_sub(1);
        if !preserve_selection
            && self.selection_user_owned
            && self
                .selected_match()
                .is_some_and(|found| found.source == FinderMatchSource::Resource(item))
        {
            self.selection_user_owned = false;
            self.claimed_selection = None;
            self.claimed_match_source = None;
            self.selected = 0;
        }
        if preserve_selection
            && self.claimed_match_source == Some(FinderMatchSource::Resource(item))
        {
            self.claimed_match_source = None;
        }
        self.pending_free_content_items.push(item);
        self.file_rank_removed_resources.push(item);
        Some(std::mem::replace(
            &mut self.items[item],
            ResourceItem::content(
                "",
                "",
                ResourceTarget::Buffer(usize::MAX),
                ResourceKind::Buffer,
            ),
        ))
    }

    pub fn finish_content_scan(&mut self, limited: bool) {
        self.loading = false;
        self.limited = limited;
    }

    pub(crate) fn finish_content_scan_unmerged(&mut self, limited: bool) {
        self.loading = false;
        self.limited = limited;
        self.publish_resource_matches();
        self.free_content_items
            .append(&mut self.pending_free_content_items);
    }

    pub fn replace_items(&mut self, items: Vec<ResourceItem>, picker: &FilePicker, query: &str) {
        let selected = self.preserved_selection(picker);
        self.items = items;
        self.loading = false;
        self.limited = false;
        self.suppressed_paths.clear();
        self.name_rank = None;
        self.unpublished_resource_matches = 0;
        self.rebuild_file_suppressed_paths();
        self.rank_resources(query);
        self.merge(picker, query, selected.as_ref());
    }

    pub(crate) fn replace_items_unmerged(&mut self, items: Vec<ResourceItem>, _query: &str) {
        self.note_file_rank_change();
        self.file_rank_replace_resources = true;
        self.file_rank_removed_resources.clear();
        self.file_rank_remap_resources = None;
        self.items = items;
        self.matches.clear();
        self.file_match_scan = None;
        self.resource_matches.clear();
        self.loading = false;
        self.limited = false;
        self.suppressed_paths.clear();
        self.selection_user_owned = false;
        self.claimed_selection = None;
        self.claimed_match_source = None;
        self.selected = 0;
        self.name_rank = None;
        self.unpublished_resource_matches = 0;
        self.rebuild_file_suppressed_paths();
    }

    pub fn rank(&mut self, picker: &FilePicker, query: &str) {
        self.selection_user_owned = false;
        self.claimed_selection = None;
        self.claimed_match_source = None;
        self.rank_resources(query);
        self.merge(picker, query, None);
    }

    /// Starts a cooperative name-ranking pass. Query input only replaces this
    /// cursor; the event loop performs bounded slices between input events.
    pub(crate) fn begin_name_rank(&mut self, query: &str, reset_selection: bool) {
        if reset_selection {
            self.selection_user_owned = false;
            self.claimed_selection = None;
            self.claimed_match_source = None;
            self.selected = 0;
        }
        self.note_file_rank_change();
        self.file_rank_replace_resources = true;
        self.file_rank_removed_resources.clear();
        self.file_rank_remap_resources = None;
        self.resource_matches.clear();
        self.loading = !self.items.is_empty();
        self.unpublished_resource_matches = 0;
        self.name_rank = (!self.items.is_empty()).then(|| (query.to_owned(), 0));
    }

    pub(crate) fn name_rank_pending(&self) -> bool {
        self.name_rank.is_some()
    }

    pub(crate) fn advance_name_rank(&mut self, limit: usize) -> bool {
        let Some((query, first)) = self.name_rank.take() else {
            return false;
        };
        let last = first.saturating_add(limit).min(self.items.len());
        let parsed = ParsedQuery::new(&query, true);
        let additions = self.items[first..last]
            .iter()
            .enumerate()
            .filter_map(|(offset, candidate)| {
                parsed.score(candidate).map(|found| (first + offset, found))
            })
            .map(
                |(item, (score, type_boost, emphasis, detail_emphasis))| ResourceMatch {
                    item,
                    emphasis,
                    detail_emphasis,
                    score,
                    type_boost,
                },
            )
            .collect::<Vec<_>>();
        self.unpublished_resource_matches += additions.len();
        self.resource_matches.extend(additions);
        if last < self.items.len() {
            self.name_rank = Some((query, last));
            false
        } else {
            self.loading = false;
            self.publish_resource_matches();
            true
        }
    }

    pub(crate) fn take_file_rank_context(&mut self, query: &str) -> FinderFileRankContext {
        let parsed = ParsedQuery::new(query, self.mode == FinderMode::Names);
        FinderFileRankContext {
            revision: self.file_rank_revision,
            resource_matches: std::mem::take(&mut self.resource_matches),
            replace_resources: std::mem::take(&mut self.file_rank_replace_resources),
            removed_resources: std::mem::take(&mut self.file_rank_removed_resources),
            remap_resources: self.file_rank_remap_resources.take(),
            suppressed_paths: self.file_suppressed_paths.clone(),
            file_boost: self.mode == FinderMode::Names && parsed.file_boost,
            sort: !parsed.terms.is_empty() || parsed.has_type_hint(),
        }
    }

    pub(crate) fn file_rank_revision(&self) -> u64 {
        self.file_rank_revision
    }

    fn note_file_rank_change(&mut self) {
        self.file_rank_revision = self.file_rank_revision.wrapping_add(1);
    }

    fn rebuild_file_suppressed_paths(&mut self) {
        self.file_suppressed_paths = Arc::new(
            self.items
                .iter()
                .filter_map(|item| item.path.clone())
                .chain(self.suppressed_paths.iter().cloned())
                .collect(),
        );
    }

    fn publish_resource_matches(&mut self) {
        self.unpublished_resource_matches = 0;
        self.note_file_rank_change();
    }

    pub(crate) fn apply_background_matches(
        &mut self,
        scan_id: u64,
        matches: Vec<FinderMatch>,
        positions: &HashMap<FinderMatchSource, usize>,
    ) -> Vec<FinderMatch> {
        self.file_match_scan = Some(scan_id);
        let claimed_target = self.claimed_selection.is_some();
        let selected = self.selection_user_owned.then(|| {
            if claimed_target {
                self.claimed_match_source
            } else {
                self.selected_match().map(|found| found.source)
            }
        });
        let old = std::mem::replace(&mut self.matches, matches);
        self.selected = selected
            .flatten()
            .and_then(|source| positions.get(&source).copied())
            .unwrap_or(0);
        if claimed_target && selected.flatten().is_none() {
            self.selection_user_owned = false;
            self.claimed_selection = None;
            self.claimed_match_source = None;
        }
        self.selected_preview = None;
        old
    }

    /// Sets the file rows aside before a content re-query retires the rest.
    ///
    /// A content item belongs to the query that collected it, so a row naming
    /// one goes with that query. A file row names a scan the new query does
    /// not replace, so it can stay on screen until the ranker answers instead
    /// of blanking the list for the length of a round trip. What is left
    /// behind in `matches` is what the retirement moves off this thread.
    pub(crate) fn take_file_rows(&mut self) -> KeptFileRows {
        let mut rows = Vec::new();
        let mut retired = Vec::with_capacity(self.matches.len());
        for found in std::mem::take(&mut self.matches) {
            if matches!(found.source, FinderMatchSource::File(_)) {
                rows.push(found);
            } else {
                retired.push(found);
            }
        }
        self.matches = retired;
        KeptFileRows {
            rows,
            scan: self.file_match_scan,
        }
    }

    /// Puts those rows back as the list the new content scan starts from.
    ///
    /// They keep the scan they were ranked against rather than adopting the
    /// current one: a scan that restarted underneath them is what retires
    /// them, and `file_entry` is what notices.
    pub(crate) fn restore_file_rows(&mut self, kept: KeptFileRows) {
        self.matches = kept.rows;
        self.file_match_scan = kept.scan;
    }

    pub fn merge_files(&mut self, picker: &FilePicker, query: &str) {
        let selected = self.preserved_selection(picker);
        self.merge(picker, query, selected.as_ref());
    }

    /// Re-ranks one name-mode terminal after its title or activity metadata
    /// changes, without rebuilding unrelated buffers and terminal sessions.
    pub fn replace_terminal(
        &mut self,
        terminal: TerminalId,
        item: ResourceItem,
        picker: &FilePicker,
        query: &str,
    ) {
        let Some(index) = self
            .items
            .iter()
            .position(|item| matches!(item.target, ResourceTarget::Terminal(id) if id == terminal))
        else {
            self.append_items([item], picker, query);
            return;
        };
        let selected = self.preserved_selection(picker);
        self.items[index] = item;
        self.rebuild_file_suppressed_paths();
        self.resource_matches.retain(|found| found.item != index);
        self.matches.retain(
            |found| !matches!(found.source, FinderMatchSource::Resource(item) if item == index),
        );
        let parsed = ParsedQuery::new(query, true);
        if let Some((score, type_boost, emphasis, detail_emphasis)) =
            parsed.score(&self.items[index])
        {
            let found = ResourceMatch {
                item: index,
                emphasis,
                detail_emphasis,
                score,
                type_boost,
            };
            merge_sorted_by(
                &mut self.matches,
                vec![resource_to_finder_match(&found)],
                finder_match_order,
            );
            merge_sorted_by(
                &mut self.resource_matches,
                vec![found],
                resource_match_order,
            );
        }
        self.restore_selection(picker, selected.as_ref());
    }

    /// Whether `item` says anything the finder does not already hold about
    /// the terminal it describes.
    ///
    /// In name mode a terminal contributes its title, its command, and its
    /// activity — not its output. A child writing continuously marks the
    /// terminal changed on every chunk and then almost always produces the
    /// identical item, and acting on one of those costs a rank of the whole
    /// file corpus and replaces every row in the list. That is a list that
    /// moves under the reader for no new information, so ask first.
    pub(crate) fn terminal_item_differs(&self, terminal: TerminalId, item: &ResourceItem) -> bool {
        self.items
            .iter()
            .find(|held| matches!(held.target, ResourceTarget::Terminal(id) if id == terminal))
            .is_none_or(|held| held != item)
    }

    pub(crate) fn replace_terminal_unmerged(
        &mut self,
        terminal: TerminalId,
        item: ResourceItem,
        query: &str,
    ) {
        self.note_file_rank_change();
        let Some(index) = self
            .items
            .iter()
            .position(|item| matches!(item.target, ResourceTarget::Terminal(id) if id == terminal))
        else {
            self.append_items_unmerged([item], query);
            return;
        };
        self.items[index] = item;
        self.file_rank_removed_resources.push(index);
        let parsed = ParsedQuery::new(query, true);
        if let Some((score, type_boost, emphasis, detail_emphasis)) =
            parsed.score(&self.items[index])
        {
            merge_sorted_by(
                &mut self.resource_matches,
                vec![ResourceMatch {
                    item: index,
                    emphasis,
                    detail_emphasis,
                    score,
                    type_boost,
                }],
                resource_match_order,
            );
        }
    }

    /// Drops the content rows a refresh is about to read again.
    ///
    /// A refresh reads only what a child has added, so a terminal's earlier
    /// rows keep the results they already produced: `kept` names, per
    /// terminal, the line identities that survive. A terminal listed with an
    /// empty set gives up everything it contributed, which is what a screen
    /// the session no longer has — cleared, swapped, or evicted from bounded
    /// history — amounts to. A terminal that is not listed is untouched.
    pub fn retain_terminal_content(
        &mut self,
        kept: &HashMap<TerminalId, HashSet<TerminalLineId>>,
        picker: &FilePicker,
    ) {
        let selected = self.preserved_selection(picker);
        let mut remap = vec![None; self.items.len()];
        let mut retained = Vec::with_capacity(self.items.len());
        for (old, item) in std::mem::take(&mut self.items).into_iter().enumerate() {
            if let ResourceTarget::TerminalLocation {
                terminal, line_id, ..
            } = item.target
                && kept
                    .get(&terminal)
                    .is_some_and(|kept| !kept.contains(&line_id))
            {
                continue;
            }
            remap[old] = Some(retained.len());
            retained.push(item);
        }
        self.items = retained;
        self.rebuild_file_suppressed_paths();
        self.resource_matches.retain_mut(|found| {
            let Some(item) = remap[found.item] else {
                return false;
            };
            found.item = item;
            true
        });
        self.matches.retain_mut(|found| match &mut found.source {
            FinderMatchSource::File(_) => true,
            FinderMatchSource::Resource(item) => {
                let Some(next) = remap[*item] else {
                    return false;
                };
                *item = next;
                true
            }
        });
        self.loading = true;
        self.restore_selection(picker, selected.as_ref());
    }

    fn rank_resources(&mut self, query: &str) {
        let parsed = ParsedQuery::new(query, self.mode == FinderMode::Names);
        self.resource_matches = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(item, candidate)| parsed.score(candidate).map(|found| (item, found)))
            .map(
                |(item, (score, type_boost, emphasis, detail_emphasis))| ResourceMatch {
                    item,
                    emphasis,
                    detail_emphasis,
                    score,
                    type_boost: self.mode == FinderMode::Names && type_boost,
                },
            )
            .collect::<Vec<_>>();
        self.sort_resource_matches(&parsed);
    }

    fn sort_resource_matches(&mut self, parsed: &ParsedQuery) {
        if !parsed.terms.is_empty() || parsed.has_type_hint() {
            self.resource_matches.sort_by(resource_match_order);
        }
    }

    fn merge(&mut self, picker: &FilePicker, query: &str, selected: Option<&FinderTarget>) {
        let parsed = ParsedQuery::new(query, self.mode == FinderMode::Names);
        let live_paths = self
            .items
            .iter()
            .filter_map(|item| item.path.as_deref())
            .chain(self.suppressed_paths.iter().map(PathBuf::as_path))
            .collect::<HashSet<_>>();
        let mut matches = picker
            .matches
            .iter()
            .filter(|found| {
                picker
                    .view(found.entry)
                    .is_some_and(|entry| !live_paths.contains(entry.path))
            })
            .map(|found| FinderMatch {
                source: FinderMatchSource::File(found.entry),
                emphasis: found.positions.clone(),
                detail_emphasis: Vec::new(),
                score: found.score,
                type_boost: self.mode == FinderMode::Names && parsed.file_boost,
            })
            .chain(self.resource_matches.iter().map(|found| FinderMatch {
                source: FinderMatchSource::Resource(found.item),
                emphasis: found.emphasis.clone(),
                detail_emphasis: found.detail_emphasis.clone(),
                score: found.score,
                type_boost: found.type_boost,
            }))
            .collect::<Vec<_>>();
        if !parsed.terms.is_empty() || parsed.has_type_hint() {
            matches.sort_by(finder_match_order);
        }
        if self.mode == FinderMode::Contents && matches.len() > CONTENT_ENTRY_LIMIT {
            matches.truncate(CONTENT_ENTRY_LIMIT);
        }
        self.matches = matches;
        self.file_match_scan = Some(picker.scan_id);
        self.restore_selection(picker, selected);
    }

    fn preserved_selection(&mut self, picker: &FilePicker) -> Option<FinderTarget> {
        if !self.selection_user_owned {
            return None;
        }
        if self.claimed_selection.is_none() {
            self.claimed_match_source = self.selected_match().map(|found| found.source);
            self.claimed_selection = self.selected_target(picker);
        }
        self.claimed_selection.clone()
    }

    pub(crate) fn preserve_selection(&mut self, picker: &FilePicker) {
        let _ = self.preserved_selection(picker);
    }

    fn restore_selection(&mut self, picker: &FilePicker, selected: Option<&FinderTarget>) {
        self.selected_preview = None;
        self.selected = selected
            .and_then(|target| {
                self.matches
                    .iter()
                    .position(|found| self.target_for_match(picker, found).as_ref() == Some(target))
            })
            .unwrap_or(0);
        self.claimed_match_source =
            selected.and_then(|_| self.matches.get(self.selected).map(|found| found.source));
    }

    pub fn selected_match(&self) -> Option<&FinderMatch> {
        self.matches
            .get(self.selected.min(self.matches.len().saturating_sub(1)))
    }

    pub fn selected_item(&self) -> Option<&ResourceItem> {
        let FinderMatchSource::Resource(item) = self.selected_match()?.source else {
            return None;
        };
        self.items.get(item)
    }

    pub fn selected_target(&self, picker: &FilePicker) -> Option<FinderTarget> {
        if picker.ranking {
            return None;
        }
        self.selected_match()
            .and_then(|found| self.target_for_match(picker, found))
    }

    /// The picker entry a file match names, read from the table it was
    /// ranked against: the one on hand, or the one a content re-scan has
    /// replaced but not yet retired. `None` once that table is gone.
    ///
    /// Rows, previews, and what `Enter` opens all resolve a file match
    /// through here, so none of them can read one scan's index in another
    /// scan's table. A restarted scan refills `entries` from scratch while
    /// the finder still holds the rows the previous ranking produced, and an
    /// index read across that boundary silently names an unrelated line.
    pub fn file_entry<'a>(&self, picker: &'a FilePicker, entry: usize) -> Option<EntryView<'a>> {
        picker.view_in(self.file_match_scan?, entry)
    }

    fn target_for_match(&self, picker: &FilePicker, found: &FinderMatch) -> Option<FinderTarget> {
        match found.source {
            FinderMatchSource::File(entry) => {
                let view = self.file_entry(picker, entry)?;
                let column = view.column
                    + view
                        .row
                        .and_then(|_| found.emphasis.first().copied())
                        .unwrap_or(0);
                Some(FinderTarget::File(PickerTarget {
                    path: view.path.to_path_buf(),
                    row: view.row,
                    column,
                }))
            }
            FinderMatchSource::Resource(item) => self
                .items
                .get(item)
                .map(|item| FinderTarget::Resource(item.target)),
        }
    }

    /// The selected resource's preview, in the same shape a file preview
    /// takes so that a content match in a buffer or terminal is shown, and
    /// highlighted, exactly as one in a file on disk is.
    pub fn selected_preview(&self) -> Option<&FilePreview> {
        self.selected_preview.as_ref()
    }

    pub fn set_selected_preview(&mut self, preview: Option<FilePreview>) {
        self.selected_preview = preview;
    }

    pub fn down(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
            self.selection_user_owned = true;
            self.claimed_selection = None;
            self.claimed_match_source = None;
        }
    }

    pub fn up(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.matches.len() - 1) % self.matches.len();
            self.selection_user_owned = true;
            self.claimed_selection = None;
            self.claimed_match_source = None;
        }
    }

    pub fn page_down(&mut self, amount: usize) {
        self.selected = self
            .selected
            .saturating_add(amount.max(1))
            .min(self.matches.len().saturating_sub(1));
        self.selection_user_owned = true;
        self.claimed_selection = None;
        self.claimed_match_source = None;
    }

    pub fn page_up(&mut self, amount: usize) {
        self.selected = self.selected.saturating_sub(amount.max(1));
        self.selection_user_owned = true;
        self.claimed_selection = None;
        self.claimed_match_source = None;
    }

    pub fn first(&mut self) {
        self.selected = 0;
        self.selection_user_owned = true;
        self.claimed_selection = None;
        self.claimed_match_source = None;
    }

    pub fn last(&mut self) {
        self.selected = self.matches.len().saturating_sub(1);
        self.selection_user_owned = true;
        self.claimed_selection = None;
        self.claimed_match_source = None;
    }
}

pub(crate) fn resource_to_finder_match(found: &ResourceMatch) -> FinderMatch {
    FinderMatch {
        source: FinderMatchSource::Resource(found.item),
        emphasis: found.emphasis.clone(),
        detail_emphasis: found.detail_emphasis.clone(),
        score: found.score,
        type_boost: found.type_boost,
    }
}

fn resource_match_order(left: &ResourceMatch, right: &ResourceMatch) -> std::cmp::Ordering {
    right
        .type_boost
        .cmp(&left.type_boost)
        .then_with(|| right.score.cmp(&left.score))
        .then_with(|| left.item.cmp(&right.item))
}

pub(crate) fn finder_match_order(left: &FinderMatch, right: &FinderMatch) -> std::cmp::Ordering {
    right
        .type_boost
        .cmp(&left.type_boost)
        .then_with(|| right.score.cmp(&left.score))
        .then_with(|| source_order(left.source).cmp(&source_order(right.source)))
}

/// Merges one already-sorted scan batch into the accumulated ranking. The
/// work is linear in the visible ranking instead of re-sorting every result
/// after each cooperative scan slice.
fn merge_sorted_by<T>(
    current: &mut Vec<T>,
    additions: Vec<T>,
    mut order: impl FnMut(&T, &T) -> std::cmp::Ordering,
) {
    if additions.is_empty() {
        return;
    }
    let capacity = current.len() + additions.len();
    let mut existing = std::mem::take(current).into_iter().peekable();
    let mut incoming = additions.into_iter().peekable();
    let mut merged = Vec::with_capacity(capacity);
    while let (Some(left), Some(right)) = (existing.peek(), incoming.peek()) {
        if order(left, right).is_le() {
            merged.push(existing.next().unwrap());
        } else {
            merged.push(incoming.next().unwrap());
        }
    }
    merged.extend(existing);
    merged.extend(incoming);
    *current = merged;
}

fn source_order(source: FinderMatchSource) -> (u8, usize) {
    match source {
        FinderMatchSource::File(entry) => (0, entry),
        FinderMatchSource::Resource(item) => (1, item),
    }
}

struct ParsedQuery {
    terms: Vec<String>,
    file_boost: bool,
    buffer_boost: bool,
    terminal_boost: bool,
}

impl ParsedQuery {
    fn new(query: &str, type_hints: bool) -> Self {
        let mut terms = Vec::new();
        let mut file_boost = false;
        let mut buffer_boost = false;
        let mut terminal_boost = false;
        for term in query.split_whitespace() {
            match term.to_lowercase().as_str() {
                _ if !type_hints => terms.push(term.to_owned()),
                "file" | "files" => file_boost = true,
                "buffer" | "buffers" => buffer_boost = true,
                "term" | "terminal" | "terminals" => terminal_boost = true,
                _ => terms.push(term.to_owned()),
            }
        }
        Self {
            terms,
            file_boost,
            buffer_boost,
            terminal_boost,
        }
    }

    fn matching_query(&self) -> String {
        self.terms.join(" ")
    }

    fn has_type_hint(&self) -> bool {
        self.file_boost || self.buffer_boost || self.terminal_boost
    }

    fn score(&self, item: &ResourceItem) -> Option<(i64, bool, Vec<usize>, Vec<usize>)> {
        let mut score = 0i64;
        let mut emphasis = Vec::new();
        let mut detail_emphasis = Vec::new();
        for term in &self.terms {
            let mut matcher = FuzzyMatcher::for_lines(term);
            let best = item
                .fields
                .iter()
                .filter_map(|field| matcher.score(field).map(|(score, _)| score))
                .max()?;
            score = score.saturating_add(best);
            let mut label_matcher = FuzzyMatcher::for_lines(term);
            if let Some((_, positions)) = label_matcher.score(&item.label) {
                emphasis.extend(positions);
            }
            let mut detail_matcher = FuzzyMatcher::for_lines(term);
            if let Some((_, positions)) = detail_matcher.score(&item.detail) {
                detail_emphasis.extend(positions);
            }
        }
        emphasis.sort_unstable();
        emphasis.dedup();
        detail_emphasis.sort_unstable();
        detail_emphasis.dedup();
        let type_boost = match item.kind {
            ResourceKind::Buffer => self.buffer_boost,
            ResourceKind::Terminal => self.terminal_boost,
        };
        Some((score, type_boost, emphasis, detail_emphasis))
    }
}

/// Removes the name finder's soft type hints from the text ranked by the file
/// engine. The hints affect merged ordering; they are not literal filters.
pub fn finder_matching_query(query: &str) -> String {
    ParsedQuery::new(query, true).matching_query()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: ResourceKind, label: &str, fields: &[&str], index: usize) -> ResourceItem {
        let target = match kind {
            ResourceKind::Buffer => ResourceTarget::Buffer(index),
            ResourceKind::Terminal => ResourceTarget::Terminal(TerminalId::from_raw(index as u64)),
        };
        ResourceItem::new(
            label,
            "",
            target,
            kind,
            fields.iter().map(|field| (*field).to_owned()),
        )
    }

    fn empty_picker() -> FilePicker {
        let mut picker = FilePicker::new(
            1,
            PathBuf::from("/project"),
            crate::file_picker::ScanScope::ignoring("/project"),
        );
        picker.finish(0, false);
        picker
    }

    #[test]
    fn a_name_rank_leaves_the_rows_the_reader_is_choosing_from_in_place() {
        let mut picker = FilePicker::new(
            1,
            PathBuf::from("/project"),
            crate::file_picker::ScanScope::ignoring("/project"),
        );
        picker.enable_unified_finder();
        picker.add_paths(vec![crate::file_picker::ScanEntry::file(PathBuf::from(
            "/project/picker.rs",
        ))]);
        picker.finish(0, false);
        let mut finder = ResourceFinder::default();
        finder.replace_items(
            vec![item(ResourceKind::Buffer, "notes.txt", &["notes.txt"], 0)],
            &picker,
            "",
        );
        assert_eq!(finder.matches.len(), 2);

        // The query moved on, but the answer to it is a round trip away and
        // the reader is still looking at these rows.
        let discarded = finder.retire_background_corpus(true);
        drop(discarded);
        finder.begin_name_rank("p", true);

        assert_eq!(finder.matches.len(), 2, "the rows stay where they are");
        assert!(
            finder.file_entry(&picker, 0).is_some(),
            "a file row still resolves against the scan it was ranked against"
        );
        assert_eq!(finder.selected, 0, "a new query starts at the top again");
    }

    #[test]
    fn every_non_type_term_can_match_a_different_field() {
        let picker = empty_picker();
        let mut finder = ResourceFinder::default();
        finder.replace_items(
            vec![item(
                ResourceKind::Terminal,
                "[terminal] agent",
                &["cargo", "~/code/runyte-dev"],
                1,
            )],
            &picker,
            "cargo runyte-dev",
        );
        assert_eq!(finder.matches.len(), 1);
    }

    #[test]
    fn non_type_terms_preserve_smart_case_for_files_and_resources() {
        let mut picker = FilePicker::new(
            1,
            PathBuf::from("/project"),
            crate::file_picker::ScanScope::ignoring("/project"),
        );
        picker.enable_unified_finder();
        picker.add_paths(vec![
            crate::file_picker::ScanEntry::file(PathBuf::from("/project/Foo.rs")),
            crate::file_picker::ScanEntry::file(PathBuf::from("/project/foo.rs")),
        ]);
        picker.finish(0, false);
        picker.insert_query_text("Foo");

        let mut finder = ResourceFinder::default();
        finder.replace_items(
            vec![
                item(ResourceKind::Buffer, "Foo buffer", &[], 1),
                item(ResourceKind::Buffer, "foo buffer", &[], 2),
            ],
            &picker,
            "Foo",
        );

        assert_eq!(finder.matches.len(), 2);
        assert!(finder.matches.iter().any(|found| {
            matches!(found.source, FinderMatchSource::File(_))
                && finder
                    .target_for_match(&picker, found)
                    .is_some_and(|target| matches!(target, FinderTarget::File(target) if target.path.ends_with("Foo.rs")))
        }));
        assert!(finder.matches.iter().any(|found| {
            matches!(found.source, FinderMatchSource::Resource(item) if finder.items[item].label == "Foo buffer")
        }));
    }

    #[test]
    fn type_words_rank_without_filtering() {
        let picker = empty_picker();
        let mut finder = ResourceFinder::default();
        finder.replace_items(
            vec![
                item(ResourceKind::Buffer, "notes", &["runyte-dev"], 1),
                item(ResourceKind::Terminal, "shell", &["runyte-dev"], 2),
            ],
            &picker,
            "terminal runyte-dev",
        );
        assert_eq!(finder.matches.len(), 2);
        assert_eq!(
            finder.selected_target(&picker),
            Some(FinderTarget::Resource(ResourceTarget::Terminal(
                TerminalId::from_raw(2)
            )))
        );

        finder.rank(&picker, "buffer terminal runyte-dev");
        assert_eq!(
            finder.selected_target(&picker),
            Some(FinderTarget::Resource(ResourceTarget::Buffer(1)))
        );
    }

    #[test]
    fn type_words_remain_literal_content_terms() {
        let picker = empty_picker();
        let mut finder = ResourceFinder::new(FinderMode::Contents);
        finder.replace_items(
            vec![
                ResourceItem::content(
                    "notes:1",
                    "terminal output",
                    ResourceTarget::BufferLocation {
                        buffer: 1,
                        row: 0,
                        column: 0,
                    },
                    ResourceKind::Buffer,
                ),
                ResourceItem::content(
                    "notes:2",
                    "ordinary output",
                    ResourceTarget::BufferLocation {
                        buffer: 1,
                        row: 1,
                        column: 0,
                    },
                    ResourceKind::Buffer,
                ),
            ],
            &picker,
            "terminal",
        );
        assert_eq!(finder.matches.len(), 1);
        assert_eq!(finder.selected_item().unwrap().detail, "terminal output");
        assert_eq!(
            finder.selected_match().unwrap().detail_emphasis,
            (0.."terminal".chars().count()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_better_incremental_match_becomes_selected_until_navigation_claims_it() {
        let picker = empty_picker();
        let mut finder = ResourceFinder::new(FinderMode::Contents);
        finder.begin_content_scan(&picker, "target", []);
        finder.append_items(
            [ResourceItem::content(
                "notes:1",
                "t a r g e t",
                ResourceTarget::BufferLocation {
                    buffer: 1,
                    row: 0,
                    column: 0,
                },
                ResourceKind::Buffer,
            )],
            &picker,
            "target",
        );
        assert_eq!(finder.selected_item().unwrap().detail, "t a r g e t");

        finder.append_items(
            [ResourceItem::content(
                "notes:2",
                "target",
                ResourceTarget::BufferLocation {
                    buffer: 1,
                    row: 1,
                    column: 0,
                },
                ResourceKind::Buffer,
            )],
            &picker,
            "target",
        );
        assert_eq!(finder.selected_item().unwrap().detail, "target");

        finder.down();
        let claimed = finder.selected_target(&picker).unwrap();
        finder.append_items(
            [ResourceItem::content(
                "notes:3",
                "target target",
                ResourceTarget::BufferLocation {
                    buffer: 1,
                    row: 2,
                    column: 0,
                },
                ResourceKind::Buffer,
            )],
            &picker,
            "target",
        );
        assert_eq!(finder.selected_target(&picker), Some(claimed));
    }

    #[test]
    fn incremental_batches_have_the_same_order_as_one_shot_ranking() {
        let picker = empty_picker();
        let items = (0..600)
            .map(|row| {
                let padding = "x ".repeat(row % 17);
                ResourceItem::content(
                    format!("notes:{}", row + 1),
                    format!("{padding}target {row}"),
                    ResourceTarget::BufferLocation {
                        buffer: 1,
                        row,
                        column: padding.len(),
                    },
                    ResourceKind::Buffer,
                )
            })
            .collect::<Vec<_>>();
        let mut one_shot = ResourceFinder::new(FinderMode::Contents);
        one_shot.replace_items(items.clone(), &picker, "target");

        let mut incremental = ResourceFinder::new(FinderMode::Contents);
        incremental.begin_content_scan(&picker, "target", []);
        for batch in items.chunks(128) {
            incremental.append_items(batch.iter().cloned(), &picker, "target");
        }

        let targets = |finder: &ResourceFinder| {
            finder
                .matches
                .iter()
                .map(|found| finder.target_for_match(&picker, found).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(targets(&incremental), targets(&one_shot));
    }

    #[test]
    fn large_background_content_scan_publishes_only_bounded_snapshots() {
        let mut finder = ResourceFinder::new(FinderMode::Contents);
        finder.begin_content_scan_unmerged("target", Arc::new(HashSet::new()));
        let mut publications = 0;
        for first in (0..CONTENT_ENTRY_LIMIT).step_by(128) {
            publications += usize::from(finder.append_content_items_unmerged(
                (first..(first + 128).min(CONTENT_ENTRY_LIMIT)).map(|row| {
                    ResourceItem::content(
                        format!("notes:{}", row + 1),
                        format!("target {row}"),
                        ResourceTarget::BufferLocation {
                            buffer: 1,
                            row,
                            column: 0,
                        },
                        ResourceKind::Buffer,
                    )
                }),
                "target",
            ));
        }
        finder.finish_content_scan_unmerged(false);

        assert_eq!(publications, 0, "only completion publishes the corpus");
        assert_eq!(finder.resource_matches.len(), CONTENT_ENTRY_LIMIT);
        let context = finder.take_file_rank_context("target");
        assert!(finder.resource_matches.is_empty());
        assert_eq!(context.resource_matches.len(), CONTENT_ENTRY_LIMIT);
    }

    #[test]
    fn retired_content_slot_is_not_reused_until_the_refill_finishes() {
        let mut finder = ResourceFinder::new(FinderMode::Contents);
        finder.begin_content_scan_unmerged("target", Arc::new(HashSet::new()));
        finder.append_content_items_unmerged(
            [ResourceItem::content(
                "notes:1",
                "target old",
                ResourceTarget::BufferLocation {
                    buffer: 1,
                    row: 0,
                    column: 0,
                },
                ResourceKind::Buffer,
            )],
            "target",
        );
        finder.matches = finder
            .resource_matches
            .iter()
            .map(resource_to_finder_match)
            .collect();
        finder.selection_user_owned = true;

        let retired = finder.retire_content_item(0, false).unwrap();
        finder.append_content_items_unmerged(
            [ResourceItem::content(
                "notes:2",
                "target new",
                ResourceTarget::BufferLocation {
                    buffer: 1,
                    row: 1,
                    column: 0,
                },
                ResourceKind::Buffer,
            )],
            "target",
        );

        assert_eq!(retired.detail, "target old");
        assert_eq!(finder.items.len(), 2);
        assert_eq!(finder.resource_matches.last().unwrap().item, 1);
        assert!(!finder.selection_user_owned);
        finder.finish_content_scan_unmerged(false);
        assert_eq!(finder.free_content_items, vec![0]);
    }

    #[test]
    fn finder_type_words_are_not_file_filters() {
        assert_eq!(finder_matching_query("file src picker"), "src picker");
        assert_eq!(finder_matching_query("buffer terminal"), "");
    }

    #[test]
    fn empty_query_keeps_source_order() {
        let picker = empty_picker();
        let mut finder = ResourceFinder::default();
        finder.replace_items(
            vec![
                item(ResourceKind::Buffer, "second", &[], 2),
                item(ResourceKind::Terminal, "first terminal", &[], 1),
            ],
            &picker,
            "",
        );
        assert_eq!(
            finder
                .matches
                .iter()
                .map(|found| found.source)
                .collect::<Vec<_>>(),
            vec![
                FinderMatchSource::Resource(0),
                FinderMatchSource::Resource(1)
            ]
        );
    }

    #[test]
    fn emphasis_positions_are_indices_into_the_original_unicode_label() {
        let picker = empty_picker();
        let mut finder = ResourceFinder::default();
        finder.replace_items(
            vec![item(ResourceKind::Buffer, "İalpha", &[], 1)],
            &picker,
            "alpha",
        );
        assert_eq!(finder.matches[0].emphasis, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_restarted_scan_retires_the_file_matches_ranked_against_its_entries() {
        let mut picker = FilePicker::grep(
            1,
            PathBuf::from("/project"),
            crate::file_picker::ScanScope::ignoring("/project"),
        );
        picker.enable_unified_finder();
        picker.add_content(vec![crate::file_picker::FileHits {
            path: PathBuf::from("/project/alpha.rs"),
            lines: vec![crate::file_picker::LineHit {
                row: 0,
                column: 0,
                text: "needle".to_owned(),
            }],
        }]);
        picker.finish(0, true);
        picker.insert_query_text("needle");

        let mut finder = ResourceFinder::new(FinderMode::Contents);
        finder.merge_files(&picker, "needle");
        let found = finder.selected_match().expect("the file line matched");
        assert!(matches!(found.source, FinderMatchSource::File(0)));
        assert!(finder.file_entry(&picker, 0).is_some());

        // A truncated scan is restarted under the query it could not answer,
        // which refills the entry table from nothing. The rows the previous
        // ranking produced are still here, and index a table this one has
        // replaced.
        let _discarded = picker.restart_content_scan(2);
        picker.add_content(vec![crate::file_picker::FileHits {
            path: PathBuf::from("/project/zeta.rs"),
            lines: vec![crate::file_picker::LineHit {
                row: 41,
                column: 0,
                text: "needle".to_owned(),
            }],
        }]);

        assert_eq!(
            picker.view(0).map(|entry| entry.path.to_path_buf()),
            Some(PathBuf::from("/project/zeta.rs")),
            "the rebuilt table does answer the stale index, which is the hazard"
        );
        assert_eq!(
            finder
                .file_entry(&picker, 0)
                .map(|entry| entry.path.to_path_buf()),
            Some(PathBuf::from("/project/alpha.rs")),
            "a file match reads the table it was ranked against, not the one that replaced it"
        );

        // That table is kept for one generation, so the rows stay on screen
        // while the walk replacing them runs. Once its answer lands, nothing
        // reads them and the index stops resolving at all.
        picker.forget_previous_corpus();
        assert!(
            finder.file_entry(&picker, 0).is_none(),
            "a file match cannot resolve against a table it was not ranked against"
        );
        assert!(
            finder.selected_target(&picker).is_none(),
            "and neither can what Enter would open"
        );
    }
}
