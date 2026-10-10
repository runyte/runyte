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

#[cfg(not(windows))]
pub(super) enum Events {
    Terminal(EventStream),
    Window(Box<dyn WindowEvents>),
}

#[cfg(not(windows))]
impl Events {
    pub(super) fn new(window: Option<&'static dyn WindowFrontend>) -> Self {
        match window {
            Some(window) => Self::Window(window.events()),
            None => Self::Terminal(EventStream::new()),
        }
    }
    pub(super) async fn next(&mut self) -> Option<io::Result<Event>> {
        match self {
            Self::Terminal(stream) => stream.next().await,
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
