// SPDX-License-Identifier: MPL-2.0

use super::*;

fn here(branch: &str) -> Location {
    Location {
        directory: "/work/project".into(),
        repository: Some("/work/project".into()),
        worktree: Some("/work/project".into()),
        branch: Some(branch.into()),
    }
}

fn join(channel: &mut Channel, connection: u64, provider: &str, rejoin: Option<&str>) -> String {
    channel
        .join(Join {
            identity: "agent",
            connection,
            provider,
            task: "refactor the parser",
            client: Some(Client {
                name: "claude-code".into(),
                version: "2.0".into(),
            }),
            location: here("main"),
            terminal: Some(3),
            resume: rejoin,
            fresh_id: uuid(&format!("{connection:032x}")),
            fresh_resume: format!("resume-{connection}"),
            now_ms: 1,
        })
        .unwrap()
        .id
        .clone()
}

#[test]
fn joining_names_the_agent_by_provider_and_keeps_it_across_reconnects() {
    let mut channel = Channel::default();
    let claude = join(&mut channel, 1, "Claude", None);
    let codex = join(&mut channel, 2, "codex", None);
    let second = join(&mut channel, 3, "claude", None);
    assert_eq!(channel.nick(&claude), Some("claude-1"));
    assert_eq!(channel.nick(&codex), Some("codex-1"));
    assert_eq!(channel.nick(&second), Some("claude-2"));
    assert_eq!(
        channel
            .participants()
            .map(|p| p.nick.as_str())
            .collect::<Vec<_>>(),
        ["claude-1", "codex-1", "claude-2"],
        "listed in joining order"
    );
    assert_eq!(claude.len(), 36);
    assert_eq!(&claude[14..15], "4", "a version 4 UUID");

    // The same agent on a new connection keeps its identity and nickname,
    // proving itself with the private token its join returned.
    channel.disconnect(1);
    assert!(channel.joined(1).is_none());
    assert_eq!(join(&mut channel, 9, "claude", Some("resume-1")), claude);
    assert_eq!(channel.joined(9).unwrap().nick, "claude-1");
    assert_eq!(channel.participants().count(), 3);

    // The public UUID is no token: presenting it makes a new participant.
    let other = join(&mut channel, 11, "claude", Some(&claude));
    assert_ne!(other, claude);
    assert_eq!(channel.joined(9).unwrap().id, claude, "still bound");

    // Another identity cannot take it over, even with the token.
    let taken = channel
        .join(Join {
            identity: "codex",
            connection: 10,
            provider: "codex",
            task: "review",
            client: None,
            location: here("main"),
            terminal: None,
            resume: Some("resume-9"),
            fresh_id: uuid(&"f".repeat(32)),
            fresh_resume: "resume-10".into(),
            now_ms: 2,
        })
        .unwrap()
        .id
        .clone();
    assert_ne!(taken, claude);
}

#[test]
fn a_join_must_state_a_provider_and_a_task() {
    let mut channel = Channel::default();
    for (provider, task) in [
        ("", "task"),
        ("claude code", "task"),
        ("claude", ""),
        ("claude", "a\nb"),
    ] {
        let error = channel
            .join(Join {
                identity: "agent",
                connection: 1,
                provider,
                task,
                client: None,
                location: here("main"),
                terminal: None,
                resume: None,
                fresh_id: uuid("0"),
                fresh_resume: "r".into(),
                now_ms: 1,
            })
            .err()
            .unwrap();
        assert_eq!(
            error.code,
            ErrorCode::InvalidArgument,
            "{provider:?} {task:?}"
        );
    }
    let error = channel
        .post(1, None, "hello", here("main"), &[], 1)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::CapabilityDenied);
}

#[test]
fn moving_is_recorded_before_the_message_with_both_places() {
    let mut channel = Channel::default();
    let claude = join(&mut channel, 1, "claude", None);
    channel
        .post(1, None, "now on a branch", here("feature"), &[], 2)
        .unwrap();
    let kinds = channel
        .messages()
        .map(|m| (m.kind, m.text.clone()))
        .collect::<Vec<_>>();
    assert_eq!(kinds[0].0, Kind::Joined);
    assert_eq!(
        kinds[1],
        (
            Kind::Moved,
            "moved from main in /work/project to feature in /work/project".into()
        )
    );
    assert_eq!(kinds[2], (Kind::Message, "now on a branch".into()));
    assert_eq!(
        channel
            .participant(&claude)
            .unwrap()
            .location
            .branch
            .as_deref(),
        Some("feature")
    );
    // Changing the stated task on a later join is recorded too.
    channel
        .join(Join {
            identity: "agent",
            connection: 1,
            provider: "claude",
            task: "orchestrate",
            client: None,
            location: here("feature"),
            terminal: None,
            resume: None,
            fresh_id: uuid("1"),
            fresh_resume: "r".into(),
            now_ms: 3,
        })
        .unwrap();
    let last = channel.messages().last().unwrap();
    assert_eq!(
        (last.kind, last.text.as_str()),
        (Kind::Updated, "claude: orchestrate")
    );
}

#[test]
fn direct_messages_reach_only_the_two_participants() {
    let mut channel = Channel::default();
    let claude = join(&mut channel, 1, "claude", None);
    let codex = join(&mut channel, 2, "codex", None);
    join(&mut channel, 3, "gemini", None);
    channel
        .post(1, Some("codex-1"), "just you", here("main"), &[], 2)
        .unwrap();
    channel
        .post(1, None, "everyone", here("main"), &[], 3)
        .unwrap();
    let texts = |channel: &mut Channel, connection| {
        channel
            .read(connection, 0, 200)
            .unwrap()
            .messages
            .iter()
            .filter(|m| m.kind == Kind::Message)
            .map(|m| m.text.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(texts(&mut channel, 2), ["just you", "everyone"]);
    assert_eq!(texts(&mut channel, 1), ["just you", "everyone"]);
    assert_eq!(texts(&mut channel, 3), ["everyone"]);
    assert_eq!(
        channel
            .post(1, Some(&claude), "me", here("main"), &[], 4)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        channel
            .post(1, Some("nobody"), "hi", here("main"), &[], 4)
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(channel.unread(&codex), 0, "read everything");
}

#[test]
fn unread_counts_other_peoples_messages_since_the_last_read() {
    let mut channel = Channel::default();
    let claude = join(&mut channel, 1, "claude", None);
    let codex = join(&mut channel, 2, "codex", None);
    channel.post(1, None, "one", here("main"), &[], 2).unwrap();
    channel.post(1, None, "two", here("main"), &[], 3).unwrap();
    assert_eq!(channel.unread(&codex), 2);
    assert_eq!(channel.unread(&claude), 0, "its own messages are read");
    // Both joins, then "one".
    let read = channel.read(2, 0, 3).unwrap();
    let next = read.next;
    assert_eq!(read.messages.len(), 3);
    assert_eq!(channel.unread(&codex), 1);
    let read = channel.read(2, next, 200).unwrap();
    assert!(read.messages.iter().any(|m| m.text == "two"));
    assert_eq!(channel.unread(&codex), 0);
    let head = channel.head();
    let read = channel.read(2, head, 200).unwrap();
    assert!(read.messages.is_empty());
    assert_eq!(read.next, head);
}

#[test]
fn the_log_is_bounded_and_a_read_of_dropped_messages_says_so() {
    let mut channel = Channel::default();
    join(&mut channel, 1, "claude", None);
    join(&mut channel, 2, "codex", None);
    for index in 0..MAX_MESSAGES + 10 {
        channel
            .post(1, None, &format!("m{index}"), here("main"), &[], 2)
            .unwrap();
    }
    assert_eq!(channel.messages().count(), MAX_MESSAGES);
    let read = channel.read(2, 0, 5).unwrap();
    assert!(read.lost);
    assert_eq!(read.messages.len(), 5);
    let big = "x".repeat(MAX_TEXT_BYTES);
    for _ in 0..(MAX_RETAINED_BYTES / MAX_TEXT_BYTES) + 4 {
        channel.post(1, None, &big, here("main"), &[], 3).unwrap();
    }
    assert!(channel.retained_bytes <= MAX_RETAINED_BYTES);
    assert_eq!(
        channel
            .post(1, None, &format!("{big}x"), here("main"), &[], 4)
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded
    );
    assert_eq!(
        channel
            .post(1, None, "bell\u{7}", here("main"), &[], 4)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
    // A position from an earlier host starts over rather than reading nothing.
    assert!(!channel.read(2, u64::MAX, 1).unwrap().messages.is_empty());
}

#[test]
fn a_full_channel_makes_room_by_dropping_the_longest_departed() {
    let mut channel = Channel::default();
    let first = join(&mut channel, 0, "claude", None);
    channel
        .post(0, None, "remember me", here("main"), &[], 2)
        .unwrap();
    channel.disconnect(0);
    for connection in 1..MAX_PARTICIPANTS as u64 {
        join(&mut channel, connection, "codex", None);
    }
    // Full: the departed one gives way, and its message keeps its name.
    let newest = join(&mut channel, 100, "gemini", None);
    assert!(channel.participant(&first).is_none());
    assert!(channel.participant(&newest).is_some());
    assert_eq!(channel.nick(&first), Some("claude-1"));
    // With everyone connected there is nothing to drop.
    let refused = channel
        .join(Join {
            identity: "agent",
            connection: 101,
            provider: "late",
            task: "t",
            client: None,
            location: here("main"),
            terminal: None,
            resume: None,
            fresh_id: uuid("e"),
            fresh_resume: "r".into(),
            now_ms: 3,
        })
        .err()
        .unwrap();
    assert_eq!(refused.code, ErrorCode::LimitExceeded);
}

#[test]
fn a_read_is_bounded_by_encoded_size_and_skips_others_direct_messages() {
    let mut channel = Channel::default();
    join(&mut channel, 1, "claude", None);
    join(&mut channel, 2, "codex", None);
    join(&mut channel, 3, "gemini", None);
    // Quotes double when encoded, so the budget counts them twice.
    let quoted = "\"".repeat(MAX_TEXT_BYTES);
    for _ in 0..12 {
        channel
            .post(1, None, &quoted, here("main"), &[], 2)
            .unwrap();
    }
    let read = channel.read(2, 0, 200).unwrap();
    let returned = read.messages.iter().map(|m| m.bytes()).sum::<usize>();
    assert!(returned <= MAX_READ_BYTES, "{returned}");
    assert!(read.messages.len() < 15, "stopped at the budget");
    let rest = channel.read(2, read.next, 200).unwrap();
    assert!(!rest.messages.is_empty(), "the rest follows from next");

    // A direct message between two others is passed over without counting
    // against the limit, and the position moves past it.
    let mut channel = Channel::default();
    join(&mut channel, 1, "claude", None);
    join(&mut channel, 2, "codex", None);
    join(&mut channel, 3, "gemini", None);
    let head = channel.head();
    channel
        .post(1, Some("codex-1"), "private", here("main"), &[], 2)
        .unwrap();
    channel
        .post(1, None, "public one", here("main"), &[], 3)
        .unwrap();
    channel
        .post(1, None, "public two", here("main"), &[], 4)
        .unwrap();
    let read = channel.read(3, head, 1).unwrap();
    assert_eq!(read.messages.len(), 1);
    assert_eq!(read.messages[0].text, "public one");
    let read = channel.read(3, read.next, 1).unwrap();
    assert_eq!(read.messages[0].text, "public two");
}

#[test]
fn the_participant_that_left_longest_ago_gives_way_first() {
    let mut channel = Channel::default();
    let early = join(&mut channel, 0, "claude", None);
    let later = join(&mut channel, 1, "codex", None);
    // The later joiner leaves first; the early one only just dropped out.
    channel.disconnect(1);
    channel.disconnect(0);
    for connection in 2..MAX_PARTICIPANTS as u64 {
        join(&mut channel, connection, "gemini", None);
    }
    join(&mut channel, 200, "gemini", None);
    assert!(channel.participant(&later).is_none());
    assert!(channel.participant(&early).is_some(), "it can still resume");
}

#[test]
fn a_post_names_whom_to_nudge_and_a_refused_name_stores_nothing() {
    let mut channel = Channel::default();
    let claude = join(&mut channel, 1, "claude", None);
    let codex = join(&mut channel, 2, "codex", None);
    let gemini = join(&mut channel, 3, "gemini", None);
    let names = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();

    let posted = channel
        .post(
            1,
            None,
            "review ready",
            here("main"),
            &names(&["gemini-1", &codex]),
            2,
        )
        .unwrap();
    assert_eq!(
        posted.nudge,
        [gemini.clone(), codex.clone()],
        "in the order named"
    );
    let direct = channel
        .post(
            1,
            Some("codex-1"),
            "yours",
            here("main"),
            &names(&["codex-1"]),
            3,
        )
        .unwrap();
    assert_eq!(direct.nudge, std::slice::from_ref(&codex));
    assert!(
        channel
            .post(1, None, "quiet", here("main"), &[], 4)
            .unwrap()
            .nudge
            .is_empty(),
        "nothing nudges unless asked"
    );

    let head = channel.head();
    for (to, nudge, code) in [
        (None, vec![claude.clone()], ErrorCode::InvalidArgument),
        (
            Some("codex-1"),
            names(&["gemini-1"]),
            ErrorCode::InvalidArgument,
        ),
        (
            None,
            names(&["codex-1", &codex]),
            ErrorCode::InvalidArgument,
        ),
        (None, names(&["nobody"]), ErrorCode::NotFound),
    ] {
        let error = channel
            .post(1, to, "refused", here("main"), &nudge, 5)
            .unwrap_err();
        assert_eq!(error.code, code, "{nudge:?}");
    }
    assert_eq!(channel.head(), head, "a refused nudge stores no message");
}
