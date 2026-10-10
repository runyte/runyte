// SPDX-License-Identifier: MPL-2.0

//! Private, bounded helper transport. Hayro never runs on an editor/UI worker.
mod diagnostics;
pub(super) mod render;
mod text;

use super::media::{Detail, Word};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const MAX_HEADER: usize = 8 * 1024 * 1024;
const MAX_PIXELS: usize = 8 * 1024 * 1024;
const MAX_OUTPUT: usize = 12 + MAX_HEADER + MAX_PIXELS * 4;
const MAGIC: &[u8; 8] = b"RYTPDF01";

#[derive(Serialize, Deserialize)]
pub(super) struct Header {
    pub width: u32,
    pub height: u32,
    pub pages: usize,
    pub words: Vec<Word>,
    pub text_error: Option<String>,
}
pub(super) struct Raster {
    pub header: Header,
    pub pixels: Vec<u8>,
}

fn validate_detail(detail: Detail) -> Result<()> {
    ensure!(
        detail.full.into_iter().all(|v| (1..=524288).contains(&v))
            && detail.size.into_iter().all(|v| (1..=4096).contains(&v))
            && u64::from(detail.size[0]) * u64::from(detail.size[1]) <= MAX_PIXELS as u64
            && (0..2).all(|i| detail.origin[i]
                .checked_add(detail.size[i])
                .is_some_and(|end| end <= detail.full[i])),
        "invalid PDF detail geometry"
    );
    Ok(())
}

pub(super) fn load(
    path: &Path,
    page: usize,
    detail: Option<Detail>,
    cancel: &AtomicBool,
) -> Result<Raster> {
    let mut command = super::helper::command(super::helper::Role::Pdf)?;
    command.arg(path).arg(page.to_string());
    if let Some(detail) = detail {
        validate_detail(detail)?;
        for value in detail
            .full
            .into_iter()
            .chain(detail.origin)
            .chain(detail.size)
        {
            command.arg(value.to_string());
        }
    }
    let output = run(&mut command, cancel, Duration::from_secs(15), MAX_OUTPUT)?;
    decode(&output, detail)
}

fn run(
    command: &mut Command,
    cancel: &AtomicBool,
    timeout: Duration,
    limit: usize,
) -> Result<Vec<u8>> {
    ensure!(!cancel.load(Ordering::Acquire), "PDF rendering cancelled");
    let mut child = super::helper::Process::spawn(command, super::helper::Role::Pdf)?;
    // PDF has no input; close the pipe before waiting for its output.
    child.child.stdin.take();
    let mut stdout = child.child.stdout.take().unwrap();
    let (send, receive) = mpsc::sync_channel(1);
    let reader = std::thread::Builder::new()
        .name("runyte-pdf-output".into())
        .spawn(move || {
            let result = (|| -> Result<Vec<u8>> {
                let mut bytes = Vec::new();
                let mut chunk = [0; 65536];
                loop {
                    let count = stdout.read(&mut chunk)?;
                    if count == 0 {
                        return Ok(bytes);
                    }
                    ensure!(
                        count <= limit.saturating_sub(bytes.len()),
                        "PDF helper output exceeded its size limit"
                    );
                    bytes.extend_from_slice(&chunk[..count]);
                }
            })();
            let _ = send.send(result);
        })?;
    let deadline = Instant::now() + timeout;
    let result = (|| -> Result<Vec<u8>> {
        let mut output = None;
        loop {
            ensure!(!cancel.load(Ordering::Acquire), "PDF rendering cancelled");
            ensure!(
                Instant::now() < deadline,
                "Hayro PDF rendering exceeded its deadline"
            );
            match receive.recv_timeout(Duration::from_millis(10)) {
                Ok(result) => output = Some(result?),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) if output.is_none() => {
                    anyhow::bail!("PDF helper output reader stopped")
                }
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
            let status = child.status()?;
            if let Some(status) = status {
                ensure!(status.success(), "Hayro PDF helper failed ({status})");
                if let Some(output) = output.take() {
                    return Ok(output);
                }
            }
        }
    })();
    drop(child);
    let _ = reader.join();
    result
}

fn decode(bytes: &[u8], detail: Option<Detail>) -> Result<Raster> {
    ensure!(
        bytes.len() >= 12 && &bytes[..8] == MAGIC,
        "invalid Hayro helper response"
    );
    let length = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    ensure!(
        length <= MAX_HEADER && length <= bytes.len() - 12,
        "invalid PDF response header length"
    );
    let response: Result<Header, String> = serde_json::from_slice(&bytes[12..12 + length])?;
    let header = response.map_err(anyhow::Error::msg)?;
    ensure!(
        (1..=10000).contains(&header.pages),
        "PDF must contain 1–10000 pages"
    );
    ensure!(
        header.width > 0
            && header.height > 0
            && header.width <= 4096
            && header.height <= 4096
            && u64::from(header.width) * u64::from(header.height) <= MAX_PIXELS as u64,
        "invalid PDF raster size"
    );
    if let Some(detail) = detail {
        ensure!(
            [header.width, header.height] == detail.size,
            "unexpected PDF detail dimensions"
        );
    } else {
        ensure!(
            header.width <= 1600 && header.height <= 1600,
            "unexpected PDF base dimensions"
        );
    }
    ensure!(
        header.words.len() <= 100000
            && header.words.iter().map(|w| w.text.len()).sum::<usize>() <= 4 * 1024 * 1024,
        "PDF text exceeds limits"
    );
    ensure!(
        header.words.iter().all(|w| w
            .bounds
            .iter()
            .all(|n| n.is_finite() && (0.0..=1.0).contains(n))
            && w.bounds[0] <= w.bounds[2]
            && w.bounds[1] <= w.bounds[3]),
        "invalid PDF text geometry"
    );
    let pixels = &bytes[12 + length..];
    ensure!(
        pixels.len() == header.width as usize * header.height as usize * 4,
        "invalid PDF pixel payload"
    );
    Ok(Raster {
        header,
        pixels: pixels.to_vec(),
    })
}

pub fn helper_main(mut args: impl Iterator<Item = std::ffi::OsString>) -> Result<()> {
    restrict_helper()?;
    let result = (|| -> Result<Raster> {
        let path = PathBuf::from(args.next().context("missing PDF path")?);
        let page = args
            .next()
            .context("missing PDF page")?
            .to_str()
            .context("invalid PDF page")?
            .parse::<usize>()?;
        let dimensions = args
            .map(|value| {
                value
                    .to_str()
                    .context("invalid PDF geometry")?
                    .parse::<u32>()
                    .map_err(Into::into)
            })
            .collect::<Result<Vec<_>>>()?;
        let detail = match dimensions.as_slice() {
            [] => None,
            [w, h, x, y, cw, ch] => Some(Detail {
                full: [*w, *h],
                origin: [*x, *y],
                size: [*cw, *ch],
            }),
            _ => anyhow::bail!("invalid PDF helper arguments"),
        };
        if let Some(detail) = detail {
            validate_detail(detail)?;
        }
        render::page(&path, page, detail)
    })();
    let (header, pixels) = match result {
        Ok(raster) => (Ok(raster.header), raster.pixels),
        Err(error) => (Err(format!("{error:#}")), Vec::new()),
    };
    let json = serde_json::to_vec(&header)?;
    ensure!(
        json.len() <= MAX_HEADER,
        "PDF response metadata exceeds 8 MiB"
    );
    let mut output = std::io::stdout().lock();
    output.write_all(MAGIC)?;
    output.write_all(&(json.len() as u32).to_le_bytes())?;
    output.write_all(&json)?;
    output.write_all(&pixels)?;
    Ok(())
}

fn restrict_helper() -> Result<()> {
    #[cfg(unix)]
    for (resource, limit) in [
        (libc::RLIMIT_AS, address_space_limit()?),
        (libc::RLIMIT_CPU, 15),
        (libc::RLIMIT_CORE, 0),
    ] {
        let limit = libc::rlimit {
            rlim_cur: limit,
            rlim_max: limit,
        };
        // This runs only in the already-spawned helper, before parsing a PDF.
        ensure!(
            unsafe { libc::setrlimit(resource, &limit) } == 0,
            "set PDF helper resource limit: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/pdf.rs"]
pub(super) mod tests;

// macOS maps its large shared cache into each process. Permit 1 GiB of
// additional mappings beyond the helper's startup footprint, measured before
// reading untrusted document bytes. Linux has no such baseline reservation.
#[cfg(unix)]
fn address_space_limit() -> Result<libc::rlim_t> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::mach_task_basic_info_data_t = unsafe { std::mem::zeroed() };
        let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
        #[allow(deprecated)]
        let status = unsafe {
            libc::task_info(
                libc::mach_task_self(),
                libc::MACH_TASK_BASIC_INFO,
                (&mut info as *mut libc::mach_task_basic_info_data_t).cast(),
                &mut count,
            )
        };
        ensure!(
            status == libc::KERN_SUCCESS,
            "read PDF helper memory baseline"
        );
        info.virtual_size
            .checked_add(1024 * 1024 * 1024)
            .context("PDF helper address-space limit overflow")
    }
    #[cfg(not(target_os = "macos"))]
    Ok(1024 * 1024 * 1024)
}
