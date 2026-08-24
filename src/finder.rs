// SPDX-License-Identifier: MPL-2.0

//! The in-memory half of the project finder.
//!
//! Project files keep their asynchronous, ignore-aware scanner in
//! [`crate::file_picker`]. Open buffers and terminals are already editor
//! state, so walking the filesystem for them would be both slower and less
//! truthful. This module ranks a bounded snapshot of those resources without
//! knowing how selecting one changes a pane.

use crate::{file_picker::FuzzyMatcher, terminal::TerminalId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinderMode {
    Files,
    Resources,
}

impl FinderMode {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Resources => "Buffers + terminals",
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
    Terminal(TerminalId),
}

#[derive(Clone, Debug)]
pub struct ResourceItem {
    pub label: String,
    pub detail: String,
    pub target: ResourceTarget,
    pub kind: ResourceKind,
    preview: Option<String>,
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
            preview: None,
            fields: searchable,
        }
    }

    pub fn with_preview(mut self, preview: impl Into<String>) -> Self {
        self.preview = Some(preview.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceMatch {
    pub item: usize,
    pub emphasis: Vec<usize>,
    score: i64,
    type_boost: bool,
}

#[derive(Clone, Debug)]
pub struct ResourceFinder {
    pub mode: FinderMode,
    pub items: Vec<ResourceItem>,
    pub matches: Vec<ResourceMatch>,
    pub selected: usize,
}

impl Default for ResourceFinder {
    fn default() -> Self {
        Self {
            mode: FinderMode::Files,
            items: Vec::new(),
            matches: Vec::new(),
            selected: 0,
        }
    }
}

impl ResourceFinder {
    pub fn replace_items(&mut self, items: Vec<ResourceItem>, query: &str) {
        let selected = self.selected_target();
        self.items = items;
        self.rank(query);
        if let Some(selected) = selected
            && let Some(index) = self
                .matches
                .iter()
                .position(|found| self.items[found.item].target == selected)
        {
            self.selected = index;
        }
    }

    pub fn rank(&mut self, query: &str) {
        let parsed = ParsedQuery::new(query);
        let mut matches = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(item, candidate)| parsed.score(candidate).map(|found| (item, found)))
            .map(|(item, (score, type_boost, emphasis))| ResourceMatch {
                item,
                emphasis,
                score,
                type_boost,
            })
            .collect::<Vec<_>>();
        if !query.trim().is_empty() {
            matches.sort_by(|left, right| {
                right
                    .type_boost
                    .cmp(&left.type_boost)
                    .then_with(|| right.score.cmp(&left.score))
                    .then_with(|| left.item.cmp(&right.item))
            });
        }
        self.matches = matches;
        self.selected = 0;
    }

    pub fn selected_item(&self) -> Option<&ResourceItem> {
        self.matches
            .get(self.selected.min(self.matches.len().saturating_sub(1)))
            .and_then(|found| self.items.get(found.item))
    }

    pub fn selected_match(&self) -> Option<&ResourceMatch> {
        self.matches
            .get(self.selected.min(self.matches.len().saturating_sub(1)))
    }

    pub fn selected_target(&self) -> Option<ResourceTarget> {
        self.selected_item().map(|item| item.target)
    }

    pub fn selected_preview(&self) -> Option<&str> {
        self.selected_item()?.preview.as_deref()
    }

    pub fn down(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
        }
    }

    pub fn up(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.matches.len() - 1) % self.matches.len();
        }
    }

    pub fn page_down(&mut self, amount: usize) {
        self.selected = self
            .selected
            .saturating_add(amount.max(1))
            .min(self.matches.len().saturating_sub(1));
    }

    pub fn page_up(&mut self, amount: usize) {
        self.selected = self.selected.saturating_sub(amount.max(1));
    }

    pub fn first(&mut self) {
        self.selected = 0;
    }

    pub fn last(&mut self) {
        self.selected = self.matches.len().saturating_sub(1);
    }
}

struct ParsedQuery {
    terms: Vec<String>,
    buffer_boost: bool,
    terminal_boost: bool,
}

impl ParsedQuery {
    fn new(query: &str) -> Self {
        let mut terms = Vec::new();
        let mut buffer_boost = false;
        let mut terminal_boost = false;
        for term in query.split_whitespace().map(str::to_lowercase) {
            match term.as_str() {
                "buffer" | "buffers" => buffer_boost = true,
                "term" | "terminal" | "terminals" => terminal_boost = true,
                _ => terms.push(term),
            }
        }
        Self {
            terms,
            buffer_boost,
            terminal_boost,
        }
    }

    fn score(&self, item: &ResourceItem) -> Option<(i64, bool, Vec<usize>)> {
        let mut score = 0i64;
        let mut emphasis = Vec::new();
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
        }
        emphasis.sort_unstable();
        emphasis.dedup();
        let type_boost = match item.kind {
            ResourceKind::Buffer => self.buffer_boost,
            ResourceKind::Terminal => self.terminal_boost,
        };
        Some((score, type_boost, emphasis))
    }
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

    #[test]
    fn every_non_type_term_can_match_a_different_field() {
        let mut finder = ResourceFinder::default();
        finder.replace_items(
            vec![item(
                ResourceKind::Terminal,
                "[terminal] agent",
                &["cargo", "~/code/runyte-dev"],
                1,
            )],
            "cargo runyte-dev",
        );
        assert_eq!(finder.matches.len(), 1);
    }

    #[test]
    fn type_words_rank_without_filtering() {
        let mut finder = ResourceFinder::default();
        finder.replace_items(
            vec![
                item(ResourceKind::Buffer, "notes", &["runyte-dev"], 1),
                item(ResourceKind::Terminal, "shell", &["runyte-dev"], 2),
            ],
            "terminal runyte-dev",
        );
        assert_eq!(finder.matches.len(), 2);
        assert_eq!(
            finder.selected_target(),
            Some(ResourceTarget::Terminal(TerminalId::from_raw(2)))
        );

        finder.rank("buffer terminal runyte-dev");
        assert_eq!(finder.selected_target(), Some(ResourceTarget::Buffer(1)));
    }

    #[test]
    fn empty_query_keeps_source_order() {
        let mut finder = ResourceFinder::default();
        finder.replace_items(
            vec![
                item(ResourceKind::Buffer, "second", &[], 2),
                item(ResourceKind::Terminal, "first terminal", &[], 1),
            ],
            "",
        );
        assert_eq!(
            finder
                .matches
                .iter()
                .map(|found| found.item)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn emphasis_positions_are_indices_into_the_original_unicode_label() {
        let mut finder = ResourceFinder::default();
        finder.replace_items(vec![item(ResourceKind::Buffer, "İalpha", &[], 1)], "alpha");
        assert_eq!(finder.matches[0].emphasis, vec![1, 2, 3, 4, 5]);
    }
}
