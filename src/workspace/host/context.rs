// SPDX-License-Identifier: MPL-2.0

use super::{WorkspaceHost, context_reads::ReadState};
use crate::{
    app::context_access::{Decision, Detail, Kind, Surface, label, visible},
    notification::{NotificationDraft, NotificationSeverity},
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

mod durable;

type FrameSource = (usize, u64, Option<(TerminalId, u64)>);

#[derive(Default)]
pub(super) struct State {
    events: Option<mpsc::Sender<Event>>,
    durable_root: Option<std::path::PathBuf>,
    durable_worker: Option<durable::Worker>,
    /// The workspace's agent channel, for as long as this host runs.
    pub(super) channel: crate::workspace::context::channel::Channel,
    /// The channel revision the open `[agent channel]` page last showed.
    pub(super) channel_page_revision: u64,
    /// The channel nudge each terminal has outstanding, by terminal id.
    pub(super) nudges: BTreeMap<u64, super::context_channel::Nudge>,
    /// The process behind each connection, where the transport knows it.
    pub(super) peers: BTreeMap<u64, Option<u32>>,
    storage: Option<Arc<Storage>>,
    #[cfg(unix)]
    storage_root: Option<std::path::PathBuf>,
    registration: Option<Registration>,
    server: Option<Server>,
    #[cfg(windows)]
    retiring: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    #[cfg(windows)]
    retirement_permit: Option<mpsc::OwnedPermit<Event>>,
    #[cfg(windows)]
    deferred_enable: bool,
    #[cfg(windows)]
    enable_failed: bool,
    #[cfg(windows)]
    stopping: bool,
    #[cfg(windows)]
    location: Option<storage::StorageLocation>,
    #[cfg(windows)]
    environment: Option<String>,
    grants: BTreeMap<String, (Identity, BTreeSet<Scope>)>,
    remembered: BTreeSet<String>,
    /// Identities whose grant here came from `mcp.default_grants`.
    defaulted: BTreeSet<String>,
    /// Identities the person revoked here, which keeps their default off.
    denied: BTreeSet<String>,
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

impl State {
    /// Whether the context service was started for this host.
    pub(super) fn started(&self) -> bool {
        self.events.is_some()
    }
}

#[derive(Default)]
struct InitialGrants {
    grants: BTreeMap<String, (Identity, BTreeSet<Scope>)>,
    remembered: BTreeSet<String>,
    defaulted: BTreeSet<String>,
    denied: BTreeSet<String>,
    /// Defaults that could not be applied, with why.
    skipped: Vec<String>,
}

struct Reader {
    fingerprint: String,
    durable: bool,
    /// Negotiated `wire::NUDGE_FEATURE`, so its posts may ask for nudges.
    nudge: bool,
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
        #[cfg(windows)]
        self.retiring.take();
        #[cfg(unix)]
        if let (Some(storage), Some(registration)) = (&self.storage, &self.registration) {
            let _ = storage.unregister(&registration.host_incarnation);
        }
    }
}

impl WorkspaceHost {
    pub(super) fn context_enabled(&self) -> bool {
        self.context.server.is_some()
    }
    #[cfg(unix)]
    pub fn start_context(&mut self, mode: HostMode) -> mpsc::Receiver<Event> {
        self.context.durable_root = if cfg!(test) {
            None
        } else {
            std::env::var_os("RUNYTE_CHANNEL_HOME").map(std::path::PathBuf::from)
        };
        self.start_context_with_root(mode, Storage::default_root())
    }

    #[cfg(unix)]
    fn start_context_with_root(
        &mut self,
        mode: HostMode,
        root: Option<std::path::PathBuf>,
    ) -> mpsc::Receiver<Event> {
        let (send, receive) = mpsc::channel(32);
        self.context.events = Some(send);
        self.context.mode = Some(mode);
        self.context.storage_root = root.clone();
        // Defaults may name an identity no agent has created yet, so they
        // need storage that can be written; otherwise nothing is created.
        let defaults = !self.app.config.mcp.default_grants.is_empty();
        if let Some(root) = root.filter(|root| defaults || root.exists()) {
            let loaded = (|| -> anyhow::Result<()> {
                let initial = if defaults {
                    self.context_open_storage(root.clone())?;
                    let store = self.context.storage.clone().unwrap();
                    self.context_initial_grants(&store)?
                } else {
                    self.context_initial_grants(&Storage::open_existing(root.clone())?)?
                };
                self.context_adopt_grants(initial);
                if !self.context.grants.is_empty() {
                    if self.context.storage.is_none() {
                        self.context_open_storage(root)?;
                    }
                    self.context_enable()?;
                }
                Ok(())
            })();
            if let Err(error) = loaded {
                self.report_host_error(format!(
                    "Agent context access (remembered or default) could not be loaded: {error}"
                ));
            }
        }
        receive
    }

    #[cfg(windows)]
    pub fn start_context(&mut self, mode: HostMode) -> anyhow::Result<mpsc::Receiver<Event>> {
        self.context.durable_root = if cfg!(test) {
            None
        } else {
            std::env::var_os("RUNYTE_CHANNEL_HOME").map(std::path::PathBuf::from)
        };
        let location = Storage::default_location();
        let environment = storage::environment_fingerprint();
        self.start_context_windows(mode, location, environment)
    }

    #[cfg(windows)]
    pub(crate) fn start_context_windows(
        &mut self,
        mode: HostMode,
        location: Option<storage::StorageLocation>,
        environment: String,
    ) -> anyhow::Result<mpsc::Receiver<Event>> {
        anyhow::ensure!(
            self.context.events.is_none() && !self.context.stopping,
            "Context service was already initialized or shut down"
        );
        let (send, receive) = mpsc::channel(33);
        let retirement_permit = send
            .clone()
            .try_reserve_owned()
            .map_err(|_| anyhow::anyhow!("Context lifecycle capacity is unavailable"))?;
        self.context.events = Some(send);
        self.context.retirement_permit = Some(retirement_permit);
        self.context.mode = Some(mode);
        self.context.location = location.clone();
        self.context.environment = Some(environment);
        let defaults = !self.app.config.mcp.default_grants.is_empty();
        let Some(location) = location.filter(|location| defaults || location.root().exists())
        else {
            return Ok(receive);
        };
        let loaded = (|| -> anyhow::Result<()> {
            self.context_validate_storage_root(location.root())?;
            let initial = if defaults {
                let store = Arc::new(Storage::open_location(location.clone())?);
                let initial = self.context_initial_grants(&store)?;
                self.context.storage = Some(store);
                initial
            } else {
                self.context_initial_grants(&Storage::open_existing(location.root().to_owned())?)?
            };
            let granted = !initial.grants.is_empty();
            self.context_adopt_grants(initial);
            if granted {
                if self.context.storage.is_none() {
                    self.context.storage = Some(Arc::new(Storage::open_location(location)?));
                }
                self.context_enable()?;
            }
            Ok(())
        })();
        if let Err(error) = loaded {
            self.context_adopt_grants(InitialGrants::default());
            self.context.storage = None;
            self.context.enable_failed = true;
            self.report_host_error(format!(
                "Agent context access (remembered or default) could not be loaded: {error}"
            ));
        }
        Ok(receive)
    }

    /// The grants this workspace starts with: each identity's own record,
    /// then the configured default for an identity that has none. An empty
    /// record is a revocation the person made here, and keeps the default off.
    fn context_initial_grants(&self, store: &Storage) -> anyhow::Result<InitialGrants> {
        let mut initial = InitialGrants::default();
        for identity in store.identities()? {
            match store.grant_record(&self.app.project_root, &identity)? {
                Some(scopes) if scopes.is_empty() => {
                    initial.denied.insert(identity.name().to_owned());
                }
                Some(scopes) => {
                    initial.remembered.insert(identity.name().to_owned());
                    initial
                        .grants
                        .insert(identity.fingerprint(), (identity, scopes));
                }
                None => {}
            }
        }
        for (name, scopes) in self.app.config.mcp.default_scopes() {
            if initial.remembered.contains(name) || initial.denied.contains(name) {
                continue;
            }
            // An existing identity is read without the store's lock. Creating
            // one lets an agent started later authenticate as it; its
            // credential stays in the private store. A default that cannot be
            // applied is reported without costing the workspace its own grants.
            let identity = match store.load_identity(name) {
                Ok(Some(identity)) => Ok(identity),
                Ok(None) => store.identity(name),
                Err(error) => Err(error),
            };
            match identity {
                Ok(identity) => {
                    initial.defaulted.insert(name.to_owned());
                    initial
                        .grants
                        .insert(identity.fingerprint(), (identity, scopes));
                }
                Err(error) => initial.skipped.push(format!("{name} ({error})")),
            }
        }
        Ok(initial)
    }

    /// The private store, opened for writing if it is not open yet.
    fn context_writable_storage(&mut self) -> anyhow::Result<Arc<Storage>> {
        if self.context.storage.is_none() {
            #[cfg(unix)]
            self.context_open_storage(
                self.context
                    .storage_root
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Private context storage unavailable"))?,
            )?;
            #[cfg(windows)]
            {
                let location = self
                    .context
                    .location
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Private context storage unavailable"))?;
                self.context_validate_storage_root(location.root())?;
                self.context.storage = Some(Arc::new(Storage::open_location(location)?));
            }
        }
        Ok(self.context.storage.clone().unwrap())
    }

    fn context_adopt_grants(&mut self, initial: InitialGrants) {
        if !initial.skipped.is_empty() {
            self.report_host_error(format!(
                "Default agent access could not be applied for {}",
                initial.skipped.join(", ")
            ));
        }
        self.context.grants = initial.grants;
        self.context.remembered = initial.remembered;
        self.context.defaulted = initial.defaulted;
        self.context.denied = initial.denied;
    }

    #[cfg(any(unix, test))]
    fn context_open_storage(&mut self, root: std::path::PathBuf) -> anyhow::Result<()> {
        self.context_validate_storage_root(&root)?;
        #[cfg(unix)]
        let storage = Storage::new(root)?;
        #[cfg(windows)]
        let storage = Storage::open_location(storage::StorageLocation::explicit(root)?)?;
        self.context.storage = Some(Arc::new(storage));
        Ok(())
    }

    fn context_validate_storage_root(&self, root: &std::path::Path) -> anyhow::Result<()> {
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
        #[cfg(windows)]
        anyhow::ensure!(
            self.app.project_root.to_str().is_some() && root.to_str().is_some(),
            "Windows context publication requires Unicode workspace and storage roots"
        );
        Ok(())
    }

    fn context_enable(&mut self) -> anyhow::Result<()> {
        if self.context.server.is_some() || self.context.grants.is_empty() {
            return Ok(());
        }
        #[cfg(windows)]
        if self.context.retiring.is_some() {
            self.context.deferred_enable = true;
            return Ok(());
        }
        #[cfg(windows)]
        if self.context.stopping {
            return Ok(());
        }
        #[cfg(windows)]
        if self.context.enable_failed {
            return Ok(());
        }
        #[cfg(windows)]
        anyhow::ensure!(
            self.context.retirement_permit.is_some(),
            "Context lifecycle capacity is unavailable"
        );
        let store = self
            .context
            .storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Private context storage unavailable"))?;
        let incarnation = storage::random_token()?;
        let endpoint = store.socket_path(&incarnation)?;
        #[cfg(windows)]
        let process = crate::workspace::windows_process_identity::ProcessIdentity::current()?;
        let registration = Registration {
            root: self.app.project_root.clone(),
            workspace_id: crate::workspace::identity::workspace_id(self.identity.root()),
            host_incarnation: incarnation,
            endpoint: endpoint.clone(),
            mode: self.context.mode.unwrap_or(HostMode::Standalone),
            pid: std::process::id(),
            #[cfg(windows)]
            creation_time: process.creation_time,
            #[cfg(unix)]
            environment: storage::environment_fingerprint(),
            #[cfg(windows)]
            environment: self
                .context
                .environment
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Context environment was not captured"))?,
        };
        let events = self
            .context
            .events
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Context service is not running"))?;
        #[cfg(unix)]
        let server = Server::bind(endpoint, events)?;
        #[cfg(windows)]
        let server = Server::bind(store.clone(), registration.clone(), events)?;
        #[cfg(unix)]
        store.register(&registration)?;
        self.context.registration = Some(registration);
        self.context.server = Some(server);
        #[cfg(windows)]
        {
            self.context.deferred_enable = false;
            self.context.enable_failed = false;
        }
        Ok(())
    }

    pub fn sync_context(&mut self) {
        #[cfg(windows)]
        {
            if self.context.events.is_none() {
                self.app.context_ui.requested_identity = None;
                self.app.context_ui.requested_binding = None;
                self.app.context_ui.decision = None;
                self.app.context_ui.surface = None;
                return;
            }
            if self.context.stopping {
                return;
            }
        }
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
        if std::mem::take(&mut self.app.context_ui.channel_page_requested) {
            let page = self.context_channel_page();
            self.context.channel_page_revision = self.context.channel.revision();
            self.app.open_agent_channel_page(&page);
            // Opened after this frame was drawn, so ask for another.
            self.app.plugins.presentation_dirty = true;
        }
        if let Some(binding) = self.app.context_ui.requested_binding.take()
            && self.app.plugins.frontend_attached
            && !self.app.has_input_overlay()
            && let Err(error) = self.context_review_binding(&binding)
        {
            self.report_host_error(error.to_string());
        }
        if let Some(identity) = self.app.context_ui.requested_identity.take()
            && self.app.plugins.frontend_attached
            && !self.app.has_input_overlay()
        {
            let mut identities: BTreeSet<String> = self
                .context
                .grants
                .values()
                .map(|(id, _)| id.name().to_owned())
                .collect();
            // Listing paired identities does not require a workspace grant or
            // a writable store. Reopen on demand to include newly paired agents.
            #[cfg(unix)]
            let root = self.context.storage_root.clone();
            #[cfg(windows)]
            let root = self.context.location.as_ref().map(|l| l.root().to_owned());
            let inventory = self.context.storage.clone().or_else(|| {
                let root = root?;
                self.context_validate_storage_root(&root).ok()?;
                Storage::open_existing(root).ok().map(Arc::new)
            });
            if let Some(store) = inventory
                && let Ok(known) = store.identities()
            {
                identities.extend(known.into_iter().map(|id| id.name().to_owned()));
            }
            let identity = if identity.is_empty() {
                self.context
                    .grants
                    .values()
                    .next()
                    .map(|(id, _)| id.name().to_owned())
                    .or_else(|| identities.first().cloned())
                    .unwrap_or_else(|| "agent".into())
            } else {
                identity
            };
            identities.insert(identity.clone());
            self.app.context_ui.identities = identities.into_iter().collect();
            let granted = self
                .context
                .grants
                .values()
                .find(|(id, _)| id.name() == identity);
            let scopes = granted
                .map(|(_, scopes)| scopes.clone())
                .unwrap_or_else(|| [Scope::TerminalRead, Scope::EditorContextRead].into());
            let remember = self.context.remembered.contains(&identity);
            let readers = self
                .context
                .readers
                .values()
                .filter(|r| r.identity == identity && r.registered)
                .count();
            let status = if granted.is_none()
                && self.context.denied.contains(&identity)
                && self.app.config.mcp.default_grants.contains_key(&identity)
            {
                "Revoked here, so mcp.default_grants stays off; apply to grant"
            } else if granted.is_none() {
                "Not granted; apply the selected permissions below"
            } else if remember {
                "Enabled; remembered for this workspace"
            } else if self.context.defaulted.contains(&identity) {
                "Enabled by mcp.default_grants; x revokes it here"
            } else {
                "Enabled until this editor or persistent host exits"
            };
            let mut explanation = vec![
                Detail::value("Identity", label(&identity)),
                Detail::value("Workspace", label(&self.app.project_root.to_string_lossy())),
                Detail::value("Status", status.into()),
                Detail::value(
                    "Connected",
                    format!("{readers} reader(s) for this identity"),
                ),
                Detail::note(
                    "Use the same --identity in runyte mcp. Agent start order is flexible.",
                ),
                Detail::note(
                    "Reads include unsaved text. Terminal text needs approval unless 5 is on.",
                ),
                Detail::note(
                    "5 trusts a terminal's paste mode to mean its program reads pastes as text. Inside tmux, zellij or screen that mode is the multiplexer's, so do not use 5 there.",
                ),
                Detail::note(
                    "7 lets another agent's post type one fixed line and Enter into this identity's terminals, 30 seconds after you last typed there. If the agent is showing an approval prompt then, that Enter may approve it.",
                ),
            ];
            if let Some(recent) = self.context.recent.back() {
                explanation.push(Detail::excerpt("Last call", recent));
            }
            self.app.plugins.presentation_dirty = true;
            self.app.context_ui.surface = Some(Surface::new(
                Kind::Grant {
                    identity,
                    scopes,
                    remember,
                },
                "MCP permissions".into(),
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
        self.context_channel_nudges();
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
                Detail::value("Agent", visible(identity)),
                Detail::value(
                    "Terminal",
                    match terminal.number() {
                        Some(number) => format!("{} (#{number})", label(&terminal.name())),
                        None => label(&terminal.name()),
                    },
                ),
                Detail::value("Workspace", label(&self.app.project_root.to_string_lossy())),
            ];
            if let Some(reason) = &p.reason {
                explanation.push(Detail::value(
                    "Reason",
                    format!("{} (unverified)", label(reason)),
                ));
            }
            explanation.push(Detail::note(
                "Inserted at the input position without Enter; it may still act.",
            ));
            if let Ok(context) = terminal.read_output(
                crate::terminal::read::Region::Tail,
                crate::terminal::read::Limits {
                    rows: 3,
                    bytes: 512,
                    cells: 2048,
                },
                None,
            ) {
                // Context for the reader, not part of what is approved, so a
                // long row is cut rather than allowed to push the proposal
                // onto a second page.
                for (index, row) in context.rows.iter().enumerate() {
                    explanation.push(Detail::excerpt(
                        if index == 0 { "Output" } else { "" },
                        &label(row.text.trim_end()),
                    ));
                }
                explanation.push(Detail::note(
                    "Recent output does not show whether the input line is empty.",
                ));
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
            Decision::ChannelBinding(binding) => self.context_apply_binding(binding)?,
            Decision::Grant {
                identity,
                scopes,
                remember,
            } => {
                wire::validate_scopes(&scopes).map_err(|e| anyhow::anyhow!(e.message))?;
                let store = self.context_writable_storage()?;
                let identity = store.identity(&identity)?;
                // Applying nothing is a revocation and is kept like one.
                if remember || scopes.is_empty() {
                    store.grant(&self.app.project_root, &identity, scopes.clone())?;
                } else {
                    store.revoke(&self.app.project_root, &identity)?;
                }
                self.context_revoke(identity.name());
                // An applied grant replaces a default and clears a revocation.
                self.context.defaulted.remove(identity.name());
                if scopes.is_empty() {
                    self.context.denied.insert(identity.name().to_owned());
                } else {
                    self.context.denied.remove(identity.name());
                }
                if remember && !scopes.is_empty() {
                    self.context.remembered.insert(identity.name().to_owned());
                } else {
                    self.context.remembered.remove(identity.name());
                }
                if !scopes.is_empty() {
                    self.context
                        .grants
                        .insert(identity.fingerprint(), (identity, scopes));
                }
                #[cfg(windows)]
                {
                    // A fresh physical grant is the explicit retry boundary
                    // after a deferred bind failure.
                    self.context.enable_failed = false;
                }
                #[cfg(unix)]
                self.context_enable()?;
                #[cfg(windows)]
                if let Err(error) = self.context_enable() {
                    self.context.enable_failed = true;
                    return Err(error);
                }
            }
            Decision::Revoke(name) => {
                self.context.remembered.remove(&name);
                self.context.defaulted.remove(&name);
                self.context_revoke(&name);
                // Kept as an empty record, so a default configured now or
                // later does not grant again what the person took away. A
                // workspace that never had a grant has not opened the store
                // yet, and that is just where a revocation must be kept.
                let store = self.context_writable_storage()?;
                let identity = store.identity(&name)?;
                store.grant(&self.app.project_root, &identity, BTreeSet::new())?;
                self.context.denied.insert(name);
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
        self.context.channel.clear_terminal_bindings(name);
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
            #[cfg(unix)]
            {
                self.context.server.take();
                if let (Some(store), Some(registration)) =
                    (&self.context.storage, self.context.registration.take())
                {
                    let _ = store.unregister(&registration.host_incarnation);
                }
            }
            #[cfg(windows)]
            {
                self.context.registration.take();
                self.context.deferred_enable = false;
                self.context_begin_retirement();
            }
        }
    }

    fn context_disconnect(&mut self, id: u64) {
        if let Some(reader) = self.context.readers.remove(&id) {
            reader.lease.cancel();
        }
        self.context.peers.remove(&id);
        self.context.channel.disconnect(id);
        self.context_channel_changed();
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

    #[cfg(windows)]
    fn context_begin_retirement(&mut self) {
        let Some(mut server) = self.context.server.take() else {
            return;
        };
        debug_assert!(self.context.retiring.is_none());
        let permit = self
            .context
            .retirement_permit
            .take()
            .expect("an active context service retains lifecycle capacity");
        server.stop();
        self.context.retiring = Some(tokio::spawn(async move {
            let result = server.shutdown().await;
            let error = result.as_ref().err().map(ToString::to_string);
            permit.send(Event::Retired { error });
            result
        }));
    }

    #[cfg(windows)]
    fn context_retired(&mut self, error: Option<String>) {
        // Receipt of this event proves that Server::shutdown joined the
        // transport owner and retired its exact publication. The small outer
        // task owns no resource after sending it.
        self.context.retiring.take();
        let retirement_failed = error.is_some();
        if let Some(error) = error {
            self.context.enable_failed = true;
            self.context.deferred_enable = false;
            self.report_host_error(format!("Agent context transport shutdown failed: {error}"));
        }
        if self.context.stopping {
            return;
        }
        self.context.retirement_permit = self
            .context
            .events
            .as_ref()
            .cloned()
            .and_then(|events| events.try_reserve_owned().ok());
        if self.context.retirement_permit.is_none() {
            self.context.enable_failed = true;
            self.report_host_error("Agent context lifecycle capacity was lost");
        }
        let enable = !retirement_failed
            && std::mem::take(&mut self.context.deferred_enable)
            && !self.context.grants.is_empty()
            && self.context.retirement_permit.is_some();
        if enable && let Err(error) = self.context_enable() {
            self.context.enable_failed = true;
            self.report_host_error(format!(
                "Agent context transport could not restart: {error}"
            ));
        }
    }

    #[cfg(windows)]
    pub async fn shutdown_context(&mut self) -> std::io::Result<()> {
        if !self.context.stopping {
            self.context.stopping = true;
            self.app.context_ui.requested_identity = None;
            self.app.context_ui.decision = None;
            self.app.context_ui.surface = None;
            for reader in self.context.readers.values() {
                reader.lease.cancel();
            }
            self.context.readers.clear();
            for proposal in self.context.proposals.values() {
                if let Some(delivery) = &proposal.delivery {
                    delivery.cancel();
                }
            }
            self.context.proposals.clear();
            self.context.grants.clear();
            self.context.frame = None;
            self.context.frame_sources.clear();
            self.context.frame_review_sources.clear();
            self.context.registration.take();
            self.context.deferred_enable = false;
            self.context_begin_retirement();
        }
        let result = match self.context.retiring.as_mut() {
            Some(retirement) => match (&mut *retirement).await {
                Ok(result) => result,
                Err(error) => Err(std::io::Error::other(format!(
                    "context retirement task failed: {error}"
                ))),
            },
            None => Ok(()),
        };
        self.context.retiring.take();
        self.context.retirement_permit.take();
        self.context.events.take();
        self.context.storage.take();
        self.app.terminals.set_external_retained_bytes(0);
        result
    }

    pub fn handle_context_event(&mut self, event: Event) {
        match event {
            #[cfg(windows)]
            Event::Retired { error } => {
                self.context_retired(error);
            }
            Event::Closed(id) => self.context_disconnect(id),
            Event::Frame {
                connection,
                peer,
                bytes,
                reply,
            } => {
                self.context.peers.entry(connection).or_insert(peer);
                #[cfg(windows)]
                if self.context.server.is_none() || self.context.stopping {
                    self.context_disconnect(connection);
                    let _ = reply.send(failure(Error::new(
                        Code::Unavailable,
                        "Context transport is retiring",
                    )));
                    return;
                }
                if reply.is_closed() {
                    self.context_disconnect(connection);
                    return;
                }
                self.sync_context();
                #[cfg(windows)]
                if self.context.server.is_none() || self.context.stopping {
                    self.context_disconnect(connection);
                    let _ = reply.send(failure(Error::new(
                        Code::Unavailable,
                        "Context transport is retiring",
                    )));
                    return;
                }
                let mut reply = Some(reply);
                if self.context_durable_frame(connection, &bytes, &mut reply) {
                    return;
                }
                let response = self.context_frame(connection, &bytes);
                let _ = reply.unwrap().send(response);
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
                    fingerprint,
                    durable: false,
                    nudge: false,
                    identity: identity.name().to_owned(),
                    granted: scopes.clone(),
                    scopes: BTreeSet::new(),
                    registered: false,
                    lease: lease.clone(),
                    reads: ReadState::new(prefix),
                    requests: BTreeSet::new(),
                },
            );
            let mut features = vec![wire::FEATURE, wire::CHANNEL_FEATURE, wire::NUDGE_FEATURE];
            if self.context.durable_root.is_some() {
                features.push(wire::DURABLE_FEATURE);
            }
            return Reply {
                value: json!({"type":"hello","version":"runyte-1","host_version":crate::plugin::compatibility::HOST_VERSION,"features":features,"capabilities":wire::CAPABILITIES.iter().copied().chain([Scope::AgentChannel.capability()]).collect::<Vec<_>>(),"limits":wire::limits(),"workspace":self.context.registration}),
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
                let mut features = vec![wire::FEATURE];
                if registration
                    .required_features
                    .union(&registration.optional_features)
                    .any(|feature| feature == wire::CHANNEL_FEATURE)
                {
                    features.push(wire::CHANNEL_FEATURE);
                }
                reader.nudge = registration
                    .required_features
                    .union(&registration.optional_features)
                    .any(|feature| feature == wire::NUDGE_FEATURE);
                if reader.nudge {
                    features.push(wire::NUDGE_FEATURE);
                }
                if registration
                    .required_features
                    .contains(wire::DURABLE_FEATURE)
                    && self.context.durable_root.is_none()
                {
                    return Err(Error::new(
                        Code::Unsupported,
                        "Durable storage is not configured",
                    ));
                }
                reader.durable = self.context.durable_root.is_some()
                    && registration
                        .required_features
                        .union(&registration.optional_features)
                        .any(|f| f == wire::DURABLE_FEATURE);
                if reader.durable {
                    features.push(wire::DURABLE_FEATURE);
                }
                Ok(
                    json!({"type":"registered","runyte":registration.runyte,"features":features,"commands":[],"capabilities":reader.scopes,"limits":wire::limits()}),
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
            Request::BufferAppend { buffer, .. } => ("buffer.append", buffer.as_str()),
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
            Request::ChannelJoin { .. } => ("channel.join", "channel"),
            Request::ChannelPost { .. } => ("channel.post", "channel"),
            Request::ChannelRead { .. } => ("channel.read", "channel"),
            Request::ChannelWho(_) => ("channel.who", "channel"),
            Request::ChannelDurable(_) => ("channel.durable", "channel"),
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
                    let direct = reader
                        .granted
                        .contains(&Scope::TerminalInsert)
                        .then_some(reader.identity.as_str());
                    terminal.and_then(|terminal| {
                        self.context_propose(id, terminal, &text, reason, direct)
                    })
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
                Request::ChannelPost { nudge: Some(_), .. } if !reader.nudge => Err(Error::new(
                    Code::Unsupported,
                    "Nudges were not negotiated on this connection",
                )),
                request @ (Request::ChannelJoin { .. }
                | Request::ChannelPost { .. }
                | Request::ChannelRead { .. }
                | Request::ChannelWho(_)) => {
                    self.context_channel_request(id, &reader.identity, request)
                }
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

    /// Queue proposed text for review, or with `direct` (the identity of a
    /// reader whose grant skips review) for insertion now when the terminal
    /// reads pasted text. Either way the text is never followed by Enter, and
    /// the person must be attached.
    fn context_propose(
        &mut self,
        owner: u64,
        terminal: TerminalId,
        text: &str,
        reason: Option<String>,
        direct: Option<&str>,
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
            .get_mut(terminal)
            .filter(|t| t.live())
            .ok_or_else(|| Error::new(Code::Closed, "Terminal is not live"))?;
        let text = Text::new(text)
            .map_err(|_| Error::new(Code::InvalidArgument, "Invalid terminal text"))?;
        let id = storage::random_token()
            .map_err(|_| Error::new(Code::Unavailable, "Cannot allocate proposal"))?;
        let mut proposal = Proposal {
            owner,
            terminal,
            reason,
            signature: session.proposal_input_signature(),
            attachment: self.app.plugins.attachment_generation,
            deadline: Instant::now() + Duration::from_secs(wire::PROPOSAL_EXPIRY_SECONDS),
            status: "pending",
            delivery: None,
            text,
        };
        // Skipping review is safe only where the text arrives as a paste: a
        // shell or an agent's prompt asks for bracketed paste, while a pager,
        // a single-key prompt or an editor in normal mode would read the same
        // bytes as keystrokes. Those still go to the person. The mode is the
        // terminal's, not necessarily the program's: a multiplexer enables it
        // for itself and may strip the framing for whatever runs in its pane,
        // which is why the grant note and the guide warn against it there.
        let direct = direct.filter(|_| proposal.signature.modes.bracketed_paste);
        if let Some(identity) = direct {
            let delivery = session
                .append_proposal(&proposal.text)
                .map_err(|_| Error::new(Code::Unavailable, "Terminal cannot accept text"))?;
            proposal.delivery = Some(delivery);
            proposal.status = "queued";
            let target = match session.number() {
                Some(number) => format!("terminal #{number}"),
                None => "a terminal".to_owned(),
            };
            // Nothing asks the person first, so say what arrived and from
            // whom where it can still be traced afterwards.
            self.app.push_notification(NotificationDraft::new(
                NotificationSeverity::Info,
                "MCP",
                "Terminal text inserted",
                format!(
                    "{} inserted {} characters into {target} without review. Enter was not sent.",
                    visible(identity),
                    proposal.text.as_str().chars().count(),
                ),
            ));
        }
        let state = proposal.status;
        self.context.proposals.insert(id.clone(), proposal);
        Ok(json!({"proposal":id,"state":state}))
    }

    pub fn context_delay(&self) -> Option<Duration> {
        #[cfg(windows)]
        {
            if self.context.events.is_none() || self.context.stopping {
                return None;
            }
        }
        let proposals = self.context_proposal_delay();
        match (proposals, self.context_nudge_delay()) {
            (Some(proposals), Some(nudges)) => Some(proposals.min(nudges)),
            (proposals, nudges) => proposals.or(nudges),
        }
    }

    /// Whether `connection` is a reader whose grant holds `scope`, negotiated
    /// or not.
    pub(super) fn context_granted(&self, connection: u64, scope: Scope) -> bool {
        self.context
            .readers
            .get(&connection)
            .is_some_and(|reader| reader.registered && reader.granted.contains(&scope))
    }

    /// Whether a proposal for `terminal` still waits for review or delivery.
    pub(super) fn context_terminal_proposed(&self, terminal: TerminalId) -> bool {
        self.context.proposals.values().any(|proposal| {
            proposal.terminal == terminal
                && matches!(proposal.status, "pending" | "queued" | "writing")
        })
    }

    fn context_proposal_delay(&self) -> Option<Duration> {
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

#[cfg(all(test, unix))]
#[path = "tests/context_access.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "tests/context_access_windows.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "tests/context_transport.rs"]
mod transport_tests;

#[cfg(all(test, windows))]
#[path = "tests/context_transport_windows.rs"]
mod transport_tests;
