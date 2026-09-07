// SPDX-License-Identifier: MPL-2.0

//! Background syntax reparsing and the deliberately narrow stale-tree view.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use super::{DocumentSyntax, Registry, Span};
use crate::text::{Assoc, Offset, Text, Transaction};

#[derive(Clone, Debug)]
struct PendingEdit {
    forward: Transaction,
    backward: Transaction,
}

impl PendingEdit {
    fn new(before: &Text, forward: &Transaction) -> Self {
        let mut after = before.clone();
        let backward = after.apply(forward).into_transaction();
        Self {
            forward: forward.clone(),
            backward,
        }
    }
}

/// A retained parser tree whose offsets describe an older text revision.
///
/// Its tree is private and this type exposes only translated highlighting.
/// Structural callers can therefore use only a current [`DocumentSyntax`]; a
/// new structural query cannot accidentally opt into stale offsets.
#[derive(Clone, Debug)]
pub(crate) struct StaleSyntax {
    syntax: DocumentSyntax,
    text: Text,
    edits: Vec<PendingEdit>,
    current_text: Text,
}

impl StaleSyntax {
    pub(crate) fn new(
        syntax: DocumentSyntax,
        before: &Text,
        current: &Text,
        transaction: &Transaction,
    ) -> Self {
        Self {
            syntax,
            text: before.clone(),
            edits: vec![PendingEdit::new(before, transaction)],
            current_text: current.clone(),
        }
    }

    pub(crate) fn append(&mut self, before: &Text, current: &Text, transaction: &Transaction) {
        debug_assert_eq!(self.current_text.revision(), before.revision());
        self.edits.push(PendingEdit::new(before, transaction));
        self.current_text = current.clone();
    }

    pub(crate) fn request(&self, buffer: usize, generation: u64) -> ParseRequest {
        ParseRequest {
            buffer,
            generation,
            language: self.syntax.language(),
            target: self.current_text.clone(),
            base: Some((self.syntax.clone(), self.text.clone())),
        }
    }

    pub(crate) fn translated_spans(
        &self,
        current: &Text,
        registry: &Registry,
        from: Offset,
        to: Offset,
    ) -> TranslatedSpans {
        let from = from.min(current.len_chars());
        let to = to.min(current.len_chars());
        if from >= to {
            return TranslatedSpans { spans: Vec::new() };
        }
        let (mut stale_from, mut stale_to) = (from.min(to), from.max(to));
        for edit in self.edits.iter().rev() {
            stale_from = edit.backward.map_offset(stale_from, Assoc::Before);
            stale_to = edit.backward.map_offset(stale_to, Assoc::After);
        }
        stale_from = stale_from.min(self.text.len_chars());
        stale_to = stale_to.min(self.text.len_chars()).max(stale_from);

        let mut spans = self
            .syntax
            .spans(&self.text, registry, stale_from, stale_to);
        for span in &mut spans {
            for edit in &self.edits {
                span.from = edit.forward.map_offset(span.from, Assoc::After);
                span.to = edit.forward.map_offset(span.to, Assoc::Before);
            }
            span.from = span.from.clamp(from, to);
            span.to = span.to.clamp(from, to);
        }
        spans.retain(|span| span.from < span.to);
        TranslatedSpans { spans }
    }
}

/// Highlight spans translated from a retained older parse revision.
///
/// This wrapper is intentionally distinct from [`DocumentSyntax`]. It carries
/// no parser tree and can answer no structural query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TranslatedSpans {
    spans: Vec<Span>,
}

impl TranslatedSpans {
    pub(crate) fn into_spans(self) -> Vec<Span> {
        self.spans
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ParseRequest {
    pub(crate) buffer: usize,
    pub(crate) generation: u64,
    pub(crate) language: super::LanguageId,
    pub(crate) target: Text,
    base: Option<(DocumentSyntax, Text)>,
}

impl ParseRequest {
    pub(crate) fn full(
        buffer: usize,
        generation: u64,
        language: super::LanguageId,
        target: Text,
    ) -> Self {
        Self {
            buffer,
            generation,
            language,
            target,
            base: None,
        }
    }

    pub(crate) fn accepts(&self, event: &SyntaxEvent) -> bool {
        self.buffer == event.buffer
            && self.generation == event.generation
            && self.language == event.language
            && self.target.revision() == event.text_revision
    }
}

/// A completed parse. Only the application may publish it against live text.
#[derive(Clone, Debug)]
pub struct SyntaxEvent {
    pub(crate) buffer: usize,
    pub(crate) generation: u64,
    pub(crate) language: super::LanguageId,
    pub(crate) text_revision: u64,
    pub(crate) syntax: Option<DocumentSyntax>,
    pub(crate) failure: Option<String>,
}

#[derive(Default, Debug)]
struct Queue {
    pending: HashMap<usize, ParseRequest>,
    order: std::collections::VecDeque<usize>,
    completed: HashMap<usize, (SyntaxEvent, Text)>,
    /// Only worker-produced trees enter retirement, never each typed snapshot.
    /// Drained before another parse, so typing cannot build a disposal backlog.
    retired: Vec<DocumentSyntax>,
    completion_order: std::collections::VecDeque<usize>,
    // Includes the active request, so cancellation also invalidates its result.
    generations: HashMap<usize, u64>,
    stopped: bool,
    preferred: Option<usize>,
    last_preferred: bool,
}

#[derive(Default, Debug)]
struct Shared {
    queue: Mutex<Queue>,
    wake: std::sync::Condvar,
    finished: tokio::sync::Notify,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn stop(&self) {
        let mut queue = self.lock();
        queue.stopped = true;
        // The parser thread drains ownership outside the mutex. Clearing these
        // maps here could delete a whole unread tree on the input thread.
        drop(queue);
        self.wake.notify_one();
        self.finished.notify_one();
    }
}

#[derive(Debug)]
struct Sender(Arc<Shared>);

impl Drop for Sender {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// Non-blocking editor handle. Clones share one shutdown owner.
#[derive(Clone, Debug)]
pub struct SyntaxHandle(Arc<Sender>);

impl SyntaxHandle {
    pub(crate) fn send(&self, request: ParseRequest) -> bool {
        let shared = &self.0.0;
        let mut queue = shared.lock();
        if queue.stopped {
            return false;
        }
        let buffer = request.buffer;
        queue.generations.insert(buffer, request.generation);
        // Hide an unread completion, but let the worker reuse or dispose of it.
        queue.completion_order.retain(|id| *id != buffer);
        let generation = request.generation;
        let mut previous = queue.pending.insert(buffer, request);
        if let Some(previous) = previous.as_mut() {
            if previous.generation != generation
                && let Some((syntax, _)) = previous.base.take()
            {
                queue.retired.push(syntax);
            }
        } else {
            queue.order.push_back(buffer);
        }
        drop(queue);
        // A same-generation base is shared with its replacement. Last-owner
        // bases above were transferred to the worker instead.
        drop(previous);
        shared.wake.notify_one();
        true
    }

    /// Prefer the active document at the next job boundary, alternating with
    /// FIFO work so other open documents cannot be starved by typing.
    pub(crate) fn prioritize(&self, buffer: usize) {
        self.0.0.lock().preferred = Some(buffer);
    }

    pub(crate) fn cancel(&self, buffer: usize) {
        let shared = &self.0.0;
        let mut queue = shared.lock();
        let mut request = queue.pending.remove(&buffer);
        if let Some(request) = request.as_mut()
            && let Some((syntax, _)) = request.base.take()
        {
            queue.retired.push(syntax);
        }
        queue.order.retain(|id| *id != buffer);
        let mut completed = queue.completed.remove(&buffer);
        if let Some((event, _)) = completed.as_mut()
            && let Some(syntax) = event.syntax.take()
        {
            queue.retired.push(syntax);
        }
        queue.completion_order.retain(|id| *id != buffer);
        queue.generations.remove(&buffer);
        drop(queue);
        drop((request, completed));
        shared.wake.notify_one();
    }
}

/// Coalesced completed parses. Dropping this receiver also stops queued work.
#[derive(Debug)]
pub struct SyntaxEvents {
    shared: Arc<Shared>,
}

impl Drop for SyntaxEvents {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

impl SyntaxEvents {
    pub async fn recv(&mut self) -> Option<SyntaxEvent> {
        loop {
            let notified = self.shared.finished.notified();
            {
                let mut queue = self.shared.lock();
                if queue.stopped {
                    return None;
                }
                if let Some(buffer) = queue.completion_order.pop_front() {
                    return queue.completed.remove(&buffer).map(|(event, _)| event);
                }
            }
            notified.await;
        }
    }
}

/// Starts one owned parser thread, independent of the Tokio blocking pool.
/// Shutdown never joins an in-flight parse; the thread owns only snapshots.
pub fn spawn_background(registry: Arc<Registry>) -> (SyntaxHandle, SyntaxEvents) {
    let shared = Arc::new(Shared::default());
    let parser_shared = Arc::clone(&shared);
    if std::thread::Builder::new()
        .name("runyte-syntax".into())
        .spawn(move || run_worker(registry, parser_shared))
        .is_err()
    {
        shared.stop();
    }
    (
        SyntaxHandle(Arc::new(Sender(Arc::clone(&shared)))),
        SyntaxEvents { shared },
    )
}

fn run_worker(registry: Arc<Registry>, shared: Arc<Shared>) {
    run_worker_with_parse(registry, shared, parse);
}

fn run_worker_with_parse(
    registry: Arc<Registry>,
    shared: Arc<Shared>,
    parse_request: impl Fn(
        ParseRequest,
        Option<(DocumentSyntax, Text)>,
        &Registry,
    ) -> Option<DocumentSyntax>,
) {
    run_worker_with_disposal(registry, shared, parse_request, || {});
}

fn run_worker_with_disposal(
    registry: Arc<Registry>,
    shared: Arc<Shared>,
    parse_request: impl Fn(
        ParseRequest,
        Option<(DocumentSyntax, Text)>,
        &Registry,
    ) -> Option<DocumentSyntax>,
    before_disposal: impl Fn(),
) {
    // The observer is a deterministic test seam; production compiles it away.
    let dispose = |value| {
        before_disposal();
        drop(value);
    };
    let mut checkpoints = HashMap::<usize, (u64, DocumentSyntax, Text)>::new();
    loop {
        let (request, completed) = {
            let mut queue = shared.lock();
            if queue.stopped {
                let retired = std::mem::replace(
                    &mut *queue,
                    Queue {
                        stopped: true,
                        ..Queue::default()
                    },
                );
                drop(queue);
                before_disposal();
                drop((retired, checkpoints));
                return;
            }
            // Extract obsolete values under the lock, release their trees only
            // after unlocking. A destructor may walk the entire document.
            let retired = std::mem::take(&mut queue.retired);
            let expired = checkpoints
                .iter()
                .filter_map(|(buffer, (generation, _, _))| {
                    (!queue
                        .pending
                        .get(buffer)
                        .is_some_and(|request| request.generation == *generation))
                    .then_some(*buffer)
                })
                .collect::<Vec<_>>();
            let expired = expired
                .into_iter()
                .map(|buffer| checkpoints.remove(&buffer).expect("checkpoint"))
                .collect::<Vec<_>>();
            if !retired.is_empty() || !expired.is_empty() {
                drop(queue);
                before_disposal();
                drop((retired, expired));
                continue;
            }
            if queue.order.is_empty() {
                drop(
                    shared
                        .wake
                        .wait(queue)
                        .unwrap_or_else(|error| error.into_inner()),
                );
                continue;
            }
            let preferred = (!queue.last_preferred)
                .then(|| {
                    queue
                        .order
                        .iter()
                        .position(|buffer| Some(*buffer) == queue.preferred)
                })
                .flatten();
            let buffer = preferred
                .and_then(|position| queue.order.remove(position))
                .or_else(|| queue.order.pop_front())
                .expect("queued buffer");
            queue.last_preferred = Some(buffer) == queue.preferred;
            (
                queue.pending.remove(&buffer).expect("queued request"),
                queue.completed.remove(&buffer),
            )
        };
        let target = request.target.clone();
        let initial = request.base.is_none();
        let checkpoint = checkpoints
            .remove(&request.buffer)
            .filter(|(generation, _, _)| *generation == request.generation)
            .map(|(_, syntax, text)| (syntax, text));
        let checkpoint = match completed {
            Some((mut event, text))
                if initial
                    && event.generation == request.generation
                    && event.language == request.language =>
            {
                event
                    .syntax
                    .take()
                    .map(|syntax| (syntax, text))
                    .or(checkpoint)
            }
            Some((event, _)) => {
                if let Some(syntax) = event.syntax {
                    dispose(vec![syntax]);
                }
                checkpoint
            }
            None => checkpoint,
        };
        let mut event = SyntaxEvent {
            buffer: request.buffer,
            generation: request.generation,
            language: request.language,
            text_revision: target.revision(),
            syntax: None,
            failure: None,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parse_request(request, checkpoint, &registry)
        })) {
            Ok(Some(syntax)) => event.syntax = Some(syntax),
            Ok(None) => event.failure = Some("syntax parsing failed or timed out".into()),
            Err(_) => event.failure = Some("syntax parser panicked".into()),
        }
        let mut queue = shared.lock();
        if queue.stopped || queue.generations.get(&event.buffer) != Some(&event.generation) {
            drop(queue);
            if let Some(syntax) = event.syntax {
                dispose(vec![syntax]);
            }
            continue;
        }
        if queue.pending.contains_key(&event.buffer) {
            drop(queue);
            if initial && let Some(syntax) = event.syntax {
                let previous = checkpoints.insert(event.buffer, (event.generation, syntax, target));
                if let Some((_, syntax, _)) = previous {
                    dispose(vec![syntax]);
                }
            }
        } else {
            let buffer = event.buffer;
            let replaced = queue.completed.insert(buffer, (event, target));
            if replaced.is_none() {
                queue.completion_order.push_back(buffer);
            }
            queue.generations.remove(&buffer);
            drop(queue);
            if let Some((event, _)) = replaced
                && let Some(syntax) = event.syntax
            {
                dispose(vec![syntax]);
            }
            shared.finished.notify_one();
        }
    }
}

fn parse(
    request: ParseRequest,
    checkpoint: Option<(DocumentSyntax, Text)>,
    registry: &Registry,
) -> Option<DocumentSyntax> {
    if let Some((mut syntax, text)) = request.base.or(checkpoint) {
        let parsed = text.change_to(&request.target).is_none_or(|transaction| {
            syntax.update(&text, &request.target, &transaction, registry)
        });
        parsed.then_some(syntax)
    } else {
        DocumentSyntax::new(&request.target, request.language, registry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_request(registry: &Registry, buffer: usize, source: &str) -> ParseRequest {
        let language = registry.language_for_name("rust").unwrap();
        let before = Text::from_str(source);
        let syntax = DocumentSyntax::new(&before, language, registry).unwrap();
        let transaction = Transaction::insert(0, "// changed\n");
        let mut current = before.clone();
        current.apply(&transaction);
        StaleSyntax::new(syntax, &before, &current, &transaction).request(buffer, 1)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn distinct_buffer_requests_are_not_coalesced_away() {
        let registry = Arc::new(Registry::new());
        let (worker, mut events) = spawn_background(Arc::clone(&registry));

        worker.send(pending_request(&registry, 3, "fn first() {}\n"));
        worker.send(pending_request(&registry, 7, "fn second() {}\n"));

        let first = events.recv().await.unwrap().buffer;
        let second = events.recv().await.unwrap().buffer;
        assert_eq!(
            [first, second]
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            [3, 7].into_iter().collect()
        );
    }

    #[test]
    fn reversed_stale_highlight_ranges_are_empty() {
        let registry = Registry::new();
        let language = registry.language_for_name("rust").unwrap();
        let before = Text::from_str("fn main() {}\n");
        let syntax = DocumentSyntax::new(&before, language, &registry).unwrap();
        let transaction = Transaction::insert(0, "// changed\n");
        let mut current = before.clone();
        current.apply(&transaction);
        let stale = StaleSyntax::new(syntax, &before, &current, &transaction);

        assert!(
            stale
                .translated_spans(&current, &registry, 8, 2)
                .into_spans()
                .is_empty()
        );
    }
}

#[cfg(test)]
#[path = "tests/background_lifecycle.rs"]
mod lifecycle_tests;
