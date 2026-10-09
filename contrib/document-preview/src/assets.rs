// SPDX-License-Identifier: MPL-2.0
use blitz_traits::net::{NetHandler, NetProvider, Request};
use std::{
    io::Read,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
pub struct LocalImages {
    root: Option<PathBuf>,
    count: AtomicUsize,
}
impl LocalImages {
    pub fn new(root: Option<PathBuf>) -> Self {
        Self {
            root,
            count: AtomicUsize::new(0),
        }
    }
}
impl NetProvider for LocalImages {
    fn fetch(&self, _: usize, request: Request, handler: Box<dyn NetHandler>) {
        // Deny network, data URLs, stylesheets, fonts and iframe documents.
        if request.url.scheme() != "file" || self.count.fetch_add(1, Ordering::Relaxed) >= 16 {
            return;
        }
        let Some(root) = &self.root else { return };
        let Ok(path) = request
            .url
            .to_file_path()
            .and_then(|p| p.canonicalize().map_err(|_| ()))
        else {
            return;
        };
        if !path.starts_with(root)
            || !matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("png" | "jpg" | "jpeg" | "webp")
            )
        {
            return;
        }
        let Ok(file) = std::fs::File::open(&path) else {
            return;
        };
        if !file
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() <= 2 * 1024 * 1024)
        {
            return;
        }
        let mut bytes = Vec::new();
        if file
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() > 2 * 1024 * 1024
        {
            return;
        }
        // Decode dimensions before handing anything to the engine's decoder.
        let Ok(reader) =
            image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format()
        else {
            return;
        };
        if !reader
            .into_dimensions()
            .is_ok_and(|(w, h)| w <= 2048 && h <= 2048)
        {
            return;
        }
        handler.bytes(request.url.to_string(), bytes.into());
    }
}
