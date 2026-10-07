// SPDX-License-Identifier: MPL-2.0

//! One bounded media worker. Decoding and Poppler never run on either UI loop.
use anyhow::{Context, Result, ensure};
use gpui::RenderImage;
use std::{
    collections::HashMap,
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
    pub width: f32,
    pub height: f32,
    pub words: Vec<Word>,
    pub text_error: Option<String>,
}
struct Loaded {
    key: Key,
    result: Result<(Arc<Page>, usize), String>,
}
type Cached = Result<Arc<Page>, String>;
pub(super) struct Loader {
    requests: mpsc::SyncSender<Key>,
    results: mpsc::Receiver<Loaded>,
    cache: HashMap<Key, Option<Cached>>,
    cancel: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Loader {
    pub fn new(bridge: Arc<super::Bridge>) -> Self {
        let (requests, input) = mpsc::sync_channel::<Key>(8);
        let (output, results) = mpsc::sync_channel(8);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker = std::thread::Builder::new()
            .name("runyte-media".into())
            .spawn(move || {
                while !worker_cancel.load(Ordering::Acquire) {
                    let key = match input.recv_timeout(Duration::from_millis(50)) {
                        Ok(key) => key,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(_) => break,
                    };
                    let result = load(&key, &worker_cancel).map_err(|error| format!("{error:#}"));
                    if output.try_send(Loaded { key, result }).is_err() {
                        break;
                    }
                    let _ = bridge.wake.try_send(());
                }
            })
            .expect("start media worker");
        Self {
            requests,
            results,
            cache: HashMap::new(),
            cancel,
            worker: Some(worker),
        }
    }
    pub fn get(&mut self, path: &Path, page: usize) -> Option<Cached> {
        let metadata = path.metadata().ok();
        let key = Key {
            path: path.to_owned(),
            page,
            modified: metadata.as_ref().and_then(|m| m.modified().ok()),
            length: metadata.map_or(0, |m| m.len()),
        };
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }
        // Keep at most eight entries, including in-flight requests.
        if self.cache.len() >= 8 {
            self.cache.retain(|_, value| value.is_none());
        }
        if self.cache.len() < 8 && self.requests.try_send(key.clone()).is_ok() {
            self.cache.insert(key, None);
        }
        None
    }
    pub fn poll(&mut self, bridge: &super::Bridge) -> bool {
        let mut changed = false;
        while let Ok(loaded) = self.results.try_recv() {
            let current = loaded.key.path.metadata().ok();
            if current.as_ref().and_then(|m| m.modified().ok()) != loaded.key.modified
                || current.as_ref().map_or(0, |m| m.len()) != loaded.key.length
            {
                self.cache.remove(&loaded.key);
                changed = true;
                continue;
            }
            let value = match loaded.result {
                Ok((image, pages)) => {
                    bridge
                        .pages
                        .lock()
                        .unwrap()
                        .push((loaded.key.path.clone(), pages));
                    let (w, h) = *bridge.dimensions.lock().unwrap();
                    bridge.send(crossterm::event::Event::Resize(w, h));
                    Ok(image)
                }
                Err(error) => Err(error),
            };
            if self.cache.contains_key(&loaded.key) {
                self.cache.insert(loaded.key, Some(value));
            }
            changed = true;
        }
        changed
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
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
        wait(
            Command::new("pdftoppm")
                .args([
                    "-f",
                    &key.page.to_string(),
                    "-l",
                    &key.page.to_string(),
                    "-singlefile",
                    "-scale-to",
                    "1600",
                    "-png",
                ])
                .arg(&key.path)
                .arg(&prefix)
                .stdout(Stdio::null()),
            cancel,
            Some((&prefix.with_extension("png"), 16 * 1024 * 1024)),
        )?;
        (prefix.with_extension("png"), pages)
    } else {
        (key.path.clone(), 1)
    };
    let (words, text_error) = if pdf {
        match pdf_words(&key.path, key.page, temporary.path(), cancel) {
            Ok(words) => (words, None),
            Err(error) => (Vec::new(), Some(format!("PDF text unavailable: {error:#}"))),
        }
    } else {
        (Vec::new(), None)
    };
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    use image::ImageDecoder;
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut decoded = image::DynamicImage::from_decoder(decoder)?;
    decoded.apply_orientation(orientation);
    let decoded = if decoded.width() > 2048 || decoded.height() > 2048 {
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
