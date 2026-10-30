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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Target {
    pub pane: usize,
    pub source: usize,
    pub generation: u64,
}
impl Target {
    pub fn from_frame(frame: &FrameData) -> Option<Self> {
        if !frame.media_input {
            return None;
        }
        frame
            .previews
            .iter()
            .find(|pane| pane.active)
            .map(|pane| Self {
                pane: pane.pane,
                source: pane.document.source,
                generation: pane.document.generation,
            })
    }
}

#[derive(Clone)]
pub(super) struct Pane {
    pub pane: usize,
    pub active: bool,
    pub body: runyte::layout::Rect,
    pub document: runyte::document_preview::DocumentPreview,
    /// The editor theme's colours, or `None` for the original light page.
    pub palette: Option<runyte::document_preview::Palette>,
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
    theme: Option<runyte::document_preview::Palette>,
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
    #[serde(default)]
    themed: bool,
}
struct Output {
    scale: f32,
    zoom: f32,
    theme: Option<runyte::document_preview::Palette>,
    reply: Reply,
    image: Arc<RenderImage>,
}
impl Output {
    fn matches(&self, request: &Request) -> bool {
        self.reply.width == request.width
            && self.reply.height == request.height
            && self.scale == request.scale
            && self.zoom == request.zoom
            && self.theme == request.theme
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
    keys_bindings: Option<runyte::keymap::native_preview::Bindings>,
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
    Forward,
    DismissAndForward,
    ForwardInput,
}
fn navigation_action(action: runyte::keymap::native_preview::Action, page: f64) -> Navigation {
    use runyte::keymap::native_preview::Action as A;
    match action {
        A::Down => Navigation::Scroll(0.0, 40.0),
        A::Up => Navigation::Scroll(0.0, -40.0),
        A::Left => Navigation::Scroll(-40.0, 0.0),
        A::Right => Navigation::Scroll(40.0, 0.0),
        A::PageDown => Navigation::Scroll(0.0, page * 0.9),
        A::PageUp => Navigation::Scroll(0.0, -page * 0.9),
        A::HalfPageDown => Navigation::Scroll(0.0, page * 0.5),
        A::HalfPageUp => Navigation::Scroll(0.0, -page * 0.5),
        A::Top => Navigation::Top,
        A::Bottom => Navigation::Bottom,
        A::ZoomIn => Navigation::Zoom(1.2),
        A::ZoomOut => Navigation::Zoom(1.0 / 1.2),
        A::ResetZoom => Navigation::ResetZoom,
        A::Copy => Navigation::Copy,
        A::Back => Navigation::Back,
        A::Forward => Navigation::Forward,
        A::ForwardInput => Navigation::ForwardInput,
        A::DismissAndForward => Navigation::DismissAndForward,
    }
}
fn effective_navigation(
    keys: &mut runyte::keymap::KeySequence,
    key: KeyEvent,
    page: f64,
    bindings: &runyte::keymap::native_preview::Bindings,
) -> Option<Navigation> {
    use runyte::keymap::native_preview::Lookup;
    let Ok(Some(runyte::input::InputEvent::Key(stroke))) =
        runyte::tui::input::convert_event(Event::Key(key))
    else {
        return None;
    };
    if let Lookup::Action(
        action @ (runyte::keymap::native_preview::Action::Forward
        | runyte::keymap::native_preview::Action::ForwardInput),
    ) = bindings.lookup(&runyte::keymap::KeySequence::from(stroke))
    {
        keys.clear();
        return Some(navigation_action(action, page));
    }
    keys.push(stroke);
    let found = match bindings.lookup(keys) {
        Lookup::Action(action) => Some(navigation_action(action, page)),
        Lookup::Prefix => Some(Navigation::Prefix),
        Lookup::None => None,
    };
    if found != Some(Navigation::Prefix) {
        keys.clear();
    }
    found
}
#[cfg(test)]
fn factory_bindings() -> runyte::keymap::native_preview::Bindings {
    runyte::keymap::default_keymap().native_preview(
        runyte::command::Mode::Normal,
        runyte::keymap::BindingScope::Global,
    )
}
#[cfg(test)]
fn preview_navigation(
    keys: &mut runyte::keymap::KeySequence,
    key: KeyEvent,
    page: f64,
) -> Option<Navigation> {
    let value = effective_navigation(keys, key, page, &factory_bindings());
    // This historical unit helper tests only the local navigation subset.
    value.filter(|value| {
        !matches!(
            value,
            Navigation::Forward | Navigation::ForwardInput | Navigation::DismissAndForward
        )
    })
}
#[cfg(test)]
fn navigation_hints(keys: &runyte::keymap::KeySequence) -> String {
    factory_bindings().hints(keys)
}
#[cfg(test)]
fn editor_pane_key(key: KeyEvent) -> bool {
    let Ok(Some(runyte::input::InputEvent::Key(stroke))) =
        runyte::tui::input::convert_event(Event::Key(key))
    else {
        return false;
    };
    factory_bindings().lookup(&runyte::keymap::KeySequence::from(stroke))
        == runyte::keymap::native_preview::Lookup::Action(
            runyte::keymap::native_preview::Action::Forward,
        )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputRoute {
    Local,
    Host,
    Discard,
}
fn input_route(
    captured: Option<Target>,
    current: Option<Target>,
    source: Option<(usize, usize)>,
    eligible: bool,
    handoff: Option<Target>,
) -> InputRoute {
    let Some(captured) = captured else {
        return InputRoute::Host;
    };
    if source != Some((captured.pane, captured.source)) {
        return InputRoute::Discard;
    }
    if handoff == Some(captured) && current.is_none_or(|current| current == captured) {
        return InputRoute::Host;
    }
    if current != Some(captured) {
        return InputRoute::Discard;
    }
    if eligible {
        InputRoute::Local
    } else {
        InputRoute::Host
    }
}
fn forwards_structural_input(
    forwarded: &mut Option<Target>,
    target: Option<Target>,
    structural: bool,
) -> bool {
    if target.is_some() && *forwarded == target {
        if structural {
            return true;
        }
        *forwarded = None;
    }
    false
}
#[derive(Default)]
struct InputOwnership {
    handoff: Option<Target>,
    forwarded: Option<Target>,
}
impl InputOwnership {
    fn route(
        &mut self,
        captured: Option<Target>,
        current: Option<Target>,
        source: Option<(usize, usize)>,
        eligible: bool,
        structural: bool,
    ) -> InputRoute {
        let route = input_route(captured, current, source, eligible, self.handoff);
        if route == InputRoute::Local
            && forwards_structural_input(&mut self.forwarded, captured, structural)
        {
            InputRoute::Host
        } else {
            route
        }
    }
    fn forward(&mut self, target: Option<Target>, authored: bool, structural: bool) {
        if authored || !structural {
            self.forwarded = target;
        }
    }
    fn back(&mut self, target: Option<Target>) {
        self.handoff = target;
    }
}
impl State {
    fn navigation(
        &mut self,
        key: KeyEvent,
        page: f64,
        bindings: &runyte::keymap::native_preview::Bindings,
    ) -> Option<Navigation> {
        if self.keys_bindings.as_ref() != Some(bindings) {
            self.keys.clear();
            self.keys_bindings = Some(bindings.clone());
        }
        effective_navigation(&mut self.keys, key, page, bindings)
    }
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
    ownership: InputOwnership,
}
/// The palette the layer around a page should follow. Authored HTML and SVG
/// keep their own appearance and leave unpainted pixels transparent, so they
/// need the white ground they had before themes; the helper's reply says
/// which kind of page it drew, and the language stands in until it arrives.
fn page_palette<'a>(
    pane: &'a Pane,
    displayed: Option<Result<&Output, &String>>,
) -> Option<&'a runyte::document_preview::Palette> {
    let themed = match displayed {
        Some(Ok(output)) => output.reply.themed,
        _ => !matches!(pane.document.language.as_str(), "html" | "svg"),
    };
    pane.palette.as_ref().filter(|_| themed)
}
/// The colours the frontend paints around and over a document raster:
/// its ground before the first frame, progress and failure text, and the
/// link and key-hint bars. They follow the page the helper renders.
#[derive(Debug, PartialEq)]
struct LayerColours {
    ground: u32,
    text: u32,
    link: u32,
    error: u32,
}
impl LayerColours {
    fn new(palette: Option<&runyte::document_preview::Palette>) -> Self {
        let value = |[red, green, blue]: [u8; 3]| u32::from_be_bytes([0, red, green, blue]);
        match palette {
            Some(palette) => Self {
                ground: value(palette.background),
                text: value(palette.foreground),
                link: value(palette.link),
                error: value(palette.error),
            },
            None => Self {
                ground: 0xffffff,
                text: 0x333333,
                link: 0x0969da,
                error: 0x990000,
            },
        }
    }
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
            return Err(format!("{error}; helper {status:?}: {diagnostic}"));
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
        theme: request.theme.clone(),
        reply,
        image: Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])),
    })
}
#[derive(Debug, PartialEq)]
enum PointerResult {
    Forward,
    Consumed,
    Changed,
}

impl Views {
    pub(super) fn cancel_drag(&mut self) {
        for state in self.states.values_mut() {
            state.dragging = false;
        }
    }

    fn pointer(
        &mut self,
        pane: Option<&Pane>,
        media_input: bool,
        position: Point<Pixels>,
        metrics: CellMetrics,
        phase: u8,
    ) -> PointerResult {
        let captured = self
            .states
            .iter()
            .find_map(|(id, state)| state.dragging.then_some(*id));
        // Releases belong to the captured selection, even outside every body.
        if phase == 0 || phase == 2 {
            self.cancel_drag();
        }
        let Some(pane) = pane else {
            return PointerResult::Forward;
        };
        if !media_input {
            return PointerResult::Consumed;
        }
        if !pane.active {
            // The host recognizes the capture and applies this press as focus only.
            return if phase == 0 {
                PointerResult::Forward
            } else {
                PointerResult::Consumed
            };
        }
        let state = self.states.get_mut(&pane.pane).unwrap();
        // Do not select against stale raster geometry after scrolling/resizing.
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
            return PointerResult::Consumed;
        }
        let point = [
            f32::from(position.x) - pane.body.x as f32 * metrics.width,
            f32::from(position.y) - pane.body.y as f32 * metrics.height,
        ];
        if phase == 0 {
            state.selection = Some([point, point]);
            state.dragging = true;
        } else if captured == Some(pane.pane) {
            if let Some(selection) = &mut state.selection {
                selection[1] = point;
            }
        } else {
            return PointerResult::Consumed;
        }
        PointerResult::Changed
    }
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
        let attachment = self.frame.as_ref()?.attachment;
        if self.previews.attachment != attachment {
            self.previews = Views {
                attachment,
                ..Default::default()
            };
        }
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
                    keys_bindings: None,
                    request: None,
                    output: None,
                    cache: None,
                    scroll: [0.0; 2],
                    selection: None,
                    dragging: false,
                };
            }
            if !pane.active || !frame.media_input {
                state.dragging = false;
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
                theme: pane.palette.clone(),
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
            let colours = LayerColours::new(page_palette(
                pane,
                displayed.map(|result| result.map(|output| &**output)),
            ));
            let mut layer = div()
                .absolute()
                .left(px(area.x as f32 * self.metrics.width))
                .top(px(area.y as f32 * self.metrics.height))
                .w(px(area.width as f32 * self.metrics.width))
                .h(px(area.height as f32 * self.metrics.height))
                .overflow_hidden()
                .bg(rgb(colours.ground));
            let back = frame
                .preview_bindings
                .label(runyte::keymap::native_preview::Action::Back);
            let back_hint = if back == "unbound" {
                String::new()
            } else {
                format!(" {back} returns to source")
            };
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
                Some(Err(error)) => layer.child(
                    div()
                        .p_4()
                        .text_color(rgb(colours.error))
                        .child(format!("{error}{back_hint}")),
                ),
                None => layer.child(
                    div()
                        .p_4()
                        .text_color(rgb(colours.text))
                        .child(format!("Preparing document preview…{back_hint}")),
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
                        .bg(rgb(colours.ground))
                        .text_color(rgb(colours.link))
                        .child(format!("Link: {link} (navigation disabled)")),
                );
            }
            if frame.preview_bindings.unavailable {
                layer = layer.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .bg(rgb(colours.ground))
                        .text_color(rgb(colours.link))
                        .child("Preview keymap exceeds its limit; input returns to source"),
                );
            }
            if !state.keys.is_empty() {
                layer = layer.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .bg(rgb(colours.ground))
                        .text_color(rgb(colours.link))
                        .child(
                            state
                                .keys_bindings
                                .as_ref()
                                .unwrap_or(&frame.preview_bindings)
                                .hints(&state.keys),
                        ),
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
        let pane = self.preview_at(position);
        let result = self.previews.pointer(
            pane.as_ref(),
            self.frame.as_ref().is_some_and(|f| f.media_input),
            position,
            self.metrics,
            phase,
        );
        if result == PointerResult::Changed {
            cx.notify();
        }
        result != PointerResult::Forward
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
    pub(super) fn preview_key(
        &mut self,
        key: KeyEvent,
        input: &NativeInput,
        window_bindings: &runyte::keymap::native_window::Bindings,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(frame) = &self.frame else {
            return false;
        };
        if frame.attachment != input.attachment {
            return true;
        }
        let current = frame
            .previews
            .iter()
            .find(|pane| pane.active)
            .map(|pane| Target {
                pane: pane.pane,
                source: pane.document.source,
                generation: pane.document.generation,
            });
        match self.previews.ownership.route(
            input.preview_target,
            current,
            frame.preview_source,
            frame.media_input,
            frame.preview_structural,
        ) {
            InputRoute::Host => return false,
            InputRoute::Discard => return true,
            InputRoute::Local => (),
        }
        let Some(pane) = frame.previews.iter().find(|pane| pane.active) else {
            return true;
        };
        let Some(state) = self
            .previews
            .states
            .get_mut(&pane.pane)
            .filter(|state| !state.hidden && state.generation == pane.document.generation)
        else {
            return true;
        };
        let page = f64::from(pane.body.height as f32 * self.metrics.height / state.zoom);
        let navigation = state.navigation(key, page, &input.preview_bindings);
        if matches!(
            navigation,
            Some(Navigation::Forward | Navigation::ForwardInput)
        ) {
            self.previews.ownership.forward(
                input.preview_target,
                navigation == Some(Navigation::ForwardInput),
                frame.preview_structural,
            );
            cx.notify();
            return false;
        }
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
        if navigation.is_some()
            && !matches!(
                navigation,
                Some(Navigation::Back | Navigation::DismissAndForward)
            )
        {
            state.clamp_scroll();
            cx.notify();
            return true;
        }
        let fallback = match runyte::tui::input::convert_event(Event::Key(key)) {
            Ok(Some(runyte::input::InputEvent::Key(stroke))) => {
                input.preview_bindings.fallback_action(stroke)
            }
            _ => None,
        };
        if navigation == Some(Navigation::Back)
            || navigation.is_none()
                && fallback == Some(runyte::keymap::native_preview::Action::Back)
        {
            self.previews.ownership.back(input.preview_target);
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
        if navigation.is_none() && fallback == Some(runyte::keymap::native_preview::Action::Copy) {
            if let Some(Ok(output)) = &state.output {
                cx.write_to_clipboard(ClipboardItem::new_string(output.reply.selected.clone()));
            }
            return true;
        }
        if let Ok(Some(runyte::input::InputEvent::Key(stroke))) =
            runyte::tui::input::convert_event(Event::Key(key))
            && window_bindings.lookup(stroke).is_some_and(|action| {
                matches!(action, runyte::keymap::native_window::Action::FontSize(_))
            })
        {
            return false;
        }
        if matches!(
            fallback,
            Some(
                runyte::keymap::native_preview::Action::Forward
                    | runyte::keymap::native_preview::Action::ForwardInput
            )
        ) && navigation != Some(Navigation::DismissAndForward)
        {
            self.previews.ownership.forward(
                input.preview_target,
                fallback == Some(runyte::keymap::native_preview::Action::ForwardInput),
                frame.preview_structural,
            );
            return false;
        }
        if navigation == Some(Navigation::DismissAndForward)
            || fallback != Some(runyte::keymap::native_preview::Action::Forward)
        {
            self.previews.ownership.back(input.preview_target);
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
