// SPDX-License-Identifier: MPL-2.0

//! Background syntax reparsing and the deliberately narrow stale-tree view.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio::sync::mpsc;

use super::{DocumentSyntax, Registry, Span, SyntaxRevision};
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

    pub(crate) fn request(&self, buffer: usize) -> ParseRequest {
        ParseRequest {
            buffer,
            base_revision: self.syntax.revision(),
            text_revision: self.current_text.revision(),
            syntax: self.syntax.clone(),
            text: self.text.clone(),
            target: self.current_text.clone(),
        }
    }

    pub(crate) fn accepts(&self, event: &SyntaxEvent) -> bool {
        self.syntax.revision() == event.base_revision
            && self.current_text.revision() == event.text_revision
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
    buffer: usize,
    base_revision: SyntaxRevision,
    text_revision: u64,
    syntax: DocumentSyntax,
    text: Text,
    target: Text,
}

/// A finished background parse, tagged with both parser and text revisions.
#[derive(Clone, Debug)]
pub struct SyntaxEvent {
    pub(crate) buffer: usize,
    pub(crate) base_revision: SyntaxRevision,
    pub(crate) text_revision: u64,
    pub(crate) syntax: Option<DocumentSyntax>,
}

/// Non-blocking editor handle for the coalescing parse worker.
#[derive(Clone, Debug)]
pub struct SyntaxHandle {
    pending: Arc<Mutex<HashMap<usize, ParseRequest>>>,
    ready: mpsc::UnboundedSender<usize>,
}

impl SyntaxHandle {
    pub(crate) fn send(&self, request: ParseRequest) {
        let buffer = request.buffer;
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let newly_ready = pending.insert(buffer, request).is_none();
        drop(pending);
        if newly_ready {
            let _ = self.ready.send(buffer);
        }
    }
}

/// Receiver for completed parses.
#[derive(Debug)]
pub struct SyntaxEvents {
    events: mpsc::UnboundedReceiver<SyntaxEvent>,
}

impl SyntaxEvents {
    pub async fn recv(&mut self) -> Option<SyntaxEvent> {
        self.events.recv().await
    }
}

/// Starts one parser worker. Must be called inside a Tokio runtime.
pub fn spawn_background(registry: Arc<Registry>) -> (SyntaxHandle, SyntaxEvents) {
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let (ready_tx, ready_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    tokio::spawn(run_worker(
        registry,
        Arc::clone(&pending),
        ready_rx,
        event_tx,
    ));
    (
        SyntaxHandle {
            pending,
            ready: ready_tx,
        },
        SyntaxEvents { events: event_rx },
    )
}

async fn run_worker(
    registry: Arc<Registry>,
    pending: Arc<Mutex<HashMap<usize, ParseRequest>>>,
    mut ready: mpsc::UnboundedReceiver<usize>,
    events: mpsc::UnboundedSender<SyntaxEvent>,
) {
    while let Some(buffer) = ready.recv().await {
        let request = {
            let mut pending = pending.lock().unwrap_or_else(|error| error.into_inner());
            pending.remove(&buffer)
        };
        let Some(request) = request else {
            continue;
        };
        let parser_registry = Arc::clone(&registry);
        let result = tokio::task::spawn_blocking(move || parse(request, &parser_registry)).await;
        let Ok(event) = result else {
            continue;
        };
        let has_newer = pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(&buffer);
        if !has_newer && events.send(event).is_err() {
            return;
        }
    }
}

fn parse(request: ParseRequest, registry: &Registry) -> SyntaxEvent {
    let ParseRequest {
        buffer,
        base_revision,
        text_revision,
        mut syntax,
        text,
        target,
    } = request;
    let parsed = text
        .change_to(&target)
        .is_none_or(|transaction| syntax.update(&text, &target, &transaction, registry));
    SyntaxEvent {
        buffer,
        base_revision,
        text_revision,
        syntax: parsed.then_some(syntax),
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
        StaleSyntax::new(syntax, &before, &current, &transaction).request(buffer)
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
