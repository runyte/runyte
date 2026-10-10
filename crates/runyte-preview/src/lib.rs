// SPDX-License-Identifier: MPL-2.0
//! Isolated, disposable static-document renderer. No shell or network provider.
use anyrender::ImageRenderer;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_traits::shell::{ColorScheme, Viewport};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Read, Write};
mod assets;
mod format;
mod raster;

#[derive(Clone, Deserialize)]
struct Request {
    text: String,
    #[serde(default)]
    selection_only: bool,
    path: Option<std::path::PathBuf>,
    language: String,
    width: u32,
    height: u32,
    scale: f32,
    #[serde(default = "default_zoom")]
    zoom: f32,
    scroll: [f64; 2],
    selection: Option<[[f32; 2]; 2]>,
    #[serde(default)]
    adjacent: bool,
}
#[derive(Serialize)]
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
fn default_zoom() -> f32 {
    1.0
}

/// Render bounded newline-delimited requests, retaining document layout between frames.
pub fn serve(
    mut input: impl BufRead,
    mut output: impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cache = None;
    loop {
        let mut line = Vec::new();
        input
            .by_ref()
            .take(1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if line.is_empty() {
            return Ok(());
        }
        if line.len() > 1024 * 1024 || line.last() != Some(&b'\n') {
            return Err("request exceeds 1 MiB or lacks framing".into());
        }
        render(serde_json::from_slice(&line)?, &mut cache, &mut output)?;
    }
}

/// Render one bounded JSON request, using the same wire as the retained helper.
pub fn run_once(
    mut args: impl Iterator<Item = std::ffi::OsString>,
    input: impl Read,
    mut output: impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    if args.next().is_some() {
        return Err("unknown preview helper argument".into());
    }
    let mut bytes = Vec::new();
    input.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err("request exceeds 1 MiB".into());
    }
    render(serde_json::from_slice(&bytes)?, &mut None, &mut output)
}

fn build_document(request: &Request) -> HtmlDocument {
    let svg_fragment = request.selection_only
        && roxmltree::Document::parse(&request.text)
            .is_ok_and(|doc| doc.root_element().tag_name().name() == "svg");
    let html = format::document(
        &request.text,
        if svg_fragment {
            "svg"
        } else {
            &request.language
        },
    );
    let root = request
        .path
        .as_ref()
        .and_then(|p| p.parent())
        .and_then(|p| p.canonicalize().ok());
    let base_url = root
        .as_ref()
        .and_then(|p| blitz_traits::net::Url::from_directory_path(p).ok())
        .map(|u| u.to_string())
        .or_else(|| Some("https://preview.invalid/".into()));
    let mut doc = HtmlDocument::from_html(
        &html,
        DocumentConfig {
            base_url,
            net_provider: Some(std::sync::Arc::new(assets::LocalImages::new(root))),
            viewport: Some(Viewport::new(
                request.width,
                request.height,
                request.scale * request.zoom,
                ColorScheme::Light,
            )),
            ..Default::default()
        },
    );
    // usvg's default href resolver can read files independently of NetProvider.
    // Remove external SVG references before Blitz constructs SVG render trees.
    let external_svg = doc
        .query_selector_all("svg *")
        .unwrap_or_default()
        .into_iter()
        .filter(|id| {
            doc.get_node(*id)
                .and_then(|n| n.attrs())
                .is_some_and(|attrs| {
                    attrs
                        .iter()
                        .any(|a| &*a.name.local == "href" && !a.value.starts_with('#'))
                })
        })
        .collect::<Vec<_>>();
    let mut external_svg = external_svg;
    external_svg.extend(
        doc.query_selector_all("svg image, svg feImage")
            .unwrap_or_default(),
    );
    external_svg.sort_unstable();
    external_svg.dedup();
    for id in external_svg {
        doc.mutate().remove_node(id);
    }
    doc.handle_messages();
    // usvg's generic sans-serif can name a font absent on Linux. Keep authored
    // attribute families first, with explicit platform fallbacks for SVG text.
    for id in doc
        .query_selector_all("svg, svg [font-family]")
        .unwrap_or_default()
    {
        let family = doc
            .get_node(id)
            .and_then(|n| n.attr("font-family".into()))
            .unwrap_or("sans-serif")
            .to_owned();
        let name = markup5ever::QualName::new(None, "".into(), "font-family".into());
        doc.mutate().set_attribute(
            id,
            name,
            &format!("{family}, Noto Sans, DejaVu Sans, Liberation Sans, Arial, Helvetica"),
        );
    }
    doc.resolve(0.0);
    doc.handle_messages();
    doc.resolve(0.0);
    doc
}

struct Cache {
    request: Request,
    doc: HtmlDocument,
    renderer: anyrender_vello_cpu::VelloCpuImageRenderer,
    pixels: Vec<u8>,
    render_size: [u32; 2],
    cacheable: bool,
}
fn viewport_independent(doc: &HtmlDocument) -> bool {
    doc.query_selector_all("*")
        .unwrap_or_default()
        .into_iter()
        .all(|id| {
            doc.get_node(id).is_none_or(|node| {
                !matches!(
                    node.taffy_position(),
                    taffy::Position::Fixed | taffy::Position::Sticky
                )
            })
        })
}

fn render(
    request: Request,
    cache: &mut Option<Cache>,
    mut out: impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    if request.text.len() > 128 * 1024
        || request.width == 0
        || request.height == 0
        || request.width > 4096
        || request.height > 4096
        || request.width as u64 * request.height as u64 > 4_000_000
        || !request.zoom.is_finite()
        || !(0.25..=4.0).contains(&request.zoom)
        || !request.scale.is_finite()
        || !(0.01..=4.0).contains(&request.scale)
    {
        return Err("preview exceeds input or viewport budget".into());
    }
    let same_document = cache.as_ref().is_some_and(|cache| {
        let old = &cache.request;
        old.text == request.text
            && old.language == request.language
            && old.path == request.path
            && old.selection_only == request.selection_only
    });
    if !same_document {
        let doc = build_document(&request);
        let cacheable = viewport_independent(&doc);
        *cache = Some(Cache {
            request: request.clone(),
            doc,
            cacheable,
            renderer: anyrender_vello_cpu::VelloCpuImageRenderer::new(
                request.width,
                request.height,
            ),
            pixels: Vec::new(),
            render_size: [request.width, request.height],
        });
    }
    let Cache {
        request: old,
        doc,
        renderer,
        pixels,
        render_size,
        cacheable,
    } = cache.as_mut().unwrap();
    if old.width != request.width
        || old.height != request.height
        || old.scale != request.scale
        || old.zoom != request.zoom
    {
        doc.set_viewport(Viewport::new(
            request.width,
            request.height,
            request.scale * request.zoom,
            ColorScheme::Light,
        ));
        doc.resolve(0.0);
        *cacheable = viewport_independent(doc);
    }
    *old = request.clone();
    doc.clear_text_selection();
    doc.scroll_viewport_by(-1e12, -1e12);
    let maximum = doc.viewport_scroll();
    let previous = doc.viewport_scroll();
    doc.scroll_viewport_by(
        previous.x - request.scroll[0],
        previous.y - request.scroll[1],
    );
    let mut link = None;
    if let Some([a, b]) = request.selection {
        // Input positions are logical window pixels; Blitz hit testing takes
        // CSS pixels. Raster/display scale does not change the hit position.
        let a = a.map(|v| v / request.zoom);
        let b = b.map(|v| v / request.zoom);
        if let (Some((an, ao)), Some((bn, bo))) = (
            doc.find_text_position(a[0], a[1]),
            doc.find_text_position(b[0], b[1]),
        ) {
            doc.set_text_selection(an, ao, bn, bo);
        }
        if let Some(hit) = doc.hit(b[0], b[1])
            && let Ok(Some(id)) = doc.closest(hit.node_id, "a[href]")
        {
            link = doc
                .get_node(id)
                .and_then(|n| n.attr("href".into()))
                .map(str::to_owned);
        }
    }
    let scroll = [doc.viewport_scroll().x, doc.viewport_scroll().y];
    let adjacent = request.adjacent && *cacheable && request.selection.is_none();
    let raster = raster::Raster::new(
        request.width,
        request.height,
        f64::from(request.scale * request.zoom),
        scroll,
        adjacent,
    );
    if *render_size != [raster.width, raster.height] {
        renderer.resize(raster.width, raster.height);
        *render_size = [raster.width, raster.height];
    }
    let reply = Reply {
        width: request.width,
        height: request.height,
        selected: doc.get_selected_text().unwrap_or_default(),
        link,
        scroll,
        raster_width: raster.width,
        raster_height: raster.height,
        origin: raster.origin,
        cacheable: adjacent,
        max_scroll: [maximum.x, maximum.y],
    };
    doc.set_viewport_scroll(blitz_dom::Point {
        x: raster.origin[0],
        y: raster.origin[1],
    });
    renderer.reset();
    renderer.render_to_vec(
        |scene| {
            blitz_paint::paint_scene(
                scene,
                doc,
                f64::from(request.scale * request.zoom),
                raster.width,
                raster.height,
                0,
                0,
            )
        },
        pixels,
    );
    doc.set_viewport_scroll(blitz_dom::Point {
        x: scroll[0],
        y: scroll[1],
    });
    serde_json::to_writer(&mut out, &reply)?;
    out.write_all(b"\n")?;
    out.write_all(pixels)?;
    out.flush()?;
    Ok(())
}
