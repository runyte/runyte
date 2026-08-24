// SPDX-License-Identifier: MPL-2.0

//! Background word index for word completion.
//!
//! Every buffer that contributes text (see `Buffer::is_read_only`, which the
//! caller checks before sending an update) has its words extracted and
//! counted here, off the main thread, so a keystroke never waits on the
//! recount. The main thread reads a published snapshot: an `Arc` clone under
//! a short lock, never a message round trip, so a query can never block on
//! the worker's own work. A snapshot one keystroke stale is expected and
//! acceptable.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::text::Text;

const REQUEST_CAPACITY: usize = 256;
/// How long the worker waits for an update before it wakes anyway to check
/// for a pending removal. Removals travel on their own unbounded channel
/// (see [`WordIndexHandle::notify_remove`]), so this is the only thing that
/// bounds how long a closed buffer's words can outlive it when nothing else
/// happens to wake the worker.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Cap on distinct words retained per buffer, lowest-frequency evicted first.
///
/// Mirrors the non-configurable backstops in `notification.rs`
/// (`MAX_NOTIFICATION_BYTES`, `MAX_HISTORY_BYTES`): a bound that exists so a
/// pathological buffer cannot grow the index without limit, not a setting
/// anyone is expected to tune.
const MAX_WORDS_PER_BUFFER: usize = 20_000;

/// Cap on distinct buffers tracked at once. Closing a buffer removes it
/// directly; this is only a backstop against a buffer whose removal was
/// somehow never observed.
const MAX_INDEXED_BUFFERS: usize = 256;

/// Wrapper punctuation trimmed from both ends of a stored word, and from the
/// start of a completion query so it can still match a word the index
/// stored without it (see [`trim_word`] and `App::word_completion`).
pub(crate) fn is_wrapper_punctuation(c: char) -> bool {
    matches!(
        c,
        '`' | '\'' | '"' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
    )
}

/// One update sent to the worker thread.
enum WordIndexMessage {
    UpdateBuffer {
        buffer_id: usize,
        text: Text,
    },
    #[cfg(test)]
    Flush(std::sync::mpsc::Sender<()>),
}

/// A buffer's words, sorted by descending frequency, ties broken by the word
/// itself so ordering is deterministic. Sorting happens once here, worker
/// side, rather than on every completion trigger.
#[derive(Clone, Debug, Default)]
pub struct BufferWords {
    /// `(word, count)`, sorted by count descending then word ascending.
    entries: Vec<(String, u32)>,
}

impl BufferWords {
    fn from_counts(counts: HashMap<String, u32>) -> Self {
        let mut entries: Vec<(String, u32)> = counts.into_iter().collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        entries.truncate(MAX_WORDS_PER_BUFFER);
        Self { entries }
    }

    pub fn entries(&self) -> &[(String, u32)] {
        &self.entries
    }
}

/// The latest published state of the index: one word list per buffer.
#[derive(Clone, Debug, Default)]
pub struct WordIndexSnapshot {
    buffers: HashMap<usize, BufferWords>,
}

impl WordIndexSnapshot {
    pub fn buffer_words(&self, buffer_id: usize) -> Option<&BufferWords> {
        self.buffers.get(&buffer_id)
    }

    pub fn other_buffers(&self, buffer_id: usize) -> impl Iterator<Item = (usize, &BufferWords)> {
        self.buffers
            .iter()
            .filter(move |(id, _)| **id != buffer_id)
            .map(|(id, words)| (*id, words))
    }
}

/// Handle the main thread holds. Cheap to clone, safe to hold on `App`.
#[derive(Clone)]
pub struct WordIndexHandle {
    updates: SyncSender<WordIndexMessage>,
    removals: Sender<usize>,
    snapshot: Arc<Mutex<Arc<WordIndexSnapshot>>>,
}

impl WordIndexHandle {
    /// Sends the buffer's current text for reindexing. Never blocks: a full
    /// queue or a stopped worker just means this update is skipped, and the
    /// next one supersedes it.
    pub fn notify_update(&self, buffer_id: usize, text: Text) {
        let _ = self
            .updates
            .try_send(WordIndexMessage::UpdateBuffer { buffer_id, text });
    }

    /// Drops a closed buffer's words from the index.
    ///
    /// Delivered on its own unbounded channel rather than through
    /// `notify_update`'s bounded, lossy one: an update that is dropped under
    /// backpressure is harmless because the next edit supersedes it, but a
    /// dropped removal has no such successor and would leave a closed
    /// buffer's words in the published snapshot indefinitely.
    pub fn notify_remove(&self, buffer_id: usize) {
        let _ = self.removals.send(buffer_id);
    }

    /// The most recently published snapshot. Locks only long enough to clone
    /// the `Arc`, so this never waits on the worker.
    pub fn current(&self) -> Arc<WordIndexSnapshot> {
        self.snapshot
            .lock()
            .map(|guard| Arc::clone(&guard))
            .unwrap_or_default()
    }

    /// Blocks until every message sent before this call has been applied.
    /// Test-only: production code never needs the worker to be caught up,
    /// since a stale snapshot is an accepted outcome.
    #[cfg(test)]
    pub fn flush(&self) {
        let (tx, rx) = std::sync::mpsc::channel();
        if self.updates.send(WordIndexMessage::Flush(tx)).is_ok() {
            let _ = rx.recv();
        }
    }
}

/// Spawns the background worker and returns the handle the main thread keeps.
pub fn spawn() -> WordIndexHandle {
    let (tx, rx) = sync_channel(REQUEST_CAPACITY);
    let (removal_tx, removal_rx) = std::sync::mpsc::channel();
    let snapshot = Arc::new(Mutex::new(Arc::new(WordIndexSnapshot::default())));
    let handle = WordIndexHandle {
        updates: tx,
        removals: removal_tx,
        snapshot: Arc::clone(&snapshot),
    };
    std::thread::spawn(move || run(rx, removal_rx, snapshot));
    handle
}

fn run(
    requests: Receiver<WordIndexMessage>,
    removals: Receiver<usize>,
    snapshot: Arc<Mutex<Arc<WordIndexSnapshot>>>,
) {
    let mut counts: HashMap<usize, HashMap<String, u32>> = HashMap::new();
    // Recency order for the `MAX_INDEXED_BUFFERS` backstop: touched buffers
    // move to the back, so the front is the least recently updated.
    let mut recency: Vec<usize> = Vec::new();

    loop {
        // Drained once before waiting, so a removal already sent has taken
        // effect before this iteration acts on anything else.
        let mut changed = drain_removals(&removals, &mut counts, &mut recency);

        let flush_reply: Option<std::sync::mpsc::Sender<()>> =
            match requests.recv_timeout(POLL_INTERVAL) {
                Ok(WordIndexMessage::UpdateBuffer { buffer_id, text }) => {
                    let mut buffer_counts: HashMap<String, u32> = HashMap::new();
                    for word in words_in(&text) {
                        *buffer_counts.entry(word).or_insert(0) += 1;
                    }
                    counts.insert(buffer_id, buffer_counts);
                    touch(&mut recency, buffer_id);
                    while counts.len() > MAX_INDEXED_BUFFERS {
                        if recency.is_empty() {
                            break;
                        }
                        let oldest = recency.remove(0);
                        counts.remove(&oldest);
                    }
                    changed = true;
                    None
                }
                #[cfg(test)]
                Ok(WordIndexMessage::Flush(reply)) => Some(reply),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    changed |= drain_removals(&removals, &mut counts, &mut recency);
                    if changed {
                        publish(&snapshot, &counts);
                    }
                    break;
                }
            };

        // Drained again: a removal sent concurrently with whatever was just
        // received — including the message a `Flush` reply is about to
        // answer — was not yet visible to the drain above, and must still be
        // applied before that reply goes out or this iteration publishes.
        changed |= drain_removals(&removals, &mut counts, &mut recency);

        if changed {
            publish(&snapshot, &counts);
        }
        if let Some(reply) = flush_reply {
            let _ = reply.send(());
        }
    }
}

fn drain_removals(
    removals: &Receiver<usize>,
    counts: &mut HashMap<usize, HashMap<String, u32>>,
    recency: &mut Vec<usize>,
) -> bool {
    let mut changed = false;
    while let Ok(buffer_id) = removals.try_recv() {
        counts.remove(&buffer_id);
        recency.retain(|id| *id != buffer_id);
        changed = true;
    }
    changed
}

fn touch(recency: &mut Vec<usize>, buffer_id: usize) {
    recency.retain(|id| *id != buffer_id);
    recency.push(buffer_id);
}

fn publish(
    snapshot: &Arc<Mutex<Arc<WordIndexSnapshot>>>,
    counts: &HashMap<usize, HashMap<String, u32>>,
) {
    let buffers = counts
        .iter()
        .map(|(id, buffer_counts)| (*id, BufferWords::from_counts(buffer_counts.clone())))
        .collect();
    if let Ok(mut guard) = snapshot.lock() {
        *guard = Arc::new(WordIndexSnapshot { buffers });
    }
}

/// A word is a run of characters between whitespace, trimmed of punctuation
/// that merely wraps it rather than belonging to it.
///
/// Deliberately wider than an identifier: `--session-restart`,
/// `:quit-here`, and `background-color` are meant to stay whole, so only a
/// small, fixed set of wrapper punctuation is trimmed from each end, and
/// nothing is trimmed from the interior.
fn words_in(text: &Text) -> Vec<String> {
    text.lines()
        .flat_map(|line| {
            line.split_whitespace()
                .filter_map(trim_word)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn trim_word(token: &str) -> Option<String> {
    let trimmed = token.trim_matches(is_wrapper_punctuation);
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(content: &str) -> Text {
        let mut text = Text::new();
        text.apply(&crate::text::Transaction::insert(0, content.to_owned()));
        text
    }

    #[test]
    fn preserves_examples_from_the_issue() {
        assert_eq!(
            trim_word("--session-restart").as_deref(),
            Some("--session-restart")
        );
        assert_eq!(trim_word(":quit-here").as_deref(), Some(":quit-here"));
        assert_eq!(
            trim_word("background-color").as_deref(),
            Some("background-color")
        );
    }

    #[test]
    fn trims_surrounding_punctuation_only() {
        assert_eq!(
            trim_word("`--session-list`").as_deref(),
            Some("--session-list")
        );
        assert_eq!(
            trim_word("background-color,").as_deref(),
            Some("background-color")
        );
        assert_eq!(trim_word("(word)").as_deref(), Some("word"));
    }

    #[test]
    fn discards_pure_punctuation_tokens() {
        assert_eq!(trim_word(","), None);
        assert_eq!(trim_word("`\"'"), None);
    }

    #[test]
    fn keeps_interior_punctuation_untouched() {
        assert_eq!(trim_word("foo_bar.baz").as_deref(), Some("foo_bar.baz"));
    }

    #[test]
    fn splits_on_whitespace_across_lines() {
        let text = text_of("foo bar\nbaz");
        let words = words_in(&text);
        assert_eq!(words, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn worker_indexes_and_removes_buffers() {
        let handle = spawn();
        handle.notify_update(1, text_of("foo foo bar"));
        handle.notify_update(2, text_of("foo baz baz baz"));
        handle.flush();

        let snapshot = handle.current();
        let own = snapshot.buffer_words(1).unwrap();
        assert_eq!(
            own.entries(),
            &[("foo".to_owned(), 2), ("bar".to_owned(), 1)]
        );

        let other = snapshot.buffer_words(2).unwrap();
        assert_eq!(
            other.entries(),
            &[("baz".to_owned(), 3), ("foo".to_owned(), 1)]
        );

        handle.notify_remove(1);
        handle.flush();
        let snapshot = handle.current();
        assert!(snapshot.buffer_words(1).is_none());
        assert!(snapshot.buffer_words(2).is_some());
    }

    /// A removal sent while the update queue is saturated must still take
    /// effect: it travels on its own unbounded channel rather than being
    /// silently dropped by `try_send` the way a superseded update would be.
    #[test]
    fn a_removal_survives_a_saturated_update_queue() {
        let handle = spawn();
        handle.notify_update(1, text_of("gadget"));
        handle.flush();
        assert!(handle.current().buffer_words(1).is_some());

        // Saturate the bounded update channel before the worker can drain
        // it, so any `RemoveBuffer` sent the same way would have been lost.
        for _ in 0..(REQUEST_CAPACITY * 4) {
            handle.notify_update(2, text_of("filler"));
        }
        handle.notify_remove(1);
        handle.flush();

        assert!(handle.current().buffer_words(1).is_none());
    }
}
