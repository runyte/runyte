// SPDX-License-Identifier: MPL-2.0
//! Bounded, engine-independent capture for the experimental native renderer.
mod limits;
pub use limits::{MAX_BYTES, MAX_HELPER_MESSAGE_BYTES};
pub const MAX_CAPTURES: usize = 8;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentPreview {
    pub generation: u64,
    pub source: usize,
    pub text: String,
    pub selection_only: bool,
    pub language: String,
    pub path: Option<std::path::PathBuf>,
}

/// The colours a generated preview document paints with, as sRGB channels.
///
/// Derived from the resolved editor theme so a Markdown page or highlighted
/// source reads as part of the editor rather than as a white page in a dark
/// window. Authored HTML and SVG keep their own colours; only the pages the
/// preview generates use these.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Palette {
    /// Whether the ground is dark, which also answers `prefers-color-scheme`.
    pub dark: bool,
    pub background: [u8; 3],
    pub foreground: [u8; 3],
    /// Quotations and other secondary prose.
    pub muted: [u8; 3],
    /// Code blocks and alternate table rows, one small step off the ground.
    pub surface: [u8; 3],
    /// Inline code, a little further off than `surface` so code inside a
    /// striped table row still stands apart.
    pub code: [u8; 3],
    /// Rules under headings, table cells, and the quotation bar.
    pub border: [u8; 3],
    pub link: [u8; 3],
    pub heading: [u8; 3],
    pub error: [u8; 3],
    /// Each syntax scope the theme colours, by its name in
    /// [`crate::syntax::SCOPES`]. Scopes left to the foreground are absent.
    pub syntax: Vec<(&'static str, [u8; 3])>,
}

impl Palette {
    pub fn from_theme(theme: &crate::config::Theme) -> Self {
        use crate::config::{Color, ThemeAppearance};
        // A theme painting on the terminal's own ground has no appearance;
        // choose the ground `ui::native_default_colors` gives the window.
        const CONTRAST_MIDPOINT: f64 = 0.179;
        let light = match theme.appearance() {
            Some(appearance) => appearance == ThemeAppearance::Light,
            None => theme
                .foreground
                .relative_luminance()
                .is_some_and(|foreground| foreground < CONTRAST_MIDPOINT),
        };
        let (ground, ink) = if light {
            ((0xf8, 0xf8, 0xf8), (0x20, 0x20, 0x20))
        } else {
            ((0x18, 0x18, 0x18), (0xdd, 0xdd, 0xdd))
        };
        let rgb = |(red, green, blue): (u8, u8, u8)| [red, green, blue];
        let background = theme.background.channels().unwrap_or(ground);
        let foreground = rgb(theme.foreground.channels().unwrap_or(ink));
        let appearance = if light {
            ThemeAppearance::Light
        } else {
            ThemeAppearance::Dark
        };
        let step = |amount| {
            Color::Rgb(background.0, background.1, background.2)
                .stepped_off(appearance, amount)
                .channels()
                .map_or(rgb(background), rgb)
        };
        let named = |name| {
            crate::syntax::Scope::named(name)
                .and_then(|scope| theme.syntax_color(scope))
                .and_then(Color::channels)
                .map(rgb)
        };
        Self {
            dark: !light,
            background: rgb(background),
            foreground,
            muted: theme.muted.channels().map_or(foreground, rgb),
            surface: step(0.05),
            code: step(0.1),
            border: step(0.2),
            link: named("markup.link.text")
                .or_else(|| named("markup.link.url"))
                .or_else(|| theme.accent.channels().map(rgb))
                .unwrap_or(foreground),
            heading: named("markup.heading").unwrap_or(foreground),
            error: theme.error.channels().map_or(foreground, rgb),
            syntax: crate::syntax::SCOPES
                .iter()
                .filter_map(|name| Some((*name, named(name)?)))
                .collect(),
        }
    }
}

/// Capture already-resolved editor spans. Empty spans mean no explicit selection.
/// The caller supplies the same inclusive/half-open semantics used by yank.
pub fn capture(
    buffer: &crate::buffer::Buffer,
    spans: &[(usize, usize)],
) -> Result<(String, bool), &'static str> {
    if spans.is_empty() {
        if buffer.len_bytes() > MAX_BYTES {
            return Err("preview limit is 1 MiB; select a smaller section");
        }
        return Ok((buffer.to_string(), false));
    }
    if spans.len() > 256 {
        return Err("preview prototype limit is 256 selections");
    }
    let mut text = String::new();
    for &(from, to) in spans {
        if to.saturating_sub(from) > MAX_BYTES {
            return Err("selected preview exceeds 1 MiB");
        }
        let part = buffer.slice(from, to);
        let separator = usize::from(!text.is_empty());
        if text.len() + part.len() + separator > MAX_BYTES {
            return Err("selected preview exceeds 1 MiB");
        }
        if separator != 0 {
            text.push('\n');
        }
        text.push_str(&part);
    }
    Ok((text, true))
}

#[cfg(test)]
mod tests {
    use super::Palette;
    use crate::config::{Color, Config};

    #[test]
    fn palette_follows_every_built_in_theme_ground_and_syntax() {
        let config = Config::default();
        for name in config.theme_names() {
            let theme = config.resolve_theme(name).unwrap();
            let palette = Palette::from_theme(&theme);
            if let Some((red, green, blue)) = theme.background.channels() {
                assert_eq!(palette.background, [red, green, blue], "{name}");
                assert_eq!(
                    palette.dark,
                    theme.appearance() == Some(crate::config::ThemeAppearance::Dark),
                    "{name}"
                );
            }
            for &(scope, colour) in &palette.syntax {
                let scope = crate::syntax::Scope::named(scope).unwrap();
                let (red, green, blue) = theme.syntax_color(scope).unwrap().channels().unwrap();
                assert_eq!(colour, [red, green, blue], "{name}");
            }
            // Surfaces step away from the ground in the direction of its appearance.
            let lightness = |c: [u8; 3]| c.iter().map(|&v| u32::from(v)).sum::<u32>();
            let ground = lightness(palette.background);
            for surface in [palette.surface, palette.code, palette.border] {
                if palette.dark {
                    assert!(lightness(surface) >= ground, "{name}");
                } else {
                    assert!(lightness(surface) <= ground, "{name}");
                }
            }
        }
    }

    #[test]
    fn palette_prefers_link_and_heading_scopes_and_falls_back_for_terminal_grounds() {
        let mut theme = Config::default().startup_theme().unwrap().1;
        let link = crate::syntax::Scope::named("markup.link.text").unwrap();
        let url = crate::syntax::Scope::named("markup.link.url").unwrap();
        let heading = crate::syntax::Scope::named("markup.heading").unwrap();
        theme.syntax[link.index()] = Some(Color::Rgb(1, 2, 3));
        theme.syntax[heading.index()] = Some(Color::Rgb(4, 5, 6));
        let palette = Palette::from_theme(&theme);
        assert_eq!(palette.link, [1, 2, 3]);
        assert_eq!(palette.heading, [4, 5, 6]);

        theme.syntax[link.index()] = None;
        theme.syntax[url.index()] = None;
        theme.syntax[heading.index()] = None;
        theme.accent = Color::Rgb(7, 8, 9);
        theme.background = Color::Reset;
        theme.foreground = Color::Rgb(0x10, 0x10, 0x10);
        theme.muted = Color::Reset;
        let palette = Palette::from_theme(&theme);
        assert_eq!(palette.link, [7, 8, 9]);
        assert_eq!(palette.heading, [0x10, 0x10, 0x10]);
        assert_eq!(palette.muted, [0x10, 0x10, 0x10]);
        assert!(!palette.dark);
        assert_eq!(palette.background, [0xf8, 0xf8, 0xf8]);

        theme.foreground = Color::Reset;
        theme.accent = Color::Reset;
        let palette = Palette::from_theme(&theme);
        assert!(palette.dark);
        assert_eq!(palette.background, [0x18, 0x18, 0x18]);
        assert_eq!(palette.foreground, [0xdd, 0xdd, 0xdd]);
        assert_eq!(palette.link, palette.foreground);
    }
}
