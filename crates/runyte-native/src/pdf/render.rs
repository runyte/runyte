// SPDX-License-Identifier: MPL-2.0

//! Called only inside the disposable native PDF helper (or by fixture tests).
use super::super::media::Detail;
use super::{Header, Raster, text::Collector};
use anyhow::{Result, ensure};
use hayro::{
    hayro_interpret::{
        Context, InterpreterCache, InterpreterSettings, font::FontQuery, interpret_page,
        util::TransformExt,
    },
    hayro_syntax::Pdf,
    kurbo::{Affine, Rect},
    vello_cpu,
};
use std::{
    io::Read,
    path::Path,
    sync::{Arc, Mutex},
};

pub(crate) fn page(path: &Path, number: usize, detail: Option<Detail>) -> Result<Raster> {
    let diagnostics = super::diagnostics::Scope::new()?;
    let file = std::fs::File::open(path)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= 128 * 1024 * 1024,
        "PDF must be a regular file up to 128 MiB"
    );
    let mut bytes = Vec::new();
    file.take(128 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 128 * 1024 * 1024, "PDF exceeds 128 MiB");
    let pdf = Pdf::new(bytes).map_err(|error| {
        anyhow::anyhow!(
            "Hayro could not load PDF (invalid or password-protected document): {error:?}"
        )
    })?;
    let pages = pdf.pages();
    ensure!(
        (1..=10000).contains(&pages.len()),
        "PDF must contain 1–10000 pages"
    );
    ensure!(
        (1..=pages.len()).contains(&number),
        "PDF page does not exist"
    );
    let page = &pages[number - 1];
    let (width, height) = page.render_dimensions();
    ensure!(
        width.is_finite() && height.is_finite() && width > 0. && height > 0.,
        "invalid PDF page dimensions"
    );
    let initial = page.initial_transform(true).to_kurbo();
    ensure!(
        initial.as_coeffs().iter().all(|v| v.is_finite()),
        "invalid PDF page transform"
    );
    let scale = 1600. / width.max(height);
    let (size, full, origin) = if let Some(detail) = detail {
        super::validate_detail(detail)?;
        (detail.size, detail.full, detail.origin)
    } else {
        let size = [
            (width * scale).round().clamp(1., 1600.) as u32,
            (height * scale).round().clamp(1., 1600.) as u32,
        ];
        (size, size, [0, 0])
    };
    let failure = Arc::new(Mutex::new(None::<String>));
    let mut settings = InterpreterSettings::default();
    let fonts = settings.font_resolver.clone();
    let font_failure = failure.clone();
    settings.font_resolver = Arc::new(move |query| match query {
        FontQuery::Standard(_) => fonts(query),
        FontQuery::Fallback(_) => {
            *font_failure.lock().unwrap() =
                Some("non-embedded nonstandard font requires Poppler".into());
            None
        }
    });
    let cmaps = settings.cmap_resolver.clone();
    let cmap_failure = failure.clone();
    settings.cmap_resolver = Arc::new(move |query| {
        let result = cmaps(query);
        if result.is_none() {
            *cmap_failure.lock().unwrap() = Some("unresolved PDF CMap requires Poppler".into());
        }
        result
    });
    let warning_failure = failure.clone();
    settings.warning_sink = Arc::new(move |warning| {
        *warning_failure.lock().unwrap() = Some(format!("Hayro unsupported content: {warning:?}"));
    });
    let transform = Affine::translate((-(origin[0] as f64), -(origin[1] as f64)))
        * Affine::scale_non_uniform(
            full[0] as f64 / width as f64,
            full[1] as f64 / height as f64,
        )
        * initial;
    ensure!(
        transform.as_coeffs().iter().all(|v| v.is_finite()),
        "invalid PDF raster transform"
    );
    // Allocate only the visible crop, never the potentially 524288px full page.
    let mut context = vello_cpu::RenderContext::new(size[0] as u16, size[1] as u16);
    hayro::render_into(
        page,
        &hayro::RenderCache::new(),
        &settings,
        &hayro::RenderSettings::default(),
        &mut context,
        transform,
    );
    if let Some(error) = failure.lock().unwrap().take() {
        anyhow::bail!(error);
    }
    context.flush();
    let mut pixmap = vello_cpu::Pixmap::new(size[0] as u16, size[1] as u16);
    context.render_with(
        &mut pixmap,
        &mut vello_cpu::Resources::default(),
        vello_cpu::RasterizerSettings {
            target_init: vello_cpu::TargetInit::Clear(vello_cpu::color::palette::css::WHITE),
            ..Default::default()
        },
    );
    let (words, text_error) = if detail.is_none() {
        let cache = InterpreterCache::new();
        let mut context = Context::new(
            initial,
            Rect::new(0., 0., width as f64, height as f64),
            &cache,
            page.xref(),
            settings,
        );
        let mut collector = Collector::default();
        interpret_page(page, &mut context, &mut collector);
        collector.words(width as f64, height as f64)
    } else {
        (Vec::new(), None)
    };
    if let Some(error) = failure.lock().unwrap().take() {
        anyhow::bail!(error);
    }
    diagnostics.check()?;
    Ok(Raster {
        header: Header {
            width: size[0],
            height: size[1],
            pages: pages.len(),
            words,
            text_error,
        },
        pixels: pixmap.data_as_u8_slice().to_vec(),
    })
}
