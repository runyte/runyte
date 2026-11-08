// SPDX-License-Identifier: MPL-2.0

//! How an interactive attachment to a persistent session came to an end.
//!
//! A host cannot see a client crash; it sees a connection close. What it can
//! know is whether it was told to expect that, so an attachment is treated as
//! lost unless a path that ends one on purpose has said otherwise. Each host
//! keeps its own record, so a notice can only reach the workspace whose
//! attachment was lost.
//!
//! An attachment can stop being the active one several ways before its
//! connection finishes closing: a failed write notices first, or the failure
//! and the closure arrive as separate events. A record therefore lives until
//! the connection's final event and yields at most one notice there. At most
//! one attachment is active at a time, and every connection reports its own
//! closure, so the records stay bounded by the connections still closing.

use std::collections::HashMap;

use super::transport_shared::TransportFailureCause;
use crate::notification::{NotificationDraft, NotificationSeverity};
use crate::protocol::WaitToken;

const NOTICE_SOURCE: &str = "Session";
const NOTICE_TITLE: &str = "Client connection lost";
const STILL_RUNNING: &str = "This persistent session kept running.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Ending {
    /// The client asked to leave, so a failure while saying goodbye is moot.
    Leaving,
    /// The host told the client to move to another workspace. Closing is what
    /// a client does on receipt, so this is quiet only once the instruction
    /// is known to have left the connection.
    Switching,
}

#[derive(Debug, Default)]
struct Attachment {
    ending: Option<Ending>,
    failure: Option<TransportFailureCause>,
    wait: Option<WaitToken>,
    handoff_delivered: bool,
    switch_interrupted: bool,
}

/// The ending of each interactive attachment whose connection is still open.
#[derive(Debug, Default)]
pub(crate) struct AttachmentDepartures {
    attachments: HashMap<u64, Attachment>,
}

impl AttachmentDepartures {
    /// Starts tracking an accepted interactive attachment. Control clients and
    /// refused connections are never recorded, so their closure stays quiet.
    pub(crate) fn attached(&mut self, id: u64) {
        self.attachments.insert(id, Attachment::default());
    }

    pub(crate) fn expect_leaving(&mut self, id: u64) {
        if let Some(attachment) = self.attachments.get_mut(&id) {
            attachment.ending = Some(Ending::Leaving);
        }
    }

    pub(crate) fn expect_switch(&mut self, id: u64) {
        if let Some(attachment) = self.attachments.get_mut(&id) {
            attachment.ending = Some(Ending::Switching);
            // An aborted attempt earlier in this attachment wrote its own
            // reply; only a write after this instruction was queued counts.
            attachment.handoff_delivered = false;
        }
    }

    pub(crate) fn handoff_delivered(&mut self, id: u64) {
        if let Some(attachment) = self.attachments.get_mut(&id) {
            attachment.handoff_delivered = true;
        }
    }

    /// Records the wait this attachment exists to serve. A client that owns a
    /// wait closes without detaching once that wait is finished.
    pub(crate) fn note_wait(&mut self, id: u64, token: WaitToken) {
        if let Some(attachment) = self.attachments.get_mut(&id) {
            attachment.wait = Some(token);
        }
    }

    /// Keeps the first failure: later ones are consequences of it.
    pub(crate) fn transport_failed(&mut self, id: u64, cause: TransportFailureCause) {
        if let Some(attachment) = self.attachments.get_mut(&id) {
            attachment.failure.get_or_insert(cause);
        }
    }

    /// Marks a loss that happened inside a two-phase switch, before the client
    /// was released to its destination.
    #[cfg(any(windows, test))]
    pub(crate) fn switch_interrupted(&mut self, id: u64) {
        if let Some(attachment) = self.attachments.get_mut(&id) {
            attachment.switch_interrupted = true;
        }
    }

    /// Consumes the record at the connection's final event, returning the
    /// notice to retain if this attachment was lost.
    pub(crate) fn ended(
        &mut self,
        id: u64,
        wait_finished: impl FnOnce(WaitToken) -> bool,
    ) -> Option<NotificationDraft> {
        let attachment = self.attachments.remove(&id)?;
        if attachment.wait.is_some_and(wait_finished) {
            return None;
        }
        let during_switch = match attachment.ending {
            Some(Ending::Leaving) => return None,
            Some(Ending::Switching) if attachment.handoff_delivered => return None,
            Some(Ending::Switching) => true,
            None => attachment.switch_interrupted,
        };
        let lost = if during_switch {
            "The attached client was lost during a session switch"
        } else {
            "The attached client was lost"
        };
        let body = match attachment.failure {
            Some(cause) => format!("{lost}: {}. {STILL_RUNNING}", cause.reason()),
            None => format!(
                "{lost}: its connection closed without detaching. The host saw nothing \
                 more, so the cause is unknown. {STILL_RUNNING}"
            ),
        };
        Some(NotificationDraft::new(
            NotificationSeverity::Warning,
            NOTICE_SOURCE,
            NOTICE_TITLE,
            body,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: u64 = 7;

    fn token() -> WaitToken {
        crate::workspace::WaitToken::new(41).into()
    }

    fn attached() -> AttachmentDepartures {
        let mut departures = AttachmentDepartures::default();
        departures.attached(ID);
        departures
    }

    fn body(notice: Option<NotificationDraft>) -> String {
        notice.expect("the loss should be retained").body
    }

    fn no_wait(_: WaitToken) -> bool {
        panic!("no wait was recorded for this attachment");
    }

    #[test]
    fn an_unexplained_closure_is_neutral_about_its_cause() {
        let notice = attached().ended(ID, no_wait).expect("a lost attachment");
        assert_eq!(notice.severity, NotificationSeverity::Warning);
        assert_eq!(notice.source, NOTICE_SOURCE);
        assert_eq!(notice.title, NOTICE_TITLE);
        assert!(notice.body.contains("cause is unknown"), "{}", notice.body);
        for claim in ["crash", "killed", "memory", "GPU", "timed out"] {
            assert!(!notice.body.contains(claim), "{}", notice.body);
        }
    }

    #[test]
    fn a_known_failure_names_its_fixed_reason_exactly_once() {
        let mut departures = attached();
        departures.transport_failed(ID, TransportFailureCause::WriteTimedOut);
        let text = body(departures.ended(ID, no_wait));
        assert!(
            text.contains(TransportFailureCause::WriteTimedOut.reason()),
            "{text}"
        );
        assert!(!text.contains("cause is unknown"), "{text}");
        assert!(
            departures.ended(ID, no_wait).is_none(),
            "a second closure event must not retain a duplicate notice"
        );
    }

    #[test]
    fn the_first_failure_is_the_one_reported() {
        let mut departures = attached();
        departures.transport_failed(ID, TransportFailureCause::StoppedReading);
        departures.transport_failed(ID, TransportFailureCause::WriteTimedOut);
        let text = body(departures.ended(ID, no_wait));
        assert!(
            text.contains(TransportFailureCause::StoppedReading.reason()),
            "{text}"
        );
    }

    #[test]
    fn leaving_on_purpose_is_quiet_even_if_the_goodbye_fails() {
        let mut departures = attached();
        departures.expect_leaving(ID);
        departures.transport_failed(ID, TransportFailureCause::WriteTimedOut);
        assert!(departures.ended(ID, no_wait).is_none());
    }

    #[test]
    fn a_delivered_switch_is_quiet() {
        let mut departures = attached();
        departures.expect_switch(ID);
        departures.handoff_delivered(ID);
        assert!(departures.ended(ID, no_wait).is_none());
    }

    #[test]
    fn a_queued_switch_the_client_never_received_is_still_a_loss() {
        // A hang-up seen by the write, or EOF before it, records no failure.
        let mut departures = attached();
        departures.expect_switch(ID);
        let text = body(departures.ended(ID, no_wait));
        assert!(text.contains("during a session switch"), "{text}");
        assert!(text.contains("cause is unknown"), "{text}");
    }

    #[test]
    fn delivery_from_an_aborted_attempt_does_not_vouch_for_a_later_switch() {
        let mut departures = attached();
        departures.handoff_delivered(ID);
        departures.expect_switch(ID);
        assert!(departures.ended(ID, no_wait).is_some());
    }

    #[test]
    fn delivery_alone_does_not_excuse_a_loss_without_a_switch() {
        let mut departures = attached();
        departures.handoff_delivered(ID);
        assert!(departures.ended(ID, no_wait).is_some());
    }

    #[test]
    fn a_switch_that_could_not_be_delivered_is_attributed_to_the_switch() {
        let mut departures = attached();
        departures.expect_switch(ID);
        departures.transport_failed(ID, TransportFailureCause::WriteTimedOut);
        let text = body(departures.ended(ID, no_wait));
        assert!(text.contains("during a session switch"), "{text}");
        assert!(
            text.contains(TransportFailureCause::WriteTimedOut.reason()),
            "{text}"
        );
    }

    #[test]
    fn losing_the_client_inside_a_two_phase_switch_is_reported_as_such() {
        let mut departures = attached();
        departures.switch_interrupted(ID);
        let text = body(departures.ended(ID, no_wait));
        assert!(text.contains("during a session switch"), "{text}");
        assert!(text.contains("cause is unknown"), "{text}");
    }

    #[test]
    fn only_a_finished_wait_excuses_a_closure_without_detaching() {
        let mut finished = attached();
        finished.note_wait(ID, token());
        assert!(finished.ended(ID, |waited| waited == token()).is_none());

        let mut pending = attached();
        pending.note_wait(ID, token());
        assert!(pending.ended(ID, |_| false).is_some());
    }

    #[test]
    fn connections_never_attached_interactively_are_quiet() {
        let mut departures = AttachmentDepartures::default();
        departures.transport_failed(ID, TransportFailureCause::InvalidMessage);
        departures.expect_leaving(ID);
        departures.expect_switch(ID);
        departures.handoff_delivered(ID);
        departures.note_wait(ID, token());
        departures.switch_interrupted(ID);
        assert!(departures.ended(ID, no_wait).is_none());
        assert!(departures.attachments.is_empty());
    }

    #[test]
    fn a_record_is_released_when_its_connection_ends() {
        let mut departures = attached();
        departures.attached(ID + 1);
        assert!(departures.ended(ID, no_wait).is_some());
        assert_eq!(departures.attachments.len(), 1);
        assert!(departures.attachments.contains_key(&(ID + 1)));
    }

    #[test]
    fn attachments_are_attributed_by_connection() {
        let mut departures = attached();
        departures.attached(ID + 1);
        departures.expect_leaving(ID);
        departures.transport_failed(ID + 1, TransportFailureCause::ConnectionError);
        assert!(departures.ended(ID, no_wait).is_none());
        let text = body(departures.ended(ID + 1, no_wait));
        assert!(
            text.contains(TransportFailureCause::ConnectionError.reason()),
            "{text}"
        );
    }
}
