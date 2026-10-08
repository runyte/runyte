# Bundled JetBrains Mono Nerd Font

These four unmodified TrueType faces come from Nerd Fonts **v3.4.0**,
based on JetBrains Mono **2.304**:
<https://github.com/ryanoasis/nerd-fonts/releases/tag/v3.4.0>.

Archive: `JetBrainsMono.tar.xz` (SHA-256
`ef552a3e638f25125c6ad4c51176a6adcdce295ab1d2ffacf0db060caf8c1582`).
Only the `JetBrainsMono Nerd Font` family is included: Medium, Medium Italic,
Bold, and Bold Italic. The files have not been renamed internally, subsetted,
or otherwise modified by Runyte. Their combined size is about 9.5 MiB.

Native builds embed these bytes with `include_bytes!` and register them before
opening the window. No font installation or network access is required to run.
Terminal builds do not embed them; terminal applications control their own fonts.
The native grid uses Medium at 15 logical pixels, with true Bold/Italic faces
selected from cell modifiers. Glyphs outside this font's coverage can still use
GPUI's platform fallback (for example, emoji or CJK).

The font is licensed under SIL OFL 1.1, separately from Runyte's MPL-2.0 code.
Bundling and embedding are permitted with the copyright and license retained;
the font must not be sold by itself. See `licenses/jetbrains-mono/OFL.txt`,
`licenses/jetbrains-mono/NERD-FONTS-LICENSE.txt`, and the glyph notices there.
The font files themselves also contain the JetBrains copyright and OFL notice.

## File SHA-256

- `JetBrainsMonoNerdFont-Bold.ttf`: `e82e27a7f37c9a0a13cc4e417503a149c6a0280586930772d2ebed803159c864`
- `JetBrainsMonoNerdFont-BoldItalic.ttf`: `961222be7bce59f310b41a3e158368d5ea22a47a0ac31defd57caac9b9e82cb0`
- `JetBrainsMonoNerdFont-Medium.ttf`: `04a099702e3e808a922c28c4a4da656e9ea783d6fa6bed33ae67f6f4e0afb937`
- `JetBrainsMonoNerdFont-MediumItalic.ttf`: `93cea2a63565378cfaa5d4c12f88b637252c865183c4c2951d869d20e84398dd`
