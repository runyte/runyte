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
    protocol::{MAX_POINTER_REPETITIONS, validate_welcome},
    tui::{input::convert_event, windows_input::EventStream},
    ui::{self, TerminalColorDepth},
    workspace::{
        FrameId, HostFrame, NativeHostExit,
        windows_endpoint::EndpointMetadata,
        windows_transport::{BufferedLocalClient, ClientRequest, HostResponse},
    },
};
use std::{
    io::stdout,
    time::{Duration, Instant},
};

const INITIAL_FRAME_BUDGET: Duration = Duration::from_secs(5);
const FINAL_REPLY_BUDGET: Duration = Duration::from_secs(3);
// The response reader may retain 64 semantic messages, one final message and
// one coalesced visual frame when the process-exit notification arrives.
const FINAL_DRAIN_MESSAGES: usize = 128;

#[derive(Clone, Copy, Eq, PartialEq)]
enum WireOutcome {
    Sent,
    HostEnded,
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
    let result = tokio::select! {
        biased;
        event = termination.recv() => Err(terminated(event)),
        result = run_attachment(metadata, &mut terminal, depth) => result,
    };
    // `run_attachment` owns its EventStream and buffered pipe. Its completion
    // or cancellation drops and joins both before terminal restoration.
    drop(terminal);
    super::reconcile_pending_console_event(result, termination.pending_event().await)
}

async fn run_attachment(
    metadata: &EndpointMetadata,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    depth: TerminalColorDepth,
) -> Result<()> {
    // The same deadline bounds connection, Welcome and the first complete
    // decoded frame. A silent or half-speaking host never strands raw mode.
    let geometry = ui::frame_geometry(terminal.size()?.into());
    let (mut client, mut current) = tokio::time::timeout(INITIAL_FRAME_BUDGET, async {
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
        return drain_after_host_exit(&mut client).await;
    }
    // Reset Ratatui's empty back buffer without asking the terminal for its
    // cursor position; the input stream could consume that query's reply.
    terminal.resize(Rect::new(
        0,
        0,
        geometry.screen.width,
        geometry.screen.height,
    ))?;
    draw(terminal, &current, depth)?;
    let mut input = EventStream::new()?;
    let mut repeats = KeyRepeatDetector::default();
    let mut wheels = WheelBatcher::default();
    let mut wheel_tick = tokio::time::interval(Duration::from_millis(8));
    wheel_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            response = client.recv() => {
                match response? {
                    Some(HostResponse::Frame { frame }) => {
                        current = (*frame).try_into().map_err(|error: String| anyhow!(error))?;
                        draw(terminal, &current, depth)?;
                    }
                    Some(HostResponse::TerminalDamage { damage }) => {
                        if apply_damage(&mut current, &damage)? {
                            draw(terminal, &current, depth)?;
                        } else if send_or_exit(&mut client, &exit, &ClientRequest::Resynchronize).await?
                            == WireOutcome::HostEnded
                        {
                            return Ok(());
                        }
                    }
                    Some(HostResponse::Detached { .. } | HostResponse::ShuttingDown) => return Ok(()),
                    Some(HostResponse::Refused { message } | HostResponse::Error { message }) => bail!(message),
                    Some(HostResponse::SwitchWorkspace { .. } | HostResponse::ParentSwitchWorkspace { .. }) => {
                        bail!("native workspace switching is not available yet")
                    }
                    Some(_) => {}
                    None => bail!("native workspace host disconnected without ending the attachment"),
                }
            }
            _ = exit.wait() => return drain_after_host_exit(&mut client).await,
            event = input.next() => {
                let Some(event) = event.transpose()? else {
                    if flush_wheel(&mut client, &exit, &mut wheels).await? == WireOutcome::HostEnded {
                        return Ok(());
                    }
                    if send_or_exit(&mut client, &exit, &ClientRequest::Detach).await?
                        == WireOutcome::HostEnded
                    {
                        return Ok(());
                    }
                    return await_detach(&mut client).await;
                };
                if let CrosstermEvent::Resize(width, height) = event {
                    repeats.observe(None, None, Instant::now());
                    if flush_wheel(&mut client, &exit, &mut wheels).await? == WireOutcome::HostEnded {
                        return Ok(());
                    }
                    let geometry = ui::frame_geometry(Rect::new(0, 0, width, height));
                    if send_or_exit(&mut client, &exit, &ClientRequest::Resize { geometry: geometry.into() }).await?
                        == WireOutcome::HostEnded
                    {
                        return Ok(());
                    }
                    continue;
                }
                let kind = terminal_key_kind(&event);
                let Some(event) = convert_native_event(event)? else {
                    repeats.observe(kind, None, Instant::now());
                    continue;
                };
                let repeated = repeats.observe(kind, Some(&event), Instant::now());
                if let Some(message) = rejected_text_input(&event) {
                    if flush_wheel(&mut client, &exit, &mut wheels).await? == WireOutcome::HostEnded {
                        return Ok(());
                    }
                    if send_or_exit(&mut client, &exit, &ClientRequest::Notify { message }).await?
                        == WireOutcome::HostEnded
                    {
                        return Ok(());
                    }
                    continue;
                }
                if is_passive_pointer(&event) {
                    continue;
                }
                match event {
                    InputEvent::Pointer(pointer) if is_wheel_event(pointer.kind) => {
                        if let Some(batch) = wheels.push(pointer, current.id)
                            && send_or_exit(&mut client, &exit, &batch.request()).await?
                                == WireOutcome::HostEnded
                        {
                            return Ok(());
                        }
                    }
                    InputEvent::Pointer(pointer) => {
                        if flush_wheel(&mut client, &exit, &mut wheels).await? == WireOutcome::HostEnded {
                            return Ok(());
                        }
                        if send_or_exit(&mut client, &exit, &ClientRequest::Pointer {
                            event: pointer.into(),
                            frame: current.id.into(),
                            repetitions: 1,
                        }).await? == WireOutcome::HostEnded {
                            return Ok(());
                        }
                    }
                    event => {
                        if flush_wheel(&mut client, &exit, &mut wheels).await? == WireOutcome::HostEnded {
                            return Ok(());
                        }
                        if send_or_exit(&mut client, &exit, &ClientRequest::Input {
                            event: event.into(),
                            repeated,
                            presented_frame: Some(current.id.into()),
                        }).await? == WireOutcome::HostEnded {
                            return Ok(());
                        }
                    }
                }
            }
            _ = wheel_tick.tick(), if wheels.0.is_some() => {
                if flush_wheel(&mut client, &exit, &mut wheels).await? == WireOutcome::HostEnded {
                    return Ok(());
                }
            }
        }
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
}
