// SPDX-License-Identifier: MPL-2.0

//! Bounded metadata observations. Source revisions describe snapshots, never an edit log.

use super::application::{Error, ErrorCode, JobState};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const MAX_SUBSCRIPTIONS: usize = 32;
pub const MAX_SOURCES: usize = 256;
pub const MAX_PENDING: usize = 64;
pub const MAX_RELIABLE: usize = 64;
pub const MAX_STATE_BYTES: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    /// A subscription filter only; returned states always identify a concrete buffer.
    Buffers,
    Buffer {
        buffer: String,
    },
    Pane {
        pane: String,
    },
    View {
        view: String,
    },
    Job {
        job: String,
    },
    Attachment,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Snapshot {
    Buffer {
        revision: String,
        saved_revision: Option<String>,
        name: String,
        chars: usize,
        read_only: bool,
        dirty: bool,
    },
    Pane {
        buffer: Option<String>,
        selection_revision: String,
    },
    View {
        revision: String,
    },
    Job {
        state: JobState,
        progress: u8,
    },
    Attachment {
        attached: bool,
        generation: String,
    },
    Closed {},
}

impl Snapshot {
    pub fn reliable_change(&self, previous: &Self) -> bool {
        match (previous, self) {
            (_, Self::Closed {}) => true,
            (
                Self::Buffer {
                    saved_revision: old,
                    ..
                },
                Self::Buffer {
                    saved_revision: new,
                    ..
                },
            ) => old != new,
            (Self::Job { state: old, .. }, Self::Job { state: new, .. }) => old != new,
            (Self::Attachment { .. }, Self::Attachment { .. }) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceState {
    pub source: Source,
    pub revision: String,
    pub state: Snapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub subscription: String,
    pub sequence: String,
    pub sources: Vec<SourceState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub subscription: String,
    pub sources: Vec<SourceState>,
    pub coalesced: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResyncRequired {
    pub subscription: String,
}

#[derive(Clone, Debug)]
pub enum Delivery {
    Changed(Change),
    ReliableChanged(Change),
    Closed(Change),
    ResyncRequired(ResyncRequired),
}

impl Delivery {
    pub fn subscription(&self) -> &str {
        match self {
            Self::Changed(change) | Self::ReliableChanged(change) | Self::Closed(change) => {
                &change.subscription
            }
            Self::ResyncRequired(marker) => &marker.subscription,
        }
    }
}

#[derive(Clone, Default)]
struct Subscription {
    filters: Vec<Source>,
    sources: BTreeSet<Source>,
    pending: BTreeMap<Source, SourceState>,
    coalesced: usize,
    suspended: bool,
}

#[derive(Clone, Default)]
pub struct Registry {
    subscriptions: BTreeMap<String, Subscription>,
    current: BTreeMap<Source, SourceState>,
    reliable: VecDeque<Delivery>,
    next_revision: u64,
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidArgument, message)
}
fn limit(message: &str) -> Error {
    Error::new(ErrorCode::LimitExceeded, message)
}

impl Registry {
    pub fn is_empty(&self) -> bool {
        self.subscriptions.is_empty()
    }
    pub fn len(&self) -> usize {
        self.subscriptions.len()
    }
    pub fn subscriptions(&self) -> Vec<String> {
        self.subscriptions.keys().cloned().collect()
    }
    pub fn watched_sources(&self) -> Vec<Source> {
        self.current.keys().cloned().collect()
    }
    pub fn filters(&self, id: &str) -> Option<Vec<Source>> {
        self.subscriptions.get(id).map(|sub| sub.filters.clone())
    }
    pub fn sources(&self, id: &str) -> Option<Vec<Source>> {
        self.subscriptions
            .get(id)
            .map(|sub| sub.sources.iter().cloned().collect())
    }
    pub fn is_suspended(&self, id: &str) -> bool {
        self.subscriptions.get(id).is_some_and(|sub| sub.suspended)
    }

    fn pairs(&self) -> usize {
        self.subscriptions
            .values()
            .map(|sub| sub.sources.len())
            .sum()
    }

    fn validate_source(source: &Source) -> Result<(), Error> {
        let handle = match source {
            Source::Buffer { buffer } => buffer,
            Source::Pane { pane } => pane,
            Source::View { view } => view,
            Source::Job { job } => job,
            Source::Buffers | Source::Attachment => return Ok(()),
        };
        if handle.is_empty() || handle.len() > 128 || handle.chars().any(char::is_control) {
            return Err(invalid("Invalid observation source handle"));
        }
        Ok(())
    }

    fn validate_state(source: &Source, snapshot: &Snapshot) -> Result<(), Error> {
        Self::validate_source(source)?;
        let matches = matches!(
            (source, snapshot),
            (Source::Buffer { .. }, Snapshot::Buffer { .. })
                | (Source::Pane { .. }, Snapshot::Pane { .. })
                | (Source::View { .. }, Snapshot::View { .. })
                | (Source::Job { .. }, Snapshot::Job { .. })
                | (Source::Attachment, Snapshot::Attachment { .. })
        ) || (!matches!(source, Source::Buffers)
            && matches!(snapshot, Snapshot::Closed {}));
        if !matches {
            return Err(invalid("Observation snapshot does not match its source"));
        }
        let text_bytes = match snapshot {
            Snapshot::Buffer {
                revision,
                saved_revision,
                name,
                ..
            } => revision.len() + saved_revision.as_ref().map_or(0, String::len) + name.len(),
            Snapshot::Pane {
                buffer,
                selection_revision,
            } => buffer.as_ref().map_or(0, String::len) + selection_revision.len(),
            Snapshot::View { revision } => revision.len(),
            Snapshot::Attachment { generation, .. } => generation.len(),
            Snapshot::Job { .. } | Snapshot::Closed {} => 0,
        };
        if text_bytes > MAX_STATE_BYTES {
            return Err(limit("Observation source metadata is too large"));
        }
        // Include source/revision/envelope overhead. Combined with the pair limit this
        // bounds current, pending, and reliable metadata below the 4 MiB state budget.
        let bytes = serde_json::to_vec(&(source, snapshot))
            .map_err(|_| invalid("Invalid observation metadata"))?
            .len();
        if bytes + 128 > MAX_STATE_BYTES {
            return Err(limit("Observation source metadata is too large"));
        }
        Ok(())
    }

    fn validate_baseline(filters: &[Source], baseline: &[(Source, Snapshot)]) -> Result<(), Error> {
        if filters.is_empty() || filters.len() > MAX_SOURCES || baseline.len() > MAX_SOURCES {
            return Err(limit("Observation source limit exceeded"));
        }
        let unique: BTreeSet<_> = filters.iter().collect();
        if unique.len() != filters.len() {
            return Err(invalid("Duplicate observation filter"));
        }
        for source in filters {
            Self::validate_source(source)?;
        }
        let mut seen = BTreeSet::new();
        for (source, snapshot) in baseline {
            Self::validate_state(source, snapshot)?;
            if !seen.insert(source) {
                return Err(invalid("Duplicate observation source"));
            }
            if !unique.contains(source)
                && !(unique.contains(&Source::Buffers) && matches!(source, Source::Buffer { .. }))
            {
                return Err(invalid("Observation source is outside its filters"));
            }
        }
        if filters
            .iter()
            .any(|source| !matches!(source, Source::Buffers) && !seen.contains(source))
        {
            return Err(invalid("Observation baseline is incomplete"));
        }
        let bytes = serde_json::to_vec(baseline)
            .map_err(|_| invalid("Invalid observation baseline"))?
            .len();
        if bytes + baseline.len() * 128 > super::MAX_BYTES - 4096 {
            return Err(limit("Observation baseline exceeds the wire limit"));
        }
        Ok(())
    }

    pub fn subscribe(
        &mut self,
        id: String,
        filters: Vec<Source>,
        baseline: Vec<(Source, Snapshot)>,
    ) -> Result<Vec<SourceState>, Error> {
        if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
            return Err(invalid("Invalid subscription handle"));
        }
        if self.subscriptions.contains_key(&id) {
            return Err(invalid("Subscription already exists"));
        }
        if self.len() == MAX_SUBSCRIPTIONS || self.pairs() + baseline.len() > MAX_SOURCES {
            return Err(limit("Observation subscription or source limit exceeded"));
        }
        Self::validate_baseline(&filters, &baseline)?;
        // Admission is atomic, including reliable lifecycle work for existing watchers.
        let mut next = self.clone();
        for (source, snapshot) in &baseline {
            next.observe(source, snapshot.clone())?;
        }
        let sources = baseline.into_iter().map(|(source, _)| source).collect();
        next.subscriptions.insert(
            id.clone(),
            Subscription {
                filters,
                sources,
                ..Default::default()
            },
        );
        let states = next.baseline(&id);
        *self = next;
        Ok(states)
    }

    fn baseline(&self, id: &str) -> Vec<SourceState> {
        self.subscriptions[id]
            .sources
            .iter()
            .filter_map(|source| self.current.get(source).cloned())
            .collect()
    }

    pub fn resync(
        &mut self,
        id: &str,
        baseline: Vec<(Source, Snapshot)>,
    ) -> Result<Vec<SourceState>, Error> {
        let sub = self
            .subscriptions
            .get(id)
            .ok_or_else(|| Error::new(ErrorCode::NotFound, "Unknown subscription"))?;
        Self::validate_baseline(&sub.filters, &baseline)?;
        if self.pairs() - sub.sources.len() + baseline.len() > MAX_SOURCES {
            return Err(limit("Observation source limit exceeded"));
        }
        let mut next = self.clone();
        for (source, snapshot) in &baseline {
            next.observe(source, snapshot.clone())?;
        }
        let sub = next.subscriptions.get_mut(id).unwrap();
        sub.sources = baseline.into_iter().map(|(source, _)| source).collect();
        sub.pending.clear();
        sub.coalesced = 0;
        sub.suspended = false;
        // Baseline supersedes an undelivered marker; reliable lifecycle is retained.
        next.reliable.retain(|delivery| !matches!(delivery, Delivery::ResyncRequired(marker) if marker.subscription == id));
        next.prune();
        let states = next.baseline(id);
        *self = next;
        Ok(states)
    }

    pub fn observe(&mut self, source: &Source, snapshot: Snapshot) -> Result<(), Error> {
        Self::validate_state(source, &snapshot)?;
        let previous = self.current.get(source);
        if previous.is_some_and(|state| {
            state.state == snapshot || matches!(state.state, Snapshot::Closed {})
        }) {
            return Ok(());
        }
        if previous.is_none() && self.current.len() == MAX_SOURCES {
            return Err(limit("Observation source limit exceeded"));
        }
        let reliable = previous.is_some_and(|state| snapshot.reliable_change(&state.state));
        self.next_revision = self
            .next_revision
            .checked_add(1)
            .ok_or_else(|| Error::new(ErrorCode::Internal, "Observation revision exhausted"))?;
        let state = SourceState {
            source: source.clone(),
            revision: format!("o:{}", self.next_revision),
            state: snapshot,
        };
        self.current.insert(source.clone(), state.clone());
        let ids: Vec<_> = self
            .subscriptions
            .iter()
            .filter(|(_, sub)| sub.sources.contains(source))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if reliable {
                let sub = self.subscriptions.get_mut(&id).unwrap();
                let coalesced = std::mem::take(&mut sub.coalesced)
                    .saturating_add(usize::from(sub.pending.remove(source).is_some()));
                if matches!(state.state, Snapshot::Closed {}) && !sub.filters.contains(source) {
                    // The queued tombstone owns its snapshot; wildcard discovery no
                    // longer needs to retain this closed identity against the cap.
                    sub.sources.remove(source);
                }
                let change = Change {
                    subscription: id,
                    sources: vec![state.clone()],
                    coalesced,
                };
                let delivery = if matches!(state.state, Snapshot::Closed {}) {
                    Delivery::Closed(change)
                } else {
                    Delivery::ReliableChanged(change)
                };
                self.enqueue(delivery)?;
            } else {
                self.coalesce(&id, state.clone())?;
            }
        }
        if matches!(state.state, Snapshot::Closed {}) {
            self.prune();
        }
        Ok(())
    }

    pub fn discover(&mut self, id: &str, source: Source, snapshot: Snapshot) -> Result<(), Error> {
        Self::validate_state(&source, &snapshot)?;
        let sub = self
            .subscriptions
            .get(id)
            .ok_or_else(|| Error::new(ErrorCode::NotFound, "Unknown subscription"))?;
        if sub.sources.contains(&source) {
            return self.observe(&source, snapshot);
        }
        if !sub.filters.contains(&Source::Buffers) || !matches!(source, Source::Buffer { .. }) {
            return Err(invalid("Source does not match the discovery filter"));
        }
        if sub.suspended {
            return Ok(());
        }
        if self.pairs() == MAX_SOURCES {
            return self.invalidate(id);
        }
        self.observe(&source, snapshot)?;
        self.subscriptions
            .get_mut(id)
            .unwrap()
            .sources
            .insert(source.clone());
        // Opening a discovered source is a reliable lifecycle observation.
        self.enqueue(Delivery::ReliableChanged(Change {
            subscription: id.to_owned(),
            sources: vec![self.current[&source].clone()],
            coalesced: 0,
        }))
    }

    fn coalesce(&mut self, id: &str, state: SourceState) -> Result<(), Error> {
        let sub = self.subscriptions.get_mut(id).unwrap();
        if sub.suspended {
            return Ok(());
        }
        if sub.pending.contains_key(&state.source) {
            sub.coalesced = sub.coalesced.saturating_add(1);
        } else if sub.pending.len() == MAX_PENDING {
            return self.invalidate(id);
        }
        sub.pending.insert(state.source.clone(), state);
        Ok(())
    }

    fn enqueue(&mut self, delivery: Delivery) -> Result<(), Error> {
        if self.reliable.len() == MAX_RELIABLE {
            return Err(limit("Reliable observation queue is full"));
        }
        self.reliable.push_back(delivery);
        Ok(())
    }

    pub fn invalidate(&mut self, id: &str) -> Result<(), Error> {
        let sub = self
            .subscriptions
            .get_mut(id)
            .ok_or_else(|| Error::new(ErrorCode::NotFound, "Unknown subscription"))?;
        if sub.suspended {
            return Ok(());
        }
        sub.pending.clear();
        sub.coalesced = 0;
        sub.suspended = true;
        self.enqueue(Delivery::ResyncRequired(ResyncRequired {
            subscription: id.to_owned(),
        }))
    }

    pub fn peek_ready(&self) -> Option<Delivery> {
        self.reliable.front().cloned().or_else(|| {
            self.subscriptions
                .iter()
                .find(|(_, sub)| !sub.pending.is_empty())
                .map(|(id, sub)| {
                    Delivery::Changed(Change {
                        subscription: id.clone(),
                        sources: sub.pending.values().cloned().collect(),
                        coalesced: sub.coalesced,
                    })
                })
        })
    }

    /// Called immediately after admission, in the same host turn as peek_ready.
    pub fn ack_ready(&mut self) {
        if self.reliable.pop_front().is_some() {
            return;
        }
        if let Some(sub) = self
            .subscriptions
            .values_mut()
            .find(|sub| !sub.pending.is_empty())
        {
            sub.pending.clear();
            sub.coalesced = 0;
        }
    }

    pub fn unsubscribe(&mut self, id: &str) -> Vec<Delivery> {
        let mut deliveries = Vec::new();
        self.reliable.retain(|delivery| {
            if delivery.subscription() == id {
                deliveries.push(delivery.clone());
                false
            } else {
                true
            }
        });
        if let Some(sub) = self.subscriptions.remove(id)
            && !sub.pending.is_empty()
        {
            deliveries.push(Delivery::Changed(Change {
                subscription: id.to_owned(),
                sources: sub.pending.into_values().collect(),
                coalesced: sub.coalesced,
            }));
        }
        self.prune();
        deliveries
    }

    fn prune(&mut self) {
        self.current.retain(|source, _| {
            self.subscriptions
                .values()
                .any(|sub| sub.sources.contains(source))
        });
    }
}

#[cfg(test)]
#[path = "tests/observation.rs"]
mod tests;
