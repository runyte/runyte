// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::protocol::{
    MAX_DESTINATION_LABEL_BYTES, MAX_DESTINATIONS, OpenDestination, OpenDestinationEntry,
};
use std::path::Path;

#[test]
fn activity_is_taken_from_current_history_for_running_and_stopped_rows() {
    let mut running = numbering_row(Path::new("projects/running"), true);
    let mut stopped = numbering_row(Path::new("projects/stopped"), false);
    running.last_active_unix_seconds = Some(99);
    stopped.last_active_unix_seconds = Some(99);
    let history = vec![RecentEntry::new(
        running.project_root.clone(),
        None,
        None,
        Some(7),
    )];
    let mut rows = [running, stopped];
    apply_recent_activity(&mut rows, &history);
    assert_eq!(rows[0].last_active_unix_seconds, Some(7));
    assert_eq!(rows[1].last_active_unix_seconds, None);
}

#[test]
fn stale_number_merge_preserves_concurrent_changes_and_clears_unchanged_stopped_rows() {
    let first = PathBuf::from("projects/first");
    let second = PathBuf::from("projects/second");
    let snapshot = vec![
        RecentEntry::new(first.clone(), None, Some(1), None),
        RecentEntry::new(second.clone(), None, Some(2), None),
    ];
    let mut current = snapshot.clone();
    current[0].number = Some(7);
    current[0].number_pinned = true;
    let new = RecentEntry::new(PathBuf::from("projects/new"), None, Some(3), None);
    current.push(new.clone());
    let rows = [numbering_row(&first, false), numbering_row(&second, false)];
    merge_assigned_numbers(&mut current, &snapshot, &rows);
    assert_eq!(current[0].number, Some(7));
    assert!(current[0].number_pinned);
    assert_eq!(current[1].number, None);
    assert!(!current[1].number_declined);
    assert!(!current[1].number_pinned);
    assert_eq!(current[2], new);
}

#[test]
fn row_presentation_reports_protocol_and_explicit_or_directory_name() {
    let mut row = numbering_row(Path::new("projects/notes"), true);
    assert_eq!(row.state_label(), "running");
    assert_eq!(row.display_name(), "notes");
    row.incompatible_protocol = Some(99);
    assert_eq!(row.state_label(), "running (protocol 99)");
    row.name = Some("chosen".to_owned());
    assert_eq!(row.display_name(), "chosen");
    row.running = false;
    assert_eq!(row.state_label(), "stopped");
}

#[test]
fn project_only_rows_keep_selection_across_presentation_and_running_state() {
    let root = Path::new("projects/notes");
    let running = numbering_row(root, true);
    let mut stopped = numbering_row(root, false);
    stopped.name = Some("renamed".to_owned());
    stopped.unsaved_buffers = Some(3);
    assert_eq!(running.selection(), stopped.selection());
    assert_eq!(running.selection().publication_key(), None);
    assert_eq!(running.selection().project_root(), root);
    assert_ne!(
        running.selection(),
        numbering_row(Path::new("projects/other"), true).selection()
    );
}

#[cfg(windows)]
#[test]
fn native_publication_key_uses_exact_framed_tuple_without_mutable_display_fields() {
    use crate::workspace::{
        windows_endpoint::{EndpointMetadata, PipeAddress},
        windows_process_identity::ProcessIdentity,
    };

    let mut metadata = EndpointMetadata {
        protocol: 1,
        id: "display-id".to_owned(),
        name: Some("old".to_owned()),
        // An unpaired surrogate distinguishes exact native bytes from a
        // lossy path string. Key construction never decodes this field.
        project_root_bytes: vec![b'C', 0, b':', 0, b'\\', 0, 0xff, 0xd8],
        process: ProcessIdentity {
            pid: 123,
            creation_time: 456,
        },
        incarnation: "a".repeat(64),
        address: PipeAddress::try_from(format!(r"\\.\pipe\runyte-v1-{}", "b".repeat(64))).unwrap(),
    };
    let original = PublicationKey::from_authenticated_metadata(&metadata);
    assert_eq!(
        original,
        PublicationKey::from_authenticated_metadata(&metadata)
    );

    metadata.name = Some("new".to_owned());
    metadata.id = "another-display-id".to_owned();
    metadata.protocol = 99;
    assert_eq!(
        original,
        PublicationKey::from_authenticated_metadata(&metadata)
    );

    let mut changed = metadata.clone();
    changed.project_root_bytes.push(0);
    assert_ne!(
        original,
        PublicationKey::from_authenticated_metadata(&changed)
    );
    changed = metadata.clone();
    changed.project_root_bytes[6] = 0xfe;
    assert_ne!(
        original,
        PublicationKey::from_authenticated_metadata(&changed)
    );
    changed = metadata.clone();
    changed.process.pid += 1;
    assert_ne!(
        original,
        PublicationKey::from_authenticated_metadata(&changed)
    );
    changed = metadata.clone();
    changed.process.creation_time += 1;
    assert_ne!(
        original,
        PublicationKey::from_authenticated_metadata(&changed)
    );
    changed = metadata.clone();
    changed.incarnation = "c".repeat(64);
    assert_ne!(
        original,
        PublicationKey::from_authenticated_metadata(&changed)
    );
    changed = metadata.clone();
    changed.address =
        PipeAddress::try_from(format!(r"\\.\pipe\runyte-v1-{}", "d".repeat(64))).unwrap();
    assert_ne!(
        original,
        PublicationKey::from_authenticated_metadata(&changed)
    );
}

fn entry(project_root: PathBuf, name: Option<String>) -> RecentEntry {
    RecentEntry::new(project_root, name, None, None)
}

/// One listing row, in whatever running state the numbering is about.
fn numbering_row(project_root: &Path, running: bool) -> WorkspaceRow {
    WorkspaceRow {
        publication_key: None,
        unread_terminals: None,
        terminal_bell: None,
        id: "aaaaaaaaaaaaaaaa".to_owned(),
        name: None,
        number: None,
        last_active_unix_seconds: None,
        project_root: project_root.to_path_buf(),
        running,
        incompatible_protocol: None,
        unsaved_buffers: None,
        open_buffers: None,
        pending_wait_requests: None,
        plugin_jobs: None,
        activity_leases: None,
        activities: Vec::new(),
        live_terminals: None,
        terminal_sessions: None,
        terminal_line_activity_unix_seconds: None,
        interactive_attached: None,
        git: None,
        missing_directory: false,
    }
}

#[test]
fn abbreviated_ids_stay_six_characters_while_they_tell_workspaces_apart() {
    let ids = [
        "658471a65ca7c48244bef5867d3e80bc",
        "fe973e03d42260785cd4cb9386a8168d",
        "b98d692c4574a80e508cf6fa38e6f27a",
        "3ab49e5b3d4fc8cc6600e204dd5a571e",
    ];

    assert_eq!(abbreviated_id_width(ids), ABBREVIATED_WORKSPACE_ID);
}

#[test]
fn abbreviated_ids_grow_only_far_enough_to_separate_a_shared_prefix() {
    let ids = [
        "aaaaaaaa1111111111111111111111ff",
        "aaaaaaaa2222222222222222222222ff",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    ];

    assert_eq!(abbreviated_id_width(ids), 9);
}

#[test]
fn ids_that_never_separate_abbreviate_to_their_whole_length() {
    assert_eq!(abbreviated_id_width(["abcdef0123", "abcdef0123"]), 10);
    assert_eq!(
        abbreviated_id_width(["abc", "abc"]),
        ABBREVIATED_WORKSPACE_ID
    );
    assert_eq!(abbreviated_id_width([]), ABBREVIATED_WORKSPACE_ID);
}

#[test]
fn recent_names_fill_unnamed_running_rows_without_overriding_explicit_names() {
    let unnamed_root = PathBuf::from("/workspace/unnamed");
    let explicit_root = PathBuf::from("/workspace/explicit");
    let mut rows = vec![
        WorkspaceRow {
            publication_key: None,
            unread_terminals: None,
            terminal_bell: None,
            id: "11111111111111111111111111111111".to_owned(),
            name: None,
            number: None,
            last_active_unix_seconds: None,
            project_root: unnamed_root.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(0),
            pending_wait_requests: None,
            plugin_jobs: None,
            activity_leases: None,
            activities: Vec::new(),
            live_terminals: None,
            terminal_sessions: None,
            terminal_line_activity_unix_seconds: None,
            interactive_attached: Some(false),
            open_buffers: None,
            git: None,
            missing_directory: false,
        },
        WorkspaceRow {
            publication_key: None,
            unread_terminals: None,
            terminal_bell: None,
            id: "22222222222222222222222222222222".to_owned(),
            name: Some("chosen".to_owned()),
            number: None,
            last_active_unix_seconds: None,
            project_root: explicit_root.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(0),
            pending_wait_requests: None,
            plugin_jobs: None,
            activity_leases: None,
            activities: Vec::new(),
            live_terminals: None,
            terminal_sessions: None,
            terminal_line_activity_unix_seconds: None,
            interactive_attached: Some(false),
            open_buffers: None,
            git: None,
            missing_directory: false,
        },
    ];

    apply_recent_names(
        &mut rows,
        &[
            entry(unnamed_root, Some("default".to_owned())),
            entry(explicit_root, Some("stale-default".to_owned())),
        ],
    );

    assert_eq!(rows[0].name.as_deref(), Some("default"));
    assert_eq!(rows[1].name.as_deref(), Some("chosen"));
}

#[test]
fn automatic_running_numbers_compact_after_a_stopped_session_releases_its_digit() {
    let stopped = PathBuf::from("/w/stopped");
    let running = PathBuf::from("/w/running");
    let started = PathBuf::from("/w/started");
    let entries = vec![
        RecentEntry::new(stopped.clone(), None, Some(1), None),
        RecentEntry::new(running.clone(), None, Some(2), None),
        RecentEntry::new(started.clone(), None, None, None),
    ];
    let mut rows = vec![
        numbering_row(&stopped, false),
        numbering_row(&running, true),
        numbering_row(&started, true),
    ];

    assign_running_workspace_numbers(&mut rows, &entries);

    assert_eq!(rows[0].number, None, "a stopped session holds no digit");
    assert_eq!(
        rows[1].number,
        Some(1),
        "the first automatic running assignment closes the gap"
    );
    assert_eq!(
        rows[2].number,
        Some(2),
        "the next running session follows it in order"
    );
}

#[test]
fn an_automatic_session_does_not_retain_its_old_number_after_a_stop() {
    let restarted = PathBuf::from("/w/restarted");
    let entries = vec![RecentEntry::new(restarted.clone(), None, Some(4), None)];
    let mut rows = vec![numbering_row(&restarted, true)];

    assign_running_workspace_numbers(&mut rows, &entries);

    assert_eq!(rows[0].number, Some(1));
}

#[test]
fn an_explicitly_renumbered_running_session_keeps_its_chosen_digit() {
    let workspace = PathBuf::from("/w/pinned");
    let mut entry = RecentEntry::new(workspace.clone(), None, Some(4), None);
    entry.number_pinned = true;
    let mut rows = vec![numbering_row(&workspace, true)];

    assign_running_workspace_numbers(&mut rows, &[entry]);

    assert_eq!(rows[0].number, Some(4));
}

/// Records are unique while Runyte writes them, so this is the safety net
/// for a catalog edited by hand: the listing still hands one digit to one
/// session, and the more recently visited row keeps it.
#[test]
fn a_digit_two_records_claim_goes_to_the_row_the_listing_shows_first() {
    let first = PathBuf::from("/w/first");
    let second = PathBuf::from("/w/second");
    let entries = vec![
        RecentEntry::new(first.clone(), None, Some(1), None),
        RecentEntry::new(second.clone(), None, Some(1), None),
    ];
    let mut rows = vec![numbering_row(&first, true), numbering_row(&second, true)];

    assign_running_workspace_numbers(&mut rows, &entries);

    assert_eq!(rows[0].number, Some(1));
    assert_eq!(rows[1].number, Some(2));
}

#[test]
fn numbered_sessions_lead_the_listing_and_the_rest_follow_by_visit() {
    let mut rows = Vec::new();
    for (path, number, last_active) in [
        ("/w/never", None, None),
        ("/w/recent", None, Some(9_000)),
        ("/w/second", Some(2), Some(1)),
        ("/w/old", None, Some(1_000)),
        ("/w/first", Some(1), None),
    ] {
        let mut row = numbering_row(Path::new(path), number.is_some());
        row.number = number;
        row.last_active_unix_seconds = last_active;
        rows.push(row);
    }

    order_workspace_rows(&mut rows);

    assert_eq!(
        rows.iter()
            .map(|row| row.project_root.display().to_string())
            .collect::<Vec<_>>(),
        vec!["/w/first", "/w/second", "/w/old", "/w/recent", "/w/never"],
        "digits lead in order, then the least recently visited, then the \
         sessions nothing has ever attached to"
    );
}

#[test]
fn received_inventory_rejects_invalid_and_duplicate_identities_and_oversized_rows() {
    let entry = OpenDestinationEntry {
        destination: OpenDestination::Buffer(1),
        label: "notes".to_owned(),
        detail: String::new(),
    };
    assert!(validate_destination_inventory("a".repeat(64), vec![entry.clone()], false).is_ok());
    assert!(validate_destination_inventory("a".repeat(63), vec![], false).is_err());
    assert!(
        validate_destination_inventory("a".repeat(64), vec![entry.clone(), entry.clone()], false)
            .is_err()
    );
    assert!(
        validate_destination_inventory(
            "a".repeat(64),
            vec![entry.clone(); MAX_DESTINATIONS + 1],
            true
        )
        .is_err()
    );
    let mut zero = entry.clone();
    zero.destination = OpenDestination::Buffer(0);
    assert!(validate_destination_inventory("a".repeat(64), vec![zero], false).is_err());
    let mut oversized = entry;
    oversized.detail = "x".repeat(MAX_DESTINATION_LABEL_BYTES + 1);
    assert!(validate_destination_inventory("a".repeat(64), vec![oversized], false).is_err());
}
