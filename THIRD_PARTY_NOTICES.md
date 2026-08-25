# Third-party notices

This document records external material incorporated into Runyte, along with
any asset whose provenance or distribution status should remain visible. It
supplements, but does not replace, the root `LICENSE`.

## Built-in color themes

Runyte incorporates palette values from the projects below into the built-in
themes defined in `src/config.rs`. Runyte maps those values onto its own editor,
syntax, selection, Git, and diff roles; it does not embed or execute the
upstream theme implementations.

The `dark`, `light`, and `paper` built-in themes are Runyte-authored palettes.
The audit found no third-party source material requiring a notice for those
three themes.

### GitHub Light

Project: GitHub Theme for Neovim

Author: projekt0n̅

Source:
<https://github.com/projekt0n/github-nvim-theme/tree/c106c9472154d6b2c74b74565616b877ae8ed31d>

Upstream revision: `c106c9472154d6b2c74b74565616b877ae8ed31d`

License: MIT; copyright (c) 2021 projekt0n̅. The generated Primer light
primitives incorporated upstream carry a separate MIT notice, copyright (c)
2018 GitHub Inc.

Runyte's `github-light` theme maps the upstream palette and syntax
specification onto Runyte's presentation roles and adds its own mode-specific
cursors and jump-label colors. Both upstream MIT notices are preserved in
`licenses/GitHub-Nvim-Theme-MIT.txt`.

### Base16 Default Dark

Project: Base16 Default Dark, maintained in Tinted Theming schemes

Original scheme author: Chris Kempson

Source:
<https://github.com/tinted-theming/schemes/blob/fdca32a0d14ec80ad83a78a9ccb85592ca6cb9e1/base16/default-dark.yaml>

Upstream revision: `fdca32a0d14ec80ad83a78a9ccb85592ca6cb9e1`

License: MIT; copyright (c) 2022 Tinted Theming

Runyte's `base16` theme uses the Default Dark palette for its principal editor
and syntax colors. Runyte adds its own mode cursors, jump labels, selection
distinction, and diff backgrounds. The exact upstream license text is
preserved in `licenses/Base16-Default-Dark-MIT.txt`.

### Gruvbox

Project: Gruvbox

Author: Pavel Pertsev (`morhetz`)

Source:
<https://github.com/morhetz/gruvbox/blob/5d15b2765f59754d7ac263c88a0f6e3e58124951/colors/gruvbox.vim>

Upstream revision: `5d15b2765f59754d7ac263c88a0f6e3e58124951`

License: MIT/X11, as declared by the upstream README and `package.json`

Runyte's `gruvbox` theme uses colors from Gruvbox's canonical dark palette and
assigns them to Runyte editor and syntax roles. Runyte supplies its own
selection and diff-background tints. The upstream repository does not contain
a standalone license file or a copyright year; `licenses/Gruvbox-MIT.txt`
therefore records the named author, the upstream license declaration, and the
complete standard MIT terms without inventing a year.

### Everforest

Project: Everforest

Author: sainnhe

Source:
<https://github.com/sainnhe/everforest/blob/85a86eb62409e3ec88713bff3d1b9d7374e112e4/autoload/everforest.vim>

Upstream revision: `85a86eb62409e3ec88713bff3d1b9d7374e112e4`

License: MIT; copyright (c) 2019 sainnhe

Runyte's six `everforest-dark-*` and `everforest-light-*` themes use
Everforest's hard, medium, and soft background variants and its dark and light
foreground palettes. Runyte maps those colors to its presentation roles and
uses deliberately darker purple shades for small jump-label text in the light
variants. The exact upstream license text is preserved in
`licenses/Everforest-MIT.txt`.

### Catppuccin

Project: Catppuccin Palette

Author: Catppuccin

Source:
<https://github.com/catppuccin/palette/blob/07d02aa110ef9eb7e7427afca5c73ba9cf7f8ebd/palette.json>

Upstream revision: `07d02aa110ef9eb7e7427afca5c73ba9cf7f8ebd`

License: MIT; copyright (c) 2021 Catppuccin

Runyte's `latte`, `frappe`, `macchiato`, and `mocha` themes use Catppuccin's
canonical palette values. Runyte shares one role mapping across the four
flavours and adds palette-local selection and diff-background tints. The exact
upstream license text is preserved in `licenses/Catppuccin-MIT.txt`.

### Nightfox: Nordfox and Terafox

Project: Nightfox

Author: James Simpson (`EdenEast`)

Sources:

- <https://github.com/EdenEast/nightfox.nvim/blob/4dacd3f0185a2227bdf3b6c0975a8f0bf87cac9a/lua/nightfox/palette/nordfox.lua>;
  and
- <https://github.com/EdenEast/nightfox.nvim/blob/4dacd3f0185a2227bdf3b6c0975a8f0bf87cac9a/lua/nightfox/palette/terafox.lua>.

Upstream revision: `4dacd3f0185a2227bdf3b6c0975a8f0bf87cac9a`

License: MIT; copyright (c) 2021 James Simpson

Runyte's `nordfox`, `nordfox-warm`, and `terafox` themes map the corresponding
Nightfox palette and syntax-spec values to Runyte roles and add Runyte-specific
selection and diff-background tints. The exact upstream license text is
preserved in `licenses/Nightfox-MIT.txt`.

### Zenbones

Project: Zenbones

Author: Michael Chris Lopez

Source:
<https://github.com/zenbones-theme/zenbones.nvim/tree/8304d8df9b823ff11e103afa62f38c39f534abe6>

Upstream revision: `8304d8df9b823ff11e103afa62f38c39f534abe6`

License: MIT; copyright (c) 2022 Michael Chris Lopez

Runyte's 19 concrete `*-light` and `*-dark` Zenbones variants map every light
and dark palette supported by the named upstream colorschemes onto Runyte's
presentation roles. The values come from the upstream generated Vim highlight
groups. `randombones` is a runtime selector over those colorschemes and
supplies no separate palette, so Runyte does not expose it as a theme. The
exact upstream license text is preserved in `licenses/Zenbones-MIT.txt`.

## Syntax highlighting and language grammars

Runyte statically links the following syntax implementation crates from the
Helix `tree-house` project:

- `tree-house 0.4.0`, revision
  `75d213c4033ccf3ea98a5539d7d7c4005d8fb2ea`; and
- `tree-house-bindings 0.3.2`, revision
  `358188cc6693cc7e78ed637b66822401df320803`.

Both crates declare the Mozilla Public License 2.0. The complete MPL-2.0 text
is included in the repository's root `LICENSE`. The bindings crate also vendors
Tree-sitter C source under the MIT License, with copyright (c) 2018 Max
Brunsfeld.

Runyte also statically links these grammar crates and uses the highlight or
injection queries shipped inside them:

- `tree-sitter-bash 0.25.1`;
- `tree-sitter-c 0.24.2`;
- `tree-sitter-cpp 0.23.4`;
- `tree-sitter-css 0.25.0`;
- `tree-sitter-go 0.25.0`;
- `tree-sitter-html 0.23.2`;
- `tree-sitter-java 0.23.5`;
- `tree-sitter-javascript 0.25.0`;
- `tree-sitter-json 0.24.8`;
- `tree-sitter-kotlin-sg 0.4.1`;
- `tree-sitter-md 0.5.3`;
- `tree-sitter-python 0.25.0`;
- `tree-sitter-rust 0.24.2`;
- `tree-sitter-swift 0.7.3`;
- `tree-sitter-toml-ng 0.7.0`;
- `tree-sitter-typescript 0.23.2`; and
- `tree-sitter-yaml 0.7.2`.

Each grammar crate declares the MIT License. The supporting
`tree-sitter-language 0.1.7` crate also declares MIT. Package repositories,
exact upstream revisions, Cargo checksums, and the query material used by
Runyte are recorded in `docs/dependency-license-inventory.md`.

Copyright notices present in the audited crate archives include:

- Copyright (c) 2017 Max Brunsfeld (`tree-sitter-bash`);
- Copyright (c) 2014 Max Brunsfeld (`tree-sitter-c`);
- Copyright (c) 2014 Max Brunsfeld (`tree-sitter-javascript`);
- Copyright (c) 2018 Max Brunsfeld (`tree-sitter-css`);
- Copyright (c) 2014 Max Brunsfeld (`tree-sitter-go`);
- Copyright (c) 2017 Ayman Nadeem (`tree-sitter-java`);
- Copyright (c) 2019 fwcd (`tree-sitter-kotlin-sg`);
- Copyright (c) 2016 Max Brunsfeld (`tree-sitter-python`);
- Copyright (c) 2017 Maxim Sokolov (`tree-sitter-rust`);
- Copyright (c) 2021 alex-pinkus (`tree-sitter-swift`); and
- Copyright (c) 2019-2021 Ika and copyright (c) 2024 tree-sitter-grammars
  contributors (`tree-sitter-yaml`).

The `tree-sitter-html 0.23.2` archive declares MIT but omits its repository
license file. The license at its packaged revision carries copyright (c) 2014
Max Brunsfeld. Release assembly must obtain and preserve that license text
deliberately.

The `tree-sitter-java 0.23.5` archive likewise declares MIT but omits its
repository license file. The license at its packaged revision carries
copyright (c) 2017 Ayman Nadeem. Release assembly must obtain and preserve
that license text deliberately.

### Kotlin highlight query

The `queries/highlights.scm` distributed in `tree-sitter-kotlin-sg 0.4.1`
states that it is based on nvim-treesitter's Kotlin highlight query at
revision `f8ab59861eed4a1c168505e3433462ed800f2bae`:

<https://github.com/nvim-treesitter/nvim-treesitter/blob/f8ab59861eed4a1c168505e3433462ed800f2bae/queries/kotlin/highlights.scm>

The packaged header describes removal of patterns using `#lua-match?`. An
exact comparison with the cited revision also finds substantial
grammar-specific adaptations to comment, string/interpolation, regex, null,
operator, keyword-node, and related patterns. The adapted highlight query
remains under the Apache License 2.0. The complete license text is preserved
in `licenses/Apache-2.0.txt`; the query's source and revision remain in its
compiled crate constant and in `docs/source-provenance.md`.

The MIT permission notice and warranty disclaimer shipped by those projects
applies to their respective code and MIT-licensed query material. The Kotlin
highlight query described above is instead covered by Apache-2.0. Release
assembly must preserve the applicable upstream copyright and license text;
this notice is a source inventory and does not replace those license files.

## Project assets

### Runyte logo

Files:

- `logo/runyte_logo.svg`, SHA-256
  `57b62be0f987bf4ac4b22679e12ae1dce9f6944831021a13b1f606d627f368c3`; and
- `logo/runyte.png`, SHA-256
  `ab90034c188786a35d676f93bb2d515400394b52dd5051c52312e07e7a356876`.

The logo is an original design drawn by the project owner in Inkscape. It is
not AI-generated, and it incorporates no third-party artwork. The SVG is the
source; the PNG is a 300x300 export of it. Neither file carries a C2PA
manifest or any other generative-provenance claim; the PNG's only text chunk
records Inkscape as the exporting software.

The assets are the project owner's own work and are covered by the root
`LICENSE` along with the rest of the repository. They are binary and SVG
artwork and have not been given textual SPDX headers.

This entry supersedes the AI-generated `logo/R.png` that these files replaced.
