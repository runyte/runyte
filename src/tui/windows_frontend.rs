// SPDX-License-Identifier: MPL-2.0

//! Native attachment to one already-ready, exact host publication.

use super::{
    KeyRepeatDetector, TerminalGuard, TerminationSignals, is_passive_pointer, is_wheel_event,
    rejected_text_input, terminal_color_depth, terminal_key_kind, terminated,
};
use anyhow::{Context, Result, anyhow, bail};
use crossterm::event::Event as CrosstermEvent;
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use runyte::{
    cwd_handoff::Prepared as PreparedCwdHandoff,
    input::{InputEvent, PointerEvent},
    protocol::{
        DestinationVisit, MAX_POINTER_REPETITIONS, NativeSwitchCandidate, WaitStatus, WaitToken,
        decode_path, encode_path, validate_welcome,
    },
    tui::{input::convert_event, windows_input::EventStream},
    ui::{self, TerminalColorDepth},
    workspace::{
        FrameId, HostFrame, NativeHostExit, PublicationKey, WorkspaceSelection,
        windows_catalog::HistoryTarget,
        windows_control::ControlSnapshot,
        windows_endpoint::{EndpointMetadata, PipeAddress},
        windows_location::DiscoveryScope,
        windows_process_identity::ProcessIdentity,
        windows_transport::{BufferedLocalClient, ClientRequest, HostResponse},
    },
};
use std::{
    collections::VecDeque,
    io::stdout,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const INITIAL_FRAME_BUDGET: Duration = Duration::from_secs(5);
const FINAL_REPLY_BUDGET: Duration = Duration::from_secs(3);
const SWITCH_BUDGET: Duration = Duration::from_secs(10);
// The response reader may retain 64 semantic messages, one final message and
// one coalesced visual frame when the process-exit notification arrives.
const FINAL_DRAIN_MESSAGES: usize = 128;
const RETURN_HISTORY_LIMIT: usize = 16;

enum WireOutcome {
    Sent,
    HostEnded(HostEnd),
}

impl WireOutcome {
    fn ended(&self) -> bool {
        matches!(self, Self::HostEnded(_))
    }

    fn attachment_outcome(self) -> Option<AttachmentOutcome> {
        match self {
            Self::Sent => None,
            Self::HostEnded(HostEnd::Detached(None)) => Some(AttachmentOutcome::Detached),
            Self::HostEnded(HostEnd::Detached(Some(directory))) => {
                Some(AttachmentOutcome::DirectoryHandoff(directory))
            }
            Self::HostEnded(HostEnd::ShuttingDown) => Some(AttachmentOutcome::Stopped),
        }
    }
}

struct Attachment {
    client: BufferedLocalClient,
    current: HostFrame,
    exit: NativeHostExit,
    identity: WorkspaceSelection,
    initial_frame_acknowledged: bool,
}

enum AttachmentOutcome {
    Detached,
    DirectoryHandoff(PathBuf),
    Stopped,
    Switch {
        receipt: u64,
        candidate: Box<NativeSwitchCandidate>,
        visit: Option<DestinationVisit>,
    },
}

#[derive(Debug, Eq, PartialEq)]
enum HostEnd {
    Detached(Option<PathBuf>),
    ShuttingDown,
}

enum SwitchReceiptState {
    Pending,
    ParentCommitAccepted,
    Complete,
}

#[derive(Clone, Copy)]
struct WheelBatch {
    event: PointerEvent,
    frame: FrameId,
    repetitions: u16,
}

impl WheelBatch {
    fn request(self) -> ClientRequest {
        ClientRequest::Pointer {
            event: self.event.into(),
            frame: self.frame.into(),
            repetitions: self.repetitions,
        }
    }
}

#[derive(Default)]
struct WheelBatcher(Option<WheelBatch>);

impl WheelBatcher {
    fn push(&mut self, event: PointerEvent, frame: FrameId) -> Option<WheelBatch> {
        if let Some(batch) = self.0.as_mut()
            && batch.event == event
            && batch.repetitions < MAX_POINTER_REPETITIONS
        {
            batch.frame = frame;
            batch.repetitions += 1;
            return None;
        }
        self.0.replace(WheelBatch {
            event,
            frame,
            repetitions: 1,
        })
    }

    fn take(&mut self) -> Option<WheelBatch> {
        self.0.take()
    }
}

struct FrontendLoop {
    geometry: runyte::app::FrameGeometry,
    input: EventStream,
    repeats: KeyRepeatDetector,
    wheels: WheelBatcher,
    wheel_tick: tokio::time::Interval,
}

impl FrontendLoop {
    fn new(geometry: runyte::app::FrameGeometry) -> Result<Self> {
        let mut wheel_tick = tokio::time::interval(Duration::from_millis(8));
        wheel_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Ok(Self {
            geometry,
            input: EventStream::new()?,
            repeats: KeyRepeatDetector::default(),
            wheels: WheelBatcher::default(),
            wheel_tick,
        })
    }
}

/// Owns one terminal guard for the whole attachment and keeps every joined
/// input/transport worker inside that guard's lifetime.
#[cfg(test)]
pub(super) async fn attach_exact(
    metadata: &EndpointMetadata,
    termination: &mut TerminationSignals,
    mouse_enabled: bool,
) -> Result<()> {
    attach_exact_with_options(metadata, termination, mouse_enabled, None, None, None).await
}

pub(super) async fn attach_exact_with_catalog(
    metadata: &EndpointMetadata,
    termination: &mut TerminationSignals,
    mouse_enabled: bool,
    scope: &DiscoveryScope,
    configured_state: &Path,
    cwd_handoff: Option<&PreparedCwdHandoff>,
) -> Result<()> {
    attach_exact_with_options(
        metadata,
        termination,
        mouse_enabled,
        None,
        Some((scope, configured_state)),
        cwd_handoff,
    )
    .await
}

pub(super) async fn attach_exact_for_wait(
    metadata: &EndpointMetadata,
    termination: &mut TerminationSignals,
    mouse_enabled: bool,
    token: WaitToken,
) -> Result<()> {
    attach_exact_with_options(
        metadata,
        termination,
        mouse_enabled,
        Some(token),
        None,
        None,
    )
    .await
}

async fn attach_exact_with_options(
    metadata: &EndpointMetadata,
    termination: &mut TerminationSignals,
    mouse_enabled: bool,
    wait_token: Option<WaitToken>,
    return_catalog: Option<(&DiscoveryScope, &Path)>,
    cwd_handoff: Option<&PreparedCwdHandoff>,
) -> Result<()> {
    let depth = terminal_color_depth();
    let guard = TerminalGuard::enter(mouse_enabled)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let geometry = ui::frame_geometry(terminal.size()?.into());
    let result = tokio::select! {
        biased;
        event = termination.recv() => Err(terminated(event)),
        result = run_switching_session(metadata, &mut terminal, depth, geometry, wait_token, return_catalog, cwd_handoff.is_some()) => result,
    };
    // The switching session owns its EventStream and every buffered pipe. Its
    // completion or cancellation drops and joins them before restoration.
    drop(terminal);
    drop(guard);
    let pending = termination.pending_event().await;
    let directory = match result {
        Ok(directory) => {
            super::reconcile_pending_console_event(Ok(()), pending)?;
            directory
        }
        Err(error) => return super::reconcile_pending_console_event(Err(error), pending),
    };
    if let Some(directory) = directory {
        cwd_handoff
            .context("native host requested a shell directory without a prepared handoff")?
            .write(&directory)
            .context("cannot publish PowerShell directory handoff")?;
    }
    Ok(())
}

async fn run_switching_session(
    metadata: &EndpointMetadata,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    depth: TerminalColorDepth,
    geometry: runyte::app::FrameGeometry,
    wait_token: Option<WaitToken>,
    return_catalog: Option<(&DiscoveryScope, &Path)>,
    directory_handoff: bool,
) -> Result<Option<PathBuf>> {
    let startup_deadline = tokio::time::Instant::now() + INITIAL_FRAME_BUDGET;
    let mut attachment = tokio::time::timeout_at(
        startup_deadline,
        connect_attachment(metadata, geometry, directory_handoff),
    )
    .await
    .context("native workspace attachment timed out before its first frame")??;
    terminal.resize(Rect::new(
        0,
        0,
        geometry.screen.width,
        geometry.screen.height,
    ))?;
    draw(terminal, &attachment.current, depth)?;
    tokio::time::timeout_at(startup_deadline, acknowledge_initial_frame(&mut attachment))
        .await
        .context("native workspace attachment timed out acknowledging its first frame")??;
    if let Some(token) = wait_token {
        tokio::time::timeout(
            INITIAL_FRAME_BUDGET,
            attach_wait(&mut attachment, terminal, depth, token),
        )
        .await
        .context("native wait attachment timed out before token acknowledgement")??;
    }
    let mut loop_state = FrontendLoop::new(geometry)?;
    let mut history = VecDeque::new();

    loop {
        match run_attachment(&mut attachment, terminal, depth, &mut loop_state).await? {
            AttachmentOutcome::Detached => return Ok(None),
            AttachmentOutcome::DirectoryHandoff(directory) => return Ok(Some(directory)),
            AttachmentOutcome::Stopped => {
                if let Some((scope, configured_state)) = return_catalog {
                    tokio::time::timeout(SWITCH_BUDGET, attachment.exit.wait())
                        .await
                        .context("stopped workspace host did not finish exiting")?;
                    let stopped = attachment.identity.clone();
                    history.retain(|entry| entry != &stopped);
                    if let Some(returned) = return_from_quit(
                        (scope, configured_state),
                        &history,
                        &stopped,
                        &loop_state,
                        terminal,
                        depth,
                        directory_handoff,
                    )
                    .await?
                    {
                        attachment = returned;
                        loop_state.repeats = KeyRepeatDetector::default();
                        continue;
                    }
                }
                return Ok(None);
            }
            AttachmentOutcome::Switch {
                receipt,
                candidate,
                visit,
            } => {
                loop_state.wheels.0 = None;
                let switch_deadline = tokio::time::Instant::now() + SWITCH_BUDGET;
                let destination_metadata = match candidate_metadata(&candidate) {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        recover_switch_failure(&mut attachment, receipt, &error, switch_deadline)
                            .await?;
                        draw(terminal, &attachment.current, depth)?;
                        continue;
                    }
                };
                let destination = tokio::select! {
                    biased;
                    _ = attachment.exit.wait() => {
                        drain_after_host_exit(&mut attachment.client).await?;
                        bail!("source workspace host exited during native switch")
                    }
                    result = connect_attachment(&destination_metadata, loop_state.geometry, directory_handoff) => result,
                };
                let mut destination = match destination {
                    Ok(destination) => destination,
                    Err(error) => {
                        recover_switch_failure(&mut attachment, receipt, &error, switch_deadline)
                            .await?;
                        draw(terminal, &attachment.current, depth)?;
                        continue;
                    }
                };
                if let Err(error) = ensure_candidate_identity(&candidate, &destination.identity) {
                    drop(destination);
                    recover_switch_failure(&mut attachment, receipt, &error, switch_deadline)
                        .await?;
                    draw(terminal, &attachment.current, depth)?;
                    continue;
                }
                if let Some(visit) = visit
                    && let Err(error) =
                        visit_destination(&mut destination, visit, terminal, depth, switch_deadline)
                            .await
                {
                    drop(destination);
                    recover_switch_failure(&mut attachment, receipt, &error, switch_deadline)
                        .await?;
                    draw(terminal, &attachment.current, depth)?;
                    continue;
                }
                // The destination becomes visible before the source reservation
                // is released. A failed commit never silently presents it as
                // the current attachment.
                draw(terminal, &destination.current, depth)?;
                tokio::select! {
                    biased;
                    _ = attachment.exit.wait() => {
                        drain_after_host_exit(&mut attachment.client).await?;
                        bail!("source workspace host exited during native switch")
                    }
                    result = tokio::time::timeout_at(
                        switch_deadline,
                        acknowledge_initial_frame(&mut destination),
                    ) => {
                        result.context("native workspace switch timed out acknowledging its first frame")??;
                    }
                }
                commit_switch(&mut attachment, receipt, switch_deadline).await?;
                let previous = attachment.identity.clone();
                history.retain(|entry| entry != &previous && entry != &destination.identity);
                history.push_back(previous.clone());
                if history.len() > RETURN_HISTORY_LIMIT {
                    history.pop_front();
                }
                attachment = destination;
                record_previous_publication(
                    &mut attachment,
                    &previous,
                    terminal,
                    depth,
                    switch_deadline,
                )
                .await?;
                loop_state.repeats = KeyRepeatDetector::default();
            }
        }
    }
}

async fn attach_wait(
    attachment: &mut Attachment,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    depth: TerminalColorDepth,
    token: WaitToken,
) -> Result<()> {
    if send_or_exit(
        &mut attachment.client,
        &attachment.exit,
        &ClientRequest::AttachWait { token },
    )
    .await?
    .ended()
    {
        bail!("native workspace host exited before wait attachment");
    }
    loop {
        match attachment.client.recv().await? {
            Some(HostResponse::WaitState {
                token: received,
                status: WaitStatus::Pending { .. },
                ..
            }) if received == token => return Ok(()),
            Some(HostResponse::WaitState {
                token: received,
                status: WaitStatus::Completed,
                ..
            }) if received == token => return Ok(()),
            Some(HostResponse::WaitState {
                token: received,
                status: WaitStatus::Cancelled { reason },
                ..
            }) if received == token => bail!(reason),
            Some(HostResponse::Frame { frame }) => {
                attachment.current = (*frame)
                    .try_into()
                    .map_err(|error: String| anyhow!(error))?;
                draw(terminal, &attachment.current, depth)?;
            }
            Some(HostResponse::TerminalDamage { damage }) => {
                if apply_damage(&mut attachment.current, &damage)? {
                    draw(terminal, &attachment.current, depth)?;
                } else if send_or_exit(
                    &mut attachment.client,
                    &attachment.exit,
                    &ClientRequest::Resynchronize,
                )
                .await?
                .ended()
                {
                    bail!("native workspace host exited before wait attachment");
                }
            }
            Some(HostResponse::Refused { message } | HostResponse::Error { message }) => {
                bail!(message)
            }
            Some(HostResponse::Detached { .. } | HostResponse::ShuttingDown) | None => {
                bail!("native workspace host ended before wait attachment");
            }
            Some(_) => {}
        }
    }
}

async fn visit_destination(
    attachment: &mut Attachment,
    visit: DestinationVisit,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    depth: TerminalColorDepth,
    deadline: tokio::time::Instant,
) -> Result<()> {
    let request = ClientRequest::VisitDestination {
        incarnation: visit.incarnation,
        destination: visit.destination,
    };
    if tokio::time::timeout_at(
        deadline,
        send_or_exit(&mut attachment.client, &attachment.exit, &request),
    )
    .await
    .context("native destination visit timed out")??
    .ended()
    {
        bail!("destination workspace host exited before accepting its visit");
    }
    loop {
        let response = tokio::time::timeout_at(deadline, attachment.client.recv())
            .await
            .context("destination workspace host did not answer its visit")??;
        match response {
            Some(HostResponse::DestinationVisitResult { error: None }) => break,
            Some(HostResponse::DestinationVisitResult { error: Some(error) }) => bail!(error),
            Some(HostResponse::Frame { frame }) => {
                attachment.current = (*frame)
                    .try_into()
                    .map_err(|error: String| anyhow!(error))?;
            }
            Some(HostResponse::TerminalDamage { damage }) => {
                let _ = apply_damage(&mut attachment.current, &damage)?;
            }
            Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
                bail!(message)
            }
            Some(HostResponse::Detached { .. } | HostResponse::ShuttingDown) | None => {
                bail!("destination workspace host ended before accepting its visit")
            }
            Some(_) => {}
        }
    }
    // A visit changes the destination's active pane after its initial frame.
    // Request one complete frame so the source is released only after that
    // exact accepted resource can be drawn.
    if tokio::time::timeout_at(
        deadline,
        send_or_exit(
            &mut attachment.client,
            &attachment.exit,
            &ClientRequest::Resynchronize,
        ),
    )
    .await
    .context("native destination frame request timed out")??
    .ended()
    {
        bail!("destination workspace host exited after accepting its visit");
    }
    loop {
        let response = tokio::time::timeout_at(deadline, attachment.client.recv())
            .await
            .context("destination workspace host did not show its accepted visit")??;
        match response {
            Some(HostResponse::Frame { frame }) => {
                attachment.current = (*frame)
                    .try_into()
                    .map_err(|error: String| anyhow!(error))?;
                draw(terminal, &attachment.current, depth)?;
                return Ok(());
            }
            Some(HostResponse::TerminalDamage { damage }) => {
                let _ = apply_damage(&mut attachment.current, &damage)?;
            }
            Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
                bail!(message)
            }
            Some(HostResponse::Detached { .. } | HostResponse::ShuttingDown) | None => {
                bail!("destination workspace host ended before showing its accepted visit")
            }
            Some(_) => {}
        }
    }
}

async fn record_previous_publication(
    attachment: &mut Attachment,
    previous: &WorkspaceSelection,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    depth: TerminalColorDepth,
    deadline: tokio::time::Instant,
) -> Result<()> {
    let publication_key = previous
        .publication_key()
        .context("previous native publication has no exact identity")?
        .to_bytes();
    let request = ClientRequest::NativePreviousPublication {
        project_root_bytes: encode_path(previous.project_root()),
        publication_key,
    };
    if tokio::time::timeout_at(
        deadline,
        send_or_exit(&mut attachment.client, &attachment.exit, &request),
    )
    .await
    .context("native previous publication transfer timed out")??
    .ended()
    {
        bail!("destination workspace host exited before previous publication transfer");
    }
    loop {
        let response = tokio::time::timeout_at(deadline, attachment.client.recv())
            .await
            .context("destination workspace host did not acknowledge previous publication")??;
        match response {
            Some(HostResponse::NativePreviousPublicationRecorded) => return Ok(()),
            Some(HostResponse::Frame { frame }) => {
                attachment.current = (*frame)
                    .try_into()
                    .map_err(|error: String| anyhow!(error))?;
                draw(terminal, &attachment.current, depth)?;
            }
            Some(HostResponse::TerminalDamage { damage }) => {
                if apply_damage(&mut attachment.current, &damage)? {
                    draw(terminal, &attachment.current, depth)?;
                }
            }
            Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
                bail!(message)
            }
            Some(HostResponse::Detached { .. } | HostResponse::ShuttingDown) | None => {
                bail!("destination workspace host ended before previous publication transfer")
            }
            Some(response) => {
                bail!("unexpected previous publication acknowledgement: {response:?}")
            }
        }
    }
}

async fn return_from_quit(
    catalog_source: (&DiscoveryScope, &Path),
    history: &VecDeque<WorkspaceSelection>,
    stopped: &WorkspaceSelection,
    loop_state: &FrontendLoop,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    depth: TerminalColorDepth,
    directory_handoff: bool,
) -> Result<Option<Attachment>> {
    let (scope, configured_state) = catalog_source;
    let deadline = tokio::time::Instant::now() + SWITCH_BUDGET;
    let snapshot = tokio::time::timeout_at(
        deadline,
        ControlSnapshot::observe(scope, configured_state, false),
    )
    .await
    .context("native session return catalog timed out")??;
    let catalog = snapshot.history();
    let mut candidates = Vec::new();
    // The physical frontend's successful attachment history has priority.
    // Each entry is matched against a fresh exact row before connecting.
    for selection in history.iter().rev() {
        if selection == stopped {
            continue;
        }
        if let Some(index) = catalog.select_selection(selection)?
            && let Some(HistoryTarget::Live { publication, .. }) = catalog.target(index)
        {
            candidates.push(publication.metadata().clone());
        }
    }
    // A fresh live session can be visited even if this frontend has never
    // attached to it. The bounded list excludes the just stopped publication.
    for index in 0..catalog.entries().len() {
        let Some(HistoryTarget::Live { publication, row }) = catalog.target(index) else {
            continue;
        };
        if row.selection() == *stopped
            || candidates
                .iter()
                .any(|metadata| metadata == publication.metadata())
        {
            continue;
        }
        candidates.push(publication.metadata().clone());
        if candidates.len() >= RETURN_HISTORY_LIMIT {
            break;
        }
    }
    for metadata in candidates.into_iter().take(RETURN_HISTORY_LIMIT) {
        if metadata.protocol != runyte::protocol::VERSION {
            continue;
        }
        let candidate = tokio::time::timeout_at(
            deadline,
            connect_attachment(&metadata, loop_state.geometry, directory_handoff),
        )
        .await;
        let Ok(Ok(mut candidate)) = candidate else {
            continue;
        };
        if draw(terminal, &candidate.current, depth).is_err() {
            continue;
        }
        if tokio::time::timeout_at(deadline, acknowledge_initial_frame(&mut candidate))
            .await
            .map_or(true, |result| result.is_err())
        {
            continue;
        }
        if let Some(previous) = history
            .iter()
            .rev()
            .find(|selection| *selection != &candidate.identity && *selection != stopped)
        {
            record_previous_publication(&mut candidate, previous, terminal, depth, deadline)
                .await?;
        }
        return Ok(Some(candidate));
    }
    Ok(None)
}

async fn connect_attachment(
    metadata: &EndpointMetadata,
    geometry: runyte::app::FrameGeometry,
    directory_handoff: bool,
) -> Result<Attachment> {
    // The same deadline bounds connection, Welcome and the first complete
    // decoded frame. A silent or half-speaking host never strands raw mode.
    let (mut client, current) = tokio::time::timeout(INITIAL_FRAME_BUDGET, async {
        let mut client =
            BufferedLocalClient::connect_with_handoff(metadata, geometry, directory_handoff)
                .await?;
        match client.recv_handshake().await? {
            Some(response @ HostResponse::Welcome { .. }) => {
                validate_welcome(&response, true).map_err(anyhow::Error::msg)?;
            }
            Some(HostResponse::Refused { message }) => bail!(message),
            Some(response) => bail!("unexpected native workspace handshake: {response:?}"),
            None => bail!("native workspace host disconnected during handshake"),
        }
        let frame = match client.recv().await? {
            Some(HostResponse::Frame { frame }) => (*frame)
                .try_into()
                .map_err(|error: String| anyhow!(error))?,
            Some(response) => bail!("native workspace host sent no initial frame: {response:?}"),
            None => bail!("native workspace host disconnected before its initial frame"),
        };
        Ok::<_, anyhow::Error>((client, frame))
    })
    .await
    .context("native workspace attachment timed out before its first frame")??;

    let peer = client
        .peer()
        .context("native workspace connection has no authenticated host proof")?;
    let exit = NativeHostExit::new(std::sync::Arc::clone(peer))?;
    if !peer.is_alive()? {
        drain_after_host_exit(&mut client).await?;
        bail!("native workspace host exited before attachment")
    }
    let identity = WorkspaceSelection::selected(
        metadata.project_root()?,
        PublicationKey::from_authenticated_metadata(metadata),
    );
    Ok(Attachment {
        client,
        current,
        exit,
        identity,
        initial_frame_acknowledged: false,
    })
}

async fn acknowledge_initial_frame(attachment: &mut Attachment) -> Result<()> {
    if attachment.initial_frame_acknowledged {
        return Ok(());
    }
    let request = ClientRequest::FrameDrawn {
        frame: attachment.current.id.into(),
    };
    if send_or_exit(&mut attachment.client, &attachment.exit, &request)
        .await?
        .ended()
    {
        bail!("native workspace host exited before initial-frame acknowledgement")
    }
    attachment.initial_frame_acknowledged = true;
    Ok(())
}

async fn run_attachment(
    attachment: &mut Attachment,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    depth: TerminalColorDepth,
    loop_state: &mut FrontendLoop,
) -> Result<AttachmentOutcome> {
    let client = &mut attachment.client;
    let current = &mut attachment.current;
    let exit = &attachment.exit;

    loop {
        tokio::select! {
            response = client.recv() => {
                match response? {
                    Some(HostResponse::Frame { frame }) => {
                        *current = (*frame).try_into().map_err(|error: String| anyhow!(error))?;
                        draw(terminal, current, depth)?;
                    }
                    Some(HostResponse::TerminalDamage { damage }) => {
                        if apply_damage(current, &damage)? {
                            draw(terminal, current, depth)?;
                        } else if let Some(outcome) = send_or_exit(client, exit, &ClientRequest::Resynchronize)
                            .await?
                            .attachment_outcome()
                        {
                            return Ok(outcome);
                        }
                    }
                    Some(HostResponse::Detached { directory_bytes }) => {
                        return directory_bytes
                            .map(decode_path)
                            .transpose()?
                            .map_or(Ok(AttachmentOutcome::Detached), |directory| {
                                Ok(AttachmentOutcome::DirectoryHandoff(directory))
                            });
                    }
                    Some(HostResponse::ShuttingDown) => return Ok(AttachmentOutcome::Stopped),
                    Some(HostResponse::Refused { message } | HostResponse::Error { message }) => bail!(message),
                    Some(HostResponse::NativeSwitchPrepared { receipt, candidate, visit }) => {
                        return Ok(AttachmentOutcome::Switch { receipt, candidate, visit });
                    }
                    Some(HostResponse::NativeSwitchUnchanged) => {}
                    Some(HostResponse::SwitchWorkspace { .. } | HostResponse::ParentSwitchWorkspace { .. }) => {
                        bail!("this native workspace switch route is not available yet")
                    }
                    Some(HostResponse::NativeSwitchAborted { .. }
                        | HostResponse::NativeSwitchCommitted { .. }
                        | HostResponse::NativeParentSwitchCommitAccepted { .. }) => {
                        bail!("native workspace host sent an unexpected switch receipt")
                    }
                    Some(_) => {}
                    None => bail!("native workspace host disconnected without ending the attachment"),
                }
            }
            _ = exit.wait() => {
                return match drain_after_host_exit_kind(client).await? {
                    HostEnd::Detached(None) => Ok(AttachmentOutcome::Detached),
                    HostEnd::Detached(Some(directory)) => {
                        Ok(AttachmentOutcome::DirectoryHandoff(directory))
                    }
                    HostEnd::ShuttingDown => Ok(AttachmentOutcome::Stopped),
                };
            }
            event = loop_state.input.next() => {
                let Some(event) = event.transpose()? else {
                    if let Some(outcome) = flush_wheel(client, exit, &mut loop_state.wheels)
                        .await?
                        .attachment_outcome() {
                        return Ok(outcome);
                    }
                    if let Some(outcome) = send_or_exit(client, exit, &ClientRequest::Detach)
                        .await?
                        .attachment_outcome() {
                        return Ok(outcome);
                    }
                    return Ok(WireOutcome::HostEnded(drain_after_host_exit_kind(client).await?)
                        .attachment_outcome()
                        .expect("final reply ends the attachment"));
                };
                if let CrosstermEvent::Resize(width, height) = event {
                    loop_state.repeats.observe(None, None, Instant::now());
                    if let Some(outcome) = flush_wheel(client, exit, &mut loop_state.wheels)
                        .await?
                        .attachment_outcome() {
                        return Ok(outcome);
                    }
                    loop_state.geometry = ui::frame_geometry(Rect::new(0, 0, width, height));
                    if let Some(outcome) = send_or_exit(client, exit, &ClientRequest::Resize { geometry: loop_state.geometry.into() })
                        .await?
                        .attachment_outcome() {
                        return Ok(outcome);
                    }
                    continue;
                }
                let kind = terminal_key_kind(&event);
                let Some(event) = convert_native_event(event)? else {
                    loop_state.repeats.observe(kind, None, Instant::now());
                    continue;
                };
                let repeated = loop_state.repeats.observe(kind, Some(&event), Instant::now());
                if let Some(message) = rejected_text_input(&event) {
                    if let Some(outcome) = flush_wheel(client, exit, &mut loop_state.wheels)
                        .await?
                        .attachment_outcome() {
                        return Ok(outcome);
                    }
                    if let Some(outcome) = send_or_exit(client, exit, &ClientRequest::Notify { message })
                        .await?
                        .attachment_outcome() {
                        return Ok(outcome);
                    }
                    continue;
                }
                if is_passive_pointer(&event) {
                    continue;
                }
                match event {
                    InputEvent::Pointer(pointer) if is_wheel_event(pointer.kind) => {
                        if let Some(batch) = loop_state.wheels.push(pointer, current.id)
                            && let Some(outcome) = send_or_exit(client, exit, &batch.request())
                                .await?
                                .attachment_outcome()
                        {
                            return Ok(outcome);
                        }
                    }
                    InputEvent::Pointer(pointer) => {
                        if let Some(outcome) = flush_wheel(client, exit, &mut loop_state.wheels)
                            .await?
                            .attachment_outcome() {
                            return Ok(outcome);
                        }
                        if let Some(outcome) = send_or_exit(client, exit, &ClientRequest::Pointer {
                            event: pointer.into(),
                            frame: current.id.into(),
                            repetitions: 1,
                        }).await?.attachment_outcome() {
                            return Ok(outcome);
                        }
                    }
                    event => {
                        if let Some(outcome) = flush_wheel(client, exit, &mut loop_state.wheels)
                            .await?
                            .attachment_outcome() {
                            return Ok(outcome);
                        }
                        if let Some(outcome) = send_or_exit(client, exit, &ClientRequest::Input {
                            event: event.into(),
                            repeated,
                            presented_frame: Some(current.id.into()),
                        }).await?.attachment_outcome() {
                            return Ok(outcome);
                        }
                    }
                }
            }
            _ = loop_state.wheel_tick.tick(), if loop_state.wheels.0.is_some() => {
                if let Some(outcome) = flush_wheel(client, exit, &mut loop_state.wheels)
                    .await?
                    .attachment_outcome() {
                    return Ok(outcome);
                }
            }
        }
    }
}

fn candidate_metadata(candidate: &NativeSwitchCandidate) -> Result<EndpointMetadata> {
    let metadata = EndpointMetadata {
        protocol: candidate.protocol,
        id: candidate.id.clone(),
        name: candidate.name.clone(),
        project_root_bytes: candidate.project_root_bytes.clone(),
        process: ProcessIdentity {
            pid: candidate.process_pid,
            creation_time: candidate.process_creation_time,
        },
        incarnation: candidate.incarnation.clone(),
        address: PipeAddress::try_from(candidate.address.clone())?,
    };
    metadata.validate()?;
    anyhow::ensure!(
        PublicationKey::from_authenticated_metadata(&metadata).to_bytes()
            == candidate.publication_key,
        "native switch candidate identity does not match its endpoint"
    );
    Ok(metadata)
}

fn ensure_candidate_identity(
    candidate: &NativeSwitchCandidate,
    identity: &WorkspaceSelection,
) -> Result<()> {
    anyhow::ensure!(
        identity.publication_key().map(PublicationKey::to_bytes) == Some(candidate.publication_key),
        "authenticated native switch destination changed identity"
    );
    Ok(())
}

async fn abort_switch(
    attachment: &mut Attachment,
    receipt: u64,
    deadline: tokio::time::Instant,
) -> Result<()> {
    tokio::time::timeout_at(deadline, async {
        if send_or_exit(
            &mut attachment.client,
            &attachment.exit,
            &ClientRequest::NativeSwitchAbort { receipt },
        )
        .await?
        .ended()
        {
            bail!("source workspace host ended before switch abort")
        }
        await_switch_receipt(attachment, receipt, false, deadline).await
    })
    .await
    .context("native switch abort exceeded its whole-operation deadline")?
}

async fn recover_switch_failure(
    attachment: &mut Attachment,
    receipt: u64,
    error: &anyhow::Error,
    deadline: tokio::time::Instant,
) -> Result<()> {
    abort_switch(attachment, receipt, deadline).await?;
    let message = format!("native session switch failed: {error:#}");
    tokio::time::timeout_at(deadline, async {
        if send_or_exit(
            &mut attachment.client,
            &attachment.exit,
            &ClientRequest::Notify { message },
        )
        .await?
        .ended()
        {
            bail!("source workspace host ended while reporting switch failure")
        }
        Ok(())
    })
    .await
    .context("native switch recovery exceeded its whole-operation deadline")??;
    Ok(())
}

async fn commit_switch(
    attachment: &mut Attachment,
    receipt: u64,
    deadline: tokio::time::Instant,
) -> Result<()> {
    if tokio::time::timeout_at(
        deadline,
        send_or_exit(
            &mut attachment.client,
            &attachment.exit,
            &ClientRequest::NativeSwitchCommit { receipt },
        ),
    )
    .await
    .context("native switch commit exceeded its whole-operation deadline")??
    .ended()
    {
        bail!("source workspace host ended before switch commit")
    }
    await_switch_receipt(attachment, receipt, true, deadline).await
}

async fn await_switch_receipt(
    attachment: &mut Attachment,
    receipt: u64,
    committed: bool,
    operation_deadline: tokio::time::Instant,
) -> Result<()> {
    let deadline = operation_deadline.min(tokio::time::Instant::now() + FINAL_REPLY_BUDGET);
    loop {
        let response = tokio::select! {
            biased;
            _ = attachment.exit.wait() => {
                drain_after_host_exit(&mut attachment.client).await?;
                bail!("source workspace host exited before switch acknowledgement")
            }
            response = tokio::time::timeout_at(deadline, attachment.client.recv()) => {
                response.context("source workspace host did not acknowledge native switch")??
            }
        };
        let Some(response) = response else {
            bail!("source workspace host disconnected before switch acknowledgement")
        };
        match observe_switch_receipt(response, receipt, committed)? {
            SwitchReceiptState::Pending => {}
            SwitchReceiptState::ParentCommitAccepted => {
                let outcome = tokio::time::timeout_at(
                    deadline,
                    send_or_exit(
                        &mut attachment.client,
                        &attachment.exit,
                        &ClientRequest::NativeParentSwitchCommitObserved { receipt },
                    ),
                )
                .await
                .context("source frontend did not confirm parent switch commit")??;
                if outcome.ended() {
                    bail!("source workspace host ended before parent switch confirmation")
                }
                // The original frontend has now observed the source commit and
                // confirmed it on that same connection. This is the one
                // irreversible parent handoff point: a lost final source
                // receipt cannot make this frontend abandon the already drawn,
                // authenticated destination.
                return Ok(());
            }
            SwitchReceiptState::Complete => return Ok(()),
        }
    }
}

fn observe_switch_receipt(
    response: HostResponse,
    receipt: u64,
    committed: bool,
) -> Result<SwitchReceiptState> {
    match response {
        HostResponse::NativeParentSwitchCommitAccepted { receipt: received }
            if committed && received == receipt =>
        {
            Ok(SwitchReceiptState::ParentCommitAccepted)
        }
        HostResponse::NativeSwitchCommitted { receipt: received }
            if committed && received == receipt =>
        {
            Ok(SwitchReceiptState::Complete)
        }
        HostResponse::NativeSwitchAborted { receipt: received }
            if !committed && received == receipt =>
        {
            Ok(SwitchReceiptState::Complete)
        }
        HostResponse::WaitState { .. }
        | HostResponse::Frame { .. }
        | HostResponse::TerminalDamage { .. } => Ok(SwitchReceiptState::Pending),
        HostResponse::Error { message } | HostResponse::Refused { message } => bail!(message),
        response => bail!("unexpected native switch acknowledgement: {response:?}"),
    }
}

async fn send_or_exit(
    client: &mut BufferedLocalClient,
    exit: &NativeHostExit,
    request: &ClientRequest,
) -> Result<WireOutcome> {
    tokio::select! {
        biased;
        _ = exit.wait() => {
            // Cancelling a partially written send permanently closes that
            // capability. The independent reader remains owned and usable.
            Ok(WireOutcome::HostEnded(drain_after_host_exit_kind(client).await?))
        }
        result = client.send(request) => {
            match result {
                Ok(()) => Ok(WireOutcome::Sent),
                Err(send_error) => {
                    // The host may have accepted :quit-here and closed its
                    // writer before this unrelated queued send completed.
                    // Its independent reader still owns the final response.
                    let recovered = drain_after_host_exit_kind(client).await;
                    final_after_send_failure(send_error, recovered)
                }
            }
        }
    }
}

fn final_after_send_failure(
    send_error: anyhow::Error,
    recovered: Result<HostEnd>,
) -> Result<WireOutcome> {
    match recovered {
        Ok(end) => Ok(WireOutcome::HostEnded(end)),
        Err(recovery_error) => Err(send_error.context(format!(
            "native workspace send failed and no final attachment response arrived: {recovery_error:#}"
        ))),
    }
}

async fn flush_wheel(
    client: &mut BufferedLocalClient,
    exit: &NativeHostExit,
    wheels: &mut WheelBatcher,
) -> Result<WireOutcome> {
    if let Some(batch) = wheels.take() {
        send_or_exit(client, exit, &batch.request()).await
    } else {
        Ok(WireOutcome::Sent)
    }
}

fn convert_native_event(event: CrosstermEvent) -> Result<Option<InputEvent>> {
    // The host owns the App context and decides whether image paste is legal
    // in a terminal, overlay, command mode or jump. A frontend has no App.
    if matches!(&event, CrosstermEvent::Paste(text) if text.is_empty()) {
        Ok(Some(InputEvent::ClipboardPaste))
    } else {
        Ok(convert_event(event)?)
    }
}

fn draw(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    frame: &HostFrame,
    depth: TerminalColorDepth,
) -> Result<()> {
    terminal.draw(|area| ui::render_host_frame(area, frame, depth))?;
    Ok(())
}

fn apply_damage(
    current: &mut HostFrame,
    damage: &runyte::protocol::TerminalDamageFrame,
) -> Result<bool> {
    let mut wire: runyte::protocol::HostFrame = current.clone().into();
    if !damage.apply(&mut wire) {
        return Ok(false);
    }
    *current = wire.try_into().map_err(|error: String| anyhow!(error))?;
    Ok(true)
}

async fn drain_after_host_exit(client: &mut BufferedLocalClient) -> Result<()> {
    drain_after_host_exit_kind(client).await.map(|_| ())
}

async fn drain_after_host_exit_kind(client: &mut BufferedLocalClient) -> Result<HostEnd> {
    let deadline = tokio::time::Instant::now() + FINAL_REPLY_BUDGET;
    let mut first_error = None;
    for _ in 0..FINAL_DRAIN_MESSAGES {
        match tokio::time::timeout_at(deadline, client.recv()).await {
            Ok(Ok(Some(response))) => {
                if let Some(outcome) = observe_exit_response(&mut first_error, response) {
                    return outcome;
                }
            }
            Ok(Ok(None)) => break,
            Ok(Err(error)) => {
                if let Some(message) = first_error {
                    return Err(anyhow!(message)).context(format!(
                        "native workspace host exited while reading final replies: {error:#}"
                    ));
                }
                return Err(error)
                    .context("native workspace host exited while reading final replies");
            }
            Err(_) => break,
        }
    }
    if let Some(message) = first_error {
        bail!(message);
    }
    bail!("native workspace host exited without ending the attachment")
}

fn observe_exit_response(
    first_error: &mut Option<String>,
    response: HostResponse,
) -> Option<Result<HostEnd>> {
    match response {
        HostResponse::Error { message } | HostResponse::Refused { message } => {
            first_error.get_or_insert(message);
            None
        }
        HostResponse::ShuttingDown => Some(
            first_error
                .take()
                .map_or(Ok(HostEnd::ShuttingDown), |message| Err(anyhow!(message))),
        ),
        HostResponse::Detached { directory_bytes } => Some(match first_error.take() {
            Some(message) => Err(anyhow!(message)),
            None => directory_bytes
                .map(decode_path)
                .transpose()
                .map(HostEnd::Detached)
                .map_err(anyhow::Error::from),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runyte::{
        test_support::TestRuntimeRoot,
        workspace::windows_endpoint::{EndpointLocation, RegistrySet},
    };

    fn candidate() -> (NativeSwitchCandidate, EndpointMetadata) {
        let root = TestRuntimeRoot::new("native-switch-candidate").unwrap();
        let endpoint = EndpointLocation::new(
            root.path(),
            root.join("endpoint"),
            RegistrySet::with_inventory(&[root.join("registry")], Some(root.join("inventory")))
                .unwrap(),
        )
        .unwrap();
        let prepared = endpoint.prepare(Some("candidate".into())).unwrap();
        let metadata = prepared.metadata().clone();
        drop(prepared);
        let key = PublicationKey::from_authenticated_metadata(&metadata).to_bytes();
        (
            NativeSwitchCandidate {
                protocol: metadata.protocol,
                id: metadata.id.clone(),
                name: metadata.name.clone(),
                project_root_bytes: metadata.project_root_bytes.clone(),
                process_pid: metadata.process.pid,
                process_creation_time: metadata.process.creation_time,
                incarnation: metadata.incarnation.clone(),
                address: metadata.address.as_str().to_owned(),
                publication_key: key,
            },
            metadata,
        )
    }

    #[test]
    fn empty_native_paste_delegates_context_to_host() {
        assert_eq!(
            convert_native_event(CrosstermEvent::Paste(String::new())).unwrap(),
            Some(InputEvent::ClipboardPaste)
        );
        assert_eq!(
            convert_native_event(CrosstermEvent::Paste("text".into())).unwrap(),
            Some(InputEvent::Text("text".into()))
        );
    }

    #[test]
    fn final_host_error_survives_later_shutdown_response() {
        let mut first_error = None;
        assert!(
            observe_exit_response(
                &mut first_error,
                HostResponse::Error {
                    message: "first failure".into(),
                }
            )
            .is_none()
        );
        assert!(
            observe_exit_response(
                &mut first_error,
                HostResponse::Refused {
                    message: "later refusal".into(),
                }
            )
            .is_none()
        );
        let result = observe_exit_response(&mut first_error, HostResponse::ShuttingDown)
            .expect("shutdown ends the drain");
        assert_eq!(result.unwrap_err().to_string(), "first failure");

        let mut clean = None;
        assert!(
            observe_exit_response(
                &mut clean,
                HostResponse::Detached {
                    directory_bytes: None,
                }
            )
            .unwrap()
            .is_ok()
        );
    }

    #[test]
    fn final_directory_handoff_survives_exit_first_drain_but_not_an_earlier_error() {
        let directory = PathBuf::from(r"C:\selected [café]");
        let response = || HostResponse::Detached {
            directory_bytes: Some(encode_path(&directory)),
        };
        let mut first_error = None;
        let result = observe_exit_response(&mut first_error, response())
            .expect("directory handoff ends the attachment")
            .unwrap();
        assert!(matches!(&result, HostEnd::Detached(Some(path)) if path == &directory));
        let outcome = WireOutcome::HostEnded(result).attachment_outcome();
        assert!(
            matches!(outcome, Some(AttachmentOutcome::DirectoryHandoff(path)) if path == directory)
        );

        let mut first_error = Some("earlier host failure".to_owned());
        let result = observe_exit_response(&mut first_error, response())
            .expect("directory handoff ends the attachment");
        assert_eq!(result.unwrap_err().to_string(), "earlier host failure");
    }

    #[test]
    fn failed_send_uses_queued_directory_reply_and_keeps_original_error_without_one() {
        let directory = PathBuf::from(r"C:\selected [café]");
        let recovered = final_after_send_failure(
            anyhow!("broken pipe"),
            Ok(HostEnd::Detached(Some(directory.clone()))),
        )
        .unwrap()
        .attachment_outcome();
        assert!(
            matches!(recovered, Some(AttachmentOutcome::DirectoryHandoff(path)) if path == directory)
        );

        let error =
            final_after_send_failure(anyhow!("broken pipe"), Err(anyhow!("no final reply")))
                .err()
                .expect("missing final reply retains the send error");
        assert!(error.to_string().contains("no final reply"));
        assert_eq!(error.root_cause().to_string(), "broken pipe");
    }

    #[test]
    fn input_eof_detach_reply_retains_directory_and_prior_error() {
        let directory = PathBuf::from(r"C:\selected [café]");
        let response = HostResponse::Detached {
            directory_bytes: Some(encode_path(&directory)),
        };
        let end = observe_exit_response(&mut None, response.clone())
            .expect("final reply")
            .unwrap();
        assert!(matches!(
            WireOutcome::HostEnded(end).attachment_outcome(),
            Some(AttachmentOutcome::DirectoryHandoff(path)) if path == directory
        ));
        let result =
            observe_exit_response(&mut Some("first error".into()), response).expect("final reply");
        assert_eq!(result.unwrap_err().to_string(), "first error");
    }

    #[test]
    fn native_switch_candidate_binds_every_endpoint_identity_field() {
        let (candidate, metadata) = candidate();
        assert_eq!(candidate_metadata(&candidate).unwrap(), metadata);

        let mut changed = candidate.clone();
        changed.publication_key[0] ^= 1;
        assert!(
            candidate_metadata(&changed)
                .unwrap_err()
                .to_string()
                .contains("does not match")
        );

        let mut malformed = candidate;
        malformed.address.push('0');
        assert!(candidate_metadata(&malformed).is_err());
    }

    #[test]
    fn same_project_switch_identity_does_not_collapse_publications() {
        let (candidate, metadata) = candidate();
        let exact = WorkspaceSelection::selected(
            metadata.project_root().unwrap(),
            PublicationKey::from_bytes(candidate.publication_key),
        );
        ensure_candidate_identity(&candidate, &exact).unwrap();

        let replacement = WorkspaceSelection::selected(
            metadata.project_root().unwrap(),
            PublicationKey::from_bytes([3; 32]),
        );
        assert!(ensure_candidate_identity(&candidate, &replacement).is_err());
    }

    #[test]
    fn asynchronous_wait_state_may_precede_commit_or_abort_receipt() {
        for committed in [false, true] {
            let wait = HostResponse::WaitState {
                token: serde_json::from_str("1").unwrap(),
                status: runyte::protocol::WaitStatus::Pending {
                    buffers: Vec::new(),
                    remaining: Vec::new(),
                },
                interactive_attached: true,
            };
            assert!(matches!(
                observe_switch_receipt(wait, 9, committed).unwrap(),
                SwitchReceiptState::Pending
            ));
            let receipt = if committed {
                HostResponse::NativeSwitchCommitted { receipt: 9 }
            } else {
                HostResponse::NativeSwitchAborted { receipt: 9 }
            };
            assert!(matches!(
                observe_switch_receipt(receipt, 9, committed).unwrap(),
                SwitchReceiptState::Complete
            ));
            if committed {
                assert!(matches!(
                    observe_switch_receipt(
                        HostResponse::NativeParentSwitchCommitAccepted { receipt: 9 },
                        9,
                        true,
                    )
                    .unwrap(),
                    SwitchReceiptState::ParentCommitAccepted
                ));
            }
        }
        assert!(
            observe_switch_receipt(HostResponse::NativeSwitchCommitted { receipt: 8 }, 9, true,)
                .is_err()
        );
    }
}
