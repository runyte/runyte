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
// Share pure limits without linking the editor into the renderer library.
#[path = "../../../src/document_preview/limits.rs"]
mod limits;

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
    /// The editor theme for generated pages; absent keeps the light page.
    #[serde(default)]
    theme: Option<format::Palette>,
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
    /// Whether the page took the request's theme; authored pages never do.
    themed: bool,
}
/// Font files the host bundles. Code on generated pages, in blocks and inline,
/// names their family first; system fonts set the prose and cover what those
/// faces lack.
pub type Fonts = &'static [&'static [u8]];

fn default_zoom() -> f32 {
    1.0
}
/// What `prefers-color-scheme` answers, so authored HTML can follow the theme.
fn color_scheme(request: &Request) -> ColorScheme {
    match &request.theme {
        Some(theme) if theme.dark => ColorScheme::Dark,
        _ => ColorScheme::Light,
    }
}

/// Render bounded newline-delimited requests, retaining document layout between frames.
pub fn serve(
    mut input: impl BufRead,
    mut output: impl Write,
    fonts: Fonts,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cache = None;
    loop {
        let mut line = Vec::new();
        input
            .by_ref()
            .take((limits::MAX_HELPER_MESSAGE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if line.is_empty() {
            return Ok(());
        }
        if line.len() > limits::MAX_HELPER_MESSAGE_BYTES || line.last() != Some(&b'\n') {
            return Err("preview request exceeds wire budget or lacks framing".into());
        }
        render(
            serde_json::from_slice(&line)?,
            &mut cache,
            &mut output,
            fonts,
        )?;
    }
}

/// Render one bounded JSON request, using the same wire as the retained helper.
pub fn run_once(
    mut args: impl Iterator<Item = std::ffi::OsString>,
    input: impl Read,
    mut output: impl Write,
    fonts: Fonts,
) -> Result<(), Box<dyn std::error::Error>> {
    if args.next().is_some() {
        return Err("unknown preview helper argument".into());
    }
    let mut bytes = Vec::new();
    input
        .take((limits::MAX_HELPER_MESSAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limits::MAX_HELPER_MESSAGE_BYTES {
        return Err("preview request exceeds wire budget".into());
    }
    render(
        serde_json::from_slice(&bytes)?,
        &mut None,
        &mut output,
        fonts,
    )
}

/// Blitz's list-marker face and the bundled faces over the system collection.
/// Supplying a context replaces Blitz's default one, which registers the
/// marker face itself, so it has to be registered here too.
fn font_context(fonts: Fonts) -> parley::FontContext {
    use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
    let mut context = parley::FontContext {
        source_cache: SourceCache::new_shared(),
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: true,
        }),
    };
    for face in std::iter::once(&blitz_dom::BULLET_FONT).chain(fonts) {
        context
            .collection
            .register_fonts(Blob::new(std::sync::Arc::new(*face) as _), None);
    }
    context
}

fn build_document(request: &Request, fonts: Fonts) -> (HtmlDocument, bool) {
    let svg_fragment = request.selection_only
        && roxmltree::Document::parse(&request.text)
            .is_ok_and(|doc| doc.root_element().tag_name().name() == "svg");
    let page = format::document(
        &request.text,
        if svg_fragment {
            "svg"
        } else {
            &request.language
        },
        request.theme.as_ref(),
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
        &page.html,
        DocumentConfig {
            base_url,
            net_provider: Some(std::sync::Arc::new(assets::LocalImages::new(root))),
            font_ctx: Some(font_context(fonts)),
            viewport: Some(Viewport::new(
                request.width,
                request.height,
                request.scale * request.zoom,
                color_scheme(request),
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
    (doc, page.themed)
}

struct Cache {
    request: Request,
    doc: HtmlDocument,
    renderer: anyrender_vello_cpu::VelloCpuImageRenderer,
    pixels: Vec<u8>,
    render_size: [u32; 2],
    cacheable: bool,
    themed: bool,
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
    fonts: Fonts,
) -> Result<(), Box<dyn std::error::Error>> {
    if request.text.len() > limits::MAX_BYTES
        || request.width == 0
        || request.height == 0
        || request.width > 4096
        || request.height > 4096
        || request.width as u64 * request.height as u64 > 4_000_000
        || !request.zoom.is_finite()
        || !(0.25..=4.0).contains(&request.zoom)
        || !request.scale.is_finite()
        || !(0.01..=4.0).contains(&request.scale)
        || request
            .theme
            .as_ref()
            .is_some_and(|theme| theme.syntax.len() > format::MAX_SYNTAX_SCOPES)
    {
        return Err("preview exceeds input or viewport budget".into());
    }
    let same_document = cache.as_ref().is_some_and(|cache| {
        let old = &cache.request;
        old.text == request.text
            && old.language == request.language
            && old.path == request.path
            && old.selection_only == request.selection_only
            && old.theme == request.theme
    });
    if !same_document {
        let (doc, themed) = build_document(&request, fonts);
        let cacheable = viewport_independent(&doc);
        *cache = Some(Cache {
            request: request.clone(),
            doc,
            cacheable,
            themed,
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
        themed,
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
            color_scheme(&request),
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
                .map(|href| href.chars().take(200).collect());
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
    let selected = doc.get_selected_text().unwrap_or_default();
    if selected.len() > limits::MAX_BYTES {
        return Err("selected preview text exceeds 1 MiB; select a smaller section".into());
    }
    let reply = Reply {
        width: request.width,
        height: request.height,
        selected,
        link,
        scroll,
        raster_width: raster.width,
        raster_height: raster.height,
        origin: raster.origin,
        cacheable: adjacent,
        max_scroll: [maximum.x, maximum.y],
        themed: *themed,
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

#[cfg(test)]
#[path = "tests/budgets.rs"]
mod budget_tests;

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    /// The family `preview.css` asks for, as the bundled faces name themselves.
    const BUNDLED_FAMILY: &str = "JetBrainsMono Nerd Font";
    /// The faces the desktop executable hands the helper, embedded for tests only.
    const FONTS: super::Fonts = &[
        include_bytes!("../../../assets/fonts/jetbrains-mono/JetBrainsMonoNerdFont-Medium.ttf"),
        include_bytes!(
            "../../../assets/fonts/jetbrains-mono/JetBrainsMonoNerdFont-MediumItalic.ttf"
        ),
        include_bytes!("../../../assets/fonts/jetbrains-mono/JetBrainsMonoNerdFont-Bold.ttf"),
        include_bytes!("../../../assets/fonts/jetbrains-mono/JetBrainsMonoNerdFont-BoldItalic.ttf"),
    ];

    fn palette(dark: bool, syntax: usize) -> Value {
        let ground = if dark {
            [0x10, 0x20, 0x30]
        } else {
            [0xf0, 0xf0, 0xe0]
        };
        json!({
            "dark": dark, "background": ground, "foreground": [0x80, 0x80, 0x80],
            "muted": [0x80, 0x80, 0x80], "surface": ground, "code": ground,
            "border": [0x80, 0x80, 0x80], "link": [0x80, 0x80, 0x80],
            "heading": [0x80, 0x80, 0x80], "error": [0x80, 0x80, 0x80],
            "syntax": vec![json!(["keyword", [1, 2, 3]]); syntax],
        })
    }
    fn request(text: &str, language: &str, theme: Value) -> Value {
        json!({
            "text": text, "language": language, "path": null, "width": 32,
            "height": 24, "scale": 1, "scroll": [0, 0], "selection": null,
            "theme": theme,
        })
    }
    /// Each reply header with the first RGBA pixel of its raster.
    fn serve(requests: &[Value]) -> Result<Vec<(Value, [u8; 4])>, String> {
        let mut input = Vec::new();
        for request in requests {
            serde_json::to_writer(&mut input, request).unwrap();
            input.push(b'\n');
        }
        let mut output = Vec::new();
        super::serve(&input[..], &mut output, FONTS).map_err(|e| e.to_string())?;
        let mut frames = Vec::new();
        let mut rest = &output[..];
        while !rest.is_empty() {
            let end = rest.iter().position(|b| *b == b'\n').unwrap();
            let header: Value = serde_json::from_slice(&rest[..end]).unwrap();
            let pixels = header["raster_width"].as_u64().unwrap()
                * header["raster_height"].as_u64().unwrap()
                * 4;
            let pixel = rest[end + 1..end + 5].try_into().unwrap();
            rest = &rest[end + 1 + pixels as usize..];
            frames.push((header, pixel));
        }
        Ok(frames)
    }

    #[test]
    fn theme_change_rebuilds_the_retained_document() {
        let frames = serve(&[
            request("# Title", "markdown", Value::Null),
            request("# Title", "markdown", palette(true, 1)),
            request("# Title", "markdown", Value::Null),
        ])
        .unwrap();
        let summary = frames
            .iter()
            .map(|(header, pixel)| (header["themed"].as_bool().unwrap(), *pixel))
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            [
                (false, [0xff; 4]),
                (true, [0x10, 0x20, 0x30, 0xff]),
                (false, [0xff; 4]),
            ]
        );
    }

    #[test]
    fn authored_html_answers_prefers_color_scheme_without_taking_the_theme() {
        let html = "<style>@media (prefers-color-scheme: dark) \
                    { html { background: #000000 } }</style><p>x</p>";
        let frames = serve(&[
            request(html, "html", palette(true, 0)),
            request(html, "html", palette(false, 0)),
            request(html, "html", Value::Null),
        ])
        .unwrap();
        assert!(frames.iter().all(|(header, _)| header["themed"] == false));
        assert_eq!(frames[0].1, [0, 0, 0, 0xff]);
        // Unstyled authored pages stay transparent over the frontend's white ground.
        assert_eq!(frames[1].1[3], 0);
        assert_eq!(frames[2].1[3], 0);
    }

    #[test]
    fn oversized_scope_lists_are_refused() {
        let limit = super::format::MAX_SYNTAX_SCOPES;
        assert!(serve(&[request("x", "text", palette(true, limit))]).is_ok());
        assert!(serve(&[request("x", "text", palette(true, limit + 1))]).is_err());
    }

    #[test]
    fn generated_pages_ask_for_the_family_the_bundled_faces_register() {
        let mut context = super::font_context(FONTS);
        let family = context
            .collection
            .family_by_name(BUNDLED_FAMILY)
            .expect("bundled family is registered");
        assert_eq!(family.fonts().len(), FONTS.len());
        assert!(context.collection.family_by_name("Bullet").is_some());
        // Code blocks and inline code take the bundled face; prose does not.
        let css = include_str!("../preview.css");
        let rule = |selector: &str| css.lines().find(|line| line.starts_with(selector)).unwrap();
        let wanted = format!("font-family: \"{}\", monospace;", BUNDLED_FAMILY);
        assert!(rule("pre {").contains(&wanted));
        assert!(rule("code {").contains(&wanted));
        assert!(!rule(".markdown-body {").contains(&wanted));
    }
}
