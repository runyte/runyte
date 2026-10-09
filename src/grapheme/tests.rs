// SPDX-License-Identifier: MPL-2.0

use super::*;
use ropey::Rope;

const EMOJI: [&str; 10] = ["🤷‍♂", "🤷‍♀️", "😀", "😗", "🤡", "👍🏽", "🇵🇱", "❤️", "👨‍👩‍👧‍👦", "1️⃣"];

fn boundaries(text: &str) -> Vec<usize> {
    let rope = Rope::from_str(text);
    (0..=rope.len_chars())
        .filter(|offset| floor(rope.slice(..), *offset) == *offset)
        .collect()
}

fn expected_boundaries(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    let mut offset = 0;
    for cluster in text.graphemes(true) {
        offset += cluster.chars().count();
        offsets.push(offset);
    }
    offsets
}

#[test]
fn emoji_sequences_are_one_cluster_with_their_drawn_width() {
    for emoji in EMOJI {
        let measured = clusters(emoji.chars()).collect::<Vec<_>>();
        assert_eq!(measured.len(), 1, "{emoji:?} split");
        assert_eq!(measured[0].chars, emoji.chars().count());
        assert_eq!(
            measured[0].width,
            UnicodeWidthStr::width(emoji),
            "{emoji:?}"
        );
    }
    assert_eq!(width("🤷‍♀️"), 2);
    assert_eq!(width("👨‍👩‍👧‍👦"), 2);
    assert_eq!(width("👍🏽"), 2);
    assert_eq!(str_width("a🤷‍♀️b"), 4);
    // Code-point sums were what made emoji glitch.
    assert_eq!(
        "👨‍👩‍👧‍👦"
            .chars()
            .map(|character| UnicodeWidthChar::width(character).unwrap_or(0))
            .sum::<usize>(),
        8
    );
}

#[test]
fn other_scripts_keep_the_width_their_code_points_sum_to() {
    for text in [
        "कि",
        "क्षि",
        "กำ",
        "\u{1100}\u{1161}\u{11A8}",
        "e\u{301}",
        "漢字",
        "a\tb",
    ] {
        let summed = text
            .chars()
            .map(|character| UnicodeWidthChar::width(character).unwrap_or(0))
            .sum::<usize>();
        assert_eq!(str_width(text), summed, "{text:?}");
    }
}

#[test]
fn cells_put_a_cluster_width_on_its_first_code_point() {
    let cells = cells("a🤷‍♀️b".chars()).collect::<Vec<_>>();
    assert_eq!(
        cells,
        [
            ('a', 1),
            ('🤷', 2),
            ('\u{200D}', 0),
            ('♀', 0),
            ('\u{FE0F}', 0),
            ('b', 1)
        ]
    );
    // A lone mark has nothing to attach to and is its own cluster.
    assert_eq!(
        clusters("\u{301}x".chars()).collect::<Vec<_>>(),
        [
            Cluster { chars: 1, width: 0 },
            Cluster { chars: 1, width: 1 }
        ]
    );
}

#[test]
fn boundaries_match_unicode_segmentation_around_emoji_and_line_endings() {
    for text in [
        "plain ascii text",
        "a🤷‍♀️b😀c",
        "🤷‍♂🤷‍♀️😀😗🤡",
        "👍🏽 🇵🇱🇩🇪 ❤️ 1️⃣",
        "x\r\ny\nz",
        "e\u{301}\u{302}\n🇵🇱",
        "👨‍👩‍👧‍👦👨‍👩‍👧‍👦",
    ] {
        assert_eq!(boundaries(text), expected_boundaries(text), "{text:?}");
    }
}

#[test]
fn next_and_previous_step_over_whole_clusters() {
    let rope = Rope::from_str("a🤷‍♀️b");
    let text = rope.slice(..);
    assert_eq!(next(text, 0), 1);
    assert_eq!(next(text, 1), 5);
    assert_eq!(next(text, 3), 5);
    assert_eq!(next(text, 5), 6);
    assert_eq!(next(text, 6), 6);
    assert_eq!(previous(text, 6), 5);
    assert_eq!(previous(text, 5), 1);
    assert_eq!(previous(text, 3), 0);
    assert_eq!(previous(text, 0), 0);
    assert_eq!(cluster_at(text, 2), (1, 5));
    assert_eq!(cluster_at(text, 9), (6, 6));
}

#[test]
fn long_flag_runs_and_hostile_marks_stay_bounded() {
    // Regional indicators pair from the start of their run; a window that
    // started inside the run would pair them the wrong way round.
    let flags = "🇵🇱".repeat(40);
    let text = format!("x{flags}");
    let rope = Rope::from_str(&text);
    for offset in 1..=80 {
        let start = if offset % 2 == 1 { offset } else { offset - 1 };
        assert_eq!(floor(rope.slice(..), offset), start, "flag at {offset}");
    }

    let marks = format!("a{}", "\u{301}".repeat(10_000));
    let rope = Rope::from_str(&marks);
    let text = rope.slice(..);
    let end = next(text, 0);
    assert!(end <= MAX_CLUSTER_CHARS, "unbounded cluster reached {end}");
    let middle = floor(text, 5_000);
    assert!(5_000 - middle <= MAX_CLUSTER_CHARS);
}
