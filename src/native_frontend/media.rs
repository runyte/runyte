// SPDX-License-Identifier: MPL-2.0

//! One bounded media worker. Decoding and Poppler never run on either UI loop.
use anyhow::{Context, Result, ensure};
use gpui::RenderImage;
use std::{
    collections::VecDeque,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    path: PathBuf,
    page: usize,
    modified: Option<std::time::SystemTime>,
    length: u64,
    detail: Option<Detail>,
}
/// A bounded visible-region raster in physical pixels. Page coordinates remain
/// those of the base raster, so sharper rendering never changes input geometry.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(super) struct Detail {
    pub full: [u32; 2],
    pub origin: [u32; 2],
    pub size: [u32; 2],
}
impl Detail {
    pub fn visible(
        origin: [f32; 2],
        size: [f32; 2],
        area: [f32; 2],
        density: f32,
        base: [f32; 2],
    ) -> Option<Self> {
        if origin
            .into_iter()
            .chain(size)
            .chain(area)
            .chain(base)
            .chain([density])
            .any(|v| !v.is_finite())
            || size
                .into_iter()
                .chain(area)
                .chain(base)
                .chain([density])
                .any(|v| v <= 0.)
        {
            return None;
        }
        // Cap a single refinement at 4096 per axis and eight million pixels.
        // Large/HiDPI windows degrade gracefully without allocating a full page.
        let density = density
            .min(4096. / area[0])
            .min(4096. / area[1])
            .min((8_000_000. / (area[0] * area[1])).sqrt())
            .min(524288. / size[0].max(size[1]));
        if size[0] * density <= base[0] && size[1] * density <= base[1] {
            return None;
        }
        let full = size.map(|v| (v * density).ceil() as u32);
        let mut start = [0; 2];
        let mut end = [0; 2];
        for axis in 0..2 {
            let scale = full[axis] as f32 / size[axis];
            start[axis] = ((-origin[axis] * scale).floor().max(0.) as u32).min(full[axis]);
            end[axis] = (((area[axis] - origin[axis]) * scale).ceil().max(0.) as u32)
                .min(full[axis])
                .min(start[axis].saturating_add(4096));
        }
        let size = [
            end[0].saturating_sub(start[0]),
            end[1].saturating_sub(start[1]),
        ];
        (size[0] > 0 && size[1] > 0).then_some(Self {
            full,
            origin: start,
            size,
        })
    }
    pub fn geometry(&self, origin: [f32; 2], size: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let scale = [size[0] / self.full[0] as f32, size[1] / self.full[1] as f32];
        (
            [
                origin[0] + self.origin[0] as f32 * scale[0],
                origin[1] + self.origin[1] as f32 * scale[1],
            ],
            [
                self.size[0] as f32 * scale[0],
                self.size[1] as f32 * scale[1],
            ],
        )
    }
}
#[derive(Clone, Debug)]
pub(super) struct Word {
    /// Normalized page coordinates, independent of zoom and raster resolution.
    pub bounds: [f32; 4],
    pub text: String,
    pub line: usize,
}
pub(super) struct Page {
    pub source: (Option<std::time::SystemTime>, u64),
    pub image: Arc<RenderImage>,
    pub animation: Option<super::animation::Animation>,
    pub width: f32,
    pub height: f32,
    pub words: Vec<Word>,
    pub text_error: Option<String>,
}
impl Page {
    fn bytes(&self) -> usize {
        self.animation
            .as_ref()
            .map_or_else(|| self.image.as_bytes(0).unwrap().len(), |a| a.bytes())
    }
    pub fn frame(&self, index: usize) -> &Arc<RenderImage> {
        self.animation
            .as_ref()
            .and_then(|a| a.frames.get(index))
            .unwrap_or(&self.image)
    }
}
struct Loaded {
    key: Key,
    result: Result<(Arc<Page>, usize), String>,
}
type Cached = Result<Arc<Page>, String>;
const CACHE_PAGES: usize = 8;
// Eight maximally sized animations still fit: byte accounting must not evict
// one of <=8 visible sources and make the next render decode it again.
const CACHE_BYTES: usize = 256 * 1024 * 1024;
const QUEUED_PAGES: usize = 8;

impl Key {
    fn read(path: &Path, page: usize) -> Self {
        let metadata = path.metadata().ok();
        Self {
            path: path.to_owned(),
            page,
            modified: metadata.as_ref().and_then(|m| m.modified().ok()),
            length: metadata.map_or(0, |m| m.len()),
            detail: None,
        }
    }
    fn same_source(&self, other: &Self) -> bool {
        self.path == other.path && self.modified == other.modified && self.length == other.length
    }
    fn is_pdf(&self) -> bool {
        self.path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
    }
}
struct Entry {
    key: Key,
    value: Cached,
    pages: Option<usize>,
}
struct Source {
    anchor: Key,
    pages: Option<usize>,
    announced: bool,
}
#[derive(Clone)]
struct Request {
    key: Key,
    speculative: bool,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct Schedule {
    // Oldest first; a displayed page moves to the back. Decoded memory stays bounded.
    cache: VecDeque<Entry>,
    details: VecDeque<Entry>,
    sources: VecDeque<Source>,
    demand: VecDeque<Key>,
    speculative: VecDeque<Key>,
    active: Option<Request>,
}
impl Schedule {
    fn queued(&self, key: &Key) -> bool {
        self.cache
            .iter()
            .chain(&self.details)
            .any(|entry| entry.key == *key)
            || self.demand.contains(key)
            || self.speculative.contains(key)
            || self
                .active
                .as_ref()
                .is_some_and(|job| job.key == *key && !job.cancel.load(Ordering::Acquire))
    }
    fn neighbors(&mut self, anchor: &Key, pages: usize) {
        // Only explicit demand moves this window. Prefetch completion never fans out.
        self.speculative.retain(|key| !key.same_source(anchor));
        if !anchor.is_pdf() {
            return;
        }
        for page in [
            anchor.page.checked_add(1),
            anchor.page.checked_sub(1),
            anchor.page.checked_add(2),
            anchor.page.checked_sub(2),
        ]
        .into_iter()
        .flatten()
        {
            if !(1..=pages).contains(&page) {
                continue;
            }
            let key = Key {
                page,
                ..anchor.clone()
            };
            if !self.queued(&key) && self.speculative.len() < QUEUED_PAGES {
                self.speculative.push_back(key);
            }
        }
    }
    fn retain_details(&mut self, visible: &[Key]) {
        self.demand
            .retain(|key| key.detail.is_none() || visible.contains(key));
        if let Some(active) = &self.active
            && active.key.detail.is_some()
            && !visible.contains(&active.key)
        {
            active.cancel.store(true, Ordering::Release);
        }
    }
    fn get(&mut self, key: Key) -> Option<Cached> {
        let stale = |old: &Key| old.path == key.path && !old.same_source(&key);
        self.cache.retain(|entry| !stale(&entry.key));
        self.details.retain(|entry| !stale(&entry.key));
        self.sources.retain(|source| !stale(&source.anchor));
        self.demand.retain(|old| !stale(old));
        self.speculative.retain(|old| !stale(old));
        if let Some(active) = &self.active
            && stale(&active.key)
        {
            active.cancel.store(true, Ordering::Release);
        }
        if key.detail.is_some() {
            if let Some(index) = self.details.iter().position(|entry| entry.key == key) {
                let entry = self.details.remove(index).unwrap();
                let value = entry.value.clone();
                self.details.push_back(entry);
                return Some(value);
            }
            if !self.queued(&key) {
                if let Some(active) = &self.active
                    && active.speculative
                {
                    active.cancel.store(true, Ordering::Release);
                }
                if self.demand.len() < QUEUED_PAGES {
                    self.demand.push_back(key);
                }
            }
            return None;
        }
        let cached = self
            .cache
            .iter()
            .position(|entry| entry.key == key)
            .map(|i| {
                let entry = self.cache.remove(i).unwrap();
                let value = (entry.value.clone(), entry.pages);
                self.cache.push_back(entry);
                value
            });
        let previous = self
            .sources
            .iter()
            .position(|source| source.anchor.same_source(&key))
            .and_then(|i| self.sources.remove(i));
        let moved = previous
            .as_ref()
            .is_none_or(|source| source.anchor.page != key.page);
        let pages = previous
            .as_ref()
            .and_then(|source| source.pages)
            .or_else(|| cached.as_ref().and_then(|(_, pages)| *pages));
        let announced = previous.as_ref().is_some_and(|source| source.announced);
        if self.sources.len() == CACHE_PAGES {
            self.sources.pop_front();
        }
        self.sources.push_back(Source {
            anchor: key.clone(),
            pages,
            announced,
        });
        if cached.is_none() {
            self.speculative.retain(|old| *old != key);
            if let Some(active) = &mut self.active {
                if active.key == key && !active.cancel.load(Ordering::Acquire) {
                    active.speculative = false;
                } else if active.speculative {
                    // A slow optional Poppler render must not hold up a requested page.
                    active.cancel.store(true, Ordering::Release);
                }
            }
            if !self.queued(&key) {
                if self.demand.len() == QUEUED_PAGES {
                    self.demand.pop_back();
                }
                self.demand.push_front(key.clone());
            }
        }
        if moved && let Some(pages) = pages {
            self.neighbors(&key, pages);
        }
        cached.map(|(value, _)| value)
    }
    fn next(&mut self) -> Option<Request> {
        if self.active.is_some() {
            return None;
        }
        let (key, speculative) = if let Some(key) = self.demand.pop_front() {
            (key, false)
        } else {
            (self.speculative.pop_front()?, true)
        };
        let request = Request {
            key,
            speculative,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        self.active = Some(request.clone());
        Some(request)
    }
    fn complete(&mut self, loaded: Loaded, current: &Key) -> (bool, Option<(PathBuf, usize)>) {
        let Some(active) = self.active.take() else {
            return (false, None);
        };
        if active.cancel.load(Ordering::Acquire) || !loaded.key.same_source(current) {
            return (!active.speculative, None);
        }
        if loaded.key.detail.is_some() {
            if self.details.len() == CACHE_PAGES {
                self.details.pop_front();
            }
            self.details.push_back(Entry {
                key: loaded.key,
                value: loaded.result.map(|(page, _)| page),
                pages: None,
            });
            return (true, None);
        }
        let mut publication = None;
        let (value, pages) = match loaded.result {
            Ok((image, pages)) => {
                if let Some(source) = self
                    .sources
                    .iter_mut()
                    .find(|source| source.anchor.same_source(&loaded.key))
                {
                    source.pages = Some(pages);
                    if !source.announced && !active.speculative {
                        source.announced = true;
                        publication = Some((loaded.key.path.clone(), pages));
                    }
                }
                (Ok(image), Some(pages))
            }
            Err(error) => (Err(error), None),
        };
        let bytes = value.as_ref().map_or(0, |page| page.bytes());
        while !self.cache.is_empty()
            && (self.cache.len() >= CACHE_PAGES
                || self
                    .cache
                    .iter()
                    .filter_map(|e| e.value.as_ref().ok())
                    .map(|p| p.bytes())
                    .sum::<usize>()
                    + bytes
                    > CACHE_BYTES)
        {
            self.cache.pop_front();
        }
        self.cache.push_back(Entry {
            key: loaded.key.clone(),
            value,
            pages,
        });
        if !active.speculative
            && let Some(pages) = pages
        {
            let anchor = self
                .sources
                .iter()
                .find(|source| source.anchor.same_source(&loaded.key))
                .map(|source| source.anchor.clone());
            if let Some(anchor) = anchor {
                self.neighbors(&anchor, pages);
            }
        }
        (!active.speculative, publication)
    }
}

pub(super) struct Loader {
    bridge: Arc<super::Bridge>,
    requests: Option<mpsc::SyncSender<Request>>,
    results: mpsc::Receiver<Loaded>,
    schedule: Schedule,
    worker: Option<std::thread::JoinHandle<()>>,
    visible_details: Vec<Key>,
}
impl Loader {
    pub fn new(bridge: Arc<super::Bridge>) -> Self {
        let (requests, input) = mpsc::sync_channel::<Request>(1);
        let (output, results) = mpsc::sync_channel(1);
        let worker_bridge = bridge.clone();
        let worker = std::thread::Builder::new()
            .name("runyte-media".into())
            .spawn(move || {
                while let Ok(request) = input.recv() {
                    let result = if request.cancel.load(Ordering::Acquire) {
                        Err("media request canceled".into())
                    } else {
                        load(&request.key, &request.cancel).map_err(|error| format!("{error:#}"))
                    };
                    if output
                        .try_send(Loaded {
                            key: request.key,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                    let _ = worker_bridge.wake.try_send(());
                }
            })
            .expect("start media worker");
        Self {
            bridge,
            requests: Some(requests),
            results,
            schedule: Schedule::default(),
            worker: Some(worker),
            visible_details: Vec::new(),
        }
    }
    fn dispatch(&mut self) {
        if let Some(request) = self.schedule.next()
            && self.requests.as_ref().unwrap().try_send(request).is_err()
        {
            self.schedule.active = None;
        }
    }
    pub fn get(&mut self, path: &Path, page: usize) -> Option<Cached> {
        let key = Key::read(path, page);
        let value = self.schedule.get(key.clone());
        if let Some(pages) = self
            .schedule
            .cache
            .iter()
            .find(|entry| entry.key == key)
            .and_then(|entry| entry.pages)
        {
            // Reopened buffers also need the count when their raster is already cached.
            publish_count(&self.bridge, &key, pages);
        }
        self.dispatch();
        value
    }
    pub fn begin_frame(&mut self) {
        self.visible_details.clear();
    }
    pub fn detail(&mut self, path: &Path, page: usize, detail: Detail) -> Option<Cached> {
        let mut key = Key::read(path, page);
        key.detail = Some(detail);
        if !key.is_pdf() {
            return None;
        }
        self.visible_details.push(key.clone());
        self.schedule.get(key)
    }
    pub fn end_frame(&mut self) {
        self.schedule.retain_details(&self.visible_details);
        self.dispatch();
    }
    pub fn poll(&mut self, bridge: &super::Bridge) -> bool {
        let mut changed = false;
        while let Ok(loaded) = self.results.try_recv() {
            let current = Key::read(&loaded.key.path, loaded.key.page);
            let (visible, publication) = self.schedule.complete(loaded, &current);
            changed |= visible;
            if let Some((_, pages)) = publication {
                publish_count(bridge, &current, pages);
            }
        }
        self.dispatch();
        changed
    }
}

pub(super) type PageCount = (PathBuf, Option<std::time::SystemTime>, u64, usize);

// Return true only when the host needs a new count. Cache hits refresh retention
// without generating input or redraws, and a replaced source supersedes its old count.
fn retain_count(counts: &mut Vec<PageCount>, key: &Key, pages: usize) -> bool {
    let count = (key.path.clone(), key.modified, key.length, pages);
    let previous = counts
        .iter()
        .position(|entry| entry.0 == key.path)
        .map(|index| counts.remove(index));
    let changed = previous.as_ref() != Some(&count);
    if counts.len() == CACHE_PAGES {
        counts.remove(0);
    }
    counts.push(count);
    changed
}

fn publish_count(bridge: &super::Bridge, key: &Key, pages: usize) {
    if retain_count(&mut bridge.pages.lock().unwrap(), key, pages) {
        let (w, h) = *bridge.dimensions.lock().unwrap();
        bridge.send(crossterm::event::Event::Resize(w, h));
    }
}
impl Drop for Loader {
    fn drop(&mut self) {
        if let Some(active) = &self.schedule.active {
            active.cancel.store(true, Ordering::Release);
        }
        // Closing the channel wakes an idle worker; there is no idle polling timer.
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn wait(
    command: &mut Command,
    cancel: &AtomicBool,
    output_limit: Option<(&Path, u64)>,
) -> Result<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start PDF renderer (install poppler-utils / Poppler)")?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if output_limit.is_some_and(|(path, limit)| {
            path.metadata().is_ok_and(|metadata| metadata.len() > limit)
        }) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("PDF helper output exceeded its size limit");
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                ensure!(
                    !output_limit.is_some_and(|(path, limit)| path
                        .metadata()
                        .is_ok_and(|metadata| metadata.len() > limit)),
                    "PDF helper output exceeded its size limit"
                );
                ensure!(status.success(), "PDF renderer rejected this file or page");
                return Ok(());
            }
            Ok(None) if Instant::now() < deadline && !cancel.load(Ordering::Acquire) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                match result {
                    Err(error) => return Err(error.into()),
                    _ => anyhow::bail!("PDF rendering exceeded 15 seconds"),
                }
            }
        }
    }
}
fn load(key: &Key, cancel: &AtomicBool) -> Result<(Arc<Page>, usize)> {
    let metadata = std::fs::metadata(&key.path)?;
    ensure!(metadata.is_file(), "media must be a regular file");
    ensure!(
        metadata.len() <= 128 * 1024 * 1024,
        "media exceeds 128 MiB limit"
    );
    let pdf = key
        .path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"));
    let temporary = tempfile::tempdir()?;
    let (path, pages) = if pdf {
        let info = temporary.path().join("info");
        let output = std::fs::File::create(&info)?;
        wait(
            Command::new("pdfinfo")
                .arg(&key.path)
                .env("LC_ALL", "C")
                .stdout(output),
            cancel,
            Some((&info, 65536)),
        )?;
        let mut text = String::new();
        std::fs::File::open(info)?
            .take(65536)
            .read_to_string(&mut text)?;
        let pages: usize = text
            .lines()
            .find_map(|line| line.strip_prefix("Pages:"))
            .context("PDF page count unavailable")?
            .trim()
            .parse()?;
        ensure!(
            (1..=10000).contains(&pages),
            "PDF must contain 1–10000 pages"
        );
        ensure!((1..=pages).contains(&key.page), "PDF page does not exist");
        let prefix = temporary.path().join("page");
        let mut command = Command::new("pdftoppm");
        command.args([
            "-f",
            &key.page.to_string(),
            "-l",
            &key.page.to_string(),
            "-singlefile",
            "-png",
        ]);
        if let Some(detail) = key.detail {
            ensure!(
                detail.full.into_iter().all(|v| (1..=524288).contains(&v))
                    && detail.size.into_iter().all(|v| (1..=4096).contains(&v))
                    && (0..2).all(|i| detail.origin[i]
                        .checked_add(detail.size[i])
                        .is_some_and(|end| end <= detail.full[i])),
                "invalid PDF detail bounds"
            );
            command.args([
                "-scale-dimension-before-rotation",
                "-scale-to-x",
                &detail.full[0].to_string(),
                "-scale-to-y",
                &detail.full[1].to_string(),
                "-x",
                &detail.origin[0].to_string(),
                "-y",
                &detail.origin[1].to_string(),
                "-W",
                &detail.size[0].to_string(),
                "-H",
                &detail.size[1].to_string(),
            ]);
        } else {
            command.args(["-scale-to", "1600"]);
        }
        wait(
            command.arg(&key.path).arg(&prefix).stdout(Stdio::null()),
            cancel,
            Some((
                &prefix.with_extension("png"),
                if key.detail.is_some() { 64 } else { 16 } * 1024 * 1024,
            )),
        )?;
        (prefix.with_extension("png"), pages)
    } else {
        (key.path.clone(), 1)
    };
    let (words, text_error) = if pdf && key.detail.is_none() {
        match pdf_words(&key.path, key.page, temporary.path(), cancel) {
            Ok(words) => (words, None),
            Err(error) => (Vec::new(), Some(format!("PDF text unavailable: {error:#}"))),
        }
    } else {
        (Vec::new(), None)
    };
    let mut reader = image::ImageReader::open(&path)?.with_guessed_format()?;
    if !pdf
        && let Some(format) = reader.format()
        && let Some(animation) = super::animation::decode(&path, format, cancel)?
    {
        let image = animation.frames[0].clone();
        let size = image.size(0);
        return Ok((
            Arc::new(Page {
                source: (key.modified, key.length),
                width: size.width.0 as f32,
                height: size.height.0 as f32,
                image,
                animation: Some(animation),
                words,
                text_error,
            }),
            pages,
        ));
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    use image::ImageDecoder;
    let mut decoder = reader.into_decoder()?;
    if let Some(detail) = key.detail {
        ensure!(
            decoder.dimensions() == (detail.size[0], detail.size[1]),
            "PDF renderer returned unexpected detail dimensions"
        );
    }
    let orientation = decoder.orientation()?;
    let mut decoded = image::DynamicImage::from_decoder(decoder)?;
    decoded.apply_orientation(orientation);
    let decoded = if !pdf && (decoded.width() > 2048 || decoded.height() > 2048) {
        decoded.thumbnail(2048, 2048)
    } else {
        decoded
    };
    let mut pixels = decoded.into_rgba8();
    ensure!(
        pixels.len() <= 64 * 1024 * 1024,
        "decoded image exceeds 64 MiB limit"
    );
    // GPUI's renderer consumes BGRA pixels.
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Ok((
        Arc::new(Page {
            source: (key.modified, key.length),
            width: pixels.width() as f32,
            height: pixels.height() as f32,
            image: Arc::new(RenderImage::new(
                [image::Frame::new(pixels)].into_iter().collect::<Vec<_>>(),
            )),
            animation: None,
            words,
            text_error,
        }),
        pages,
    ))
}

fn pdf_words(path: &Path, page: usize, root: &Path, cancel: &AtomicBool) -> Result<Vec<Word>> {
    let output = root.join("text.xhtml");
    wait(
        Command::new("pdftotext")
            .args([
                "-f",
                &page.to_string(),
                "-l",
                &page.to_string(),
                "-bbox-layout",
                "-enc",
                "UTF-8",
            ])
            .arg(path)
            .arg(&output)
            .stdout(Stdio::null()),
        cancel,
        Some((&output, 8 * 1024 * 1024)),
    )?;
    ensure!(
        output.metadata()?.len() <= 8 * 1024 * 1024,
        "PDF page text exceeds 8 MiB"
    );
    parse_words(&std::fs::read_to_string(output)?)
}

fn parse_words(text: &str) -> Result<Vec<Word>> {
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            nodes_limit: 200_000,
        },
    )?;
    let page = document
        .descendants()
        .find(|node| node.has_tag_name("page"))
        .context("PDF text has no page")?;
    let number = |node: roxmltree::Node<'_, '_>, attr: &str| -> Result<f32> {
        let value: f32 = node
            .attribute(attr)
            .context("missing PDF coordinate")?
            .parse()?;
        ensure!(value.is_finite(), "non-finite PDF coordinate");
        Ok(value)
    };
    let width = number(page, "width")?;
    let height = number(page, "height")?;
    ensure!(width > 0. && height > 0., "empty PDF text page");
    let mut words = Vec::new();
    for (line, node) in page
        .descendants()
        .filter(|node| node.has_tag_name("line"))
        .enumerate()
    {
        for word in node.children().filter(|node| node.has_tag_name("word")) {
            let bounds = [
                number(word, "xMin")? / width,
                number(word, "yMin")? / height,
                number(word, "xMax")? / width,
                number(word, "yMax")? / height,
            ];
            ensure!(
                bounds[0] <= bounds[2] && bounds[1] <= bounds[3],
                "reversed PDF text bounds"
            );
            words.push(Word {
                bounds,
                text: word.text().unwrap_or_default().to_owned(),
                line,
            });
        }
    }
    Ok(words)
}

#[cfg(test)]
#[path = "tests/media.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/media_loader.rs"]
mod loader_tests;
