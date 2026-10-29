// SPDX-License-Identifier: MPL-2.0

use super::{WorkspaceHost, context_reads::ReadState};
use crate::{
    app::context_access::{Decision, Detail, Grant, Surface, label, visible},
    plugin::application::{Error, ErrorCode as Code},
    terminal::{TerminalId, proposal::Delivery},
    workspace::context::{
        storage::{self, HostMode, Identity, Registration, Storage},
        transport::{Event, Lease, Reply, Server},
        wire::{self, Permission, Request, Scope},
    },
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::Duration,
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
    /// Enters agents asked for, waiting out their delay or kept briefly for
    /// their outcome, by handle.
    pub(super) enters: BTreeMap<String, super::context_input::Enter>,
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
    grants: BTreeMap<String, (Identity, BTreeSet<Permission>)>,
    remembered: BTreeSet<String>,
    /// Identities whose grant here came from `mcp.default_grants`.
    defaulted: BTreeSet<String>,
    /// Identities the person revoked here, which keeps their default off.
    denied: BTreeSet<String>,
    pub(super) readers: BTreeMap<u64, Reader>,
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
    grants: BTreeMap<String, (Identity, BTreeSet<Permission>)>,
    remembered: BTreeSet<String>,
    defaulted: BTreeSet<String>,
    denied: BTreeSet<String>,
    /// Defaults that could not be applied, with why.
    skipped: Vec<String>,
}

pub(super) struct Reader {
    fingerprint: String,
    durable: bool,
    pub(super) identity: String,
    /// Every capability the identity's grant here allows.
    granted: BTreeSet<Scope>,
    /// The capabilities it negotiated, within `granted`.
    pub(super) scopes: BTreeSet<Scope>,
    registered: bool,
    lease: Arc<Lease>,
    pub(super) reads: ReadState,
    requests: BTreeSet<String>,
    /// Lines it typed that may not have been written yet, with the frontend
    /// attachment they were typed under. They are cancelled with the reader,
    /// or when that frontend goes.
    pub(super) writes: Vec<(Delivery, u64)>,
}

impl State {
    pub(super) fn retained_reader_bytes(&self) -> usize {
        self.readers
            .values()
            .map(|reader| reader.reads.retained_bytes())
            .sum()
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for reader in self.readers.values() {
            reader.lease.cancel();
            for (delivery, _) in &reader.writes {
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
                Some(permissions) if permissions.is_empty() => {
                    initial.denied.insert(identity.name().to_owned());
                }
                Some(permissions) => {
                    initial.remembered.insert(identity.name().to_owned());
                    initial
                        .grants
                        .insert(identity.fingerprint(), (identity, permissions));
                }
                None => {}
            }
        }
        for name in self.app.config.mcp.narrow_defaults() {
            initial.skipped.push(format!(
                "{name} (its default names only some of the reads earlier releases had; Read now covers buffers, terminals and the channel together, so write read to grant it)"
            ));
        }
        for (name, permissions) in self.app.config.mcp.default_permissions() {
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
                        .insert(identity.fingerprint(), (identity, permissions));
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
            // Nothing is preselected: an identity reads nothing until the
            // person turns Read on.
            let permissions = granted
                .map(|(_, permissions)| permissions.clone())
                .unwrap_or_default();
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
                    "Use the same --identity in runyte mcp. Agent start order is flexible. Changes apply at once to running agents.",
                ),
                Detail::note(
                    "Read covers every buffer and terminal here, unsaved text and terminal history included, and the agent channel.",
                ),
                Detail::note(
                    "Write lets agents edit buffers and type into any terminal here, shells and other agents' prompts included, and press Enter there. Enter waits 5 seconds, shown in the pane title, and is dropped if anything appears or is typed meanwhile. Grant it for a job, then turn it off.",
                ),
            ];
            if let Some(recent) = self.context.recent.back() {
                explanation.push(Detail::excerpt("Last call", recent));
            }
            self.app.plugins.presentation_dirty = true;
            self.app.context_ui.surface = Some(Surface::new(
                Grant {
                    identity,
                    permissions,
                    remember,
                },
                "MCP permissions".into(),
                explanation,
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
        self.context_advance_enters();
    }

    fn context_decision(&mut self, decision: Decision) -> anyhow::Result<()> {
        match decision {
            Decision::Grant {
                identity,
                permissions,
                remember,
            } => {
                wire::validate_permissions(&permissions).map_err(|e| anyhow::anyhow!(e.message))?;
                let store = self.context_writable_storage()?;
                let identity = store.identity(&identity)?;
                // Applying nothing is a revocation and is kept like one.
                if remember || permissions.is_empty() {
                    store.grant(&self.app.project_root, &identity, permissions.clone())?;
                } else {
                    store.revoke(&self.app.project_root, &identity)?;
                }
                // Readers negotiated the old grant; they reconnect under the
                // new one, so a narrower grant holds from now on.
                self.context_disconnect_identity(identity.name());
                // An applied grant replaces a default and clears a revocation.
                self.context.defaulted.remove(identity.name());
                if permissions.is_empty() {
                    self.context.denied.insert(identity.name().to_owned());
                } else {
                    self.context.denied.remove(identity.name());
                }
                if remember && !permissions.is_empty() {
                    self.context.remembered.insert(identity.name().to_owned());
                } else {
                    self.context.remembered.remove(identity.name());
                }
                if permissions.is_empty() {
                    self.context
                        .grants
                        .retain(|_, (granted, _)| granted.name() != identity.name());
                    self.context_retire_if_ungranted();
                    return Ok(());
                }
                self.context
                    .grants
                    .insert(identity.fingerprint(), (identity, permissions));
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
        }
        Ok(())
    }

    fn context_revoke(&mut self, name: &str) {
        self.context_disconnect_identity(name);
        self.context
            .grants
            .retain(|_, (identity, _)| identity.name() != name);
        self.context_retire_if_ungranted();
    }

    fn context_disconnect_identity(&mut self, name: &str) {
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
    }

    /// Stops listening once no identity is granted anything here.
    fn context_retire_if_ungranted(&mut self) {
        if !self.context.grants.is_empty() {
            return;
        }
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

    fn context_disconnect(&mut self, id: u64) {
        if let Some(reader) = self.context.readers.remove(&id) {
            reader.lease.cancel();
            for (delivery, _) in &reader.writes {
                delivery.cancel();
            }
        }
        self.context.peers.remove(&id);
        self.context.channel.disconnect(id);
        self.context_channel_changed();
        self.context_cancel_enters(id);
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
            let readers = self.context.readers.keys().copied().collect::<Vec<_>>();
            for id in readers {
                self.context_disconnect(id);
            }
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
            let Some((identity, permissions)) = self.context.grants.get(&fingerprint) else {
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
                    identity: identity.name().to_owned(),
                    granted: wire::granted_scopes(permissions),
                    scopes: BTreeSet::new(),
                    registered: false,
                    lease: lease.clone(),
                    reads: ReadState::new(prefix),
                    requests: BTreeSet::new(),
                    writes: Vec::new(),
                },
            );
            let mut features = vec![
                wire::FEATURE,
                wire::CHANNEL_FEATURE,
                wire::TERMINAL_WRITE_FEATURE,
            ];
            if self.context.durable_root.is_some() {
                features.push(wire::DURABLE_FEATURE);
            }
            let capabilities = wire::CAPABILITIES
                .iter()
                .copied()
                .chain([
                    Scope::TerminalWrite.capability(),
                    Scope::AgentChannel.capability(),
                ])
                .collect::<Vec<_>>();
            return Reply {
                value: json!({"type":"hello","version":"runyte-1","host_version":crate::plugin::compatibility::HOST_VERSION,"features":features,"capabilities":capabilities,"limits":wire::limits(),"workspace":self.context.registration}),
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
                if registration
                    .required_features
                    .union(&registration.optional_features)
                    .any(|feature| feature == wire::TERMINAL_WRITE_FEATURE)
                {
                    features.push(wire::TERMINAL_WRITE_FEATURE);
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
                // Out of the table, so cancel its queued text here; the lease
                // stays live until this error reply is delivered.
                for (delivery, _) in &reader.writes {
                    delivery.cancel();
                }
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
            Request::TerminalInputWrite { terminal, .. } => {
                ("terminal.input.write", terminal.as_str())
            }
            Request::TerminalInputEnter { terminal, .. } => {
                ("terminal.input.enter", terminal.as_str())
            }
            Request::TerminalInputEnterStatus { enter } => {
                ("terminal.input.enter.status", enter.as_str())
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
                Request::TerminalInputWrite { terminal, text } => reader
                    .reads
                    .terminal(&terminal)
                    .and_then(|terminal| self.context_write_terminal(&mut reader, terminal, &text)),
                Request::TerminalInputEnter {
                    terminal,
                    expected_revision,
                } => reader.reads.terminal(&terminal).and_then(|terminal| {
                    self.context_request_enter(id, terminal, &expected_revision)
                }),
                Request::TerminalInputEnterStatus { enter } => {
                    self.context_enter_status(id, &enter)
                }
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

    pub fn context_delay(&self) -> Option<Duration> {
        #[cfg(windows)]
        {
            if self.context.events.is_none() || self.context.stopping {
                return None;
            }
        }
        // Retained snapshots expire after an idle interval.
        let snapshots = self
            .context
            .readers
            .values()
            .any(|r| r.reads.retained_bytes() > 0)
            .then_some(Duration::from_secs(30));
        match (self.context_enter_delay(), snapshots) {
            (Some(enters), Some(snapshots)) => Some(enters.min(snapshots)),
            (enters, snapshots) => enters.or(snapshots),
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
