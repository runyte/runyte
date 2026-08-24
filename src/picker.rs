// SPDX-License-Identifier: MPL-2.0

//! A semantically typed filterable result list.
//!
//! Symbols, references, diagnostics, and code actions are all the same
//! interaction: a list, a substring filter, a selection, and an Enter. This is
//! that interaction once, holding presentation-neutral state the way
//! other editor result surfaces do. The buffer and global-search
//! pickers reuse it instead of copying that interaction. [`ListPurpose`]
//! keeps reports, finite choices, and buffer management from acquiring picker
//! behavior merely because they share the same rows.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListPurpose {
    Picker,
    Choice,
    Manager,
    Report,
}

#[derive(Clone, Debug)]
pub struct PickerItem {
    pub label: String,
    pub detail: String,
    /// What the row stands for, resolved by whoever opened the picker.
    ///
    /// The picker deliberately knows nothing about locations, actions, or
    /// buffers: filtering reorders and hides rows, so an index into the
    /// opener's own results is the only thing that stays meaningful.
    pub index: usize,
    /// Lowercased haystack the filter matches against.
    search: String,
    /// Optional full text shown beside the result list.
    preview: Option<String>,
    /// Which named group the row belongs to, when the picker offers any.
    /// Deliberately outside `search`: a group is a separate axis from the
    /// filter, narrowed with Tab rather than by typing.
    tag: Option<String>,
    /// Whether the thing behind the row is dormant, for example a session
    /// whose host is not running. The row still filters, selects, and acts
    /// exactly like any other; only its weight changes.
    dimmed: bool,
}

impl PickerItem {
    pub fn new(label: impl Into<String>, detail: impl Into<String>, index: usize) -> Self {
        let label = label.into();
        let detail = detail.into();
        Self {
            search: format!("{label} {detail}").to_lowercase(),
            label,
            detail,
            index,
            preview: None,
            tag: None,
            dimmed: false,
        }
    }

    pub fn searchable(
        label: impl Into<String>,
        detail: impl Into<String>,
        search: impl Into<String>,
        index: usize,
    ) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            search: search.into().to_lowercase(),
            index,
            preview: None,
            tag: None,
            dimmed: false,
        }
    }

    pub fn with_preview(mut self, preview: impl Into<String>) -> Self {
        self.preview = Some(preview.into());
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Marks the row as dormant so frontends draw it at reduced weight.
    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }

    pub fn is_dimmed(&self) -> bool {
        self.dimmed
    }
}

#[derive(Clone, Debug)]
pub struct ListPicker {
    pub title: String,
    pub purpose: ListPurpose,
    /// User-facing verb performed by Enter. Reports deliberately have none.
    pub primary_action: Option<String>,
    /// Optional additional action and its key hint, for example buffer actions.
    pub secondary_action: Option<(String, String)>,
    pub items: Vec<PickerItem>,
    pub filter: String,
    pub selected: usize,
    /// First report row shown in the viewport. Reports have no visible
    /// selection, so their scroll position cannot borrow `selected` without
    /// making initial navigation appear to do nothing.
    pub report_offset: usize,
    pub show_preview: bool,
    preview_title: Option<String>,
    /// Named groups the rows can be narrowed to, in the order Tab cycles
    /// through them. Empty means the picker offers no such narrowing.
    ///
    /// The picker does not know what a tag means: the opener names the groups
    /// and gives each item one, so a theme's dark or light ground and any
    /// later grouping are the same interaction rather than two.
    tags: Vec<String>,
    /// Which group is shown; `None` shows every row.
    tag: Option<usize>,
    fuzzy: bool,
}

impl ListPicker {
    pub fn new(title: impl Into<String>, items: Vec<PickerItem>) -> Self {
        Self {
            title: title.into(),
            purpose: ListPurpose::Picker,
            primary_action: Some("open".to_owned()),
            secondary_action: None,
            items,
            filter: String::new(),
            selected: 0,
            report_offset: 0,
            show_preview: true,
            preview_title: None,
            tags: Vec::new(),
            tag: None,
            fuzzy: false,
        }
    }

    pub fn fuzzy(title: impl Into<String>, items: Vec<PickerItem>) -> Self {
        Self {
            fuzzy: true,
            ..Self::new(title, items)
        }
    }

    pub fn with_preview(mut self, title: impl Into<String>) -> Self {
        self.preview_title = Some(title.into());
        self
    }

    /// Offers Tab-cycled narrowing to each named group in turn, starting from
    /// the unnarrowed list. Rows carrying no tag appear only in that view.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self.tag = None;
        self
    }

    pub fn with_primary_action(mut self, action: impl Into<String>) -> Self {
        self.primary_action = Some(action.into());
        self
    }

    pub fn as_choice(mut self, action: impl Into<String>) -> Self {
        self.purpose = ListPurpose::Choice;
        self.primary_action = Some(action.into());
        self
    }

    pub fn as_manager(
        mut self,
        action: impl Into<String>,
        secondary_key: impl Into<String>,
        secondary_action: impl Into<String>,
    ) -> Self {
        self.purpose = ListPurpose::Manager;
        self.primary_action = Some(action.into());
        self.secondary_action = Some((secondary_key.into(), secondary_action.into()));
        self
    }

    pub fn as_report(mut self) -> Self {
        self.purpose = ListPurpose::Report;
        self.primary_action = None;
        self.secondary_action = None;
        self
    }

    pub fn accepts_filter_input(&self) -> bool {
        self.purpose != ListPurpose::Report
    }

    pub fn has_tags(&self) -> bool {
        !self.tags.is_empty()
    }

    /// What the current view is called, for the key hint Tab is named in.
    pub fn tag_label(&self) -> &str {
        self.tag
            .and_then(|index| self.tags.get(index))
            .map_or("all", String::as_str)
    }

    /// Shows the next group, wrapping back to the unnarrowed list.
    ///
    /// The row under the cursor keeps its selection wherever it survives the
    /// change, so cycling past a group a theme is not in does not silently
    /// preview a different one.
    pub fn cycle_tag(&mut self) {
        if self.tags.is_empty() {
            return;
        }
        let visible = self.visible_indices();
        let selected = visible
            .get(self.selected.min(visible.len().saturating_sub(1)))
            .copied();
        self.tag = match self.tag {
            None => Some(0),
            Some(index) if index + 1 < self.tags.len() => Some(index + 1),
            Some(_) => None,
        };
        let visible = self.visible_indices();
        self.selected = selected
            .and_then(|item| visible.iter().position(|index| *index == item))
            .unwrap_or(0);
    }

    pub fn has_preview(&self) -> bool {
        self.preview_title.is_some()
    }

    pub fn preview_title(&self) -> Option<&str> {
        self.preview_title.as_deref()
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        let query = self.filter.to_lowercase();
        let tag = self.tag.and_then(|index| self.tags.get(index));
        let in_group = |item: &PickerItem| match tag {
            None => true,
            Some(tag) => item.tag.as_ref() == Some(tag),
        };
        if !self.fuzzy {
            return self
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    (in_group(item) && (query.is_empty() || item.search.contains(&query)))
                        .then_some(index)
                })
                .collect();
        }
        let mut matcher = crate::file_picker::FuzzyMatcher::new(&query);
        let mut matches = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| in_group(item))
            .filter_map(|(index, item)| {
                matcher.score(&item.search).map(|(score, _)| (index, score))
            })
            .collect::<Vec<_>>();
        if !query.is_empty() {
            matches.sort_by(|(left_index, left_score), (right_index, right_score)| {
                right_score
                    .cmp(left_score)
                    .then_with(|| left_index.cmp(right_index))
            });
        }
        matches.into_iter().map(|(index, _)| index).collect()
    }

    pub fn selected_item(&self) -> Option<&PickerItem> {
        let indices = self.visible_indices();
        indices
            .get(self.selected.min(indices.len().saturating_sub(1)))
            .and_then(|index| self.items.get(*index))
    }

    pub fn selected_preview(&self) -> Option<&str> {
        self.selected_item()?.preview.as_deref()
    }

    pub fn selected_preview_emphasis(&self) -> Vec<usize> {
        let Some(preview) = self.selected_preview() else {
            return Vec::new();
        };
        crate::file_picker::fuzzy_match(&self.filter.to_lowercase(), preview)
            .map_or_else(Vec::new, |(_, positions)| positions)
    }

    pub fn item_label_emphasis(&self, item: &PickerItem) -> Vec<usize> {
        crate::file_picker::fuzzy_match(&self.filter.to_lowercase(), &item.label)
            .map_or_else(Vec::new, |(_, positions)| positions)
    }

    pub fn push_filter(&mut self, character: char) {
        self.filter.push(character);
        self.selected = 0;
    }

    pub fn pop_filter(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.selected = 0;
    }

    pub fn up(&mut self) {
        let count = self.visible_indices().len();
        if count > 0 {
            self.selected = (self.selected.min(count - 1) + count - 1) % count;
        }
    }

    pub fn down(&mut self) {
        let count = self.visible_indices().len();
        if count > 0 {
            self.selected = (self.selected.min(count - 1) + 1) % count;
        }
    }

    pub fn page_up(&mut self, page: usize) {
        self.selected = self.selected.saturating_sub(page);
    }

    pub fn page_down(&mut self, page: usize) {
        let last = self.visible_indices().len().saturating_sub(1);
        self.selected = self.selected.saturating_add(page).min(last);
    }

    pub fn first(&mut self) {
        self.selected = 0;
    }

    pub fn last(&mut self) {
        self.selected = self.visible_indices().len().saturating_sub(1);
    }

    pub fn report_up(&mut self) {
        self.report_offset = self.report_offset.saturating_sub(1);
    }

    pub fn report_down(&mut self) {
        self.report_offset = self
            .report_offset
            .saturating_add(1)
            .min(self.visible_indices().len().saturating_sub(1));
    }

    pub fn report_page_up(&mut self, page: usize) {
        self.report_offset = self.report_offset.saturating_sub(page);
    }

    pub fn report_page_down(&mut self, page: usize) {
        self.report_offset = self
            .report_offset
            .saturating_add(page)
            .min(self.visible_indices().len().saturating_sub(1));
    }

    pub fn report_first(&mut self) {
        self.report_offset = 0;
    }

    pub fn report_last(&mut self) {
        self.report_offset = self.visible_indices().len().saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker() -> ListPicker {
        ListPicker::new(
            "Symbols",
            vec![
                PickerItem::new("alpha", "function", 0),
                PickerItem::new("beta", "struct", 1),
                PickerItem::new("gamma", "function", 2),
            ],
        )
    }

    #[test]
    fn filtering_matches_the_label_and_the_detail() {
        let mut picker = picker();
        for character in "struct".chars() {
            picker.push_filter(character);
        }
        assert_eq!(picker.visible_indices(), vec![1]);
        assert_eq!(picker.selected_item().unwrap().label, "beta");
    }

    #[test]
    fn navigation_wraps_and_clamps_to_the_filtered_rows() {
        let mut picker = picker();
        picker.up();
        assert_eq!(picker.selected_item().unwrap().label, "gamma");
        picker.down();
        assert_eq!(picker.selected_item().unwrap().label, "alpha");
        picker.last();
        assert_eq!(picker.selected, 2);
        picker.push_filter('b');
        assert_eq!(picker.selected, 0);
        assert_eq!(picker.selected_item().unwrap().label, "beta");
    }

    #[test]
    fn an_empty_result_has_no_selection_and_does_not_panic() {
        let mut picker = ListPicker::new("Symbols", Vec::new());
        picker.down();
        picker.up();
        picker.page_down(10);
        picker.last();
        assert!(picker.selected_item().is_none());
    }

    fn tagged() -> ListPicker {
        ListPicker::new(
            "theme",
            vec![
                PickerItem::new("dawn", "choice", 0).with_tag("light"),
                PickerItem::new("dusk", "choice", 1).with_tag("dark"),
                PickerItem::new("midnight", "choice", 2).with_tag("dark"),
                PickerItem::new("terminal", "choice", 3),
            ],
        )
        .with_tags(vec!["dark".to_owned(), "light".to_owned()])
    }

    #[test]
    fn cycling_tags_narrows_to_each_group_and_back_to_every_row() {
        let mut picker = tagged();
        assert_eq!(picker.tag_label(), "all");
        assert_eq!(picker.visible_indices(), vec![0, 1, 2, 3]);

        picker.cycle_tag();
        assert_eq!(picker.tag_label(), "dark");
        assert_eq!(picker.visible_indices(), vec![1, 2]);

        picker.cycle_tag();
        assert_eq!(picker.tag_label(), "light");
        assert_eq!(picker.visible_indices(), vec![0]);

        picker.cycle_tag();
        assert_eq!(picker.tag_label(), "all");
        assert_eq!(picker.visible_indices(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn cycling_tags_keeps_the_selected_row_wherever_it_survives() {
        let mut picker = tagged();
        picker.last();
        assert_eq!(picker.selected_item().unwrap().label, "terminal");

        // An untagged row belongs to no group, so the narrowed list falls back
        // to its first row rather than keeping a stale index.
        picker.cycle_tag();
        assert_eq!(picker.selected_item().unwrap().label, "dusk");

        picker.cycle_tag();
        picker.cycle_tag();
        assert_eq!(picker.tag_label(), "all");

        picker.down();
        picker.down();
        assert_eq!(picker.selected_item().unwrap().label, "midnight");
        picker.cycle_tag();
        assert_eq!(picker.tag_label(), "dark");
        assert_eq!(picker.selected_item().unwrap().label, "midnight");
    }

    #[test]
    fn the_tag_narrowing_and_the_typed_filter_apply_together() {
        let mut picker = tagged();
        picker.cycle_tag();
        for character in "du".chars() {
            picker.push_filter(character);
        }
        assert_eq!(picker.visible_indices(), vec![1]);
        assert_eq!(picker.selected_item().unwrap().label, "dusk");

        // "dawn" matches the filter but not the group.
        picker.cycle_tag();
        assert_eq!(picker.tag_label(), "light");
        assert!(picker.visible_indices().is_empty());
    }

    #[test]
    fn a_picker_without_tags_ignores_the_cycle() {
        let mut picker = picker();
        assert!(!picker.has_tags());
        picker.cycle_tag();
        assert_eq!(picker.tag_label(), "all");
        assert_eq!(picker.visible_indices(), vec![0, 1, 2]);
    }

    #[test]
    fn report_navigation_moves_its_viewport_from_the_first_key() {
        let mut report = picker().as_report();

        report.report_down();
        assert_eq!(report.report_offset, 1);
        report.report_page_down(10);
        assert_eq!(report.report_offset, 2);
        report.report_up();
        assert_eq!(report.report_offset, 1);
        report.report_first();
        assert_eq!(report.report_offset, 0);
    }

    #[test]
    fn fuzzy_filter_matches_ordered_non_contiguous_characters_and_ranks_them() {
        let mut substring = ListPicker::new(
            "Commits",
            vec![PickerItem::new("Workspace Git refresh", "", 0)],
        );
        for character in "wgr".chars() {
            substring.push_filter(character);
        }
        assert!(substring.visible_indices().is_empty());

        let mut picker = ListPicker::fuzzy(
            "Commits",
            vec![
                PickerItem::new("Fix the workspace gutter", "abc", 0),
                PickerItem::new("Workspace Git refresh", "def", 1),
                PickerItem::new("Unrelated", "ghi", 2),
            ],
        );
        for character in "wgr".chars() {
            picker.push_filter(character);
        }
        let visible = picker.visible_indices();
        assert_eq!(visible, vec![1, 0]);
        assert_eq!(picker.selected_item().unwrap().index, 1);
    }

    #[test]
    fn preview_matches_distinguish_direct_text_from_fuzzy_subsequences() {
        let item = PickerItem::searchable(
            "abcdef123456 Refresh workspace Git state",
            "",
            "Refresh workspace Git state Ada 2026-08-16 abcdef123456",
            0,
        )
        .with_preview("Ada · 2026-08-16\n\nRefresh workspace Git state\nFull body");
        let mut picker = ListPicker::fuzzy("Git commits", vec![item]).with_preview("Commit");

        for character in "workspace Git".chars() {
            picker.push_filter(character);
        }
        assert!(crate::file_picker::is_direct_match(
            &picker.selected_preview_emphasis(),
            &picker.filter
        ));

        picker.clear_filter();
        for character in "wgs".chars() {
            picker.push_filter(character);
        }
        let emphasis = picker.selected_preview_emphasis();
        assert!(!emphasis.is_empty());
        assert!(!crate::file_picker::is_direct_match(
            &emphasis,
            &picker.filter
        ));
    }
}
