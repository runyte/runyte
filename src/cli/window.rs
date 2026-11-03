// SPDX-License-Identifier: MPL-2.0
//! The bundled window adapter. Terminal rendering stays concrete in this module.

#[cfg(test)]
#[path = "tests/window.rs"]
mod tests;

use std::{future::Future, io, pin::Pin};

use crossterm::event::Event;
#[cfg(not(windows))]
use crossterm::event::EventStream;
#[cfg(not(windows))]
use futures_util::StreamExt;
use ratatui::{Frame, Terminal, backend::CrosstermBackend};

use crate::{
    app::App,
    clipboard::SystemClipboard,
    input::InputEvent,
    key_hints::KeyHintState,
    media::ViewRequest,
    protocol::ClientRequest,
    ui::TerminalColorDepth,
    workspace::{FrameId, HostFrame, WorkspaceHost},
};

/// Bundled frontend services supplied explicitly by the edition executable.
pub trait WindowFrontend: Sync {
    fn acknowledge_input_barrier(&self, _serial: u64) {}
    fn launch(
        &self,
        worker: Box<dyn FnOnce() -> anyhow::Result<()> + Send>,
        font_size: usize,
    ) -> anyhow::Result<()>;
    fn system_clipboard(&self) -> Option<Box<dyn SystemClipboard>>;
    fn surface(&self) -> io::Result<Box<dyn WindowSurface>>;
    fn events(&self) -> Box<dyn WindowEvents>;
    fn dimensions(&self) -> (u16, u16);
    fn take_close_request(&self) -> bool;
    fn begin_attachment(&self);
    fn update_media(&self, app: &mut App);
    fn capture_media(&self, snapshot: &HostFrame, app: &mut App, hints: &KeyHintState);
    fn render_frame(
        &self,
        frame: &mut Frame<'_>,
        app: &App,
        snapshot: &HostFrame,
        depth: TerminalColorDepth,
    );
    fn attached_media_requests(&self, snapshot: &HostFrame) -> Vec<ClientRequest>;
    fn receive_media_action(&self, frame: FrameId, request: ViewRequest);
}

/// Object-safe window rendering, with no allocation required for draw callbacks.
pub trait WindowSurface {
    fn resize(&mut self, area: ratatui::layout::Rect) -> io::Result<()>;
    fn draw(&mut self, draw: &mut dyn FnMut(&mut Frame<'_>)) -> io::Result<()>;
    fn draw_host_frame(
        &mut self,
        snapshot: &HostFrame,
        depth: TerminalColorDepth,
    ) -> io::Result<()>;
}

/// Window input carries the identity of the frame physically presented.
pub trait WindowEvents {
    fn input_barrier(&self) -> u64 {
        0
    }
    fn next(&mut self) -> Pin<Box<dyn Future<Output = Option<io::Result<Event>>> + '_>>;
    fn defer_frame(&mut self, applied_key_or_text: bool, pending: &mut bool) -> bool;
    fn presented_frame(&self, synchronous: Option<FrameId>) -> Option<FrameId>;
    fn is_presentation_acknowledgement(&self) -> bool;
    fn accepts_input(&self, host: &WorkspaceHost, input: &InputEvent, pending: bool) -> bool;
}

pub(super) enum Surface {
    Terminal(Terminal<CrosstermBackend<io::Stdout>>),
    Window(Box<dyn WindowSurface>),
}

impl Surface {
    pub(super) fn new(
        backend: CrosstermBackend<io::Stdout>,
        window: Option<&'static dyn WindowFrontend>,
    ) -> io::Result<Self> {
        match window {
            Some(window) => window.surface().map(Self::Window),
            None => Terminal::new(backend).map(Self::Terminal),
        }
    }

    #[cfg(unix)]
    pub(super) fn resize(&mut self, area: ratatui::layout::Rect) -> io::Result<()> {
        match self {
            Self::Terminal(terminal) => terminal.resize(area),
            Self::Window(window) => window.resize(area),
        }
    }

    pub(super) fn draw(&mut self, draw: impl FnOnce(&mut Frame<'_>)) -> io::Result<()> {
        match self {
            Self::Terminal(terminal) => {
                terminal.draw(draw)?;
                Ok(())
            }
            Self::Window(window) => {
                let mut draw = Some(draw);
                window.draw(&mut |frame| draw.take().expect("one draw per frame")(frame))
            }
        }
    }

    #[cfg(unix)]
    pub(super) fn draw_host_frame(
        &mut self,
        snapshot: &HostFrame,
        depth: TerminalColorDepth,
    ) -> io::Result<()> {
        match self {
            Self::Terminal(terminal) => {
                terminal.draw(|frame| crate::ui::render_host_frame(frame, snapshot, depth))?;
                Ok(())
            }
            Self::Window(window) => window.draw_host_frame(snapshot, depth),
        }
    }
}

/// Terminal input read as soon as the terminal delivers it.
///
/// Crossterm tells the Esc key from the start of an Alt chord by what a single
/// read returns: Escape followed by another byte in the same buffer is an Alt
/// chord, so `Esc` then `:` read together become one `Alt-:`. An `EventStream`
/// reads the terminal only when its owner asks for an event and every event
/// from the previous read has been taken. An editor loop busy with earlier
/// input would leave later keystrokes queued in the kernel until it asked
/// again, and an Escape typed before a busy stretch ended would then share a
/// read with the key typed after it. Draining the stream on a thread that does
/// nothing else gives each burst the terminal writes its own read, however
/// long the editor takes to handle it.
///
/// Dropping the value closes the channel. The thread notices when it next
/// runs and drops the stream, which stops Crossterm's helper, so reading stops
/// shortly after the owner is gone; keys already read by then are discarded.
/// After a PTY hangup Crossterm can loop inside a read that never returns. The
/// thread is detached so that only it, and never the editor loop, is held
/// there until the process exits.
#[cfg(not(windows))]
pub(super) struct TerminalInput {
    events: tokio::sync::mpsc::UnboundedReceiver<io::Result<Event>>,
}

#[cfg(not(windows))]
impl TerminalInput {
    pub(super) fn new() -> io::Result<Self> {
        let (sender, events) = tokio::sync::mpsc::unbounded_channel();
        let stream = EventStream::new();
        std::thread::Builder::new()
            .name("runyte-terminal-input".into())
            .spawn(move || {
                // Built here rather than by the caller: a runtime dropped on
                // the editor's own runtime thread would panic. It drives only
                // this task, and Crossterm's helper thread waits for the
                // terminal and wakes it, so it needs no IO or timer driver.
                match tokio::runtime::Builder::new_current_thread().build() {
                    Ok(runtime) => runtime.block_on(forward_terminal_input(stream, sender)),
                    Err(error) => {
                        let _ = sender.send(Err(error));
                    }
                }
            })?;
        Ok(Self { events })
    }

    pub(super) async fn next(&mut self) -> Option<io::Result<Event>> {
        self.events.recv().await
    }
}

#[cfg(not(windows))]
async fn forward_terminal_input(
    mut stream: EventStream,
    sender: tokio::sync::mpsc::UnboundedSender<io::Result<Event>>,
) {
    loop {
        let event = tokio::select! {
            biased;
            () = sender.closed() => return,
            event = stream.next() => event,
        };
        let Some(event) = event else {
            return;
        };
        let failed = event.is_err();
        if sender.send(event).is_err() || failed {
            return;
        }
    }
}

#[cfg(not(windows))]
pub(super) enum Events {
    Terminal(TerminalInput),
    Window(Box<dyn WindowEvents>),
}

#[cfg(not(windows))]
impl Events {
    pub(super) fn input_barrier(&self) -> u64 {
        match self {
            Self::Terminal(_) => 0,
            Self::Window(window) => window.input_barrier(),
        }
    }
    pub(super) fn new(window: Option<&'static dyn WindowFrontend>) -> io::Result<Self> {
        Ok(match window {
            Some(window) => Self::Window(window.events()),
            None => Self::Terminal(TerminalInput::new()?),
        })
    }
    pub(super) async fn next(&mut self) -> Option<io::Result<Event>> {
        match self {
            Self::Terminal(input) => input.next().await,
            Self::Window(window) => window.next().await,
        }
    }
    pub(super) fn defer_frame(&mut self, applied: bool, pending: &mut bool) -> bool {
        match self {
            Self::Terminal(_) => false,
            Self::Window(window) => window.defer_frame(applied, pending),
        }
    }
    pub(super) fn presented_frame(&self, current: Option<FrameId>) -> Option<FrameId> {
        match self {
            Self::Terminal(_) => current,
            Self::Window(window) => window.presented_frame(current),
        }
    }
    pub(super) fn is_presentation_acknowledgement(&self) -> bool {
        match self {
            Self::Terminal(_) => false,
            Self::Window(window) => window.is_presentation_acknowledgement(),
        }
    }
    pub(super) fn accepts_input(
        &self,
        host: &WorkspaceHost,
        input: &InputEvent,
        pending: bool,
    ) -> bool {
        match self {
            Self::Terminal(_) => true,
            Self::Window(window) => window.accepts_input(host, input, pending),
        }
    }
}
