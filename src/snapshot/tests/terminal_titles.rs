// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn unread_channel_messages_mark_the_title_but_not_the_label() {
    let mut terminals = crate::terminal::TerminalSessions::new();
    let id = terminals.insert_test_session(20, 3);
    let session = terminals.get_mut(id).unwrap();
    assert!(!terminal_title(session, None).contains("unread"));
    session.set_channel_unread(1);
    assert!(terminal_title(session, None).ends_with(" [1 unread]"));
    session.set_channel_unread(3);
    assert!(terminal_title(session, None).ends_with(" [3 unread]"));
    assert!(!terminal_label(session).contains("unread"));
}
