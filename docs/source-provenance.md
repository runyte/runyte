# Source provenance

This register records the source and review status of external or generated
material in Runyte. It is intended to survive refactoring and to be updated on
future provenance-audit runs.

This targeted record began with findings confirmed on 2026-07-27 and was
extended for the V7 syntax dependency baseline on 2026-08-08. It is not a
claim that a complete repository-wide provenance audit is finished.

## Register

| Runyte path | External project or producer | Upstream file or artifact | Upstream revision | Class | Confidence | Evidence | Applicable license or status | Notice added | Last reviewed commit | Review date | Unresolved questions |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `src/config.rs` (`github-light`) | projekt0n/github-nvim-theme and its generated GitHub Primer light primitives | `lua/github-theme/palette/github_light.lua`, `primitives/light.lua`, and the generated syntax/editor specs | `c106c9472154d6b2c74b74565616b877ae8ed31d` | B — adapted or translated code | High | The palette, generated spec, color blending, editor groups, and syntax groups were read at the pinned revision. Runyte translates them into its smaller semantic role set and adds mode cursors and jump-label colors. | MIT: copyright (c) 2021 projekt0n̅; generated Primer primitives copyright (c) 2018 GitHub Inc. | `THIRD_PARTY_NOTICES.md` entry and both notices in `licenses/GitHub-Nvim-Theme-MIT.txt` | Working tree based on `c61b03b` | 2026-08-19 | Recompare the palette and generated Primer primitives when updating the pin. |
| `logo/runyte_logo.svg` and `logo/runyte.png` | None — original work by the project owner | No upstream source work; the SVG is the design source and the PNG is its 300x300 export | Not applicable | F — no meaningful similarity to an identifiable external source found | High | The owner states the mark is their own Inkscape design. The SVG carries Inkscape editing metadata and an `inkscape:export-filename` of `runyte.png`; the PNG's only text chunk is `Software=www.inkscape.org`. Neither file carries a C2PA manifest or any other generative-provenance claim, and neither declares a third-party author. The committed hashes are `57b62be0f987bf4ac4b22679e12ae1dce9f6944831021a13b1f606d627f368c3` (SVG) and `ab90034c188786a35d676f93bb2d515400394b52dd5051c52312e07e7a356876` (PNG). | Project owner's own work; covered by the root `LICENSE` with the rest of the repository. | Asset entry in `THIRD_PARTY_NOTICES.md`; files left unchanged | `49435c7` | 2026-08-10 | None identified. These files replaced the AI-generated `logo/R.png` recorded in this register before 2026-08-10; the earlier distribution-rights question no longer applies. |
| `src/syntax/mod.rs` | Helix `tree-house` project | `tree-house 0.4.0` (`highlighter`) and `tree-house-bindings 0.3.2` (`bindings`) crates | `75d213c4033ccf3ea98a5539d7d7c4005d8fb2ea` and `358188cc6693cc7e78ed637b66822401df320803` | E — ordinary dependency use | High | Cargo manifests, lock checksums, crate VCS metadata, and shipped license files; exact details are recorded in `docs/dependency-license-inventory.md`. | MPL-2.0; vendored Tree-sitter C source in the bindings crate carries MIT terms. Runyte keeps all dependency types inside `src/syntax/`. | Exact manifest pins, dependency inventory, and `THIRD_PARTY_NOTICES.md` entry | `fc68a437cc2a1c5d63ad1f63e9401730848c7a29` | 2026-08-08 | Re-audit API, license files, and isolation before either exact pin changes. |
| `src/syntax/grammars.rs` and `src/syntax/queries/` | Tree-sitter grammar repositories | Statically linked grammar crates and the highlight/injection/locals query constants enumerated in `docs/dependency-license-inventory.md`; local structural, indentation, and fold queries target the exact registered grammar versions | Per-package revisions recorded in `docs/dependency-license-inventory.md` | E — ordinary dependency use for grammar/query constants; F — no external query text used for Runyte-owned capabilities | High | Cargo manifests and checksums, crate VCS metadata, query constant definitions, the Runyte grammar table, and exact node-type compilation against every registered grammar. Upstream highlight/injection/locals text remains in dependency crates. Local indentation/fold queries are purpose-written for Runyte's limited capture dialect; C++ and TypeScript/TSX composition is explicit, Markdown root indentation is explicitly unsupported, and TOML coverage is limited to truthful containers. | Each audited grammar crate declares MIT. Local structural, indentation, and fold queries are Runyte-authored MPL-2.0 files carrying target-version comments. No upstream indentation/fold query was copied. The Kotlin highlight query has the separate Apache-2.0 provenance recorded below. | Per-package dependency inventory and grouped `THIRD_PARTY_NOTICES.md` entry | `a4ad251` | 2026-08-09 | Seven audited crate archives omit a top-level license file, including `tree-sitter-typescript 0.23.2`, `tree-sitter-html 0.23.2`, and `tree-sitter-java 0.23.5`; Kotlin SG ships its MIT license. Re-audit all local capability queries when a grammar version changes. |
| `tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY` used by `src/syntax/grammars.rs` | nvim-treesitter via `ast-grep/tree-sitter-kotlin` | `queries/kotlin/highlights.scm`, substantially adapted by the Kotlin grammar from the cited upstream query | nvim-treesitter `f8ab59861eed4a1c168505e3433462ed800f2bae`; packaged by Kotlin SG at `1a6f9b1ee1125a7357493eeb95da48d16ac302b4` | E — ordinary dependency use with separately licensed query material | High | The packaged header identifies its source revision and Apache license and mentions removed `#lua-match?` patterns. Exact comparison also finds adaptations to comment, string/interpolation, regex, null, operator, keyword-node, and related patterns; the crate exports the resulting file as `HIGHLIGHTS_QUERY`. | Apache License 2.0 for the adapted highlight query; the Kotlin grammar and generated parser separately declare MIT. | Exact attribution in `THIRD_PARTY_NOTICES.md` and full terms in `licenses/Apache-2.0.txt` | `4a83370` | 2026-08-09 | Re-check the complete adaptation diff, upstream revision, and any upstream NOTICE file on upgrade. |

## Classification key

- **A — Direct copy:** substantially unchanged external material.
- **B — Adapted or translated code:** expressive structure retained through a
  translation, reorganization, or rewrite.
- **C — Close reconstruction with uncertain provenance:** strong similarity
  without enough history to determine its source.
- **D — Behavioral compatibility or inspiration:** shared behavior or public
  interface without copied implementation.
- **E — Ordinary dependency use:** an external package used through its public
  API.
- **F — No meaningful similarity:** no attribution-triggering similarity to an
  identifiable external work was found.

## Audit baseline

- Initial Runyte commit reviewed:
  `32fa579a210396829880d67fcbdfb139c3a757d1`
- Initial review date: 2026-07-27.
- Initial working tree: the MPL-2.0 transition and this provenance record were
  uncommitted at the time of review.
- V7 syntax dependency baseline commit before documentation:
  `fc68a437cc2a1c5d63ad1f63e9401730848c7a29`.
- V7 syntax dependency review date: 2026-08-08.
- Logo replacement commit: the owner's original artwork replaced the
  AI-generated `logo/R.png` on 2026-08-10.
- Files requiring review on the next audit:
  - `logo/runyte_logo.svg` and `logo/runyte.png`
  - `THIRD_PARTY_NOTICES.md`

On the next audit, compare the logo hashes with the register. Also compare the
exact syntax dependency pins, lock checksums, packaged revisions, query paths,
and shipped license files with `docs/dependency-license-inventory.md`.
