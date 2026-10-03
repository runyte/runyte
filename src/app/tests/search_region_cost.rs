// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn scoped_matching_preserves_individual_regions_with_unordered_and_overlapping_bounds() {
    let text = "αabcβabcγ";
    let regions = [(5, 7), (2, 4), (0, 2), (1, 3), (5, 8), (9, 9)];
    for pattern in ["a", "abc", "αa", "αabc", "βabc", "(?m)^|$", ".*"] {
        let all = text_matches(text, pattern, SearchMode::Regex, None).unwrap();
        let expected = SearchMode::Regex
            .compile(pattern)
            .unwrap()
            .find_iter(text)
            .zip(all)
            .filter_map(|(found, range)| {
                let from = text[..found.start()].chars().count();
                let to = text[..found.end()].chars().count();
                regions
                    .iter()
                    .any(|(start, end)| *start <= from && to <= *end)
                    .then_some(range)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            text_matches(text, pattern, SearchMode::Regex, Some(&regions)).unwrap(),
            expected,
            "{pattern}"
        );
        assert!(
            text_matches(text, pattern, SearchMode::Regex, Some(&[]))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn scoped_matching_handles_many_disjoint_selections() {
    let count = 20_000;
    let text = "aa ".repeat(count);
    let regions = (0..count).map(|i| (i * 3, i * 3 + 2)).collect::<Vec<_>>();
    let matches = text_matches(&text, "a", SearchMode::Sensitive, Some(&regions)).unwrap();
    assert_eq!(matches.len(), count * 2);
    for (i, pair) in matches.chunks_exact(2).enumerate() {
        assert_eq!(pair, [Range::point(i * 3), Range::point(i * 3 + 1)]);
    }
}
