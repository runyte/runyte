// SPDX-License-Identifier: MPL-2.0

//! Background syntax reparsing and the deliberately narrow stale-tree view.

use std::sync::Arc;

use tokio::sync::watch;

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
    requests: watch::Sender<Option<ParseRequest>>,
}

impl SyntaxHandle {
    pub(crate) fn send(&self, request: ParseRequest) {
        self.requests.send_replace(Some(request));
    }
}

/// Receiver for completed parses. Only the newest undrained result is kept.
#[derive(Debug)]
pub struct SyntaxEvents {
    events: watch::Receiver<Option<SyntaxEvent>>,
}

impl SyntaxEvents {
    pub async fn recv(&mut self) -> Option<SyntaxEvent> {
        loop {
            if self.events.changed().await.is_err() {
                return None;
            }
            if let Some(event) = self.events.borrow_and_update().clone() {
                return Some(event);
            }
        }
    }
}

/// Starts one parser worker. Must be called inside a Tokio runtime.
pub fn spawn_background(registry: Arc<Registry>) -> (SyntaxHandle, SyntaxEvents) {
    let (request_tx, request_rx) = watch::channel(None);
    let (event_tx, event_rx) = watch::channel(None);
    tokio::spawn(run_worker(registry, request_rx, event_tx));
    (
        SyntaxHandle {
            requests: request_tx,
        },
        SyntaxEvents { events: event_rx },
    )
}

async fn run_worker(
    registry: Arc<Registry>,
    mut requests: watch::Receiver<Option<ParseRequest>>,
    events: watch::Sender<Option<SyntaxEvent>>,
) {
    let mut next = None;
    loop {
        let request = match next.take() {
            Some(request) => request,
            None => {
                if requests.changed().await.is_err() {
                    return;
                }
                let Some(request) = requests.borrow_and_update().clone() else {
                    continue;
                };
                request
            }
        };
        let parser_registry = Arc::clone(&registry);
        let mut task = tokio::task::spawn_blocking(move || parse(request, &parser_registry));
        loop {
            tokio::select! {
                changed = requests.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    next = requests.borrow_and_update().clone();
                }
                result = &mut task => {
                    if next.is_none() && let Ok(event) = result {
                        events.send_replace(Some(event));
                    }
                    break;
                }
            }
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
