// SPDX-License-Identifier: MPL-2.0
//! Adapt the GPUI frontend to the shared CLI's window boundary.

use super::*;
use runyte::cli::window::{WindowEvents, WindowFrontend, WindowSurface};

pub struct Window;
pub static WINDOW: Window = Window;

impl WindowFrontend for Window {
    fn acknowledge_input_barrier(&self, serial: u64) {
        if let Some(bridge) = BRIDGE.get() {
            bridge.remote_serial.store(serial, Ordering::Release);
        }
    }
    fn launch(
        &self,
        worker: Box<dyn FnOnce() -> anyhow::Result<()> + Send>,
        font_size: usize,
    ) -> anyhow::Result<()> {
        launch(worker, font_size)
    }
    fn system_clipboard(&self) -> Option<Box<dyn runyte::clipboard::SystemClipboard>> {
        #[cfg(not(windows))]
        {
            Some(Box::new(crate::clipboard::NativeClipboard::default()))
        }
        #[cfg(windows)]
        {
            None
        }
    }
    fn surface(&self) -> io::Result<Box<dyn WindowSurface>> {
        Surface::new(CrosstermBackend::new(io::stdout()), true)
            .map(|surface| Box::new(surface) as _)
    }
    fn events(&self) -> Box<dyn WindowEvents> {
        Box::new(Events::new(true))
    }
    fn dimensions(&self) -> (u16, u16) {
        dimensions()
    }
    fn take_close_request(&self) -> bool {
        take_close_request()
    }
    fn begin_attachment(&self) {
        begin_attachment();
    }
    fn update_media(&self, app: &mut runyte::app::App) {
        update_media(app);
    }
    fn capture_media(
        &self,
        snapshot: &runyte::workspace::HostFrame,
        app: &mut runyte::app::App,
        hints: &runyte::key_hints::KeyHintState,
    ) {
        capture_media(snapshot, app, hints);
    }
    fn render_frame(
        &self,
        frame: &mut ratatui::Frame<'_>,
        app: &runyte::app::App,
        snapshot: &runyte::workspace::HostFrame,
        depth: runyte::ui::TerminalColorDepth,
    ) {
        render_frame(frame, app, snapshot, depth);
    }
    fn attached_media_requests(
        &self,
        snapshot: &runyte::workspace::HostFrame,
    ) -> Vec<runyte::protocol::ClientRequest> {
        attached_media_requests(snapshot)
    }
    fn receive_media_action(
        &self,
        frame: runyte::workspace::FrameId,
        request: runyte::media::ViewRequest,
    ) {
        receive_media_action(frame, request);
    }
}

impl WindowSurface for Surface {
    fn resize(&mut self, area: ratatui::layout::Rect) -> io::Result<()> {
        Surface::resize(self, area)
    }
    fn draw(&mut self, draw: &mut dyn FnMut(&mut ratatui::Frame<'_>)) -> io::Result<()> {
        Surface::draw(self, draw)
    }
    fn draw_host_frame(
        &mut self,
        snapshot: &runyte::workspace::HostFrame,
        depth: runyte::ui::TerminalColorDepth,
    ) -> io::Result<()> {
        Surface::draw_host_frame(self, snapshot, depth)
    }
}

impl WindowEvents for Events {
    fn input_barrier(&self) -> u64 {
        match self {
            Self::Native { routing_serial, .. } => *routing_serial,
            Self::Tui(_) => 0,
        }
    }
    fn next(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<io::Result<Event>>> + '_>> {
        Box::pin(Events::next(self))
    }
    fn defer_frame(&mut self, applied: bool, pending: &mut bool) -> bool {
        Events::defer_frame(self, applied, pending)
    }
    fn presented_frame(
        &self,
        current: Option<runyte::workspace::FrameId>,
    ) -> Option<runyte::workspace::FrameId> {
        Events::presented_frame(self, current)
    }
    fn is_presentation_acknowledgement(&self) -> bool {
        Events::is_presentation_acknowledgement(self)
    }
    fn accepts_input(
        &self,
        host: &runyte::workspace::WorkspaceHost,
        input: &runyte::input::InputEvent,
        pending: bool,
    ) -> bool {
        Events::accepts_input(self, host, input, pending)
    }
}
