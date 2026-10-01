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

#[test]
fn initials_and_labels_keep_unicode_text_and_ascii_head_spelling() {
    assert_eq!(author_initials("Ada Lovelace"), "AL");
    assert_eq!(author_initials(""), "??");
    assert_eq!(author_initials("界 界"), "界界");
    let mut value = commit("0123456789abcdef", &[]);
    value.decorations = vec!["HEAD → main".into()];
    let row = GraphLanes::default().row(value, false);
    assert!(row.text().contains("[HEAD -> main]"));
}

#[test]
fn metadata_columns_use_display_cells_with_unicode_initials() {
    use unicode_width::UnicodeWidthStr;
    for author in ["A B", "李 小龍", "A\u{301}lpha Beta", "👩‍💻 Coder"] {
        let mut summary = commit(&"a".repeat(64), &[]);
        summary.author = author.into();
        summary.subject = "SUBJECT".into();
        let row = GraphLanes::default().row(summary, false);
        let text = row.text_with_width(2);
        assert_eq!(text[..text.find("SUBJECT").unwrap()].width(), 23);
        assert_eq!(&text[..6], "aaaaaa");
    }
    assert_eq!(author_initials("A\u{301}lpha Beta"), "A\u{301}B");
}

#[test]
fn ascii_graph_preserves_unicode_metadata() {
    let mut value = commit("0123456789abcdef", &["parent"]);
    value.author = "A\u{301}lpha 李".into();
    value.decorations = vec!["HEAD → main".into(), "origin/main".into()];
    value.subject = "subject after HEAD".into();
    let row = GraphLanes::default().row(value, false);
    let prefix = row.metadata_prefix();
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(prefix.as_str()),
        GRAPH_COLUMN
    );
    assert_eq!(row.graph(), "*");
    assert_eq!(
        row.text(),
        format!("{prefix}*  [?] [HEAD -> main, origin/main] subject after HEAD")
    );
}

#[test]
fn rectangular_merge_and_rejoin_keep_columns_and_path_colors() {
    let mut lanes = GraphLanes::default();
    let merge = lanes.row(commit("merge", &["a", "b"]), false);
    assert_eq!(merge.graph(), "*-+");
    assert_eq!(lanes.colors, [0, 1]);
    assert_eq!(merge.node.cells[1].color, 1);
    assert_eq!(merge.node.cells[2].color, 1);
    assert_eq!(lanes.row(commit("a", &["root"]), false).graph(), "* |");
    let b = lanes.row(commit("b", &["root"]), false);
    assert_eq!(b.graph(), "+-*");
    assert_eq!(b.node.cells[0].color, 0);
    assert_eq!(b.node.cells[1].color, 0);
    assert_eq!(b.node.cells[2].color, 1);
    assert_eq!(lanes.pending, [Some("root".into())]);
    assert_eq!(lanes.colors, [0]);
    assert_eq!(lanes.row(commit("root", &[]), false).graph(), "*");
    assert!(lanes.pending.is_empty());
}

#[test]
fn crossings_keep_unrelated_vertical_paths_and_their_colors() {
    let mut lanes = GraphLanes {
        pending: ["base", "unrelated", "tip"]
            .map(|oid| Some(oid.into()))
            .into(),
        colors: vec![0, 1, 2],
        branches: vec![None; 3],
        next_color: 3,
    };
    let row = lanes.row(commit("tip", &["base"]), false);
    assert_eq!(row.graph(), "+-|-*");
    assert_eq!(
        row.node
            .cells
            .iter()
            .map(|cell| cell.color)
            .collect::<Vec<_>>(),
        [0, 0, 1, 0, 2]
    );
    assert_eq!(
        lanes.pending,
        [Some("base".into()), Some("unrelated".into())]
    );
    assert_eq!(lanes.colors, [0, 1]);
    assert_eq!(row.edges, [("base".into(), Some(0))]);
}

#[test]
fn ended_paths_leave_holes_without_moving_survivors_and_reuse_gets_a_new_color() {
    let mut lanes = GraphLanes::default();
    lanes.row(commit("merge", &["island", "continuing"]), false);
    assert_eq!(lanes.row(commit("island", &[]), false).graph(), "* |");
    assert_eq!(lanes.pending, [None, Some("continuing".into())]);
    let survivor = lanes.row(commit("continuing", &["old"]), false);
    assert_eq!(survivor.graph(), "  *");
    assert_eq!(survivor.lane, Some(1));
    assert_eq!(survivor.node.cells[2].color, 1);
    let new = lanes.row(commit("new", &["new-parent"]), false);
    assert_eq!(new.graph(), "* |");
    assert_eq!(new.node.cells[0].color, 2);
    assert_eq!(new.node.cells[2].color, 1);
}

#[test]
fn octopus_parents_share_one_row_and_keep_their_own_endpoint_colors() {
    let mut lanes = GraphLanes::default();
    let row = lanes.row(commit("merge", &["a", "b", "c"]), false);
    assert_eq!(row.graph(), "*-+-+");
    assert_eq!(lanes.colors, [0, 1, 2]);
    assert_eq!(
        row.node
            .cells
            .iter()
            .map(|cell| cell.color)
            .collect::<Vec<_>>(),
        [0, 1, 1, 2, 2]
    );
    for (parent, lane) in row.edges {
        assert_eq!(lanes.pending[lane.unwrap()].as_ref(), Some(&parent));
    }
}

#[test]
fn captured_cursor_replay_preserves_holes_colors_and_parent_mapping() {
    let mut lanes = GraphLanes::default();
    lanes.row(commit("merge", &["a", "b", "c"]), false);
    lanes.row(commit("b", &[]), false);
    let checkpoint = lanes.clone();
    let row = lanes.row(commit("a", &["c", "root"]), false);
    assert_eq!(
        row,
        checkpoint.clone().row(commit("a", &["c", "root"]), false)
    );
    assert_eq!(row.graph(), "*-+-+");
    assert_eq!(lanes.pending, [None, Some("root".into()), Some("c".into())]);
    assert_eq!(lanes.colors[2], 2);
}

#[test]
fn lane_limits_remain_explicit_without_losing_parent_identities() {
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
    assert_eq!(row.graph().chars().count(), MAX_NETWORK_LANES * 2 - 1);
    assert!(row.route_note().contains("overflow"));
    assert!(row.route_note().contains("shallow boundary"));
    let before = lanes.clone();
    let row = lanes.row(commit("unplaced", &["missing"]), false);
    assert_eq!(row.lane, None);
    assert_eq!(row.edges, [("missing".into(), None)]);
    assert!(row.route_note().contains("unplaced commit"));
    assert_eq!(lanes, before);
}

#[test]
fn saturated_lanes_still_route_existing_parents_across_colored_crossings() {
    for source in [0, 15] {
        let mut lanes = GraphLanes {
            pending: (0..16).map(|i| Some(format!("p{i}"))).collect(),
            colors: (0..16).map(|i| i % 4).collect(),
            branches: vec![None; 16],
            next_color: 0,
        };
        let colors = lanes.colors.clone();
        let row = lanes.row(
            commit(&format!("p{source}"), &["new-first", "missing", "p5"]),
            false,
        );
        assert_eq!(
            row.edges,
            [
                ("new-first".into(), Some(source)),
                ("missing".into(), None),
                ("p5".into(), Some(5))
            ]
        );
        assert_eq!(
            row.route_note(),
            " [overflow parents: missin; Enter inspects all parents]"
        );
        assert_eq!(lanes.colors, colors);
        assert_eq!(row.node.cells[10].character(), '+');
        assert_eq!(row.node.cells[source * 2].character(), '*');
        for column in source.min(5) * 2 + 1..source.max(5) * 2 {
            let cell = row.node.cells[column];
            assert_eq!(cell.character(), if column % 2 == 0 { '|' } else { '-' });
            assert_eq!(
                cell.color,
                if column % 2 == 0 {
                    colors[column / 2]
                } else {
                    colors[5]
                }
            );
        }
    }
}

#[test]
fn rectangular_rows_preserve_all_parent_endpoints_in_many_small_dags() {
    for seed in 0..100_u64 {
        let mut random = seed + 1;
        let mut lanes = GraphLanes::default();
        for node in 0..10 {
            let mut parents = Vec::new();
            for parent in node + 1..10 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                if random % 4 == 0 && parents.len() < 3 {
                    parents.push(parent.to_string());
                }
            }
            let before = lanes.clone();
            let row = lanes.row(
                commit(
                    &node.to_string(),
                    &parents.iter().map(String::as_str).collect::<Vec<_>>(),
                ),
                false,
            );
            assert!(row.route_note().is_empty());
            let source = row.lane.unwrap();
            assert_eq!(row.graph().chars().filter(|ch| *ch == '*').count(), 1);
            assert!(row.graph().chars().all(|ch| " *|+-".contains(ch)));
            let endpoints = row
                .node
                .cells
                .iter()
                .enumerate()
                .filter(|(_, cell)| cell.character() == '+')
                .map(|(column, _)| column / 2)
                .collect::<Vec<_>>();
            let mut expected = Vec::new();
            for parent in &parents {
                let target = lanes
                    .pending
                    .iter()
                    .position(|oid| oid.as_ref() == Some(parent))
                    .unwrap();
                if target != source {
                    expected.push(target);
                }
                assert_eq!(row.node.cells[target * 2].color, lanes.colors[target]);
            }
            expected.sort_unstable();
            assert_eq!(endpoints, expected);
            for (lane, oid) in before.pending.iter().enumerate() {
                if oid.as_ref().is_some_and(|oid| oid != &node.to_string()) {
                    assert_eq!(lanes.pending.get(lane), Some(oid));
                    assert_eq!(lanes.colors[lane], before.colors[lane]);
                }
            }
        }
        assert!(lanes.pending.is_empty());
    }
}

#[test]
fn captured_branch_labels_follow_first_parents_and_survive_cursor_replay() {
    let roots = vec![
        NetworkRoot {
            oid: "merge".into(),
            labels: vec!["HEAD → main".into(), "main".into()],
            references: vec!["refs/heads/main".into()],
        },
        NetworkRoot {
            oid: "side".into(),
            labels: vec!["side".into()],
            references: vec!["refs/heads/side".into()],
        },
        NetworkRoot {
            oid: "base".into(),
            labels: vec!["old-name".into()],
            references: vec!["refs/heads/old-name".into()],
        },
    ];
    let mut lanes = GraphLanes::default();
    let mut merge = commit("merge", &["main-child", "side"]);
    merge.decorations = roots[0].labels.clone();
    let row = lanes.row_with_roots(merge, false, &roots);
    assert_eq!(row.branch.as_deref(), Some("main"));
    assert!(row.text().contains("[main] [HEAD -> main]"));
    let checkpoint = lanes.clone();
    let side = lanes.row_with_roots(commit("side", &["side-child"]), false, &roots);
    assert_eq!(side.branch.as_deref(), Some("side"));
    assert_eq!(
        side,
        checkpoint
            .clone()
            .row_with_roots(commit("side", &["side-child"]), false, &roots)
    );
    assert_eq!(
        lanes
            .row_with_roots(commit("side-child", &["base"]), false, &roots)
            .branch
            .as_deref(),
        Some("side")
    );
    assert_eq!(
        lanes
            .row_with_roots(commit("main-child", &["base"]), false, &roots)
            .branch
            .as_deref(),
        Some("main")
    );
    // Shared history keeps the surviving displayed path's name. A different
    // ref on the shared ancestor does not retroactively rename that path.
    assert_eq!(
        lanes
            .row_with_roots(commit("base", &[]), false, &roots)
            .branch
            .as_deref(),
        Some("side")
    );
    assert!(lanes.branches.is_empty());
}

#[test]
fn remote_branch_names_are_kept_but_tags_and_detached_head_do_not_invent_branches() {
    for (reference, expected) in [
        ("refs/remotes/origin/topic", Some("origin/topic")),
        ("refs/tags/v1", None),
    ] {
        let roots = vec![NetworkRoot {
            oid: "tip".into(),
            labels: vec!["HEAD (detached)".into()],
            references: vec![reference.into()],
        }];
        let mut lanes = GraphLanes::default();
        let row = lanes.row_with_roots(commit("tip", &["base"]), false, &roots);
        assert_eq!(row.branch.as_deref(), expected);
        assert_eq!(
            lanes
                .row_with_roots(commit("base", &[]), false, &roots)
                .branch
                .as_deref(),
            expected
        );
    }
    let row = GraphLanes::default().row(commit("unreferenced", &[]), false);
    assert!(row.text().contains("[?]"));
}

#[test]
fn containment_counts_distinct_branches_across_duplicate_routes_and_bounded_prefixes() {
    let local = |name: &str, oid: &str| LocalBranch {
        name: name.into(),
        oid: oid.into(),
        current: false,
        committer_time: 0,
    };
    let ancestry = [
        ("a", vec!["left", "right"]),
        ("b", vec!["right"]),
        ("left", vec!["base"]),
        ("right", vec!["base"]),
        ("base", vec![]),
    ];
    for (tips, expected) in [
        (vec![local("preferred", "a")], "[in preferred] "),
        (
            vec![local("preferred", "a"), local("other", "b")],
            "[in preferred, ...] ",
        ),
    ] {
        let membership = NetworkMembership::capture(
            LocalBranches {
                tips,
                limited: false,
            },
            &ancestry,
            false,
        );
        let mut row = GraphLanes::default().row(commit("base", &[]), false);
        membership.label_row(&mut row);
        assert_eq!(row.branch_label(), expected);
        assert!(row.text().contains(expected));
    }
    let bounded = NetworkMembership::capture(
        LocalBranches {
            tips: vec![local("preferred", "a"), local("other", "b")],
            limited: false,
        },
        &ancestry[..4],
        true,
    );
    let mut visited = GraphLanes::default().row(commit("right", &[]), false);
    bounded.label_row(&mut visited);
    assert_eq!(visited.branch_label(), "[in preferred, ...] ");
    let mut beyond = GraphLanes::default().row(commit("base", &[]), false);
    bounded.label_row(&mut beyond);
    assert_eq!(beyond.branch_label(), "[?] ");
}
