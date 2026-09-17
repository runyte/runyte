// SPDX-License-Identifier: MPL-2.0

use super::{WorkspaceHost, context_reads::ReadState};
use crate::{
    app::context_access::{Decision, Kind, Surface, visible},
    plugin::application::{Error, ErrorCode as Code},
    terminal::{
        TerminalId,
        proposal::{Delivery, InputSignature, Text},
    },
    workspace::context::{
        storage::{self, HostMode, Identity, Registration, Storage},
        transport::{Event, Lease, Reply, Server},
        wire::{self, Request, Scope},
    },
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

type FrameSource = (usize, u64, Option<(TerminalId, u64)>);

#[derive(Default)]
pub(super) struct State {
    events: Option<mpsc::Sender<Event>>,
    storage: Option<Storage>,
    registration: Option<Registration>,
    server: Option<Server>,
    grants: BTreeMap<String, (Identity, BTreeSet<Scope>)>,
    readers: BTreeMap<u64, Reader>,
    proposals: BTreeMap<String, Proposal>,
    recent: VecDeque<String>,
    mode: Option<HostMode>,
    pub frame: Option<crate::snapshot::EditorSnapshot>,
    pub frame_id: u64,
    pub frame_review_sources: BTreeMap<usize, u64>,
    pub frame_sources: BTreeMap<usize, FrameSource>,
    pub frame_foreground: u64,
    pub frame_attachment: u64,
}

struct Reader {
    identity: String,
    granted: BTreeSet<Scope>,
    scopes: BTreeSet<Scope>,
    registered: bool,
    lease: Arc<Lease>,
    reads: ReadState,
    requests: BTreeSet<String>,
}

impl State {
    pub(super) fn retained_reader_bytes(&self) -> usize {
        self.readers
            .values()
            .map(|reader| reader.reads.retained_bytes())
            .sum()
    }
}

struct Proposal {
    owner: u64,
    terminal: TerminalId,
    text: Text,
    reason: Option<String>,
    signature: InputSignature,
    attachment: u64,
    deadline: Instant,
    status: &'static str,
    delivery: Option<Delivery>,
}

impl Drop for State {
    fn drop(&mut self) {
        for reader in self.readers.values() {
            reader.lease.cancel();
        }
        for proposal in self.proposals.values() {
            if let Some(delivery) = &proposal.delivery {
                delivery.cancel();
            }
        }
        self.server.take();
        if let (Some(storage), Some(registration)) = (&self.storage, &self.registration) {
            let _ = storage.unregister(&registration.host_incarnation);
        }
    }
}

impl WorkspaceHost {
    pub(super) fn context_enabled(&self) -> bool {
        self.context.server.is_some()
    }
    pub fn start_context(&mut self, mode: HostMode) -> mpsc::Receiver<Event> {
        let (send, receive) = mpsc::channel(32);
        self.context.events = Some(send);
        self.context.mode = Some(mode);
        if let Some(root) = Storage::default_root().filter(|root| root.exists()) {
            let loaded = (|| -> anyhow::Result<()> {
                let store = Storage::open_existing(root.clone())?;
                for identity in store.identities()? {
                    let scopes = store.scopes(&self.app.project_root, &identity)?;
                    if !scopes.is_empty() {
                        self.context
                            .grants
                            .insert(identity.fingerprint(), (identity, scopes));
                    }
                }
                if !self.context.grants.is_empty() {
                    self.context_open_storage(root)?;
                    self.context_enable()?;
                }
                Ok(())
            })();
            if loaded.is_err() {
                self.report_host_error("Remembered agent context access could not be loaded");
            }
        }
        receive
    }

    fn context_open_storage(&mut self, root: std::path::PathBuf) -> anyhow::Result<()> {
        let ancestor = root
            .ancestors()
            .find(|path| path.exists())
            .ok_or_else(|| anyhow::anyhow!("Context storage has no existing ancestor"))?;
        let resolved = ancestor.canonicalize()?.join(root.strip_prefix(ancestor)?);
        anyhow::ensure!(
            !resolved.starts_with(&self.app.project_root)
                && !root
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir)),
            "Context storage must be outside the workspace"
        );
        self.context.storage = Some(Storage::new(root)?);
        Ok(())
    }

    fn context_enable(&mut self) -> anyhow::Result<()> {
        if self.context.server.is_some() || self.context.grants.is_empty() {
            return Ok(());
        }
        let store = self
            .context
            .storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Private context storage unavailable"))?;
        let incarnation = storage::random_token()?;
        let endpoint = store.socket_path(&incarnation)?;
        let registration = Registration {
            root: self.app.project_root.clone(),
            workspace_id: crate::workspace::identity::workspace_id(self.identity.root()),
            host_incarnation: incarnation,
            endpoint: endpoint.clone(),
            mode: self.context.mode.unwrap_or(HostMode::Standalone),
            pid: std::process::id(),
            environment: storage::environment_fingerprint(),
        };
        let events = self
            .context
            .events
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Context service is not running"))?;
        let server = Server::bind(endpoint, events)?;
        store.register(&registration)?;
        self.context.registration = Some(registration);
        self.context.server = Some(server);
        Ok(())
    }

    pub fn sync_context(&mut self) {
        if !self.app.plugins.frontend_attached {
            self.context.frame = None;
            self.app.context_ui.surface = None;
        }
        for reader in self.context.readers.values_mut() {
            reader.reads.expire();
        }
        self.app.terminals.set_external_retained_bytes(
            self.context
                .readers
                .values()
                .map(|reader| reader.reads.retained_bytes())
                .sum(),
        );
        if let Some(identity) = self.app.context_ui.requested_identity.take()
            && self.app.plugins.frontend_attached
            && !self.app.has_input_overlay()
        {
            let scopes = self
                .context
                .grants
                .values()
                .find(|(id, _)| id.name() == identity)
                .map(|(_, scopes)| scopes.clone())
                .unwrap_or_else(|| [Scope::TerminalRead, Scope::EditorContextRead].into());
            let mut explanation = vec![
                format!("Identity: {}", visible(&identity)),
                format!(
                    "Workspace: {}",
                    visible(&self.app.project_root.to_string_lossy())
                ),
                "Read grants include unsaved text and sensitive terminal output.".into(),
                "Terminal proposals always require a separate approval.".into(),
            ];
            explanation.extend(self.context.readers.values().map(|reader| {
                format!(
                    "Active reader: {}; scopes: {:?}",
                    visible(&reader.identity),
                    reader.scopes
                )
            }));
            explanation.extend(self.context.recent.iter().rev().take(4).cloned());
            self.app.plugins.presentation_dirty = true;
            self.app.context_ui.surface = Some(Surface::new(
                Kind::Grant {
                    identity,
                    scopes,
                    remember: false,
                },
                "Agent context access".into(),
                explanation,
                "",
                self.app.plugins.attachment_generation,
            ));
        }
        if let Some(decision) = self.app.context_ui.decision.take() {
            self.app.plugins.presentation_dirty = true;
            let result = self.context_decision(decision);
            if let Err(error) = result {
                self.report_host_error(error.to_string());
            }
        }
        let now = Instant::now();
        self.context.proposals.retain(|_, proposal| {
            now < proposal.deadline + Duration::from_secs(120)
                || matches!(proposal.status, "queued" | "writing")
        });
        for proposal in self.context.proposals.values_mut() {
            if now >= proposal.deadline
                && proposal.delivery.as_ref().is_some_and(|delivery| {
                    matches!(
                        delivery.state(),
                        crate::terminal::proposal::DeliveryState::Queued
                            | crate::terminal::proposal::DeliveryState::Writing
                    )
                })
            {
                let delivery = proposal.delivery.take().unwrap();
                proposal.status = if delivery.cancel() {
                    "expired"
                } else {
                    "outcome_unknown"
                };
            }
            if let Some(delivery) = &proposal.delivery {
                if !self.app.plugins.frontend_attached
                    || proposal.attachment != self.app.plugins.attachment_generation
                    || self
                        .app
                        .terminals
                        .get(proposal.terminal)
                        .is_none_or(|t| !t.live())
                {
                    delivery.cancel();
                }
                use crate::terminal::proposal::DeliveryState as D;
                proposal.status = match delivery.state() {
                    D::Queued => "queued",
                    D::Writing => "writing",
                    D::Delivered => "delivered",
                    D::Cancelled => "cancelled",
                    D::OutcomeUnknown => "outcome_unknown",
                };
            }
            if proposal.status == "pending"
                && (now >= proposal.deadline
                    || !self.app.plugins.frontend_attached
                    || proposal.attachment != self.app.plugins.attachment_generation
                    || self
                        .app
                        .terminals
                        .get(proposal.terminal)
                        .is_none_or(|t| !t.live()))
            {
                proposal.status = if now >= proposal.deadline {
                    "expired"
                } else {
                    "cancelled"
                };
            }
        }
        if self
            .app
            .context_ui
            .surface
            .as_ref()
            .and_then(Surface::proposal)
            .is_some_and(|id| {
                self.context
                    .proposals
                    .get(id)
                    .is_none_or(|p| p.status != "pending")
            })
        {
            self.app.context_ui.surface = None;
            self.app.plugins.presentation_dirty = true;
        }
        if self.app.plugins.frontend_attached
            && !self.app.has_input_overlay()
            && self.app.mode != crate::command::Mode::Command
            && let Some((id, p)) = self
                .context
                .proposals
                .iter_mut()
                .find(|(_, p)| p.status == "pending")
            && let Some(terminal) = self.app.terminals.get(p.terminal)
        {
            // Capture at presentation, so a proposal waiting behind an
            // existing prompt cannot approve unseen intervening input.
            p.signature = terminal.proposal_input_signature();
            let identity = self
                .context
                .readers
                .get(&p.owner)
                .map_or("disconnected", |r| r.identity.as_str());
            let mut explanation = vec![
                format!("Agent: {}", visible(identity)),
                format!(
                    "Workspace: {}",
                    visible(&self.app.project_root.to_string_lossy())
                ),
                format!(
                    "Terminal: {} (#{}). Inserts at current input position.",
                    visible(&terminal.name()),
                    p.terminal
                ),
                "Insertion sends no Enter. Printable input can still trigger actions.".into(),
            ];
            if let Some(reason) = &p.reason {
                explanation.push(format!("Untrusted reason: {}", visible(reason)));
            }
            if let Ok(context) = terminal.read_output(
                crate::terminal::read::Region::Tail,
                crate::terminal::read::Limits {
                    rows: 3,
                    bytes: 512,
                    cells: 2048,
                },
                None,
            ) {
                explanation.push("Recent output (not an input-line guarantee):".into());
                for row in context.rows {
                    explanation.push(visible(&row.text));
                }
            }
            self.app.plugins.presentation_dirty = true;
            self.app.context_ui.surface = Some(Surface::new(
                Kind::Proposal { id: id.clone() },
                "Review terminal text".into(),
                explanation,
                p.text.as_str(),
                p.attachment,
            ));
        }
    }

    fn context_decision(&mut self, decision: Decision) -> anyhow::Result<()> {
        match decision {
            Decision::Grant {
                identity,
                scopes,
                remember,
            } => {
                wire::validate_scopes(&scopes).map_err(|e| anyhow::anyhow!(e.message))?;
                if self.context.storage.is_none() {
                    self.context_open_storage(
                        Storage::default_root().ok_or_else(|| {
                            anyhow::anyhow!("Private context storage unavailable")
                        })?,
                    )?;
                }
                let store = self.context.storage.as_ref().unwrap();
                let identity = store.identity(&identity)?;
                if remember {
                    store.grant(&self.app.project_root, &identity, scopes.clone())?;
                } else {
                    store.revoke(&self.app.project_root, &identity)?;
                }
                self.context_revoke(identity.name());
                if !scopes.is_empty() {
                    self.context
                        .grants
                        .insert(identity.fingerprint(), (identity, scopes));
                }
                self.context_enable()?;
            }
            Decision::Revoke(name) => {
                self.context_revoke(&name);
                if let Some(store) = &self.context.storage
                    && let Some(identity) = store.load_identity(&name)?
                {
                    store.revoke(&self.app.project_root, &identity)?;
                }
            }
            Decision::Proposal { id, accepted } => {
                let Some(proposal) = self.context.proposals.get_mut(&id) else {
                    return Ok(());
                };
                if proposal.status != "pending" {
                    return Ok(());
                }
                if !accepted {
                    proposal.status = "rejected";
                    return Ok(());
                }
                let reader_valid = self.context.readers.get(&proposal.owner).is_some_and(|r| {
                    r.lease.active() && r.scopes.contains(&Scope::TerminalPropose)
                });
                let terminal = self.app.terminals.get_mut(proposal.terminal);
                if !reader_valid
                    || Instant::now() >= proposal.deadline
                    || proposal.attachment != self.app.plugins.attachment_generation
                    || !self.app.plugins.frontend_attached
                    || terminal.as_ref().is_none_or(|t| {
                        !t.live() || t.proposal_input_signature() != proposal.signature
                    })
                {
                    proposal.status = "stale";
                    return Ok(());
                }
                let text = Text::new(proposal.text.as_str())
                    .map_err(|_| anyhow::anyhow!("Invalid proposal text"))?;
                match terminal.unwrap().enqueue_proposal(&text) {
                    Ok(delivery) => {
                        proposal.delivery = Some(delivery);
                        proposal.status = "queued";
                    }
                    Err(_) => proposal.status = "cancelled",
                }
            }
        }
        Ok(())
    }

    fn context_revoke(&mut self, name: &str) {
        let ids = self
            .context
            .readers
            .iter()
            .filter(|(_, r)| r.identity == name)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in ids {
            self.context_disconnect(id);
        }
        self.context
            .grants
            .retain(|_, (identity, _)| identity.name() != name);
        if self.context.grants.is_empty() {
            self.context.frame = None;
            self.context.frame_sources.clear();
            self.context.frame_review_sources.clear();
            self.context.server.take();
            if let (Some(store), Some(registration)) =
                (&self.context.storage, self.context.registration.take())
            {
                let _ = store.unregister(&registration.host_incarnation);
            }
        }
    }

    fn context_disconnect(&mut self, id: u64) {
        if let Some(reader) = self.context.readers.remove(&id) {
            reader.lease.cancel();
        }
        self.context.proposals.retain(|_, p| {
            if p.owner == id {
                if let Some(d) = &p.delivery {
                    d.cancel();
                }
                false
            } else {
                true
            }
        });
        self.app.terminals.set_external_retained_bytes(
            self.context
                .readers
                .values()
                .map(|reader| reader.reads.retained_bytes())
                .sum(),
        );
    }

    pub fn handle_context_event(&mut self, event: Event) {
        match event {
            Event::Closed(id) => self.context_disconnect(id),
            Event::Frame {
                connection,
                bytes,
                reply,
            } => {
                if reply.is_closed() {
                    self.context_disconnect(connection);
                    return;
                }
                self.sync_context();
                let response = self.context_frame(connection, &bytes);
                let _ = reply.send(response);
            }
        }
    }

    fn context_frame(&mut self, id: u64, bytes: &[u8]) -> Reply {
        let Ok(raw) = std::str::from_utf8(bytes) else {
            return failure(Error::new(Code::InvalidArgument, "Invalid UTF-8"));
        };
        if raw.trim() == "{\"type\":\"probe\"}" && !self.context.readers.contains_key(&id) {
            return Reply {
                value: json!({"type":"context_endpoint","registration":self.context.registration}),
                lease: None,
                close: true,
            };
        }
        if !self.context.readers.contains_key(&id) {
            let auth = match wire::parse_authentication(raw) {
                Ok(auth) => auth,
                Err(error) => return failure(error),
            };
            let fingerprint = crate::hash::sha256_hex(auth.credential.as_bytes());
            let Some((identity, scopes)) = self.context.grants.get(&fingerprint) else {
                return failure(Error::new(
                    Code::CapabilityDenied,
                    "Context access is not granted",
                ));
            };
            if self.context.readers.len() >= wire::MAX_READERS {
                return failure(Error::new(Code::Busy, "Context reader limit reached"));
            }
            let lease = Arc::new(Lease::default());
            let prefix = match storage::random_token() {
                Ok(token) => token,
                Err(_) => {
                    return failure(Error::new(
                        Code::Unavailable,
                        "Cannot allocate reader identity",
                    ));
                }
            };
            self.context.readers.insert(
                id,
                Reader {
                    identity: identity.name().to_owned(),
                    granted: scopes.clone(),
                    scopes: BTreeSet::new(),
                    registered: false,
                    lease: lease.clone(),
                    reads: ReadState::new(prefix),
                    requests: BTreeSet::new(),
                },
            );
            return Reply {
                value: json!({"type":"hello","version":"runyte-1","host_version":crate::plugin::compatibility::HOST_VERSION,"features":[wire::FEATURE],"capabilities":wire::CAPABILITIES,"limits":wire::limits(),"workspace":self.context.registration}),
                lease: Some(lease),
                close: false,
            };
        }
        let mut reader = self.context.readers.remove(&id).unwrap();
        if !reader.registered {
            let result = (|| -> Result<Value, Error> {
                let registration = wire::parse_registration(raw)?;
                let release = crate::plugin::compatibility::Version::parse(
                    crate::plugin::compatibility::HOST_VERSION,
                )
                .map_err(|_| Error::new(Code::Internal, "Invalid host release"))?;
                if !crate::plugin::compatibility::ReleaseRange::parse(&registration.runyte)
                    .map_err(|_| Error::new(Code::InvalidArgument, "Invalid release range"))?
                    .contains(&release)
                {
                    return Err(Error::new(
                        Code::Unsupported,
                        "Host release is outside requested range",
                    ));
                }
                let required = registration
                    .required_capabilities
                    .iter()
                    .filter_map(|c| Scope::from_capability(c))
                    .collect::<BTreeSet<_>>();
                if !required.is_subset(&reader.granted) {
                    return Err(Error::new(
                        Code::CapabilityDenied,
                        "Required context scopes not granted",
                    ));
                }
                reader.scopes = registration
                    .required_capabilities
                    .union(&registration.optional_capabilities)
                    .filter_map(|c| Scope::from_capability(c))
                    .filter(|s| reader.granted.contains(s))
                    .collect();
                wire::validate_scopes(&reader.scopes)?;
                reader.registered = true;
                Ok(
                    json!({"type":"registered","runyte":registration.runyte,"features":[wire::FEATURE],"commands":[],"capabilities":reader.scopes,"limits":wire::limits()}),
                )
            })();
            match result {
                Ok(value) => {
                    let lease = reader.lease.clone();
                    self.context.readers.insert(id, reader);
                    return Reply {
                        value,
                        lease: Some(lease),
                        close: false,
                    };
                }
                Err(error) => {
                    // Only this public, close-only error remains. The reader
                    // has already been removed; cancel its owned resources
                    // without suppressing the final negotiation error.
                    self.context_disconnect(id);
                    return failure(error);
                }
            }
        }
        let envelope = match wire::parse_request(raw) {
            Ok(request) => request,
            Err(error) => {
                self.context_disconnect(id);
                return failure(error);
            }
        };
        let request_id = envelope.id.clone();
        let (method, target) = match &envelope.request {
            Request::BufferList { .. } => ("buffer.list", "inventory"),
            Request::BufferRead { buffer, .. } => ("buffer.read", buffer.as_str()),
            Request::BufferEdit { buffer, .. } => ("buffer.edit", buffer.as_str()),
            Request::SnapshotOpen { buffer, .. } => ("buffer.snapshot.open", buffer.as_str()),
            Request::SnapshotRead { snapshot, .. } => ("buffer.snapshot.read", snapshot.as_str()),
            Request::SnapshotClose { snapshot } => ("buffer.snapshot.close", snapshot.as_str()),
            Request::SelectionGet { pane } => ("selection.get", pane.as_str()),
            Request::PaneContextList(..) => ("pane.context.list", "inventory"),
            Request::PaneViewportRead { pane, .. } => ("pane.viewport.read", pane.as_str()),
            Request::TerminalList { .. } => ("terminal.list", "inventory"),
            Request::TerminalRead { terminal, .. } => ("terminal.read", terminal.as_str()),
            Request::TerminalSnapshotOpen { terminal, .. } => {
                ("terminal.snapshot.open", terminal.as_str())
            }
            Request::TerminalSnapshotRead { snapshot, .. } => {
                ("terminal.snapshot.read", snapshot.as_str())
            }
            Request::TerminalSnapshotClose { snapshot } => {
                ("terminal.snapshot.close", snapshot.as_str())
            }
            Request::TerminalInputPropose { terminal, .. } => {
                ("terminal.input.propose", terminal.as_str())
            }
            Request::TerminalInputStatus { proposal } => {
                ("terminal.input.status", proposal.as_str())
            }
            Request::TerminalInputCancel { proposal } => {
                ("terminal.input.cancel", proposal.as_str())
            }
        };
        let inspection = format!(
            "{}: {} {} at {:?}",
            visible(&reader.identity),
            method,
            visible(target),
            std::time::SystemTime::now()
        );
        let result = if !reader.scopes.contains(&envelope.request.required_scope()) {
            Err(Error::new(
                Code::CapabilityDenied,
                "Context scope not granted",
            ))
        } else if reader.requests.len() >= wire::MAX_DEDUPLICATED_REQUESTS {
            Err(Error::new(
                Code::LimitExceeded,
                "Reader request lifetime exhausted; reconnect",
            ))
        } else if !reader.requests.insert(request_id.clone()) {
            Err(Error::new(
                Code::Conflict,
                "Request already used; do not replay uncertain mutations",
            ))
        } else {
            match envelope.request {
                Request::TerminalInputPropose {
                    terminal,
                    text,
                    reason,
                } => {
                    let terminal = reader.reads.terminal(&terminal);
                    terminal.and_then(|terminal| self.context_propose(id, terminal, &text, reason))
                }
                Request::TerminalInputStatus { proposal } => self
                    .context
                    .proposals
                    .get(&proposal)
                    .filter(|p| p.owner == id)
                    .map(|p| json!({"proposal":proposal,"state":p.status}))
                    .ok_or_else(|| Error::new(Code::NotFound, "Unknown proposal")),
                Request::TerminalInputCancel { proposal } => match self
                    .context
                    .proposals
                    .get_mut(&proposal)
                    .filter(|p| p.owner == id)
                {
                    Some(p) => {
                        if let Some(d) = &p.delivery {
                            d.cancel();
                        }
                        if p.status == "pending" {
                            p.status = "cancelled";
                        }
                        Ok(json!({}))
                    }
                    None => Err(Error::new(Code::NotFound, "Unknown proposal")),
                },
                request => self.context_read_request(&mut reader.reads, &reader.scopes, request),
            }
        };
        if result.is_ok() {
            self.context.recent.push_back(inspection);
            while self.context.recent.len() > 32 {
                self.context.recent.pop_front();
            }
        }
        let lease = reader.lease.clone();
        self.context.readers.insert(id, reader);
        self.app.terminals.set_external_retained_bytes(
            self.context
                .readers
                .values()
                .map(|reader| reader.reads.retained_bytes())
                .sum(),
        );
        let value = match result {
            Ok(mut result) => {
                if let Some(object) = result.as_object_mut() {
                    object.insert("workspace".into(), json!(self.context.registration));
                }
                json!({"type":"response","id":request_id,"result":result})
            }
            Err(error) => json!({"type":"response","id":request_id,"error":error}),
        };
        Reply {
            value,
            lease: Some(lease),
            close: false,
        }
    }

    fn context_propose(
        &mut self,
        owner: u64,
        terminal: TerminalId,
        text: &str,
        reason: Option<String>,
    ) -> Result<Value, Error> {
        if !self.app.plugins.frontend_attached {
            return Err(Error::new(
                Code::NoFrontend,
                "Attach to the target workspace to review terminal text",
            ));
        }
        if self
            .context
            .proposals
            .values()
            .filter(|p| matches!(p.status, "pending" | "queued" | "writing"))
            .count()
            >= wire::MAX_HOST_PROPOSALS
            || self
                .context
                .proposals
                .values()
                .filter(|p| {
                    p.owner == owner && matches!(p.status, "pending" | "queued" | "writing")
                })
                .count()
                >= wire::MAX_PROPOSALS
        {
            return Err(Error::new(Code::LimitExceeded, "Proposal limit reached"));
        }
        while self.context.proposals.len() >= 64 {
            let oldest = self
                .context
                .proposals
                .iter()
                .filter(|(_, p)| !matches!(p.status, "pending" | "queued" | "writing"))
                .min_by_key(|(_, p)| p.deadline)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.context.proposals.remove(&id);
            } else {
                break;
            }
        }
        let session = self
            .app
            .terminals
            .get(terminal)
            .filter(|t| t.live())
            .ok_or_else(|| Error::new(Code::Closed, "Terminal is not live"))?;
        let text = Text::new(text)
            .map_err(|_| Error::new(Code::InvalidArgument, "Invalid terminal text"))?;
        let id = storage::random_token()
            .map_err(|_| Error::new(Code::Unavailable, "Cannot allocate proposal"))?;
        self.context.proposals.insert(
            id.clone(),
            Proposal {
                owner,
                terminal,
                text,
                reason,
                signature: session.proposal_input_signature(),
                attachment: self.app.plugins.attachment_generation,
                deadline: Instant::now() + Duration::from_secs(wire::PROPOSAL_EXPIRY_SECONDS),
                status: "pending",
                delivery: None,
            },
        );
        Ok(json!({"proposal":id,"state":"pending"}))
    }

    pub fn context_delay(&self) -> Option<Duration> {
        if self
            .context
            .proposals
            .values()
            .any(|p| matches!(p.status, "queued" | "writing"))
        {
            Some(Duration::from_millis(100))
        } else if let Some(deadline) = self
            .context
            .proposals
            .values()
            .map(|p| {
                if p.status == "pending" {
                    p.deadline
                } else {
                    p.deadline + Duration::from_secs(120)
                }
            })
            .min()
        {
            Some(deadline.saturating_duration_since(Instant::now()))
        } else if self
            .context
            .readers
            .values()
            .any(|r| r.reads.retained_bytes() > 0)
        {
            Some(Duration::from_secs(30))
        } else {
            None
        }
    }
}

fn failure(error: Error) -> Reply {
    Reply {
        value: json!({"type":"registration_error","code":error.code,"message":error.message}),
        lease: None,
        close: true,
    }
}

#[cfg(test)]
#[path = "tests/context_access.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/context_transport.rs"]
mod transport_tests;
