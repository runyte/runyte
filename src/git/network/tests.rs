// SPDX-License-Identifier: MPL-2.0

use super::*;

fn commit(oid: &str, parents: &[&str]) -> CommitSummary {
    CommitSummary {
        oid: oid.into(),
        abbreviated: oid.into(),
        parents: parents.iter().map(|parent| (*parent).into()).collect(),
        author: "A B".into(),
        author_time: 0,
        author_date: "1970-01-01".into(),
        author_datetime: "1970-01-01 00:00".into(),
        subject: String::new(),
        decorations: vec![],
    }
}

fn diagram(row: &GraphRow, ascii: bool) -> Vec<String> {
    std::iter::once(&row.node)
        .chain(&row.connectors)
        .map(|line| line.text(ascii, row.width()).trim_end().to_owned())
        .collect()
}

#[test]
fn merge_and_rejoin_use_separate_diagonals_and_preserve_first_parent_color() {
    let mut lanes = GraphLanes::default();
    let merge = lanes.row(commit("merge", &["a", "b"]), false);
    assert_eq!(diagram(&merge, false), ["●", "│╲"]);
    assert_eq!(diagram(&merge, true), ["*", "|\\"]);
    assert_eq!(lanes.colors, [0, 1]);
    let a = lanes.row(commit("a", &["root"]), false);
    assert_eq!(diagram(&a, false), ["● │"]);
    assert_eq!(a.node.cells[0].color, 0);
    let b = lanes.row(commit("b", &["root"]), false);
    assert_eq!(diagram(&b, false), ["│ ●", "│╱"]);
    assert_eq!(b.connectors[0].cells[1].color, 1);
    assert_eq!(lanes.pending, [Some("root".into())]);
    assert_eq!(lanes.colors, [0]);
    let root = lanes.row(commit("root", &[]), false);
    assert_eq!(diagram(&root, false), ["●", ""]);
    assert!(lanes.pending.is_empty());
}

#[test]
fn first_parent_already_pending_retains_destination_color_through_compaction() {
    let mut lanes = GraphLanes::default();
    lanes.row(commit("merge", &["child", "base"]), false);
    let row = lanes.row(commit("child", &["base"]), false);
    assert_eq!(diagram(&row, false), ["● │", "│╱"]);
    assert_eq!(lanes.pending, [Some("base".into())]);
    assert_eq!(lanes.colors, [1]);
    assert_eq!(lanes.row(commit("base", &[]), false).node.cells[0].color, 1);
}

#[test]
fn distant_join_crosses_without_joining_intermediate_history() {
    let mut lanes = GraphLanes {
        pending: ["base", "unrelated", "tip"]
            .map(|oid| Some(oid.into()))
            .into(),
        colors: vec![0, 1, 2],
        next_color: 3,
    };
    let row = lanes.row(commit("tip", &["base"]), false);
    assert_eq!(diagram(&row, false), ["│ │ ●", "│  ╳", "│ │ │", "│╱ ╱"]);
    assert_eq!(diagram(&row, true), ["| | *", "|  x", "| | |", "|/ /"]);
    assert_eq!(row.connectors[1].cells[2].color, 2);
    assert_eq!(row.connectors[1].cells[4].color, 1);
    assert_eq!(
        lanes.pending,
        [Some("base".into()), Some("unrelated".into())]
    );
    assert_eq!(lanes.colors, [0, 1]);
    assert_eq!(row.edges, [("base".into(), Some(0))]);
}

#[test]
fn root_removal_compacts_surviving_paths_without_recoloring() {
    let mut lanes = GraphLanes::default();
    lanes.row(commit("merge", &["island", "continuing"]), false);
    let row = lanes.row(commit("island", &[]), false);
    assert_eq!(diagram(&row, false), ["● │", " ╱"]);
    assert_eq!(lanes.pending, [Some("continuing".into())]);
    assert_eq!(lanes.colors, [1]);
    let row = lanes.row(commit("continuing", &["old"]), false);
    assert_eq!(diagram(&row, false), ["●"]);
    assert_eq!(row.node.cells[0].color, 1);
}

#[test]
fn octopus_expansion_keeps_colors_and_all_parent_destinations() {
    let mut lanes = GraphLanes::default();
    let row = lanes.row(commit("merge", &["a", "b", "c"]), false);
    assert_eq!(diagram(&row, false), ["●", "│╲", "│ │", "│╲ ╲"]);
    for (parent, lane) in &row.edges {
        assert_eq!(lanes.pending[lane.unwrap()].as_ref(), Some(parent));
    }
    // The b path moves right when c is inserted and keeps its color.
    assert_eq!(row.connectors[0].cells[1].color, 1);
    assert_eq!(row.connectors[2].cells[3].color, 1);
    assert_eq!(lanes.colors, [0, 2, 1]);
}

#[test]
fn captured_cursor_replay_preserves_geometry_colors_and_parent_mapping() {
    let mut lanes = GraphLanes::default();
    lanes.row(commit("merge", &["a", "b", "c"]), false);
    let checkpoint = lanes.clone();
    let row = lanes.row(commit("a", &["c", "root"]), false);
    assert_eq!(
        row,
        checkpoint.clone().row(commit("a", &["c", "root"]), false)
    );
    for (parent, lane) in row.edges {
        assert_eq!(lanes.pending[lane.unwrap()].as_ref(), Some(&parent));
    }
}

#[test]
fn lane_and_routing_limits_are_explicit_without_losing_parent_identities() {
    let parents = (0..MAX_NETWORK_LANES + 2)
        .map(|index| format!("p{index}"))
        .collect::<Vec<_>>();
    let mut lanes = GraphLanes::default();
    let row = lanes.row(
        commit(
            "merge",
            &parents.iter().map(String::as_str).collect::<Vec<_>>(),
        ),
        true,
    );
    assert_eq!(row.edges.len(), parents.len());
    assert_eq!(lanes.pending.len(), MAX_NETWORK_LANES);
    assert!(row.route_note().contains("overflow"));
    assert!(row.route_note().contains("shallow boundary"));
    assert!(row.connectors.len() <= MAX_CONNECTOR_ROWS);
    let before = lanes.clone();
    let row = lanes.row(commit("unplaced", &["missing"]), false);
    assert_eq!(row.lane, None);
    assert_eq!(row.edges, [("missing".into(), None)]);
    assert!(row.route_note().contains("unplaced commit"));
    assert_eq!(lanes, before);
}

#[test]
fn dense_existing_parent_routes_fit_block_budget_or_report_omissions() {
    let mut lanes = GraphLanes {
        pending: (0..9).map(|index| Some(format!("p{index}"))).collect(),
        colors: (0..9).map(|index| index % 4).collect(),
        next_color: 1,
    };
    let parents = (0..9)
        .rev()
        .map(|index| format!("p{index}"))
        .collect::<Vec<_>>();
    let row = lanes.row(
        commit(
            "merge",
            &parents.iter().map(String::as_str).collect::<Vec<_>>(),
        ),
        false,
    );
    assert!(row.connectors.len() <= MAX_CONNECTOR_ROWS);
    assert!(row.width() <= MAX_NETWORK_LANES);
    assert_eq!(row.edges.len(), parents.len());
    for (parent, lane) in &row.edges {
        if let Some(lane) = lane {
            assert_eq!(lanes.pending[*lane].as_ref(), Some(parent));
        } else {
            assert!(row.route_note().contains("overflow"));
        }
    }
    assert!(
        lanes
            .pending
            .iter()
            .flatten()
            .all(|oid| parents.contains(oid))
    );
    assert_eq!(lanes.pending.len(), 9);
    assert!(
        row.edges.iter().any(|(_, lane)| lane.is_none()),
        "dense routing must label undrawn parents"
    );
    // Two-byte cells keep the additional retained graph payload bounded.
    assert_eq!(std::mem::size_of::<GraphCell>(), 2);
}

#[test]
fn initials_and_labels_keep_unicode_text_and_ascii_head_spelling() {
    assert_eq!(author_initials("Ada Lovelace"), "AL");
    assert_eq!(author_initials(""), "??");
    assert_eq!(author_initials("界 界"), "界界");
    let mut value = commit("0123456789abcdef", &[]);
    value.decorations = vec!["HEAD → main".into()];
    let row = GraphLanes::default().row(value, false);
    assert!(row.text(true).contains("[HEAD -> main]"));
    assert!(row.text(false).contains("[HEAD -> main]"));
}

#[test]
fn metadata_columns_use_display_cells_with_unicode_initials() {
    use unicode_width::UnicodeWidthStr;
    for author in ["A B", "李 小龍", "A\u{301}lpha Beta", "👩‍💻 Coder"] {
        let mut summary = commit(&"a".repeat(64), &[]);
        summary.author = author.into();
        summary.subject = "SUBJECT".into();
        let row = GraphLanes::default().row(summary, false);
        let text = row.text_with_width(false, 2);
        assert_eq!(text[..text.find("SUBJECT").unwrap()].width(), 25);
        assert_eq!(&text[..12], "aaaaaaaaaaaa");
    }
    assert_eq!(author_initials("A\u{301}lpha Beta"), "A\u{301}B");
}

#[test]
fn rendered_paths_recover_exact_ancestry_in_many_small_dags() {
    use std::collections::BTreeSet;
    // Decode rendered diagonals and crossings independently of GraphRow.edges.
    // A lane carries the child nodes whose outgoing paths have joined there.
    for seed in 0..100_u64 {
        let mut random = seed + 1;
        let mut lanes = GraphLanes::default();
        let mut live = vec![BTreeSet::new(); MAX_NETWORK_LANES];
        let mut expected = BTreeSet::new();
        let mut actual = BTreeSet::new();
        for node in 0..10 {
            let mut parents = Vec::new();
            for parent in node + 1..10 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                if random % 4 == 0 && parents.len() < 3 {
                    parents.push(parent.to_string());
                    expected.insert((node, parent));
                }
            }
            let row = lanes.row(
                commit(
                    &node.to_string(),
                    &parents.iter().map(String::as_str).collect::<Vec<_>>(),
                ),
                false,
            );
            assert!(!row.route_note().contains("overflow"));
            for (lane, incoming) in live.iter_mut().enumerate() {
                match row
                    .node
                    .cells
                    .get(lane * 2)
                    .map(|cell| cell.character(true))
                {
                    Some('*') => {
                        actual.extend(incoming.iter().map(|child| (*child, node)));
                        *incoming = BTreeSet::from([node]);
                    }
                    Some('|') => {}
                    _ => incoming.clear(),
                }
            }
            for line in row.connectors.iter().step_by(2) {
                live = decode_route(&live, line);
            }
        }
        assert_eq!(actual, expected, "rendered ancestry for seed {seed}");
        assert!(lanes.pending.is_empty());
    }
}

fn decode_route(
    live: &[std::collections::BTreeSet<usize>],
    line: &GraphLine,
) -> Vec<std::collections::BTreeSet<usize>> {
    use std::collections::BTreeSet;
    if let Some(target) = line
        .cells
        .iter()
        .position(|cell| matches!(cell.character(true), '<' | '>'))
    {
        let source = line
            .cells
            .iter()
            .position(|cell| cell.character(true) == '+')
            .unwrap();
        let mut next = live.to_vec();
        next[target / 2].extend(&live[source / 2]);
        return next;
    }
    let mut next = vec![BTreeSet::new(); MAX_NETWORK_LANES];
    for (column, cell) in line.cells.iter().enumerate() {
        let lane = column / 2;
        match cell.character(true) {
            '|' => next[lane].extend(&live[lane]),
            '/' => next[lane].extend(&live[lane + 1]),
            '\\' => next[lane + 1].extend(&live[lane]),
            'x' => {
                next[lane].extend(&live[lane + 1]);
                next[lane + 1].extend(&live[lane]);
            }
            ' ' => {}
            other => panic!("unexpected connector {other}"),
        }
    }
    next
}

#[test]
fn saturated_lanes_still_route_existing_secondary_parents_in_both_directions() {
    for source in [0, 15] {
        let mut lanes = GraphLanes {
            pending: (0..16).map(|i| Some(format!("p{i}"))).collect(),
            colors: (0..16).map(|i| i % 4).collect(),
            next_color: 0,
        };
        let colors = lanes.colors.clone();
        let row = lanes.row(commit(&format!("p{source}"), &["new-first", "p5"]), false);
        assert_eq!(
            row.edges,
            [("new-first".into(), Some(source)), ("p5".into(), Some(5))]
        );
        assert!(row.route_note().is_empty());
        assert_eq!(row.width(), 16);
        assert_eq!(lanes.pending.len(), 16);
        assert_eq!(lanes.colors, colors);
        let mut live = (0..16)
            .map(|i| std::collections::BTreeSet::from([i]))
            .collect::<Vec<_>>();
        live[source] = std::collections::BTreeSet::from([100]);
        let decoded = row
            .connectors
            .iter()
            .step_by(2)
            .fold(live.clone(), |live, line| decode_route(&live, line));
        live[5].insert(100);
        assert_eq!(
            decoded, live,
            "only the target may receive the new parent route"
        );
        let route = &row.connectors[0];
        assert_eq!(
            route.cells[10].character(true),
            if source < 5 { '>' } else { '<' }
        );
        assert_eq!(route.cells[source * 2].character(true), '+');
        for column in source.min(5) * 2 + 1..source.max(5) * 2 {
            assert_eq!(
                route.cells[column].character(true),
                if column % 2 == 0 { 'x' } else { '-' }
            );
            assert_eq!(route.cells[column].color, colors[5]);
        }
    }
}

#[test]
fn an_undrawn_new_parent_does_not_hide_a_later_existing_parent() {
    let mut lanes = GraphLanes {
        pending: (0..16).map(|i| Some(format!("p{i}"))).collect(),
        colors: (0..16).map(|i| i % 4).collect(),
        next_color: 0,
    };
    let row = lanes.row(commit("p0", &["p15", "missing", "p5"]), false);
    assert_eq!(row.edges[1], ("missing".into(), None));
    assert_eq!(
        lanes.pending[row.edges[2].1.unwrap()].as_deref(),
        Some("p5")
    );
    assert_eq!(
        row.route_note(),
        " [overflow parents: missing; Enter inspects all parents]"
    );
    assert!(!row.route_note().contains("parent lanes"));
    assert!(row.connectors.len() <= MAX_CONNECTOR_ROWS);
}

#[test]
fn graph_toggle_changes_only_graph_cells_not_metadata_coordinates() {
    let mut value = commit("0123456789abcdef", &["parent"]);
    value.author = "A\u{301}lpha 李".into();
    value.decorations = vec!["HEAD → main".into(), "origin/main".into()];
    value.subject = "subject after HEAD".into();
    let row = GraphLanes::default().row(value, false);
    let unicode = row.text(false);
    let ascii = row.text(true);
    assert_eq!(unicode.chars().count(), ascii.chars().count());
    assert_eq!(
        unicode.find("subject"),
        ascii.find("subject").map(|byte| byte + 2)
    ); // Unicode node uses two extra bytes.
    let prefix = row.metadata_prefix();
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(prefix.as_str()),
        GRAPH_COLUMN
    );
    let graph_end = prefix.chars().count() + row.width() * 2 - 1;
    assert_eq!(
        unicode.chars().skip(graph_end).collect::<String>(),
        ascii.chars().skip(graph_end).collect::<String>()
    );
}
