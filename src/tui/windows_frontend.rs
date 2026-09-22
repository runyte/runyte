// SPDX-License-Identifier: MPL-2.0

//! Private native attachment to one already-ready, exact host publication.
//! Public launch routing, switching, waits and parent handoff stay gated.

use super::{
    KeyRepeatDetector, TerminalGuard, TerminationSignals, is_passive_pointer, is_wheel_event,
    rejected_text_input, terminal_color_depth, terminal_key_kind, terminated,
};
use anyhow::{Context, Result, anyhow, bail};
use crossterm::event::Event as CrosstermEvent;
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use runyte::{
    input::{InputEvent, PointerEvent},
    protocol::{MAX_POINTER_REPETITIONS, NativeSwitchCandidate, validate_welcome},
    tui::{input::convert_event, windows_input::EventStream},
    ui::{self, TerminalColorDepth},
    workspace::{
        FrameId, HostFrame, NativeHostExit, PublicationKey, WorkspaceSelection,
        windows_endpoint::{EndpointMetadata, PipeAddress},
        windows_process_identity::ProcessIdentity,
        windows_transport::{BufferedLocalClient, ClientRequest, HostResponse},
    },
};
use std::{
    io::stdout,
    time::{Duration, Instant},
};

const INITIAL_FRAME_BUDGET: Duration = Duration::from_secs(5);
const FINAL_REPLY_BUDGET: Duration = Duration::from_secs(3);
const SWITCH_BUDGET: Duration = Duration::from_secs(10);
// The response reader may retain 64 semantic messages, one final message and
// one coalesced visual frame when the process-exit notification arrives.
const FINAL_DRAIN_MESSAGES: usize = 128;

#[derive(Clone, Copy, Eq, PartialEq)]
enum WireOutcome {
    Sent,
    HostEnded,
}

struct Attachment {
    client: BufferedLocalClient,
    current: HostFrame,
    exit: NativeHostExit,
    identity: WorkspaceSelection,
}

enum AttachmentOutcome {
    Ended,
    Switch {
        receipt: u64,
        candidate: NativeSwitchCandidate,
    },
}

enum SwitchReceiptState {
    Pending,
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
pub(super) async fn attach_exact(
    metadata: &EndpointMetadata,
    termination: &mut TerminationSignals,
    mouse_enabled: bool,
) -> Result<()> {
    let depth = terminal_color_depth();
    let _guard = TerminalGuard::enter(mouse_enabled)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let geometry = ui::frame_geometry(terminal.size()?.into());
    let result = tokio::select! {
        biased;
        event = termination.recv() => Err(terminated(event)),
        result = run_switching_session(metadata, &mut terminal, depth, geometry) => result,
    };
    // The switching session owns its EventStream and every buffered pipe. Its
    // completion or cancellation drops and joins them before restoration.
    drop(terminal);
    super::reconcile_pending_console_event(result, termination.pending_event().await)
}

async fn run_switching_session(
    metadata: &EndpointMetadata,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    depth: TerminalColorDepth,
    geometry: runyte::app::FrameGeometry,
) -> Result<()> {
    let mut attachment = connect_attachment(metadata, geometry).await?;
    terminal.resize(Rect::new(
        0,
        0,
        geometry.screen.width,
        geometry.screen.height,
    ))?;
    draw(terminal, &attachment.current, depth)?;
    let mut loop_state = FrontendLoop::new(geometry)?;

    loop {
        match run_attachment(&mut attachment, terminal, depth, &mut loop_state).await? {
            AttachmentOutcome::Ended => return Ok(()),
            AttachmentOutcome::Switch { receipt, candidate } => {
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
                    result = connect_attachment(&destination_metadata, loop_state.geometry) => result,
                };
                let destination = match destination {
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
                // The destination becomes visible before the source reservation
                // is released. A failed commit never silently presents it as
                // the current attachment.
                draw(terminal, &destination.current, depth)?;
                commit_switch(&mut attachment, receipt, switch_deadline).await?;
                attachment = destination;
                loop_state.repeats = KeyRepeatDetector::default();
            }
        }
    }
}

async fn connect_attachment(
    metadata: &EndpointMetadata,
    geometry: runyte::app::FrameGeometry,
) -> Result<Attachment> {
    // The same deadline bounds connection, Welcome and the first complete
    // decoded frame. A silent or half-speaking host never strands raw mode.
    let (mut client, current) = tokio::time::timeout(INITIAL_FRAME_BUDGET, async {
        let mut client =
            BufferedLocalClient::connect_with_handoff(metadata, geometry, false).await?;
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
    })
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
                        } else if send_or_exit(client, exit, &ClientRequest::Resynchronize).await?
                            == WireOutcome::HostEnded
                        {
                            return Ok(AttachmentOutcome::Ended);
                        }
                    }
                    Some(HostResponse::Detached { .. } | HostResponse::ShuttingDown) => return Ok(AttachmentOutcome::Ended),
                    Some(HostResponse::Refused { message } | HostResponse::Error { message }) => bail!(message),
                    Some(HostResponse::NativeSwitchPrepared { receipt, candidate }) => {
                        return Ok(AttachmentOutcome::Switch { receipt, candidate: *candidate });
                    }
                    Some(HostResponse::NativeSwitchUnchanged) => {}
                    Some(HostResponse::SwitchWorkspace { .. } | HostResponse::ParentSwitchWorkspace { .. }) => {
                        bail!("this native workspace switch route is not available yet")
                    }
                    Some(HostResponse::NativeSwitchAborted { .. } | HostResponse::NativeSwitchCommitted { .. }) => {
                        bail!("native workspace host sent an unexpected switch receipt")
                    }
                    Some(_) => {}
                    None => bail!("native workspace host disconnected without ending the attachment"),
                }
            }
            _ = exit.wait() => {
                drain_after_host_exit(client).await?;
                return Ok(AttachmentOutcome::Ended);
            }
            event = loop_state.input.next() => {
                let Some(event) = event.transpose()? else {
                    if flush_wheel(client, exit, &mut loop_state.wheels).await? == WireOutcome::HostEnded {
                        return Ok(AttachmentOutcome::Ended);
                    }
                    if send_or_exit(client, exit, &ClientRequest::Detach).await?
                        == WireOutcome::HostEnded
                    {
                        return Ok(AttachmentOutcome::Ended);
                    }
                    await_detach(client).await?;
                    return Ok(AttachmentOutcome::Ended);
                };
                if let CrosstermEvent::Resize(width, height) = event {
                    loop_state.repeats.observe(None, None, Instant::now());
                    if flush_wheel(client, exit, &mut loop_state.wheels).await? == WireOutcome::HostEnded {
                        return Ok(AttachmentOutcome::Ended);
                    }
                    loop_state.geometry = ui::frame_geometry(Rect::new(0, 0, width, height));
                    if send_or_exit(client, exit, &ClientRequest::Resize { geometry: loop_state.geometry.into() }).await?
                        == WireOutcome::HostEnded
                    {
                        return Ok(AttachmentOutcome::Ended);
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
                    if flush_wheel(client, exit, &mut loop_state.wheels).await? == WireOutcome::HostEnded {
                        return Ok(AttachmentOutcome::Ended);
                    }
                    if send_or_exit(client, exit, &ClientRequest::Notify { message }).await?
                        == WireOutcome::HostEnded
                    {
                        return Ok(AttachmentOutcome::Ended);
                    }
                    continue;
                }
                if is_passive_pointer(&event) {
                    continue;
                }
                match event {
                    InputEvent::Pointer(pointer) if is_wheel_event(pointer.kind) => {
                        if let Some(batch) = loop_state.wheels.push(pointer, current.id)
                            && send_or_exit(client, exit, &batch.request()).await?
                                == WireOutcome::HostEnded
                        {
                            return Ok(AttachmentOutcome::Ended);
                        }
                    }
                    InputEvent::Pointer(pointer) => {
                        if flush_wheel(client, exit, &mut loop_state.wheels).await? == WireOutcome::HostEnded {
                            return Ok(AttachmentOutcome::Ended);
                        }
                        if send_or_exit(client, exit, &ClientRequest::Pointer {
                            event: pointer.into(),
                            frame: current.id.into(),
                            repetitions: 1,
                        }).await? == WireOutcome::HostEnded {
                            return Ok(AttachmentOutcome::Ended);
                        }
                    }
                    event => {
                        if flush_wheel(client, exit, &mut loop_state.wheels).await? == WireOutcome::HostEnded {
                            return Ok(AttachmentOutcome::Ended);
                        }
                        if send_or_exit(client, exit, &ClientRequest::Input {
                            event: event.into(),
                            repeated,
                            presented_frame: Some(current.id.into()),
                        }).await? == WireOutcome::HostEnded {
                            return Ok(AttachmentOutcome::Ended);
                        }
                    }
                }
            }
            _ = loop_state.wheel_tick.tick(), if loop_state.wheels.0.is_some() => {
                if flush_wheel(client, exit, &mut loop_state.wheels).await? == WireOutcome::HostEnded {
                    return Ok(AttachmentOutcome::Ended);
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
            == WireOutcome::HostEnded
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
            == WireOutcome::HostEnded
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
    tokio::time::timeout_at(deadline, async {
        if send_or_exit(
            &mut attachment.client,
            &attachment.exit,
            &ClientRequest::NativeSwitchCommit { receipt },
        )
        .await?
            == WireOutcome::HostEnded
        {
            bail!("source workspace host ended before switch commit")
        }
        await_switch_receipt(attachment, receipt, true, deadline).await
    })
    .await
    .context("native switch commit exceeded its whole-operation deadline")?
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
        if matches!(
            observe_switch_receipt(response, receipt, committed)?,
            SwitchReceiptState::Complete
        ) {
            return Ok(());
        }
    }
}

fn observe_switch_receipt(
    response: HostResponse,
    receipt: u64,
    committed: bool,
) -> Result<SwitchReceiptState> {
    match response {
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
            drain_after_host_exit(client).await?;
            Ok(WireOutcome::HostEnded)
        }
        result = client.send(request) => {
            result?;
            Ok(WireOutcome::Sent)
        }
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
) -> Option<Result<()>> {
    match response {
        HostResponse::Error { message } | HostResponse::Refused { message } => {
            first_error.get_or_insert(message);
            None
        }
        HostResponse::ShuttingDown | HostResponse::Detached { .. } => Some(
            first_error
                .take()
                .map_or(Ok(()), |message| Err(anyhow!(message))),
        ),
        _ => None,
    }
}

async fn await_detach(client: &mut BufferedLocalClient) -> Result<()> {
    let deadline = tokio::time::Instant::now() + FINAL_REPLY_BUDGET;
    for _ in 0..FINAL_DRAIN_MESSAGES {
        match tokio::time::timeout_at(deadline, client.recv()).await {
            Ok(Ok(Some(HostResponse::Detached { .. } | HostResponse::ShuttingDown))) => {
                return Ok(());
            }
            Ok(Ok(Some(HostResponse::Error { message } | HostResponse::Refused { message }))) => {
                bail!(message)
            }
            Ok(Ok(Some(_))) => {}
            Ok(Ok(None)) => bail!("native workspace host disconnected before acknowledging detach"),
            Ok(Err(error)) => return Err(error).context("native workspace detach reply failed"),
            Err(_) => bail!("native workspace host did not acknowledge detach"),
        }
    }
    bail!("native workspace host did not acknowledge detach")
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
            RegistrySet::open(&[root.join("registry")]).unwrap(),
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
        }
        assert!(
            observe_switch_receipt(HostResponse::NativeSwitchCommitted { receipt: 8 }, 9, true,)
                .is_err()
        );
    }
}
