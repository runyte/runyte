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
};

use crate::{
    file_picker::{CONTENT_ENTRY_LIMIT, FilePicker, FilePreview, FuzzyMatcher, PickerTarget},
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

#[derive(Clone, Debug)]
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
    score: i64,
    type_boost: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinderMatchSource {
    File(usize),
    Resource(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinderMatch {
    pub source: FinderMatchSource,
    pub emphasis: Vec<usize>,
    pub detail_emphasis: Vec<usize>,
    score: i64,
    type_boost: bool,
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
    selection_user_owned: bool,
    claimed_selection: Option<FinderTarget>,
    selected_preview: Option<FilePreview>,
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
            selection_user_owned: false,
            claimed_selection: None,
            selected_preview: None,
        }
    }

    pub fn begin_content_scan(
        &mut self,
        picker: &FilePicker,
        query: &str,
        suppressed_paths: impl IntoIterator<Item = PathBuf>,
    ) {
        self.items.clear();
        self.resource_matches.clear();
        self.suppressed_paths = suppressed_paths.into_iter().collect();
        self.loading = true;
        self.limited = false;
        self.selection_user_owned = false;
        self.claimed_selection = None;
        self.selected_preview = None;
        self.merge(picker, query, None);
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

    pub fn finish_content_scan(&mut self, limited: bool) {
        self.loading = false;
        self.limited = limited;
    }

    pub fn replace_items(&mut self, items: Vec<ResourceItem>, picker: &FilePicker, query: &str) {
        let selected = self.preserved_selection(picker);
        self.items = items;
        self.loading = false;
        self.limited = false;
        self.suppressed_paths.clear();
        self.rank_resources(query);
        self.merge(picker, query, selected.as_ref());
    }

    pub fn rank(&mut self, picker: &FilePicker, query: &str) {
        self.selection_user_owned = false;
        self.claimed_selection = None;
        self.rank_resources(query);
        self.merge(picker, query, None);
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
        self.restore_selection(picker, selected);
    }

    fn preserved_selection(&mut self, picker: &FilePicker) -> Option<FinderTarget> {
        if !self.selection_user_owned {
            return None;
        }
        if self.claimed_selection.is_none() {
            self.claimed_selection = self.selected_target(picker);
        }
        self.claimed_selection.clone()
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
        self.selected_match()
            .and_then(|found| self.target_for_match(picker, found))
    }

    fn target_for_match(&self, picker: &FilePicker, found: &FinderMatch) -> Option<FinderTarget> {
        match found.source {
            FinderMatchSource::File(entry) => {
                let view = picker.view(entry)?;
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
        }
    }

    pub fn up(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.matches.len() - 1) % self.matches.len();
            self.selection_user_owned = true;
            self.claimed_selection = None;
        }
    }

    pub fn page_down(&mut self, amount: usize) {
        self.selected = self
            .selected
            .saturating_add(amount.max(1))
            .min(self.matches.len().saturating_sub(1));
        self.selection_user_owned = true;
        self.claimed_selection = None;
    }

    pub fn page_up(&mut self, amount: usize) {
        self.selected = self.selected.saturating_sub(amount.max(1));
        self.selection_user_owned = true;
        self.claimed_selection = None;
    }

    pub fn first(&mut self) {
        self.selected = 0;
        self.selection_user_owned = true;
        self.claimed_selection = None;
    }

    pub fn last(&mut self) {
        self.selected = self.matches.len().saturating_sub(1);
        self.selection_user_owned = true;
        self.claimed_selection = None;
    }
}

fn resource_to_finder_match(found: &ResourceMatch) -> FinderMatch {
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

fn finder_match_order(left: &FinderMatch, right: &FinderMatch) -> std::cmp::Ordering {
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
        let mut picker = FilePicker::new(1, PathBuf::from("/project"));
        picker.finish(0, false);
        picker
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
        let mut picker = FilePicker::new(1, PathBuf::from("/project"));
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
}
