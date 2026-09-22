// SPDX-License-Identifier: MPL-2.0

//! Native persistent host. Foreground and detached lifetimes share one cleanup
//! owner; the internal interactive wire is owned here while public frontend
//! attachment and parent-terminal authorization remain gated.

mod clients;

use self::clients::{Clients, ConnectedPeer, Incoming, NativeSwitchAction, RenameFuture};
use super::{
    FINDER_TERMINAL_REFRESH_INTERVAL, FRAME_INTERVAL, HostServices, MAINTENANCE_INTERVAL,
    STATUS_ANIMATION_INTERVAL, about_invocation, context_timeout, frame_publication_ready,
    initialize_logging, note_ended_service, pace_file_picker_event, report_logging_failure,
    resolve_requested_project_root, start_host_services, starts_on_about,
};
use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use runyte::{
    app::App,
    config::{self, Config},
    launch::{LaunchArguments, LaunchMode},
    log::{self as diagnostic_log, Role as LogRole},
    log_error, log_info, log_warn,
    lsp::LspCommand,
    notification::{NotificationDraft, NotificationSeverity},
    project_root,
    protocol::{HostResponse, NativeSwitchCandidate},
    startup::{StartupPhase, StartupTrace},
    workspace::{
        HostEvent, WorkspaceHost,
        windows_endpoint::NameStore,
        windows_location::{CapturedRoots, EXPECTED_LAYOUT_ENV, LocationInputs, ResolvedLayout},
        windows_parent_identity::ForegroundParentSupervisor,
        windows_transport::{LocalServer, ServerEvent},
    },
};
use std::{
    collections::HashSet,
    future::Future,
    pin::Pin,
    time::{Duration, Instant},
};

type NativeSwitchPreparation = Pin<
    Box<
        dyn Future<
                Output = (
                    runyte::workspace::WorkspaceSelection,
                    Result<runyte::workspace::windows_service::PreparedLiveTarget>,
                ),
            > + Send,
    >,
>;

struct PreparingNativeSwitch {
    owner: u64,
    receipt: u64,
    future: NativeSwitchPreparation,
}

struct PendingNativeSwitch {
    owner: u64,
    receipt: u64,
    _target: runyte::workspace::windows_service::PreparedLiveTarget,
    expires: Instant,
}

fn native_switch_candidate(
    target: &runyte::workspace::windows_service::PreparedLiveTarget,
    selection: &runyte::workspace::WorkspaceSelection,
) -> NativeSwitchCandidate {
    let metadata = target.metadata();
    NativeSwitchCandidate {
        protocol: metadata.protocol,
        id: metadata.id.clone(),
        name: metadata.name.clone(),
        project_root_bytes: metadata.project_root_bytes.clone(),
        process_pid: metadata.process.pid,
        process_creation_time: metadata.process.creation_time,
        incarnation: metadata.incarnation.clone(),
        address: metadata.address.as_str().to_owned(),
        publication_key: selection
            .publication_key()
            .expect("prepared live selection has a publication key")
            .to_bytes(),
    }
}

#[cfg(test)]
fn injected_switch_request() -> Result<Option<runyte::app::WorkspaceSwitchRequest>> {
    let Some(inbox) = std::env::var_os("RUNYTE_TEST_NATIVE_SWITCH_INBOX") else {
        return Ok(None);
    };
    let request = std::path::PathBuf::from(inbox).join("switch-target.json");
    let bytes = match std::fs::read(&request) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    std::fs::remove_file(&request)?;
    let metadata = runyte::workspace::windows_endpoint::EndpointMetadata::from_json(&bytes)?;
    let selection = runyte::workspace::WorkspaceSelection::selected(
        metadata.project_root()?,
        runyte::workspace::PublicationKey::from_authenticated_metadata(&metadata),
    );
    note_switch_fixture("consumed");
    Ok(Some(runyte::app::WorkspaceSwitchRequest {
        visit: None,
        running_only: true,
        target: runyte::app::WorkspaceSwitchTarget::Selected(selection),
        working_directory: std::env::current_dir()?,
    }))
}

#[cfg(test)]
fn note_switch_fixture(stage: &str) {
    if let Some(inbox) = std::env::var_os("RUNYTE_TEST_NATIVE_SWITCH_INBOX") {
        let _ = std::fs::write(std::path::PathBuf::from(inbox).join("switch-stage"), stage);
    }
}

#[cfg(test)]
fn drop_commit_ack_requested() -> bool {
    let Some(inbox) = std::env::var_os("RUNYTE_TEST_NATIVE_SWITCH_INBOX") else {
        return false;
    };
    let marker = std::path::PathBuf::from(inbox).join("drop-commit-ack");
    match std::fs::remove_file(marker) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

#[cfg(test)]
fn note_noop_switch() {
    if let Some(inbox) = std::env::var_os("RUNYTE_TEST_NATIVE_SWITCH_INBOX") {
        let _ = std::fs::write(std::path::PathBuf::from(inbox).join("noop-complete"), b"1");
    }
}

#[cfg(debug_assertions)]
async fn wait_at_parent_startup_fixture(
    supervisor: Option<&ForegroundParentSupervisor>,
) -> Result<()> {
    use std::path::PathBuf;

    if supervisor.is_none() {
        return Ok(());
    }
    let Some(root) = std::env::var_os("RUNYTE_TEST_FOREGROUND_PARENT_BARRIER") else {
        return Ok(());
    };
    let root = PathBuf::from(root);
    let pending = root.join("parent-pinned.pending");
    std::fs::write(&pending, b"1")?;
    std::fs::rename(pending, root.join("parent-pinned"))?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !root.join("release-parent-startup").exists() {
        ensure!(
            Instant::now() < deadline,
            "foreground parent startup fixture was not released"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}

/// A detached launcher states the exact project and never prompts on stdin.
/// A foreground host uses ordinary project discovery or an explicit root.
pub(super) async fn run(
    arguments: LaunchArguments,
    startup: &mut StartupTrace,
    termination: &mut super::TerminationSignals,
    supervisor: Option<ForegroundParentSupervisor>,
) -> Result<()> {
    ensure!(
        arguments.mode == LaunchMode::Serve && arguments.detached_host == supervisor.is_none(),
        "native host launch role and parent supervision disagree"
    );
    if let Some(parent) = supervisor.as_ref() {
        parent.ensure_alive()?;
    }
    let show_about = starts_on_about(&arguments);
    let launch_directory = std::env::current_dir()?;
    let configured_roots = CapturedRoots::capture();
    let expected_layout = std::env::var_os(EXPECTED_LAYOUT_ENV);
    let (config, config_path) = Config::load(arguments.config.as_deref())?;
    startup.mark(StartupPhase::ConfigLoaded);
    let reserved = config_path
        .as_deref()
        .map(|path| {
            let path = config::config_root_for(path, &launch_directory);
            if path.is_absolute() {
                path
            } else {
                launch_directory.join(path)
            }
        })
        .into_iter()
        .collect::<Vec<_>>();
    startup.mark(StartupPhase::ProjectResolutionStarted);
    let project = match arguments.project_root.as_deref() {
        Some(requested) => {
            let project = resolve_requested_project_root(&launch_directory, requested)?;
            startup.mark(StartupPhase::ProjectResolvedAutomatically);
            project
        }
        None if arguments.detached_host => {
            anyhow::bail!("internal native host requires an explicit --project-root")
        }
        None => {
            let project = project_root::discover(&launch_directory, &config.workspace.state)?
                .context(
                    "no project workspace was found; pass --project-root for foreground --serve",
                )?;
            startup.mark(StartupPhase::ProjectResolvedAutomatically);
            project
        }
    };
    if let Some(parent) = supervisor.as_ref() {
        parent.ensure_alive()?;
    }
    let state = project_root::resolve_state_root(&project, &config.workspace.state);
    project_root::validate_state_root(&state, &reserved)?;
    let layout = ResolvedLayout::resolve(LocationInputs {
        project_root: project.clone(),
        state_root: state,
        reserved_user_roots: reserved,
        roots: configured_roots,
    })?;
    // No registry, name store, ready record, or App exists before this check.
    layout.verify_detached_layout(arguments.detached_host, expected_layout.as_deref())?;
    #[cfg(debug_assertions)]
    wait_at_parent_startup_fixture(supervisor.as_ref()).await?;
    if let Some(parent) = supervisor.as_ref() {
        parent.ensure_alive()?;
    }
    let logging_failure =
        initialize_logging(&arguments, LogRole::Host, layout.state_root(), &project)?;
    let mut app =
        App::new_in_project_with_deferred_syntax(config, arguments.targets, project, startup)?;
    app.set_quit_directory_handoff(false);
    if let Some(path) = config_path.as_deref() {
        app.note_loaded_config(path);
    }
    if let Some(failure) = logging_failure {
        app.push_notification(NotificationDraft::new(
            NotificationSeverity::Warning,
            "Logging",
            "Diagnostic log unavailable",
            format!("{failure} · editing continues without a durable log"),
        ));
    }
    let mut host = WorkspaceHost::new(app);
    host.enable_persistent_session();
    host.note_plugin_frontend(false);
    let mut clients = Clients::default();
    let mut services = None;
    let mut server = None;
    // Every failure after host construction reaches the same cleanup, including
    // partially started services, failed publication and a dead listener.
    let outcome = async {
        if let Some(parent) = supervisor.as_ref() { parent.ensure_alive()?; }
        if show_about { host.app_mut().execute(about_invocation()?)?; }
        let native_catalog = super::NativeCatalogConfig::from_layout(
            &layout,
            host.config.workspace.state.clone(),
        );
        services = Some(start_host_services(
            &mut host,
            startup,
            config_path.as_deref(),
            true,
            Some(native_catalog),
        )?);
        let location = layout.publication_location()?;
        let names = NameStore::open(layout.state_root())?;
        let prepared = location.prepare_named(&names, None)?;
        if let Some(parent) = supervisor.as_ref() { parent.ensure_alive()?; }
        server = Some(LocalServer::bind_with_names(prepared, names)?);
        let server = server.as_mut().expect("native server constructed");
        host.app_mut().terminals.set_parent_launch(
            runyte::workspace::parent::ParentLaunch::new(server.metadata_snapshot())?,
        );
        if let Some(parent) = supervisor.as_ref() { parent.ensure_alive()?; }
        log_info!("host", "internal native persistent session published"; "workspace" => server.metadata_snapshot().id);
        if let Err(error) = startup.write_requested() { host.report_host_error(format!("failed to write startup timing report: {error}")); }
        run_loop(&mut host, server, services.as_mut().expect("services started"), &mut clients, startup, termination, supervisor.as_ref()).await
    }.await;
    if let Some(server) = server.as_mut() {
        server.stop_admission();
    }
    host.cancel_all_waits("workspace host shut down");
    if let Some(services) = services.as_ref() {
        services.language_servers.send(LspCommand::Shutdown);
    }
    // All response owners drop before the transport's bounded final drain.
    // Observing a rename is cancellable; begun mutation remains worker-owned.
    clients.clear();
    let plugins = host.shutdown_plugins().await;
    let catalog = match services.as_mut() {
        Some(services) => services.shutdown_native_catalog().await,
        None => Ok(()),
    };
    let transport = match server.as_mut() {
        Some(server) => server.shutdown().await,
        None => Ok(()),
    };
    let result = finish_cleanup(outcome, plugins, catalog, transport);
    diagnostic_log::flush(diagnostic_log::FLUSH_BUDGET);
    result
}

fn finish_cleanup(
    outcome: Result<()>,
    plugins: Result<()>,
    catalog: Result<()>,
    transport: Result<()>,
) -> Result<()> {
    let mut result = outcome;
    for (label, cleanup) in [
        ("host service shutdown failed", plugins),
        ("native catalog shutdown failed", catalog),
        ("native transport shutdown failed", transport),
    ] {
        if let Err(error) = cleanup {
            log_error!("host", "{label}: {error}");
            result = match result {
                Ok(()) => Err(error.context(label)),
                Err(primary) => Err(primary.context(format!("{label}: {error}"))),
            };
        }
    }
    result
}

fn rename(server: &LocalServer, name: &str) -> std::io::Result<RenameFuture> {
    server
        .try_rename(name)
        .map(|ticket| Box::pin(ticket.wait()) as RenameFuture)
}

async fn run_loop(
    host: &mut WorkspaceHost,
    server: &mut LocalServer,
    services: &mut HostServices,
    clients: &mut Clients,
    startup: &mut StartupTrace,
    termination: &mut super::TerminationSignals,
    supervisor: Option<&ForegroundParentSupervisor>,
) -> Result<()> {
    #[cfg(not(feature = "startup-timing"))]
    let _ = startup;
    let mut maintenance = tokio::time::interval(MAINTENANCE_INTERVAL);
    maintenance.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status_animation = tokio::time::interval(STATUS_ANIMATION_INTERVAL);
    status_animation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frame_tick = tokio::time::interval(FRAME_INTERVAL);
    frame_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut finder_refresh = tokio::time::interval(FINDER_TERMINAL_REFRESH_INTERVAL);
    finder_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut frame_pending = false;
    let mut ended = HashSet::new();
    let mut switch_receipt = 0_u64;
    let mut switch_preparation: Option<PreparingNativeSwitch> = None;
    let mut pending_switch: Option<PendingNativeSwitch> = None;
    loop {
        let attached = clients.attached();
        host.note_plugin_frontend(attached);
        if !attached {
            host.cancel_pointer_drag();
        }
        host.sync_plugin_observers();
        host.sync_context();
        let context_delay = host.context_delay();
        let hint_delay = clients.hint_delay(Instant::now());
        let picker_delay = attached
            .then(|| host.picker_pacing_delay(Instant::now()))
            .flatten();
        let pointer_delay = attached
            .then(|| host.pointer_autoscroll_delay(Instant::now()))
            .flatten();
        let mut stop = false;
        let mut changed = false;
        tokio::select! {
            event = termination.recv() => {
                log_warn!("host", "native console termination requested"; "event" => format!("{event:?}"));
                return Err(super::terminated(event));
            }
            _ = async { match supervisor {
                Some(parent) => parent.wait().await,
                None => std::future::pending().await,
            }} => {
                log_info!("host", "foreground parent exited; retiring persistent session";
                    "parent" => supervisor.expect("supervised branch").parent_identity().pid);
                return Ok(());
            }
            event = server.recv() => {
                let event = event.context("native workspace host listener stopped unexpectedly")?;
                match event {
                    ServerEvent::Connected { id, peer_process, responses, interactive, geometry, .. } =>
                        clients.connected(host, ConnectedPeer { id, proof: peer_process, responses, interactive, geometry }),
                    ServerEvent::Request { id, request } => stop = clients.incoming(host, id, Incoming::Request(request), |name| rename(server, name)),
                    ServerEvent::ProtocolError { id, message } => stop = clients.incoming(host, id, Incoming::ProtocolError(message), |name| rename(server, name)),
                    ServerEvent::TransportFailure { id, message } => {
                        log_warn!("transport", "native control connection failed: {message}"; "connection" => id);
                        clients.disconnected(host, id);
                    }
                    ServerEvent::Disconnected { id } => clients.disconnected(host, id),
                }
            }
            completion = clients.renames.next(), if !clients.renames.is_empty() => {
                if let Some((id, result)) = completion { stop = clients.renamed(host, id, result, |name| rename(server, name)); }
            }
            prepared = async {
                match switch_preparation.as_mut() {
                    Some(preparation) => preparation.future.as_mut().await,
                    None => std::future::pending().await,
                }
            } => {
                let preparation = switch_preparation.take().expect("selected preparation exists");
                let owner = preparation.owner;
                let receipt = preparation.receipt;
                let (selection, result) = prepared;
                if clients.active_id() != Some(owner) {
                    // The preparation retained authority only for its original
                    // source attachment. A later owner cannot inherit it.
                } else {
                    match result {
                        Ok(target) => {
                            let candidate = native_switch_candidate(&target, &selection);
                            if clients.begin_switch(
                                host,
                                owner,
                                receipt,
                                HostResponse::NativeSwitchPrepared {
                                    receipt,
                                    candidate: Box::new(candidate),
                                },
                            ) {
                                pending_switch = Some(PendingNativeSwitch {
                                    owner,
                                    receipt,
                                    _target: target,
                                    expires: Instant::now() + Duration::from_secs(12),
                                });
                                #[cfg(test)]
                                note_switch_fixture("prepared");
                            }
                        }
                        Err(error) => {
                            clients.cancel_switch_reservation(owner, receipt);
                            #[cfg(test)]
                            note_switch_fixture("prepare-failed");
                            host.report_host_error(format!("native session switch failed: {error:#}"));
                            clients.send_active(host, HostResponse::NativeSwitchUnchanged);
                            changed = true;
                        }
                    }
                }
            }
            _ = async {
                match pending_switch.as_ref() {
                    Some(pending) => tokio::time::sleep_until(pending.expires.into()).await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(pending) = pending_switch.take() {
                    clients.abort_switch(host, pending.owner, pending.receipt);
                }
            }
            event = services.context_events.recv(), if !ended.contains("context") => {
                if let Some(event) = event { host.handle_context_event(event); changed = true; }
                else { ended.insert("context"); } // Unavailable on Windows; closed intentionally.
            }
            _ = context_timeout(context_delay) => { host.sync_context(); changed = host.plugin_presentation_pending(); }
            _ = std::future::ready(()), if host.plugin_presentation_pending() => { changed = host.take_plugin_presentation_change(); }
            event = services.pipe_events.recv(), if !ended.contains("shell filters") => {
                if let Some(event) = event { host.handle_pipe_completion(event); changed = true; }
                else { note_ended_service(&mut ended, "shell filters"); }
            }
            event = runyte::plugin::receive(&mut services.plugin_events) => {
                if let Some(event) = event { changed = host.handle_plugin_event(event); }
                else { services.plugin_events = None; }
            }
            event = services.lsp_events.recv(), if !ended.contains("language servers") => {
                if let Some(event) = event { host.apply_event(HostEvent::Lsp(event)); changed = true; }
                else { note_ended_service(&mut ended, "language servers"); }
            }
            event = services.syntax_events.recv(), if !ended.contains("syntax") => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::Syntax(event));
                    changed = true;
                    #[cfg(feature = "startup-timing")]
                    if host.syntax.first().is_some_and(Option::is_some) && startup.note_initial_syntax_ready()
                        && let Err(error) = startup.write_requested() { host.report_host_error(format!("failed to write startup timing report: {error}")); }
                } else {
                    host.syntax_worker_stopped();
                    changed = true;
                    note_ended_service(&mut ended, "syntax");
                }
            }
            event = services.file_picker_events.recv(), if !ended.contains("file picker") => {
                if let Some(event) = event {
                    let paced = pace_file_picker_event(&event);
                    host.apply_event(HostEvent::FilePicker(event));
                    if paced { frame_pending = true; } else { changed = true; }
                }
                else { note_ended_service(&mut ended, "file picker"); }
            }
            event = services.workspace_search_events.recv(), if !ended.contains("workspace search") => {
                if let Some(event) = event { host.apply_event(HostEvent::WorkspaceSearch(event)); changed = true; }
                else { note_ended_service(&mut ended, "workspace search"); }
            }
            event = services.file_monitor_events.recv(), if !ended.contains("file monitor") => {
                if let Some(event) = event { host.apply_event(HostEvent::FileObservation(event)); changed = true; }
                else { note_ended_service(&mut ended, "file monitor"); }
            }
            event = services.git_monitor_events.recv(), if !ended.contains("Git monitor") => {
                if let Some(event) = event { host.apply_event(HostEvent::GitInvalidation(event)); changed = true; changed |= host.refresh_git_if_due(Instant::now()); }
                else { note_ended_service(&mut ended, "Git monitor"); }
            }
            output = services.terminal_events.recv(), if !ended.contains("terminals") => {
                if let Some(output) = output {
                    host.apply_terminal_output(output, attached);
                    super::terminal::drain(&mut services.terminal_events, |output| host.apply_terminal_output(output, attached));
                    frame_pending = true;
                }
                else { note_ended_service(&mut ended, "terminals"); }
            }
            event = super::receive_workspace_event(&mut services.workspace_events) => {
                if let Some(event) = event { host.apply_event(event); changed = true; }
                else { services.workspace_events = None; }
            }
            event = async { match services.native_catalog_events.as_mut() {
                Some(events) => events.recv().await,
                None => std::future::pending().await,
            }} => {
                if let Some(event) = event { host.apply_event(HostEvent::Workspace(event)); changed = true; }
                else {
                    services.native_catalog_events = None;
                    host.app_mut().detach_workspace_service();
                    note_ended_service(&mut ended, "native session catalog");
                }
            }
            event = async { match services.git_events.as_mut() { Some(events) => events.recv().await, None => std::future::pending().await } } => {
                if let Some(event) = event { host.apply_event(HostEvent::Git(event)); changed = true; }
                else { services.git_events = None; }
            }
            _ = maintenance.tick() => {
                report_logging_failure(host.app_mut());
                services.file_monitor.sync(host.file_monitor_requests());
                services.git_monitor.sync(host.git_monitor_repository());
                changed = host.refresh_git_if_due(Instant::now());
                changed |= host.app_mut().poll_external_opens(Instant::now());
                if attached { changed |= host.refresh_session_activity(); }
                let idle = Duration::from_secs((host.config.workspace.idle_retirement_minutes as u64).saturating_mul(60));
                stop = !idle.is_zero() && !attached && clients.renames.is_empty() && host.may_retire_idle() && clients.last_detached().elapsed() >= idle;
            }
            _ = status_animation.tick(), if attached && host.has_long_running_action() => { changed = true; }
            _ = frame_tick.tick(), if attached && frame_pending && !host.finder_scan_refills() => { changed = true; }
            _ = finder_refresh.tick(), if host.finder_terminals_dirty() => {
                if host.refresh_finder_terminals() && !host.resource_finder_scan_pending() { changed = true; }
            }
            _ = tokio::time::sleep(picker_delay.unwrap_or_default()), if picker_delay.is_some() && !host.finder_scan_refills() => {
                host.advance_picker_pacing(); changed = true;
            }
            _ = tokio::time::sleep(pointer_delay.unwrap_or_default()), if pointer_delay.is_some() => {
                changed = host.advance_pointer_autoscroll(Instant::now());
            }
            _ = tokio::time::sleep(hint_delay.unwrap_or_default()), if hint_delay.is_some() => {
                clients.expire_hints(Instant::now()); changed = true;
            }
            _ = tokio::task::yield_now(), if host.macro_replay_pending() => {
                if let Err(error) = host.advance_macro_replay() { host.report_host_error(error.to_string()); }
                changed = true;
            }
            _ = tokio::task::yield_now(), if host.resource_finder_scan_pending() => {
                host.advance_resource_finder_scan();
                if host.resource_finder_scan_pending() { frame_pending = true; } else { changed = true; }
            }
        }
        if switch_preparation.as_ref().is_some_and(|preparation| {
            clients.active_id() != Some(preparation.owner)
                || !clients.owns_switch_reservation(preparation.owner, preparation.receipt)
        }) {
            // Dropping the future closes its one-shot receiver. The catalog
            // worker observes that cancellation before retaining a result.
            switch_preparation = None;
        }
        if let Some(action) = clients.take_switch_action() {
            match action {
                NativeSwitchAction::Commit { owner, receipt } => {
                    if pending_switch
                        .as_ref()
                        .is_some_and(|pending| pending.owner == owner && pending.receipt == receipt)
                    {
                        pending_switch = None;
                        #[cfg(test)]
                        note_switch_fixture("committed");
                        #[cfg(test)]
                        if drop_commit_ack_requested() {
                            clients.drop_switch_without_ack(host, owner, receipt);
                        } else {
                            clients.commit_switch(host, owner, receipt);
                        }
                        #[cfg(not(test))]
                        clients.commit_switch(host, owner, receipt);
                    }
                }
                NativeSwitchAction::Abort { owner, receipt } => {
                    if pending_switch
                        .as_ref()
                        .is_some_and(|pending| pending.owner == owner && pending.receipt == receipt)
                    {
                        pending_switch = None;
                        clients.abort_switch(host, owner, receipt);
                        #[cfg(test)]
                        note_switch_fixture("aborted");
                    }
                }
            }
        }
        if pending_switch.is_some() && !clients.switch_pending() {
            pending_switch = None;
        }
        clients.reconcile(host);
        // Detached background work cannot manufacture a future physical TUI
        // handoff. Consume stale requests without executing another workspace.
        #[cfg(test)]
        let injected = match injected_switch_request() {
            Ok(request) => request,
            Err(error) => {
                note_switch_fixture("invalid");
                host.report_host_error(format!("native switch fixture request failed: {error:#}"));
                None
            }
        };
        #[cfg(not(test))]
        let injected: Option<runyte::app::WorkspaceSwitchRequest> = None;
        if let Some(request) = injected.or_else(|| host.take_workspace_switch()) {
            if switch_preparation.is_some() || pending_switch.is_some() {
                clients.refuse_switch_with(host, "a native session switch is already in progress");
            } else if request.visit.is_some() {
                clients.refuse_switch_with(host, "native destination visits are not available yet");
            } else if let runyte::app::WorkspaceSwitchTarget::Selected(selection) = request.target {
                let metadata = server.metadata_snapshot();
                let source = runyte::workspace::WorkspaceSelection::selected(
                    metadata.project_root()?,
                    runyte::workspace::PublicationKey::from_authenticated_metadata(&metadata),
                );
                if selection == source {
                    clients.send_active(host, HostResponse::NativeSwitchUnchanged);
                    #[cfg(test)]
                    {
                        note_switch_fixture("unchanged");
                        note_noop_switch();
                    }
                } else if let (Some(owner), Some(service)) =
                    (clients.active_id(), services.native_catalog.clone())
                {
                    switch_receipt = switch_receipt.wrapping_add(1).max(1);
                    let receipt = switch_receipt;
                    if clients.reserve_switch(host, owner, receipt) {
                        #[cfg(test)]
                        note_switch_fixture("reserved");
                        switch_preparation = Some(PreparingNativeSwitch {
                            owner,
                            receipt,
                            future: Box::pin(async move {
                                let result = service.prepare_selected_live(selection.clone()).await;
                                (selection, result)
                            }),
                        });
                    }
                } else {
                    #[cfg(test)]
                    note_switch_fixture("refused");
                    clients.refuse_switch_with(host, "native session service is unavailable");
                }
            } else {
                clients.refuse_switch(host);
            }
            changed = true;
        } else if let Some(request) = host.take_persistent_exit_request() {
            stop |= clients.finish_exit(host, request);
            changed = true;
        }
        changed |= clients.take_publish_requested();
        if frame_publication_ready(changed, host.finder_scan_refills(), &mut frame_pending) {
            clients.publish_frame(host);
            frame_pending = false;
        }
        if stop {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests;
