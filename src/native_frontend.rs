// SPDX-License-Identifier: MPL-2.0

//! GPUI owns the main thread; the existing host loop owns all editor state.
//! The bridge retains only the latest owned frame, never a queue of frames.

mod icon;
mod interactions;
mod media;
mod viewport;

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures_util::StreamExt;
use gpui::{prelude::*, *};
use ratatui::{
    Terminal,
    backend::{CrosstermBackend, TestBackend},
    buffer::Buffer,
    style::{Color, Modifier},
};
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tokio::sync::mpsc;

const CELL_WIDTH: f32 = 9.0;
const CELL_HEIGHT: f32 = 20.0;

#[derive(Clone)]
struct MediaPane {
    pane: usize,
    path: PathBuf,
    page: usize,
    body: runyte::layout::Rect,
}
#[derive(Clone)]
struct FrameData {
    attachment: u64,
    id: Option<runyte::workspace::FrameId>,
    cells: Buffer,
    media: Vec<MediaPane>,
    cursor: Option<ratatui::layout::Position>,
    overlays: Vec<runyte::layout::Rect>,
    media_input: bool,
    metadata_paths: Vec<PathBuf>,
}
impl FrameData {
    fn under_media(&self, x: u16, y: u16) -> bool {
        let contains = |r: &runyte::layout::Rect| {
            x >= r.x
                && x < r.x.saturating_add(r.width)
                && y >= r.y
                && y < r.y.saturating_add(r.height)
        };
        self.media.iter().any(|pane| contains(&pane.body)) && !self.overlays.iter().any(contains)
    }
}
#[derive(Default)]
struct MediaInputMask {
    blocked: Vec<runyte::layout::Rect>,
}
impl MediaInputMask {
    fn blocks(&self, position: Point<Pixels>) -> bool {
        let x = f32::from(position.x) / CELL_WIDTH;
        let y = f32::from(position.y) / CELL_HEIGHT;
        let contains = |r: &runyte::layout::Rect| {
            x >= r.x as f32
                && x < (r.x + r.width) as f32
                && y >= r.y as f32
                && y < (r.y + r.height) as f32
        };
        self.blocked.iter().any(contains)
    }
}
pub struct NativeInput {
    attachment: u64,
    event: Event,
    presented: Option<runyte::workspace::FrameId>,
    presentation_only: bool,
}
struct PendingMediaRequest {
    attachment: u64,
    frame: runyte::workspace::FrameId,
    request: runyte::media::ViewRequest,
}

/// Semantic replies can overtake coalesced visuals. Keep each action until
/// its target frame (or a newer complete replacement) is available to GPUI.
fn ready_media_requests(
    pending: &mut std::collections::VecDeque<PendingMediaRequest>,
    frame: Option<&FrameData>,
    attachment: u64,
) -> Vec<runyte::media::ViewRequest> {
    let mut ready = Vec::new();
    for _ in 0..pending.len() {
        let request = pending.pop_front().unwrap();
        if request.attachment != attachment {
            continue;
        }
        if frame.is_some_and(|frame| {
            frame.attachment == attachment && frame.id.is_some_and(|id| id >= request.frame)
        }) {
            ready.push(request.request);
        } else {
            pending.push_back(request);
        }
    }
    ready
}

struct Bridge {
    attachment: Arc<AtomicU64>,
    painted_attachment: AtomicU64,
    prepared: Mutex<Option<runyte::workspace::FrameId>>,
    presented: Mutex<Option<runyte::workspace::FrameId>>,
    frame: Mutex<Option<FrameData>>,
    media: Mutex<Vec<MediaPane>>,
    painted_media: Mutex<Vec<MediaPane>>,
    overlays: Mutex<Vec<runyte::layout::Rect>>,
    media_input: AtomicBool,
    metadata_paths: Mutex<Vec<PathBuf>>,
    blocked_media: Mutex<MediaInputMask>,
    media_requests: Mutex<std::collections::VecDeque<PendingMediaRequest>>,
    media_pointer: Mutex<Vec<(u64, usize, PathBuf, i32)>>,
    media_back: Mutex<Vec<(u64, usize, PathBuf, usize)>>,
    dimensions: Mutex<(u16, u16)>,
    input: mpsc::Sender<NativeInput>,
    receiver: Mutex<Option<mpsc::Receiver<NativeInput>>>,
    pages: Mutex<Vec<media::PageCount>>,
    close_requested: AtomicBool,
    done: AtomicBool,
    wake: async_channel::Sender<()>,
    wakes: async_channel::Receiver<()>,
}
impl Bridge {
    fn send(&self, event: Event) {
        let input = NativeInput {
            attachment: self.painted_attachment.load(Ordering::Acquire),
            event,
            presented: *self.presented.lock().unwrap(),
            presentation_only: false,
        };
        if self.input.try_send(input).is_err() {
            eprintln!("Runyte native input queue is full");
        }
    }
}
static BRIDGE: OnceLock<Arc<Bridge>> = OnceLock::new();

pub fn take_close_request() -> bool {
    BRIDGE
        .get()
        .is_some_and(|bridge| bridge.close_requested.swap(false, Ordering::AcqRel))
}

pub fn update_media(app: &mut runyte::app::App) {
    let Some(bridge) = BRIDGE.get() else { return };
    for (attachment, pane, path, page) in bridge.media_back.lock().unwrap().drain(..) {
        if attachment != bridge.attachment.load(Ordering::Acquire) {
            continue;
        }
        app.leave_native_media(pane, &path, page);
    }
    for (attachment, pane, path, delta) in bridge.media_pointer.lock().unwrap().drain(..) {
        if attachment != bridge.attachment.load(Ordering::Acquire) {
            continue;
        }
        app.navigate_native_media(pane, &path, delta);
    }
    let counts = bridge.pages.lock().unwrap().clone();
    for (path, modified, length, pages) in &counts {
        let metadata = path.metadata().ok();
        if metadata.as_ref().and_then(|m| m.modified().ok()) == *modified
            && metadata.as_ref().map_or(0, |m| m.len()) == *length
        {
            app.update_native_media_pages(path, *pages);
        }
    }
}

pub fn capture_media(
    snapshot: &runyte::workspace::HostFrame,
    app: &mut runyte::app::App,
    hints: &runyte::key_hints::KeyHintState,
) {
    let Some(bridge) = BRIDGE.get() else { return };
    {
        let mut requests = bridge.media_requests.lock().unwrap();
        for request in app.media_requests.drain(..) {
            if requests.len() < 256 {
                requests.push_back(PendingMediaRequest {
                    attachment: bridge.attachment.load(Ordering::Acquire),
                    frame: snapshot.id,
                    request,
                });
            }
        }
    }
    bridge.media_input.store(
        !app.has_input_overlay()
            && app.mode != runyte::app::Mode::Command
            && snapshot.overlays.is_empty()
            && !hints.is_visible(),
        Ordering::Release,
    );
    *bridge.overlays.lock().unwrap() =
        runyte::ui::overlay_rectangles(&snapshot.editor, &snapshot.overlays);
    let counts = bridge.pages.lock().unwrap().clone();
    capture_snapshot_media(snapshot, &counts);
}

fn capture_snapshot_media(snapshot: &runyte::workspace::HostFrame, counts: &[media::PageCount]) {
    let Some(bridge) = BRIDGE.get() else { return };
    *bridge.overlays.lock().unwrap() =
        runyte::ui::overlay_rectangles(&snapshot.editor, &snapshot.overlays);
    *bridge.metadata_paths.lock().unwrap() = snapshot
        .editor
        .panes
        .iter()
        .filter_map(|pane| pane.media.as_ref())
        .filter(|media| media.page_buffer)
        .filter_map(|media| {
            let metadata = media.path.metadata().ok();
            let modified = metadata.as_ref().and_then(|m| m.modified().ok());
            let length = metadata.map_or(0, |m| m.len());
            (!counts
                .iter()
                .any(|count| count.0 == media.path && count.1 == modified && count.2 == length))
            .then(|| media.path.clone())
        })
        .collect();
    *bridge.media.lock().unwrap() = snapshot
        .editor
        .panes
        .iter()
        .filter_map(|pane| {
            let media = pane.media.as_ref()?;
            (pane.drawable && !media.page_buffer).then(|| MediaPane {
                pane: pane.pane_id,
                path: media.path.clone(),
                page: media.page,
                body: pane.body,
            })
        })
        .collect();
    *bridge.prepared.lock().unwrap() = Some(snapshot.id);
}

/// Send navigation and fresh cached page counts for the displayed projections.
pub fn attached_media_requests(
    snapshot: &runyte::workspace::HostFrame,
) -> Vec<runyte::protocol::ClientRequest> {
    use runyte::protocol::{ClientRequest, encode_path};
    let Some(bridge) = BRIDGE.get() else {
        return Vec::new();
    };
    let mut requests = Vec::new();
    for (attachment, pane, path, page) in bridge.media_back.lock().unwrap().drain(..) {
        if attachment != bridge.attachment.load(Ordering::Acquire) {
            continue;
        }
        requests.push(ClientRequest::MediaBack {
            pane,
            path: encode_path(&path),
            page,
        });
    }
    for (attachment, pane, path, delta) in bridge.media_pointer.lock().unwrap().drain(..) {
        if attachment != bridge.attachment.load(Ordering::Acquire) {
            continue;
        }
        requests.push(ClientRequest::MediaNavigate {
            pane,
            path: encode_path(&path),
            delta,
        });
    }
    let counts = bridge.pages.lock().unwrap().clone();
    for count in &counts {
        let (path, modified, length, pages) = count;
        let metadata = path.metadata().ok();
        if snapshot
            .editor
            .panes
            .iter()
            .filter_map(|pane| pane.media.as_ref())
            .any(|media| media.path == *path && media.pages != *pages)
            && metadata.as_ref().and_then(|m| m.modified().ok()) == *modified
            && metadata.as_ref().map_or(0, |m| m.len()) == *length
        {
            requests.push(ClientRequest::MediaPages {
                path: encode_path(path),
                pages: *pages,
            });
        }
    }
    requests
}

pub fn begin_attachment() {
    let Some(bridge) = BRIDGE.get() else { return };
    bridge.attachment.fetch_add(1, Ordering::AcqRel);
    bridge.media_requests.lock().unwrap().clear();
    bridge.media_pointer.lock().unwrap().clear();
    bridge.media_back.lock().unwrap().clear();
    bridge.media.lock().unwrap().clear();
    bridge.painted_media.lock().unwrap().clear();
    *bridge.presented.lock().unwrap() = None;
}

pub fn receive_media_action(
    frame: runyte::workspace::FrameId,
    request: runyte::media::ViewRequest,
) {
    let Some(bridge) = BRIDGE.get() else { return };
    let mut requests = bridge.media_requests.lock().unwrap();
    if requests.len() < 256 {
        requests.push_back(PendingMediaRequest {
            attachment: bridge.attachment.load(Ordering::Acquire),
            frame,
            request,
        });
    }
    let _ = bridge.wake.try_send(());
}

pub fn dimensions() -> (u16, u16) {
    *BRIDGE.get().unwrap().dimensions.lock().unwrap()
}

pub fn capture_attached(snapshot: &runyte::workspace::HostFrame) {
    let Some(bridge) = BRIDGE.get() else { return };
    bridge.media_input.store(
        snapshot.editor.mode != runyte::app::Mode::Command && snapshot.overlays.is_empty(),
        Ordering::Release,
    );
    let counts = bridge.pages.lock().unwrap().clone();
    capture_snapshot_media(snapshot, &counts);
}

pub fn render_frame(
    frame: &mut ratatui::Frame<'_>,
    app: &runyte::app::App,
    snapshot: &runyte::workspace::HostFrame,
    color_depth: runyte::ui::TerminalColorDepth,
) {
    let session = if app.is_editor_mode() {
        runyte::ui::SessionMode::Editor
    } else {
        runyte::ui::SessionMode::Standalone
    };
    runyte::ui::render_native_frame(frame, snapshot, session, color_depth);
}

pub enum Surface {
    Tui(Terminal<CrosstermBackend<io::Stdout>>),
    Native(Terminal<TestBackend>),
}
impl Surface {
    pub fn new(backend: CrosstermBackend<io::Stdout>, native: bool) -> io::Result<Self> {
        if native {
            Ok(Self::Native(
                Terminal::new(TestBackend::new(120, 40)).unwrap(),
            ))
        } else {
            Terminal::new(backend).map(Self::Tui)
        }
    }
    pub fn resize(&mut self, area: ratatui::layout::Rect) -> io::Result<()> {
        match self {
            Self::Tui(terminal) => terminal.resize(area),
            Self::Native(terminal) => {
                terminal.backend_mut().resize(area.width, area.height);
                terminal.resize(area).unwrap();
                Ok(())
            }
        }
    }
    pub fn draw(&mut self, draw: impl FnOnce(&mut ratatui::Frame<'_>)) -> io::Result<()> {
        match self {
            Self::Tui(terminal) => {
                terminal.draw(draw)?;
            }
            Self::Native(terminal) => {
                let bridge = BRIDGE.get().unwrap();
                let (width, height) = *bridge.dimensions.lock().unwrap();
                terminal.backend_mut().resize(width, height);
                terminal.draw(draw).unwrap();
                *bridge.frame.lock().unwrap() = Some(FrameData {
                    attachment: bridge.attachment.load(Ordering::Acquire),
                    id: *bridge.prepared.lock().unwrap(),
                    cells: terminal.backend().buffer().clone(),
                    media: bridge.media.lock().unwrap().clone(),
                    overlays: bridge.overlays.lock().unwrap().clone(),
                    media_input: bridge.media_input.load(Ordering::Acquire),
                    metadata_paths: bridge.metadata_paths.lock().unwrap().clone(),
                    cursor: terminal
                        .backend()
                        .cursor_visible()
                        .then(|| terminal.backend().cursor_position()),
                });
                let _ = bridge.wake.try_send(());
            }
        }
        Ok(())
    }
}

pub enum Events {
    Tui(EventStream),
    Native {
        attachment: Arc<AtomicU64>,
        events: mpsc::Receiver<NativeInput>,
        presented: Option<runyte::workspace::FrameId>,
        presentation_only: bool,
    },
}
impl Events {
    pub fn new(native: bool) -> Self {
        if native {
            Self::Native {
                attachment: BRIDGE.get().unwrap().attachment.clone(),
                events: BRIDGE
                    .get()
                    .unwrap()
                    .receiver
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap(),
                presented: None,
                presentation_only: false,
            }
        } else {
            Self::Tui(EventStream::new())
        }
    }
    pub async fn next(&mut self) -> Option<io::Result<Event>> {
        match self {
            Self::Tui(events) => events.next().await,
            Self::Native {
                attachment,
                events,
                presented,
                presentation_only,
            } => {
                *presentation_only = false;
                let input = loop {
                    let input = events.recv().await?;
                    // Geometry and close wakeups describe the window, not a
                    // document in the previously painted persistent session.
                    if matches!(input.event, Event::Resize(..))
                        || input.attachment == attachment.load(Ordering::Acquire)
                    {
                        break input;
                    }
                };
                *presented = input.presented;
                *presentation_only = input.presentation_only;
                Some(Ok(input.event))
            }
        }
    }
    /// The GUI may lag or coalesce host frames. Never acknowledge a newer frame
    /// than the one painted when this physical input was captured.
    pub fn presented_frame(
        &self,
        synchronous: Option<runyte::workspace::FrameId>,
    ) -> Option<runyte::workspace::FrameId> {
        match self {
            Self::Tui(_) => synchronous,
            Self::Native { presented, .. } => *presented,
        }
    }

    pub fn is_presentation_acknowledgement(&self) -> bool {
        matches!(
            self,
            Self::Native {
                presentation_only: true,
                ..
            }
        )
    }

    /// Approvals require the frame painted when their physical input arrived.
    /// Editing remains queued; plugin actions check their captured revisions
    /// at the core invocation boundary, regardless of their key or menu path.
    pub fn accepts_input(
        &self,
        host: &runyte::workspace::WorkspaceHost,
        input: &runyte::input::InputEvent,
        publication_pending: bool,
    ) -> bool {
        if matches!(self, Self::Tui(_)) {
            return true;
        }
        host.accepts_frontend_input(input, self.presented_frame(None), publication_pending)
    }
}

pub fn launch(worker: fn() -> anyhow::Result<()>) -> anyhow::Result<()> {
    let (input, receiver) = mpsc::channel(4096);
    let (wake, wakes) = async_channel::bounded(1);
    let bridge = Arc::new(Bridge {
        attachment: Arc::new(AtomicU64::new(0)),
        painted_attachment: AtomicU64::new(0),
        prepared: Mutex::new(None),
        presented: Mutex::new(None),
        frame: Mutex::new(None),
        media: Mutex::new(Vec::new()),
        painted_media: Mutex::new(Vec::new()),
        overlays: Mutex::new(Vec::new()),
        media_input: AtomicBool::new(true),
        metadata_paths: Mutex::new(Vec::new()),
        blocked_media: Mutex::new(MediaInputMask::default()),
        media_requests: Mutex::new(Default::default()),
        media_pointer: Mutex::new(Vec::new()),
        media_back: Mutex::new(Vec::new()),
        dimensions: Mutex::new((120, 40)),
        input,
        receiver: Mutex::new(Some(receiver)),
        pages: Mutex::new(Vec::new()),
        close_requested: AtomicBool::new(false),
        done: AtomicBool::new(false),
        wake,
        wakes,
    });
    BRIDGE
        .set(bridge.clone())
        .map_err(|_| anyhow::anyhow!("native frontend already started"))?;
    let completion = bridge.clone();
    let editor = std::thread::Builder::new()
        .name("runyte-editor".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(worker)
                .unwrap_or_else(|_| Err(anyhow::anyhow!("editor worker panicked")));
            completion.done.store(true, Ordering::Release);
            let _ = completion.wake.try_send(());
            result
        })?;
    Application::new().run(move |cx| {
        cx.text_system()
            .add_fonts(vec![
                std::borrow::Cow::Borrowed(include_bytes!(
                    "../assets/fonts/jetbrains-mono/JetBrainsMonoNerdFont-Medium.ttf"
                )),
                std::borrow::Cow::Borrowed(include_bytes!(
                    "../assets/fonts/jetbrains-mono/JetBrainsMonoNerdFont-MediumItalic.ttf"
                )),
                std::borrow::Cow::Borrowed(include_bytes!(
                    "../assets/fonts/jetbrains-mono/JetBrainsMonoNerdFont-Bold.ttf"
                )),
                std::borrow::Cow::Borrowed(include_bytes!(
                    "../assets/fonts/jetbrains-mono/JetBrainsMonoNerdFont-BoldItalic.ttf"
                )),
            ])
            .expect("load bundled JetBrains Mono Nerd Font faces");
        let bridge = bridge.clone();
        let bounds = Bounds::centered(None, size(px(1080.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                app_id: Some(icon::APP_ID.into()),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Runyte".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                let close = bridge.clone();
                window.on_window_should_close(cx, move |_, _| {
                    close.close_requested.store(true, Ordering::Release);
                    let (w, h) = *close.dimensions.lock().unwrap();
                    close.send(Event::Resize(w, h));
                    false
                });
                cx.new(|cx| NativeView::new(bridge, window, cx))
            },
        )
        .expect("open Runyte window");
        icon::install();
        cx.activate(true);
    });
    editor
        .join()
        .map_err(|_| anyhow::anyhow!("editor worker panicked"))?
}

struct NativeView {
    bridge: Arc<Bridge>,
    focus: FocusHandle,
    frame: Option<FrameData>,
    media: media::Loader,
    composition: String,
    image_clipboard: Option<arboard::Clipboard>,
    viewports: std::collections::HashMap<(usize, PathBuf), viewport::Viewport>,
}
impl NativeView {
    fn new(bridge: Arc<Bridge>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
        let frame = bridge.frame.lock().unwrap().take();
        let (width, height) = *bridge.dimensions.lock().unwrap();
        bridge.send(Event::Resize(width, height));
        let wakes = bridge.wakes.clone();
        cx.spawn(async move |view, cx| {
            while wakes.recv().await.is_ok() {
                if view
                    .update(cx, |view, cx| {
                        if view.bridge.done.load(Ordering::Acquire) {
                            cx.quit();
                            return;
                        }
                        let frame = view.bridge.frame.lock().unwrap().take();
                        let changed = frame.is_some();
                        if let Some(frame) = frame {
                            if view.viewports.len() > 32 && !frame.media.is_empty() {
                                view.viewports.retain(|(id, path), _| {
                                    frame
                                        .media
                                        .iter()
                                        .any(|pane| pane.pane == *id && pane.path == *path)
                                });
                            }
                            view.frame = Some(frame);
                        }
                        let loaded = view.media.poll(&view.bridge);
                        view.apply_media_requests(cx);
                        if changed || loaded {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            media: media::Loader::new(bridge.clone()),
            bridge,
            focus,
            frame,
            composition: String::new(),
            image_clipboard: None,
            viewports: Default::default(),
        }
    }
    fn send(&self, event: Event) {
        self.bridge.send(event);
    }
}

impl Render for NativeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = window.viewport_size();
        let dimensions = (
            ((f32::from(bounds.width) / CELL_WIDTH) as u16).clamp(10, 500),
            ((f32::from(bounds.height) / CELL_HEIGHT) as u16).clamp(5, 200),
        );
        if *self.bridge.dimensions.lock().unwrap() != dimensions {
            *self.bridge.dimensions.lock().unwrap() = dimensions;
            self.send(Event::Resize(dimensions.0, dimensions.1));
        }
        let frame = self.frame.clone();
        let mut root = div()
            .font_family("JetBrainsMono Nerd Font")
            .font_weight(FontWeight::MEDIUM)
            .size_full()
            .bg(rgb(0x181818))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                if view.composition.is_empty()
                    && let Some(mut key) = translate_key(&event.keystroke)
                {
                    if event.is_held {
                        key.kind = crossterm::event::KeyEventKind::Repeat;
                    }
                    view.send(Event::Key(key));
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, _, cx| {
                    if view.media_mouse_down(event, cx) {
                        return;
                    }
                    view.send(mouse_event(
                        event.position,
                        crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                        event.modifiers,
                    ))
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, event: &MouseUpEvent, _, cx| {
                    if view.media_mouse_up(event.position, cx) {
                        return;
                    }
                    view.send(mouse_event(
                        event.position,
                        crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
                        event.modifiers,
                    ))
                }),
            )
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| {
                if view.media_mouse_move(event, cx) {
                    return;
                }
                if event.pressed_button == Some(MouseButton::Left) {
                    view.send(mouse_event(
                        event.position,
                        crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left),
                        event.modifiers,
                    ));
                }
            }))
            .on_scroll_wheel(cx.listener(|view, event: &ScrollWheelEvent, _, cx| {
                if view.media_scroll(event, cx) {
                    return;
                }
                let delta = event.delta.pixel_delta(px(CELL_HEIGHT));
                use crossterm::event::MouseEventKind::*;
                let (kind, amount) = if delta.y != px(0.) {
                    (
                        if delta.y > px(0.) {
                            ScrollUp
                        } else {
                            ScrollDown
                        },
                        f32::from(delta.y).abs() / CELL_HEIGHT,
                    )
                } else if delta.x != px(0.) {
                    (
                        if delta.x > px(0.) {
                            ScrollLeft
                        } else {
                            ScrollRight
                        },
                        f32::from(delta.x).abs() / CELL_WIDTH,
                    )
                } else {
                    return;
                };
                for _ in 0..(amount.ceil() as usize).clamp(1, 20) {
                    view.send(mouse_event(event.position, kind, event.modifiers));
                }
            }));
        for button in [MouseButton::Middle, MouseButton::Right] {
            root = root
                .on_mouse_down(
                    button,
                    cx.listener(|view, event: &MouseDownEvent, _, cx| {
                        view.media_mouse_down(event, cx);
                    }),
                )
                .on_mouse_up(
                    button,
                    cx.listener(|view, event: &MouseUpEvent, _, cx| {
                        view.media_mouse_up(event.position, cx);
                    }),
                );
        }
        if frame.is_none() {
            root = root.child(div().text_color(rgb(0xcccccc)).child("Opening workspace…"));
        }
        if let Some(frame) = &frame {
            for path in &frame.metadata_paths {
                self.media.get(path, 1);
            }
            for pane in &frame.media {
                let area = pane.body;
                let content = self.media.get(&pane.path, pane.page);
                let mut layer = div()
                    .absolute()
                    .left(px(area.x as f32 * CELL_WIDTH))
                    .top(px(area.y as f32 * CELL_HEIGHT))
                    .w(px(area.width as f32 * CELL_WIDTH))
                    .h(px(area.height as f32 * CELL_HEIGHT))
                    .overflow_hidden()
                    .bg(rgb(0x181818));
                layer = match content {
                    Some(Ok(page)) => {
                        let area = Self::media_area(pane);
                        let view = self
                            .viewports
                            .entry((pane.pane, pane.path.clone()))
                            .or_insert_with(|| viewport::Viewport::new(pane.page));
                        view.show_page(pane.page);
                        view.show_source(&page);
                        view.clamp([page.width, page.height], area);
                        let (origin, size) = view.geometry([page.width, page.height], area);
                        layer = layer.child(
                            img(page.image.clone())
                                .absolute()
                                .left(px(origin[0]))
                                .top(px(origin[1]))
                                .w(px(size[0]))
                                .h(px(size[1])),
                        );
                        for bounds in view.selected_bounds(&page) {
                            layer = layer.child(
                                div()
                                    .absolute()
                                    .left(px(origin[0] + bounds[0] * size[0]))
                                    .top(px(origin[1] + bounds[1] * size[1]))
                                    .w(px((bounds[2] - bounds[0]) * size[0]))
                                    .h(px((bounds[3] - bounds[1]) * size[1]))
                                    .bg(rgba(0x66aaff55))
                                    .border_1()
                                    .border_color(rgba(0x99ccffaa)),
                            );
                        }
                        let scale = viewport::Viewport::fit_scale([page.width, page.height], area)
                            * view.zoom;
                        let note = if !view.message.is_empty() {
                            view.message.as_str()
                        } else {
                            page.text_error.as_deref().unwrap_or("")
                        };
                        layer.child(
                            div()
                                .absolute()
                                .bottom(px(0.))
                                .left(px(0.))
                                .text_size(px(12.))
                                .text_color(rgb(0xcccccc))
                                .bg(rgba(0x181818dd))
                                .child(
                                    if pane
                                        .path
                                        .extension()
                                        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
                                    {
                                        format!(
                                            "Page {} · {:.0}%  {}",
                                            pane.page,
                                            scale * 100.,
                                            note
                                        )
                                    } else {
                                        format!("{:.0}%  {}", scale * 100., note)
                                    },
                                ),
                        )
                    }
                    Some(Err(error)) => layer.child(div().text_color(rgb(0xff8888)).child(error)),
                    None => layer.child(div().text_color(rgb(0xcccccc)).child("Loading media…")),
                };
                root = root.child(layer);
            }
        }
        let entity = cx.entity();
        let bridge = self.bridge.clone();
        let focus = self.focus.clone();
        root.child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    window.handle_input(
                        &focus,
                        ElementInputHandler::new(bounds, entity.clone()),
                        cx,
                    );
                    if let Some(frame) = &frame {
                        paint_cells(frame, bounds.origin, window, cx);
                        if frame.attachment != bridge.attachment.load(Ordering::Acquire) {
                            return;
                        }
                        let previous_attachment = bridge
                            .painted_attachment
                            .swap(frame.attachment, Ordering::AcqRel);
                        *bridge.blocked_media.lock().unwrap() = MediaInputMask {
                            blocked: if frame.media_input {
                                Vec::new()
                            } else {
                                frame.media.iter().map(|pane| pane.body).collect()
                            },
                        };
                        *bridge.painted_media.lock().unwrap() = if frame.media_input {
                            frame.media.clone()
                        } else {
                            Vec::new()
                        };
                        let previous =
                            std::mem::replace(&mut *bridge.presented.lock().unwrap(), frame.id);
                        if previous != frame.id || previous_attachment != frame.attachment {
                            let _ = bridge.input.try_send(NativeInput {
                                attachment: frame.attachment,
                                // The envelope is consumed before event conversion.
                                event: Event::FocusGained,
                                presented: frame.id,
                                presentation_only: true,
                            });
                        }
                    }
                },
            )
            .absolute()
            .size_full(),
        )
    }
}

fn modifiers(value: gpui::Modifiers) -> KeyModifiers {
    let mut modifiers = KeyModifiers::empty();
    if value.control {
        modifiers |= KeyModifiers::CONTROL;
    }
    if value.alt {
        modifiers |= KeyModifiers::ALT;
    }
    if value.shift {
        modifiers |= KeyModifiers::SHIFT;
    }
    if value.platform {
        modifiers |= KeyModifiers::SUPER;
    }
    modifiers
}
fn translate_key(key: &Keystroke) -> Option<KeyEvent> {
    let code = match key.key.as_str() {
        "enter" => KeyCode::Enter,
        "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "tab" if key.modifiers.shift => KeyCode::BackTab,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        value if value.starts_with('f') && value[1..].parse::<u8>().is_ok() => {
            KeyCode::F(value[1..].parse().ok()?)
        }
        _ => {
            let text = if key.modifiers.control || key.modifiers.alt || key.modifiers.platform {
                &key.key
            } else {
                key.key_char.as_ref().unwrap_or(&key.key)
            };
            let mut chars = text.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            KeyCode::Char(if key.modifiers.shift && ch.is_ascii_lowercase() {
                ch.to_ascii_uppercase()
            } else {
                ch
            })
        }
    };
    Some(KeyEvent::new(code, modifiers(key.modifiers)))
}
fn mouse_event(
    position: Point<Pixels>,
    kind: crossterm::event::MouseEventKind,
    mods: gpui::Modifiers,
) -> Event {
    Event::Mouse(crossterm::event::MouseEvent {
        kind,
        column: (f32::from(position.x) / CELL_WIDTH).max(0.) as u16,
        row: (f32::from(position.y) / CELL_HEIGHT).max(0.) as u16,
        modifiers: modifiers(mods),
    })
}

fn color(color: Color, default: u32) -> Hsla {
    let value = match color {
        Color::Rgb(r, g, b) => (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
        Color::Reset => default,
        Color::Indexed(index) if index >= 232 => {
            let v = 8 + u32::from(index - 232) * 10;
            v * 0x010101
        }
        Color::Indexed(index) if index >= 16 => {
            let i = index - 16;
            let part = |v: u8| if v == 0 { 0 } else { 55 + u32::from(v) * 40 };
            (part(i / 36) << 16) | (part(i / 6 % 6) << 8) | part(i % 6)
        }
        value => {
            let i = match value {
                Color::Black => 0,
                Color::Red => 1,
                Color::Green => 2,
                Color::Yellow => 3,
                Color::Blue => 4,
                Color::Magenta => 5,
                Color::Cyan => 6,
                Color::Gray => 7,
                Color::DarkGray => 8,
                Color::LightRed => 9,
                Color::LightGreen => 10,
                Color::LightYellow => 11,
                Color::LightBlue => 12,
                Color::LightMagenta => 13,
                Color::LightCyan => 14,
                Color::White => 15,
                Color::Indexed(i) => usize::from(i),
                _ => 7,
            };
            [
                0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5,
                0x7f7f7f, 0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
            ][i]
        }
    };
    rgb(value).into()
}
fn paint_cells(frame: &FrameData, origin: Point<Pixels>, window: &mut Window, cx: &mut gpui::App) {
    for y in 0..frame.cells.area.height {
        for x in 0..frame.cells.area.width {
            if frame.under_media(x, y) {
                continue;
            }
            let cell = &frame.cells[(x, y)];
            let bg = if cell.modifier.contains(Modifier::REVERSED) {
                color(cell.fg, 0xdddddd)
            } else {
                color(cell.bg, 0x181818)
            };
            let position = origin + point(px(x as f32 * CELL_WIDTH), px(y as f32 * CELL_HEIGHT));
            window.paint_quad(fill(
                Bounds::new(position, size(px(CELL_WIDTH), px(CELL_HEIGHT))),
                bg,
            ));
        }
    }
    for y in 0..frame.cells.area.height {
        let mut x = 0;
        while x < frame.cells.area.width {
            if frame.under_media(x, y) {
                x += 1;
                continue;
            }
            let cell = &frame.cells[(x, y)];
            let width = unicode_width::UnicodeWidthStr::width(cell.symbol()).max(1) as u16;
            let mut fg = color(cell.fg, 0xdddddd);
            let mut bg = color(cell.bg, 0x181818);
            if cell.modifier.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.modifier.contains(Modifier::DIM) {
                fg.l *= 0.65;
            }
            let position = origin + point(px(x as f32 * CELL_WIDTH), px(y as f32 * CELL_HEIGHT));
            if cell.symbol() != " " && !cell.modifier.contains(Modifier::HIDDEN) {
                let mut font = font("JetBrainsMono Nerd Font");
                font.weight = FontWeight::MEDIUM;
                if cell.modifier.contains(Modifier::BOLD) {
                    font.weight = FontWeight::BOLD;
                }
                if cell.modifier.contains(Modifier::ITALIC) {
                    font.style = FontStyle::Italic;
                }
                let run = TextRun {
                    len: cell.symbol().len(),
                    font,
                    color: fg,
                    background_color: None,
                    underline: cell.modifier.contains(Modifier::UNDERLINED).then_some(
                        UnderlineStyle {
                            thickness: px(1.),
                            color: Some(fg),
                            wavy: false,
                        },
                    ),
                    strikethrough: cell.modifier.contains(Modifier::CROSSED_OUT).then_some(
                        StrikethroughStyle {
                            thickness: px(1.),
                            color: Some(fg),
                        },
                    ),
                };
                let line = window.text_system().shape_line(
                    cell.symbol().to_owned().into(),
                    px(15.),
                    &[run],
                    None,
                );
                let _ = line.paint(position, px(CELL_HEIGHT), window, cx);
            }
            x += width;
        }
    }
    if let Some(cursor) = frame
        .cursor
        .filter(|cursor| !frame.under_media(cursor.x, cursor.y))
    {
        let position = origin
            + point(
                px(cursor.x as f32 * CELL_WIDTH),
                px(cursor.y as f32 * CELL_HEIGHT),
            );
        window.paint_quad(fill(
            Bounds::new(position, size(px(2.), px(CELL_HEIGHT))),
            rgb(0xffffff),
        ));
    }
}

// Composition is kept locally until committed, so it cannot execute modal
// commands or leave partial edits in the host's transaction history.
impl EntityInputHandler for NativeView {
    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        actual: &mut Option<std::ops::Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        *actual = Some(range.clone());
        String::from_utf16(
            &self
                .composition
                .encode_utf16()
                .skip(range.start)
                .take(range.len())
                .collect::<Vec<_>>(),
        )
        .ok()
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.composition.encode_utf16().count();
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }
    fn marked_text_range(
        &self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        (!self.composition.is_empty()).then(|| 0..self.composition.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.composition.clear();
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<std::ops::Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composition.clear();
        if text.len() <= runyte::input::MAX_TEXT_INPUT_BYTES {
            self.send(Event::Paste(text.to_owned()));
        }
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<std::ops::Range<usize>>,
        text: &str,
        _: Option<std::ops::Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if text.len() <= 4096 {
            self.composition = text.to_owned();
        }
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: std::ops::Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let cursor = self.frame.as_ref()?.cursor.unwrap_or_default();
        Some(Bounds::new(
            bounds.origin
                + point(
                    px(cursor.x as f32 * CELL_WIDTH),
                    px(cursor.y as f32 * CELL_HEIGHT),
                ),
            size(px(CELL_WIDTH), px(CELL_HEIGHT)),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

#[cfg(test)]
#[path = "native_frontend/tests/input.rs"]
mod tests;
