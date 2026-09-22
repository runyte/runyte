// SPDX-License-Identifier: MPL-2.0

//! Dedicated internal native host. Public attachment, foreground supervision
//! and parent-terminal authorization remain unavailable until their acceptance.

mod clients;

use self::clients::{Clients, Incoming, RenameFuture};
use super::{
    HostServices, MAINTENANCE_INTERVAL, about_invocation, context_timeout, initialize_logging,
    note_ended_service, report_logging_failure, resolve_requested_project_root,
    start_host_services, starts_on_about,
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
    startup::{StartupPhase, StartupTrace},
    workspace::{
        HostEvent, WorkspaceHost,
        windows_endpoint::NameStore,
        windows_location::{CapturedRoots, EXPECTED_LAYOUT_ENV, LocationInputs, ResolvedLayout},
        windows_transport::{LocalServer, ServerEvent},
    },
};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

/// Called before ordinary App/terminal construction. The dedicated launcher
/// states the exact project; this mode must never prompt on a detached stdin.
pub(super) async fn run(
    arguments: LaunchArguments,
    startup: &mut StartupTrace,
    termination: &mut super::TerminationSignals,
) -> Result<()> {
    ensure!(
        arguments.mode == LaunchMode::Serve && arguments.detached_host,
        "foreground persistent mode is not yet supported on Windows"
    );
    let show_about = starts_on_about(&arguments);
    let launch_directory = std::env::current_dir()?;
    let configured_roots = CapturedRoots::capture();
    let expected_layout = std::env::var_os(EXPECTED_LAYOUT_ENV);
    let (config, config_path) = Config::load(arguments.config.as_deref())?;
    startup.mark(StartupPhase::ConfigLoaded);
    startup.mark(StartupPhase::ProjectResolutionStarted);
    let requested = arguments
        .project_root
        .as_deref()
        .context("internal native host requires an explicit --project-root")?;
    let project = resolve_requested_project_root(&launch_directory, requested)?;
    let state = project_root::resolve_state_root(&project, &config.workspace.state);
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
        .collect();
    let layout = ResolvedLayout::resolve(LocationInputs {
        project_root: project.clone(),
        state_root: state,
        reserved_user_roots: reserved,
        roots: configured_roots,
    })?;
    // No registry, name store, ready record, or App exists before this check.
    layout.verify_detached_layout(true, expected_layout.as_deref())?;
    startup.mark(StartupPhase::ProjectResolvedAutomatically);
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
        server = Some(LocalServer::bind_with_names(prepared, names)?);
        let server = server.as_mut().expect("native server constructed");
        log_info!("host", "internal native persistent session published"; "workspace" => server.metadata_snapshot().id);
        if let Err(error) = startup.write_requested() { host.report_host_error(format!("failed to write startup timing report: {error}")); }
        run_loop(&mut host, server, services.as_mut().expect("services started"), &mut clients, startup, termination).await
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
) -> Result<()> {
    #[cfg(not(feature = "startup-timing"))]
    let _ = startup;
    let last_detached = Instant::now();
    let mut maintenance = tokio::time::interval(MAINTENANCE_INTERVAL);
    maintenance.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut ended = HashSet::new();
    loop {
        host.note_plugin_frontend(false);
        host.cancel_pointer_drag();
        host.sync_plugin_observers();
        host.sync_context();
        let context_delay = host.context_delay();
        let mut stop = false;
        tokio::select! {
            event = termination.recv() => {
                log_warn!("host", "native console termination requested"; "event" => format!("{event:?}"));
                return Err(super::terminated(event));
            }
            event = server.recv() => {
                let event = event.context("native workspace host listener stopped unexpectedly")?;
                match event {
                    ServerEvent::Connected { id, peer_process, responses, interactive, .. } => clients.connected(id, peer_process, responses, interactive),
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
            event = services.context_events.recv(), if !ended.contains("context") => {
                if let Some(event) = event { host.handle_context_event(event); }
                else { ended.insert("context"); } // Unavailable on Windows; closed intentionally.
            }
            _ = context_timeout(context_delay) => { host.sync_context(); }
            _ = std::future::ready(()), if host.plugin_presentation_pending() => { host.take_plugin_presentation_change(); }
            event = services.pipe_events.recv(), if !ended.contains("shell filters") => {
                if let Some(event) = event { host.handle_pipe_completion(event); }
                else { note_ended_service(&mut ended, "shell filters"); }
            }
            event = runyte::plugin::receive(&mut services.plugin_events) => {
                if let Some(event) = event { host.handle_plugin_event(event); }
                else { services.plugin_events = None; }
            }
            event = services.lsp_events.recv(), if !ended.contains("language servers") => {
                if let Some(event) = event { host.apply_event(HostEvent::Lsp(event)); }
                else { note_ended_service(&mut ended, "language servers"); }
            }
            event = services.syntax_events.recv(), if !ended.contains("syntax") => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::Syntax(event));
                    #[cfg(feature = "startup-timing")]
                    if host.syntax.first().is_some_and(Option::is_some) && startup.note_initial_syntax_ready()
                        && let Err(error) = startup.write_requested() { host.report_host_error(format!("failed to write startup timing report: {error}")); }
                } else {
                    host.syntax_worker_stopped();
                    note_ended_service(&mut ended, "syntax");
                }
            }
            event = services.file_picker_events.recv(), if !ended.contains("file picker") => {
                if let Some(event) = event { host.apply_event(HostEvent::FilePicker(event)); }
                else { note_ended_service(&mut ended, "file picker"); }
            }
            event = services.workspace_search_events.recv(), if !ended.contains("workspace search") => {
                if let Some(event) = event { host.apply_event(HostEvent::WorkspaceSearch(event)); }
                else { note_ended_service(&mut ended, "workspace search"); }
            }
            event = services.file_monitor_events.recv(), if !ended.contains("file monitor") => {
                if let Some(event) = event { host.apply_event(HostEvent::FileObservation(event)); }
                else { note_ended_service(&mut ended, "file monitor"); }
            }
            event = services.git_monitor_events.recv(), if !ended.contains("Git monitor") => {
                if let Some(event) = event { host.apply_event(HostEvent::GitInvalidation(event)); host.refresh_git_if_due(Instant::now()); }
                else { note_ended_service(&mut ended, "Git monitor"); }
            }
            output = services.terminal_events.recv(), if !ended.contains("terminals") => {
                if let Some(output) = output { host.apply_terminal_output(output, false); }
                else { note_ended_service(&mut ended, "terminals"); }
            }
            event = super::receive_workspace_event(&mut services.workspace_events) => {
                if let Some(event) = event { host.apply_event(event); }
                else { services.workspace_events = None; }
            }
            event = async { match services.native_catalog_events.as_mut() {
                Some(events) => events.recv().await,
                None => std::future::pending().await,
            }} => {
                if let Some(event) = event { host.apply_event(HostEvent::Workspace(event)); }
                else {
                    services.native_catalog_events = None;
                    host.app_mut().detach_workspace_service();
                    note_ended_service(&mut ended, "native session catalog");
                }
            }
            event = async { match services.git_events.as_mut() { Some(events) => events.recv().await, None => std::future::pending().await } } => {
                if let Some(event) = event { host.apply_event(HostEvent::Git(event)); }
                else { services.git_events = None; }
            }
            _ = maintenance.tick() => {
                report_logging_failure(host.app_mut());
                services.file_monitor.sync(host.file_monitor_requests());
                services.git_monitor.sync(host.git_monitor_repository());
                host.refresh_git_if_due(Instant::now());
                host.app_mut().poll_external_opens(Instant::now());
                let idle = Duration::from_secs((host.config.workspace.idle_retirement_minutes as u64).saturating_mul(60));
                stop = !idle.is_zero() && clients.renames.is_empty() && host.may_retire_idle() && last_detached.elapsed() >= idle;
            }
            _ = tokio::task::yield_now(), if host.macro_replay_pending() => {
                if let Err(error) = host.advance_macro_replay() { host.report_host_error(error.to_string()); }
            }
            _ = tokio::task::yield_now(), if host.resource_finder_scan_pending() => { host.advance_resource_finder_scan(); }
        }
        clients.reconcile(host);
        // Detached background work cannot manufacture a future physical TUI
        // handoff. Consume stale requests without executing another workspace.
        let _ = host.take_workspace_switch();
        let _ = host.take_persistent_exit_request();
        if stop {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests;
