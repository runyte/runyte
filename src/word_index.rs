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
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};

use crate::text::Text;

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

enum PendingAction {
    Update(Text),
    Remove,
}

#[derive(Default)]
struct PendingState {
    next_sequence: u64,
    actions: HashMap<usize, (u64, PendingAction)>,
}

impl PendingState {
    fn insert(&mut self, buffer_id: usize, action: PendingAction) {
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("word-index action sequence exhausted");
        self.actions.insert(buffer_id, (sequence, action));
    }

    fn take_ordered(&mut self) -> Vec<(usize, PendingAction)> {
        let mut actions = std::mem::take(&mut self.actions)
            .into_iter()
            .map(|(buffer, (sequence, action))| (sequence, buffer, action))
            .collect::<Vec<_>>();
        actions.sort_unstable_by_key(|(sequence, _, _)| *sequence);
        actions
            .into_iter()
            .map(|(_, buffer, action)| (buffer, action))
            .collect()
    }
}

/// One wakeup or ordering barrier sent to the worker thread. Buffer actions
/// live in the shared map so each buffer retains its latest action without a
/// bounded queue dropping the final state.
enum WorkerMessage {
    Wake,
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
    messages: SyncSender<WorkerMessage>,
    pending: Arc<Mutex<PendingState>>,
    snapshot: Arc<Mutex<Arc<WordIndexSnapshot>>>,
}

impl WordIndexHandle {
    /// Sends the buffer's current text for reindexing. Never blocks: a full
    /// stopped worker is harmless. Repeated updates for one buffer coalesce to
    /// the newest text, while distinct buffers retain independent actions.
    pub fn notify_update(&self, buffer_id: usize, text: Text) {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(buffer_id, PendingAction::Update(text));
        let _ = self.messages.try_send(WorkerMessage::Wake);
    }

    /// Drops a closed buffer's words from the index.
    ///
    /// Removal replaces any pending update for the same buffer in the shared
    /// latest-action map, so an older queued wakeup cannot resurrect it.
    pub fn notify_remove(&self, buffer_id: usize) {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(buffer_id, PendingAction::Remove);
        let _ = self.messages.try_send(WorkerMessage::Wake);
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
        if self.messages.send(WorkerMessage::Flush(tx)).is_ok() {
            let _ = rx.recv();
        }
    }
}

/// Spawns the background worker and returns the handle the main thread keeps.
pub fn spawn() -> WordIndexHandle {
    let (tx, rx) = sync_channel(1);
    let pending = Arc::new(Mutex::new(PendingState::default()));
    let snapshot = Arc::new(Mutex::new(Arc::new(WordIndexSnapshot::default())));
    let handle = WordIndexHandle {
        messages: tx,
        pending: Arc::clone(&pending),
        snapshot: Arc::clone(&snapshot),
    };
    std::thread::spawn(move || run(rx, pending, snapshot));
    handle
}

fn run(
    messages: Receiver<WorkerMessage>,
    pending: Arc<Mutex<PendingState>>,
    snapshot: Arc<Mutex<Arc<WordIndexSnapshot>>>,
) {
    let mut counts: HashMap<usize, HashMap<String, u32>> = HashMap::new();
    // Recency order for the `MAX_INDEXED_BUFFERS` backstop: touched buffers
    // move to the back, so the front is the least recently updated.
    let mut recency: Vec<usize> = Vec::new();

    while let Ok(message) = messages.recv() {
        let flush_reply: Option<std::sync::mpsc::Sender<()>> = match message {
            WorkerMessage::Wake => None,
            #[cfg(test)]
            WorkerMessage::Flush(reply) => Some(reply),
        };
        let mut changed = false;
        loop {
            let actions = {
                let mut pending = pending.lock().unwrap_or_else(|error| error.into_inner());
                pending.take_ordered()
            };
            if actions.is_empty() {
                break;
            }
            for (buffer_id, action) in actions {
                match action {
                    PendingAction::Update(text) => {
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
                    }
                    PendingAction::Remove => {
                        counts.remove(&buffer_id);
                        recency.retain(|id| *id != buffer_id);
                        changed = true;
                    }
                }
            }
        }

        if changed {
            publish(&snapshot, &counts);
        }
        if let Some(reply) = flush_reply {
            let _ = reply.send(());
        }
    }
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

    #[test]
    fn the_latest_update_survives_a_large_burst() {
        let handle = spawn();
        for index in 0..1_000 {
            handle.notify_update(1, text_of(&format!("word{index}")));
        }
        handle.notify_update(1, text_of("final-word"));
        handle.flush();

        assert_eq!(
            handle.current().buffer_words(1).unwrap().entries(),
            &[("final-word".to_owned(), 1)]
        );
    }

    #[test]
    fn a_removal_supersedes_every_older_update() {
        let handle = spawn();
        for index in 0..1_000 {
            handle.notify_update(1, text_of(&format!("word{index}")));
        }
        handle.notify_remove(1);
        handle.flush();

        assert!(handle.current().buffer_words(1).is_none());
    }

    fn queue_one_batch(
        handle: &WordIndexHandle,
        actions: impl IntoIterator<Item = (usize, PendingAction)>,
    ) {
        let mut pending = handle.pending.lock().unwrap();
        for (buffer, action) in actions {
            pending.insert(buffer, action);
        }
        drop(pending);
        let _ = handle.messages.try_send(WorkerMessage::Wake);
    }

    #[test]
    fn same_batch_removal_precedes_capacity_replacement() {
        let handle = spawn();
        for buffer in 0..MAX_INDEXED_BUFFERS {
            handle.notify_update(buffer, text_of(&format!("word{buffer}")));
        }
        handle.flush();

        queue_one_batch(
            &handle,
            [
                (100, PendingAction::Remove),
                (MAX_INDEXED_BUFFERS, PendingAction::Update(text_of("new"))),
            ],
        );
        handle.flush();

        let snapshot = handle.current();
        assert_eq!(snapshot.buffers.len(), MAX_INDEXED_BUFFERS);
        assert!(snapshot.buffer_words(0).is_some());
        assert!(snapshot.buffer_words(100).is_none());
        assert!(snapshot.buffer_words(MAX_INDEXED_BUFFERS).is_some());
    }

    #[test]
    fn same_batch_refresh_controls_the_next_capacity_eviction() {
        let handle = spawn();
        for buffer in 0..MAX_INDEXED_BUFFERS {
            handle.notify_update(buffer, text_of(&format!("word{buffer}")));
        }
        handle.flush();

        queue_one_batch(
            &handle,
            [
                (0, PendingAction::Update(text_of("refreshed"))),
                (MAX_INDEXED_BUFFERS, PendingAction::Update(text_of("new"))),
            ],
        );
        handle.flush();

        let snapshot = handle.current();
        assert_eq!(snapshot.buffers.len(), MAX_INDEXED_BUFFERS);
        assert!(snapshot.buffer_words(0).is_some());
        assert!(snapshot.buffer_words(1).is_none());
        assert!(snapshot.buffer_words(MAX_INDEXED_BUFFERS).is_some());
    }
}
