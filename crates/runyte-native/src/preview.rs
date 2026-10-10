// SPDX-License-Identifier: MPL-2.0
//! Frontend-owned document views. The killable helper owns every engine type.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    process::{ChildStdin, ChildStdout},
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(super) struct Pane {
    pub pane: usize,
    pub active: bool,
    pub body: runyte::layout::Rect,
    pub document: runyte::document_preview::DocumentPreview,
}
#[derive(Clone, Serialize, PartialEq)]
struct Request {
    text: String,
    selection_only: bool,
    language: String,
    path: Option<PathBuf>,
    width: u32,
    height: u32,
    scale: f32,
    zoom: f32,
    scroll: [f64; 2],
    selection: Option<[[f32; 2]; 2]>,
    adjacent: bool,
}
#[derive(Deserialize)]
struct Reply {
    width: u32,
    height: u32,
    selected: String,
    link: Option<String>,
    scroll: [f64; 2],
    max_scroll: [f64; 2],
    raster_width: u32,
    raster_height: u32,
    origin: [f64; 2],
    cacheable: bool,
}
struct Output {
    scale: f32,
    zoom: f32,
    reply: Reply,
    image: Arc<RenderImage>,
}
impl Output {
    fn matches(&self, request: &Request) -> bool {
        self.reply.width == request.width
            && self.reply.height == request.height
            && self.scale == request.scale
            && self.zoom == request.zoom
    }
    fn display_scroll(&self, target: [f64; 2]) -> [f64; 2] {
        if !self.reply.cacheable {
            return self.reply.origin;
        }
        let scale = f64::from(self.scale * self.zoom);
        std::array::from_fn(|axis| {
            let viewport = f64::from([self.reply.width, self.reply.height][axis]) / scale;
            let extent =
                f64::from([self.reply.raster_width, self.reply.raster_height][axis]) / scale;
            target[axis].clamp(
                self.reply.origin[axis],
                self.reply.origin[axis] + extent - viewport,
            )
        })
    }
    fn covers(&self, scroll: [f64; 2], guard: bool) -> bool {
        if !self.reply.cacheable {
            return false;
        }
        let scale = f64::from(self.scale * self.zoom);
        for axis in 0..2 {
            let viewport = f64::from([self.reply.width, self.reply.height][axis]) / scale;
            let cached =
                f64::from([self.reply.raster_width, self.reply.raster_height][axis]) / scale;
            let margin = if guard {
                ((cached - viewport) * 0.25).min(viewport * 0.5)
            } else {
                0.0
            };
            let start = (scroll[axis] - margin).max(0.0);
            let end =
                (scroll[axis] + viewport + margin).min(self.reply.max_scroll[axis] + viewport);
            if self.reply.origin[axis] > start + 1e-6
                || self.reply.origin[axis] + cached < end - 1e-6
            {
                return false;
            }
        }
        true
    }
}
struct Job {
    cancel: Arc<AtomicBool>,
    pane: usize,
    generation: u64,
    serial: u64,
    request: Request,
}
struct Done {
    pane: usize,
    generation: u64,
    serial: u64,
    result: Result<Output, String>,
}
#[derive(Default)]
struct State {
    cancel: Option<Arc<AtomicBool>>,
    generation: u64,
    serial: u64,
    request: Option<Request>,
    output: Option<Result<Arc<Output>, String>>,
    cache: Option<Arc<Output>>,
    scroll: [f64; 2],
    selection: Option<[[f32; 2]; 2]>,
    dragging: bool,
    hidden: bool,
    zoom: f32,
    keys: runyte::keymap::KeySequence,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Navigation {
    Scroll(f64, f64),
    Top,
    Bottom,
    Zoom(f32),
    ResetZoom,
    Copy,
    Back,
    Prefix,
}
fn navigation_command(target: runyte::keymap::BindingTarget, page: f64) -> Option<Navigation> {
    use Navigation::*;
    use runyte::{command::EditorCommand as C, keymap::BindingTarget};
    let BindingTarget::Editor(command) = target else {
        return None;
    };
    Some(match command {
        C::MediaMoveDown | C::MoveDown | C::ScrollViewDown => Scroll(0.0, 40.0),
        C::MediaMoveUp | C::MoveUp | C::ScrollViewUp => Scroll(0.0, -40.0),
        C::MoveLeft | C::MediaPanLeft => Scroll(-40.0, 0.0),
        C::MoveRight | C::MediaPanRight => Scroll(40.0, 0.0),
        C::PageDown => Scroll(0.0, page * 0.9),
        C::PageUp => Scroll(0.0, -page * 0.9),
        C::HalfPageDown => Scroll(0.0, page * 0.5),
        C::HalfPageUp => Scroll(0.0, -page * 0.5),
        C::MoveFileStart => Top,
        C::MoveFileEnd => Bottom,
        C::MediaZoomIn => Zoom(1.2),
        C::MediaZoomOut => Zoom(1.0 / 1.2),
        C::MediaFit | C::MediaActualSize => ResetZoom,
        C::MediaCopySelection => Copy,
        C::MediaBack => Back,
        _ => return None,
    })
}
// Pane focus belongs to the editor even while a document owns navigation.
fn editor_pane_key(key: KeyEvent) -> bool {
    // These directional chords are resolved by the editor's configured keymap,
    // not by the preview's built-in document-navigation subset.
    if key.modifiers == KeyModifiers::CONTROL
        && matches!(
            key.code,
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
        )
    {
        return true;
    }
    use runyte::{
        command::{EditorCommand as C, Mode},
        keymap::{BindingTarget, KeySequence, Lookup},
    };
    let Ok(Some(runyte::input::InputEvent::Key(stroke))) =
        runyte::tui::input::convert_event(Event::Key(key))
    else {
        return false;
    };
    // The editor decides whether fast pane keys are enabled. Keep the native
    // surface intact while forwarding them, just as for configured Ctrl-arrows.
    if runyte::keymap::is_fast_pane_key(stroke) {
        return true;
    }
    matches!(runyte::keymap::default_keymap().lookup(Mode::Normal, &KeySequence::new([stroke])),
        Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. }
        if matches!(binding.target, BindingTarget::Editor(C::FocusWindowLeft | C::FocusWindowRight | C::FocusWindowUp | C::FocusWindowDown | C::NextWindow)))
}

fn preview_navigation(
    keys: &mut runyte::keymap::KeySequence,
    key: KeyEvent,
    page: f64,
) -> Option<Navigation> {
    use runyte::keymap::{BindingScope, Lookup};
    let Ok(Some(runyte::input::InputEvent::Key(stroke))) =
        runyte::tui::input::convert_event(Event::Key(key))
    else {
        return None;
    };
    keys.push(stroke);
    let found = match runyte::keymap::default_keymap().lookup_in(
        runyte::command::Mode::Normal,
        BindingScope::Media,
        keys,
    ) {
        Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => {
            navigation_command(binding.target, page)
        }
        Lookup::Prefix(bindings)
            if bindings
                .iter()
                .any(|b| navigation_command(b.target, page).is_some()) =>
        {
            Some(Navigation::Prefix)
        }
        _ => None,
    };
    if found != Some(Navigation::Prefix) {
        keys.clear();
    }
    found
}
fn navigation_hints(keys: &runyte::keymap::KeySequence) -> String {
    use runyte::keymap::BindingScope;
    runyte::keymap::default_keymap()
        .bindings_for_scope(runyte::command::Mode::Normal, BindingScope::Media)
        .filter(|b| b.sequence.starts_with(keys) && navigation_command(b.target, 1.0).is_some())
        .map(|b| format!("{}: {}", b.sequence, b.target.description()))
        .collect::<Vec<_>>()
        .join(" · ")
}
impl State {
    fn clamp_scroll(&mut self) {
        if let Some(Ok(output)) = &self.output {
            for (offset, maximum) in self.scroll.iter_mut().zip(output.reply.max_scroll) {
                *offset = offset.clamp(0.0, maximum);
            }
        }
    }
    fn cancel(&self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel();
    }
}
#[derive(Default)]
pub(super) struct Views {
    states: HashMap<usize, State>,
    dismissed: HashMap<usize, u64>,
    worker: Option<(mpsc::SyncSender<Job>, mpsc::Receiver<Done>)>,
    attachment: u64,
}
// Retain logical layout and hit coordinates while bounding the physical raster.
fn raster_viewport(width: f32, height: f32, display_scale: f32) -> (u32, u32, f32) {
    let width = f64::from(width.max(1.0));
    let height = f64::from(height.max(1.0));
    let scale = f64::from(display_scale.clamp(0.5, 4.0))
        .min(4096.0 / width)
        .min(4096.0 / height)
        .min((3_990_000.0 / (width * height)).sqrt()) as f32;
    (
        (width * f64::from(scale)).max(1.0) as u32,
        (height * f64::from(scale)).max(1.0) as u32,
        scale,
    )
}

struct Helper {
    child: super::helper::Process,
    pipes: Option<(ChildStdin, BufReader<ChildStdout>)>,
    errors: Arc<Mutex<Vec<u8>>>,
}
impl Helper {
    fn start() -> Result<Self, String> {
        let mut command =
            super::helper::command(super::helper::Role::Preview).map_err(|e| e.to_string())?;
        command.arg("--serve");
        let mut child = super::helper::Process::spawn(&mut command, super::helper::Role::Preview)
            .map_err(|e| format!("Document preview could not start: {e:#}"))?;
        let input = child.child.stdin.take().unwrap();
        let output = BufReader::new(child.child.stdout.take().unwrap());
        let errors = child.errors.clone();
        Ok(Self {
            child,
            pipes: Some((input, output)),
            errors,
        })
    }
}
fn render_request(
    request: &Request,
    cancel: &AtomicBool,
    helper: &mut Option<Helper>,
) -> Result<Output, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Preview cancelled".into());
    }
    if helper.is_none() {
        *helper = Some(Helper::start()?);
    }
    let process = helper.as_mut().unwrap();
    let (mut input, mut output) = process.pipes.take().ok_or("Preview transport is busy")?;
    let mut bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    let (tx, rx) = mpsc::sync_channel(1);
    let expected = (request.width, request.height);
    std::thread::spawn(move || {
        let result = (|| -> Result<(Reply, Vec<u8>), String> {
            input
                .write_all(&bytes)
                .and_then(|_| input.flush())
                .map_err(|e| e.to_string())?;
            let mut header = Vec::new();
            output
                .by_ref()
                .take(200_000)
                .read_until(b'\n', &mut header)
                .map_err(|e| e.to_string())?;
            if header.last() != Some(&b'\n') {
                return Err("Incomplete preview header".into());
            }
            let reply: Reply = serde_json::from_slice(&header).map_err(|e| e.to_string())?;
            if (reply.width, reply.height) != expected
                || reply.selected.len() > 128 * 1024
                || reply.raster_width < reply.width
                || reply.raster_height < reply.height
                || reply.raster_width > 8192
                || reply.raster_height > 8192
                || u64::from(reply.raster_width) * u64::from(reply.raster_height) > 8_000_000
                || reply
                    .origin
                    .iter()
                    .chain(reply.scroll.iter())
                    .chain(reply.max_scroll.iter())
                    .any(|x| !x.is_finite() || *x < 0.0)
            {
                return Err("Invalid preview dimensions or selection".into());
            }
            let mut pixels =
                vec![0; reply.raster_width as usize * reply.raster_height as usize * 4];
            output.read_exact(&mut pixels).map_err(|e| e.to_string())?;
            Ok((reply, pixels))
        })();
        let _ = tx.send((input, output, result));
    });
    let started = Instant::now();
    let result = loop {
        if cancel.load(Ordering::Acquire) {
            break Err("Preview cancelled".to_owned());
        }
        if started.elapsed() >= Duration::from_secs(5) {
            break Err("Preview exceeded its five-second budget".to_owned());
        }
        match rx.recv_timeout(Duration::from_millis(5)) {
            Ok((input, output, result)) => {
                process.pipes = Some((input, output));
                break result;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => (),
            Err(_) => break Err("Preview transport stopped".into()),
        }
    };
    let (reply, mut pixels) = match result {
        Ok(value) => value,
        Err(error) => {
            let status = process.child.status().ok().flatten();
            let diagnostic: String = String::from_utf8_lossy(&process.errors.lock().unwrap())
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            *helper = None;
            return Err(format!(
                "{error}; helper {status:?}: {diagnostic}. Escape returns to source"
            ));
        }
    };
    // GPUI's RenderImage stores BGRA, while Vello CPU supplies premultiplied RGBA.
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let pixels = image::RgbaImage::from_raw(reply.raster_width, reply.raster_height, pixels)
        .ok_or("Invalid raster")?;
    Ok(Output {
        scale: request.scale,
        zoom: request.zoom,
        reply,
        image: Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])),
    })
}
impl Views {
    fn worker(&mut self, bridge: &Arc<Bridge>) -> &mpsc::SyncSender<Job> {
        if self.worker.is_none() {
            let (tx, rx) = mpsc::sync_channel::<Job>(8);
            let (done, results) = mpsc::sync_channel(8);
            let wake = bridge.wake.clone();
            std::thread::spawn(move || {
                let mut helper = None;
                let mut identity = None;
                loop {
                    let job = if helper.is_some() {
                        match rx.recv_timeout(Duration::from_secs(30)) {
                            Ok(job) => job,
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                helper = None;
                                continue;
                            }
                            Err(_) => return,
                        }
                    } else {
                        match rx.recv() {
                            Ok(job) => job,
                            Err(_) => return,
                        }
                    };
                    let mut pending = HashMap::new();
                    pending.insert(job.pane, job);
                    for job in rx.try_iter() {
                        pending.insert(job.pane, job);
                    }
                    for job in pending.into_values() {
                        if identity != Some((job.pane, job.generation)) {
                            helper = None;
                            identity = Some((job.pane, job.generation));
                        }
                        let result = render_request(&job.request, &job.cancel, &mut helper);
                        if done
                            .send(Done {
                                pane: job.pane,
                                generation: job.generation,
                                serial: job.serial,
                                result,
                            })
                            .is_err()
                        {
                            return;
                        }
                        let _ = wake.try_send(());
                    }
                }
            });
            self.worker = Some((tx, results));
        }
        &self.worker.as_ref().unwrap().0
    }
    pub fn poll(&mut self) -> bool {
        let Some((_, rx)) = &self.worker else {
            return false;
        };
        let mut changed = false;
        while let Ok(done) = rx.try_recv() {
            changed = true;
            if let Some(state) = self.states.get_mut(&done.pane)
                && state.generation == done.generation
            {
                let result = done.result.map(Arc::new);
                if let Ok(output) = &result {
                    if !state.request.as_ref().is_some_and(|r| output.matches(r)) {
                        continue;
                    }
                    if state.serial == done.serial
                        && state
                            .request
                            .as_ref()
                            .is_some_and(|r| r.scroll == state.scroll)
                    {
                        state.scroll = output.reply.scroll;
                        if let Some(request) = &mut state.request {
                            request.scroll = state.scroll;
                        }
                    }
                    if output.reply.cacheable {
                        state.cache = Some(output.clone());
                    }
                    // A cache hit can move the target without queuing a request. Never rewind it.
                    state.output = Some(result);
                    state.clamp_scroll();
                } else if state.serial == done.serial {
                    state.cache = None;
                    state.output = Some(result);
                }
            }
        }
        changed
    }
}
impl NativeView {
    pub(super) fn prepare_previews(
        &mut self,
        window: &mut Window,
    ) -> Option<std::rc::Rc<FrameData>> {
        if self.frame.as_ref()?.previews.is_empty() {
            self.previews.states.clear();
            self.previews.worker = None;
            return self.frame.clone();
        }
        let mut frame = self.frame.as_deref()?.clone();
        if self.previews.attachment != frame.attachment {
            self.previews = Views {
                attachment: frame.attachment,
                ..Default::default()
            };
        }
        self.previews
            .states
            .retain(|id, _| frame.previews.iter().any(|p| p.pane == *id));
        frame.previews.retain(|pane| {
            let state = self.previews.states.entry(pane.pane).or_default();
            if state.generation != pane.document.generation {
                *state = State {
                    generation: pane.document.generation,
                    hidden: self.previews.dismissed.get(&pane.pane)
                        == Some(&pane.document.generation),
                    serial: state.serial + 1,
                    cancel: Some(Arc::new(AtomicBool::new(false))),
                    zoom: 1.0,
                    keys: Default::default(),
                    request: None,
                    output: None,
                    cache: None,
                    scroll: [0.0; 2],
                    selection: None,
                    dragging: false,
                };
            }
            !state.hidden
        });
        if frame.previews.is_empty() {
            self.previews.worker = None;
        }
        for pane in &frame.previews {
            let state = &self.previews.states[&pane.pane];
            let (width, height, scale) = raster_viewport(
                pane.body.width as f32 * self.metrics.width,
                pane.body.height as f32 * self.metrics.height,
                window.scale_factor(),
            );
            let request = Request {
                text: pane.document.text.clone(),
                selection_only: pane.document.selection_only,
                language: pane.document.language.clone(),
                path: pane.document.path.clone(),
                width,
                height,
                scale,
                zoom: state.zoom,
                scroll: state.scroll,
                selection: state.selection,
                adjacent: pane.active,
            };
            if request.selection.is_none()
                && state.cache.as_ref().is_some_and(|cache| {
                    cache.matches(&request) && cache.covers(request.scroll, true)
                })
            {
                continue;
            }
            if state.request.as_ref() == Some(&request) {
                continue;
            }
            let serial = state.serial + 1;
            let cancel = state.cancel.as_ref().unwrap().clone();
            if self
                .previews
                .worker(&self.bridge)
                .try_send(Job {
                    cancel: cancel.clone(),
                    pane: pane.pane,
                    generation: pane.document.generation,
                    serial,
                    request: request.clone(),
                })
                .is_ok()
            {
                let state = self.previews.states.get_mut(&pane.pane).unwrap();

                state.serial = serial;
                state.request = Some(request);
            }
        }
        Some(std::rc::Rc::new(frame))
    }
    pub(super) fn preview_layers(
        &self,
        frame: &FrameData,
        mut root: Div,
        images: &mut Vec<Arc<RenderImage>>,
    ) -> Div {
        for pane in &frame.previews {
            let state = &self.previews.states[&pane.pane];
            let area = pane.body;
            let mut layer = div()
                .absolute()
                .left(px(area.x as f32 * self.metrics.width))
                .top(px(area.y as f32 * self.metrics.height))
                .w(px(area.width as f32 * self.metrics.width))
                .h(px(area.height as f32 * self.metrics.height))
                .overflow_hidden()
                .bg(rgb(0xffffff));
            let cached = state.cache.as_ref().filter(|cache| {
                state.selection.is_none()
                    && state
                        .request
                        .as_ref()
                        .is_some_and(|request| cache.matches(request))
            });
            let displayed = cached
                .map(Ok)
                .or_else(|| state.output.as_ref().map(|result| result.as_ref()));
            layer = match displayed {
                Some(Ok(output)) => {
                    images.push(output.image.clone());
                    let display_scroll = output.display_scroll(state.scroll);
                    layer.child(
                        img(output.image.clone())
                            .absolute()
                            .left(px(((output.reply.origin[0] - display_scroll[0])
                                * f64::from(state.zoom))
                                as f32))
                            .top(px(((output.reply.origin[1] - display_scroll[1])
                                * f64::from(state.zoom))
                                as f32))
                            .w(px(output.reply.raster_width as f32 / output.scale))
                            .h(px(output.reply.raster_height as f32 / output.scale)),
                    )
                }
                Some(Err(error)) => {
                    layer.child(div().p_4().text_color(rgb(0x990000)).child(error.clone()))
                }
                None => layer.child(
                    div()
                        .p_4()
                        .text_color(rgb(0x333333))
                        .child("Preparing document preview… Escape returns to source"),
                ),
            };
            if state.selection.is_some()
                && let Some(Ok(output)) = &state.output
                && let Some(link) = &output.reply.link
            {
                let link: String = link.chars().take(200).collect();
                layer = layer.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .bg(rgb(0xffffff))
                        .text_color(rgb(0x0969da))
                        .child(format!("Link: {link} (navigation disabled)")),
                );
            }
            if !state.keys.is_empty() {
                layer = layer.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .bg(rgb(0xffffff))
                        .text_color(rgb(0x0969da))
                        .child(navigation_hints(&state.keys)),
                );
            }
            root = root.child(layer);
        }
        root
    }
    fn preview_at(&self, position: Point<Pixels>) -> Option<Pane> {
        let frame = self.frame.as_ref()?;
        if frame.attachment != self.bridge.attachment.load(Ordering::Acquire) {
            return None;
        }
        frame
            .previews
            .iter()
            .find(|pane| {
                self.previews
                    .states
                    .get(&pane.pane)
                    .is_some_and(|s| !s.hidden)
                    && f32::from(position.x) >= pane.body.x as f32 * self.metrics.width
                    && f32::from(position.x)
                        < (pane.body.x + pane.body.width) as f32 * self.metrics.width
                    && f32::from(position.y) >= pane.body.y as f32 * self.metrics.height
                    && f32::from(position.y)
                        < (pane.body.y + pane.body.height) as f32 * self.metrics.height
            })
            .cloned()
    }
    pub(super) fn preview_pointer(
        &mut self,
        position: Point<Pixels>,
        phase: u8,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(pane) = self.preview_at(position) else {
            return false;
        };
        if !pane.active || !self.frame.as_ref().is_some_and(|f| f.media_input) {
            return true;
        }
        let state = self.previews.states.get_mut(&pane.pane).unwrap();
        // A resized or scrolled old raster may remain visible while its replacement
        // is prepared. Do not start a selection against geometry not yet displayed.
        if phase == 0
            && !state.request.as_ref().is_some_and(|request| {
                state.cache.as_ref().is_some_and(|cache| {
                    cache.matches(request) && cache.covers(state.scroll, false)
                }) || state.output.as_ref().is_some_and(|result| {
                    result.as_ref().is_ok_and(|output| {
                        output.matches(request) && output.reply.scroll == state.scroll
                    })
                })
            })
        {
            return true;
        }
        let point = [
            f32::from(position.x) - pane.body.x as f32 * self.metrics.width,
            f32::from(position.y) - pane.body.y as f32 * self.metrics.height,
        ];
        if phase == 0 {
            state.selection = Some([point, point]);
            state.dragging = true;
        } else if state.dragging {
            if let Some(selection) = &mut state.selection {
                selection[1] = point;
            }
            if phase == 2 {
                state.dragging = false;
            }
        } else {
            return true;
        }
        cx.notify();
        true
    }
    pub(super) fn preview_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(pane) = self.preview_at(event.position) else {
            return false;
        };
        if !self.frame.as_ref().is_some_and(|f| f.media_input) {
            return true;
        }
        let delta = event.delta.pixel_delta(px(self.metrics.height));
        let state = self.previews.states.get_mut(&pane.pane).unwrap();
        if event.modifiers.shift {
            state.scroll[0] =
                (state.scroll[0] - f64::from(f32::from(delta.y) / state.zoom)).max(0.0);
        } else {
            state.scroll[0] =
                (state.scroll[0] - f64::from(f32::from(delta.x) / state.zoom)).max(0.0);
            state.scroll[1] =
                (state.scroll[1] - f64::from(f32::from(delta.y) / state.zoom)).max(0.0);
        }
        state.clamp_scroll();
        state.selection = None;
        cx.notify();
        true
    }
    pub(super) fn preview_key(&mut self, key: KeyEvent, cx: &mut Context<Self>) -> bool {
        let Some(frame) = &self.frame else {
            return false;
        };
        if !frame.media_input {
            return false;
        }
        let Some(pane) = frame
            .previews
            .iter()
            .find(|p| p.active && self.previews.states.get(&p.pane).is_some_and(|s| !s.hidden))
        else {
            return false;
        };
        let state = self.previews.states.get_mut(&pane.pane).unwrap();
        if editor_pane_key(key) {
            state.keys.clear();
            cx.notify();
            return false;
        }
        let page = f64::from(pane.body.height as f32 * self.metrics.height / state.zoom);
        let navigation = preview_navigation(&mut state.keys, key, page);
        match navigation {
            Some(Navigation::Scroll(x, y)) => {
                state.scroll[0] = (state.scroll[0] + x).max(0.0);
                state.scroll[1] = (state.scroll[1] + y).max(0.0);
                state.selection = None;
            }
            Some(Navigation::Top) => {
                state.scroll = [0.0; 2];
                state.selection = None;
            }
            Some(Navigation::Bottom) => {
                state.scroll[1] = 1e12;
                state.selection = None;
            }
            Some(Navigation::Zoom(factor)) => {
                state.zoom = (state.zoom * factor).clamp(0.25, 4.0);
                state.selection = None;
            }
            Some(Navigation::ResetZoom) => {
                state.zoom = 1.0;
                state.selection = None;
            }
            Some(Navigation::Copy) => {
                if let Some(Ok(output)) = &state.output {
                    cx.write_to_clipboard(ClipboardItem::new_string(output.reply.selected.clone()));
                }
            }
            Some(Navigation::Prefix) => (),
            _ => (),
        }
        if navigation.is_some() && navigation != Some(Navigation::Back) {
            state.clamp_scroll();
            cx.notify();
            return true;
        }
        if key.code == KeyCode::Esc || navigation == Some(Navigation::Back) {
            self.bridge
                .dismiss_preview(frame.attachment, pane.pane, pane.document.generation);
            state.cancel();
            state.hidden = true;
            state.cache = None;
            state.output = None;
            self.previews
                .dismissed
                .insert(pane.pane, pane.document.generation);
            if self.previews.dismissed.len() > 8
                && let Some(oldest) = self
                    .previews
                    .dismissed
                    .iter()
                    .min_by_key(|(_, generation)| **generation)
                    .map(|(id, _)| *id)
            {
                self.previews.dismissed.remove(&oldest);
            }
            cx.notify();
            return true;
        }
        if matches!(key.code, KeyCode::Char('c' | 'C'))
            && (key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER))
        {
            if let Some(Ok(output)) = &state.output {
                cx.write_to_clipboard(ClipboardItem::new_string(output.reply.selected.clone()));
            }
            return true;
        }
        if let Ok(Some(runyte::input::InputEvent::Key(stroke))) =
            runyte::tui::input::convert_event(Event::Key(key))
            && runyte::keymap::native_window::lookup(stroke).is_some_and(|b| {
                matches!(b.action, runyte::keymap::native_window::Action::FontSize(_))
            })
        {
            return false;
        }
        // Editor leaders retain registry dispatch. Any other key returns to the source first.
        if !matches!(key.code, KeyCode::Char(':' | ' '))
            && !(key.code == KeyCode::Char('w') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            self.bridge
                .dismiss_preview(frame.attachment, pane.pane, pane.document.generation);
            state.cancel();
            state.hidden = true;
            state.cache = None;
            state.output = None;
            self.previews
                .dismissed
                .insert(pane.pane, pane.document.generation);
            if self.previews.dismissed.len() > 8
                && let Some(oldest) = self
                    .previews
                    .dismissed
                    .iter()
                    .min_by_key(|(_, generation)| **generation)
                    .map(|(id, _)| *id)
            {
                self.previews.dismissed.remove(&oldest);
            }
            cx.notify();
        }
        false
    }
}

#[cfg(test)]
#[path = "tests/preview.rs"]
mod tests;
