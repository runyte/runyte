# Dependency and license inventory

This is the reviewed dependency baseline for the committed V7 editor cycle. It
records the syntax stack in enough detail to review an upgrade without relying
on whatever a registry serves later. `Cargo.lock` remains the machine-readable
inventory for the complete dependency graph; this document records license and
source facts that the lock file does not contain.

This is an engineering record, not legal advice. Release packaging must still
carry the license and notice material required by each distributed dependency.

## Baseline

- Review date: 2026-08-08.
- Runyte commit before this inventory was added:
  `fc68a437cc2a1c5d63ad1f63e9401730848c7a29`.
- `Cargo.lock` format: version 4, with 191 package records.
- Direct normal dependencies resolved by the lock: 27.
- Test inventory: 558 tests, comprising 552 normally executed tests and six
  ignored environmental/provider tests.
- Audit toolchain: `rustc 1.90.0 (1159e78c4 2025-09-14)` and
  `cargo 1.90.0 (840b83a10 2025-07-30)` on
  `x86_64-unknown-linux-gnu`.
- Runyte does not currently declare `package.rust-version` or ship a
  `rust-toolchain.toml`. Edition 2024 therefore supplies the effective compiler
  floor, but the supported project MSRV is not yet an explicit contract.

The two work-in-progress syntax implementation crates in the Phase 0 baseline
are deliberately exact requirements in `Cargo.toml`: `tree-house` at `=0.4.0`
and `tree-house-bindings` at `=0.3.2`. The other baseline dependencies retain
normal compatible Cargo requirements and are made reproducible by the committed
lock file.

The first Phase 2 grammar-expansion batch also deliberately exact-pins
`tree-sitter-javascript` at `=0.25.0` and `tree-sitter-typescript` at `=0.23.2`.
The second batch exact-pins `tree-sitter-html` at `=0.23.2` and
`tree-sitter-css` at `=0.25.0`.
The third batch exact-pins `tree-sitter-go` at `=0.25.0` and
`tree-sitter-bash` at `=0.25.1`.
The fourth batch exact-pins `tree-sitter-java` at `=0.23.5`.
The fifth batch exact-pins `tree-sitter-kotlin-sg` at `=0.4.1`; the similarly
named `tree-sitter-kotlin-ng` package is deliberately not part of the graph.
Against the audited `tree-sitter-kotlin-ng 1.1.0` candidate, SG models Kotlin 2
guarded `when` conditions and multi-dollar interpolation, and ships a
compatible highlight query plus its MIT license. NG lacked those current
Kotlin 2 nodes, query files, and a packaged license file.
The sixth batch exact-pins `tree-sitter-sequel` at `=0.3.11`,
`tree-sitter-lua` at `=0.5.0`, `tree-sitter-c-sharp` at `=0.23.5`,
`tree-sitter-zig` at `=1.1.2`, `tree-sitter-cmake` at `=0.7.4`,
`tree-sitter-proto` at `=0.5.0`, `tree-sitter-make` at `=1.1.1`, and
`tree-sitter-ini` at `=1.4.0`. These pins protect local capability queries and
the adapted query files carried for Zig, CMake, and Protobuf from silent node
or predicate changes.
Future grammar upgrades must repeat the query, checksum, revision, archive, and
license review below rather than arriving through a compatible-version update.

On 2026-08-29, stripped LTO release builds on `x86_64-unknown-linux-gnu`
measured 34,244,352 bytes at baseline commit `06e859d` and 43,168,320 bytes with
the sixth grammar batch, an increase of 8,923,968 bytes (8.51 MiB, 26.1%).

## Audited syntax stack

The checksum is the crates.io checksum in `Cargo.lock`. The revision comes from
the crate archive's `.cargo_vcs_info.json`. A path after a revision identifies
the package's directory in its upstream repository.

| Package | License declared by crate | Repository and packaged revision | Cargo checksum | Material used by Runyte |
| --- | --- | --- | --- | --- |
| `tree-house 0.4.0` | MPL-2.0 | `helix-editor/tree-house` at `75d213c4033ccf3ea98a5539d7d7c4005d8fb2ea`, path `highlighter` | `333476442882205ab249a8ced263ae53db3d0797c727cc3424836a6fb221d7b1` | Highlighter, injected syntax layers, tree cursors, and query iteration behind `src/syntax/` |
| `tree-house-bindings 0.3.2` | MPL-2.0; its vendored Tree-sitter C source carries MIT terms | `helix-editor/tree-house` at `358188cc6693cc7e78ed637b66822401df320803`, path `bindings` | `6f5d0eed0db98578618e598b8f6f413f7c172bade1f12f8242d71c38dd5deb0f` | Tree-sitter bindings, grammar conversion, points, and edits |
| `tree-sitter-language 0.1.7` | MIT | `tree-sitter/tree-sitter` at `470813116b99578956e67abb7138e993833af67a`, path `crates/language` | `009994f150cc0cd50ff54917d5bc8bffe8cad10ca10d81c34da2ec421ae61782` | Stable grammar-language function bridge |
| `tree-sitter-javascript 0.25.0` | MIT | `tree-sitter/tree-sitter-javascript` at `44c892e0be055ac465d5eeddae6d3e194424e7de` | `68204f2abc0627a90bdf06e605f5c470aa26fdcb2081ea553a04bdad756693f5` | Grammar plus `queries/highlights.scm`, `queries/highlights-jsx.scm`, and `queries/locals.scm`; upstream injections are deliberately disabled |
| `tree-sitter-typescript 0.23.2` | MIT | `tree-sitter/tree-sitter-typescript` at `f975a621f4e7f532fe322e13c4f79495e0a7b2e7` | `6c5f76ed8d947a75cc446d5fccd8b602ebf0cde64ccf2ffa434d873d7a575eff` | TypeScript and TSX grammars plus `queries/highlights.scm` and `queries/locals.scm`, composed after the JavaScript query base; TSX also composes the JavaScript JSX query before the TypeScript additions |
| `tree-sitter-html 0.23.2` | MIT | `tree-sitter/tree-sitter-html` at `5a5ca8551a179998360b4a4ca2c0f366a35acc03` | `261b708e5d92061ede329babaaa427b819329a9d427a1d710abb0f67bbef63ee` | Grammar plus `queries/highlights.scm` and the complete upstream `queries/injections.scm`; its two static targets are the registered `javascript` and `css` languages |
| `tree-sitter-css 0.25.0` | MIT | `tree-sitter/tree-sitter-css` at `dda5cfc5722c429eaba1c910ca32c2c0c5bb1a3f` | `a5cbc5e18f29a2c6d6435891f42569525cf95435a3e01c2f1947abcde178686f` | Grammar plus `queries/highlights.scm`; no injection or locals query is published by the Rust crate |
| `tree-sitter-go 0.25.0` | MIT | `tree-sitter/tree-sitter-go` at `1547678a9da59885853f5f5cc8a99cc203fa2e2c` | `c8560a4d2f835cc0d4d2c2e03cbd0dde2f6114b43bc491164238d333e28b16ea` | Grammar plus the upstream `queries/highlights.scm`; Runyte-authored structural queries cover functions, methods, function literals, struct/interface class-like objects, grouped parameters, and declarations in the outline |
| `tree-sitter-bash 0.25.1` | MIT | `tree-sitter/tree-sitter-bash` at `a06c2e4415e9bc0346c6b86d401879ffb44058f7` | `9e5ec769279cc91b561d3df0d8a5deb26b0ad40d183127f409494d6d8fc53062` | Grammar plus the upstream `queries/highlights.scm`; Runyte-authored structural queries truthfully expose functions and leave class/parameter capabilities unsupported |
| `tree-sitter-java 0.23.5` | MIT | `tree-sitter/tree-sitter-java` at `94703d5a6bed02b98e438d7cad1136c01a60ba2c` | `0aa6cbcdc8c679b214e616fd3300da67da0e492e066df01bcf5a5921a71e90d6` | Grammar ABI 14 plus upstream `queries/highlights.scm`; Runyte-authored structural queries cover methods, constructors, lambdas, classes, interfaces, enums, records, annotation types, formal/receiver/vararg/inferred parameters, modules, and the document outline |
| `tree-sitter-kotlin-sg 0.4.1` | MIT for the grammar; its highlight query records Apache-2.0 provenance | `ast-grep/tree-sitter-kotlin` at `1a6f9b1ee1125a7357493eeb95da48d16ac302b4` | `c06ec43ae3c12165d4ac08afe4e1f5fc6757ffe274fa7bd5af9007ef11ba4319` | Grammar ABI 14, external scanner, and the packaged `queries/highlights.scm`, substantially adapted from nvim-treesitter revision `f8ab59861eed4a1c168505e3433462ed800f2bae`: `#lua-match?` patterns are removed and comment, string/interpolation, regex, null, operator, keyword-node, and related patterns differ; Runyte-authored structural queries cover functions, constructors, lambdas, classes/interfaces/enums/objects, primary/function/lambda parameters, properties, aliases, constants, and the outline |
| `tree-sitter-sequel 0.3.11` | MIT | `derekstride/tree-sitter-sql` at `7b51ecda191d36b92f5a90a8d1bc3faef1c7b8b8` | `9d198ad3c319c02e43c21efa1ec796b837afcb96ffaef1a40c1978fbdcec7d17` | Grammar and upstream `queries/highlights.scm`, followed by a Runyte-authored comment precedence repair; the local indentation query adapts the packaged node coverage to Runyte's capture dialect, while Runyte authors the folds and outline |
| `tree-sitter-lua 0.5.0` | MIT | `tree-sitter-grammars/tree-sitter-lua` at `10fe0054734eec83049514ea2e718b2a56acd0c9` | `8daaf5f4235188a58603c39760d5fa5d4b920d36a299c934adddae757f32a10c` | Grammar plus upstream highlight, C injection, and locals queries; Runyte-authored queries provide function, class-like table, and parameter objects, outline, indentation, and folds |
| `tree-sitter-c-sharp 0.23.5` | MIT | `tree-sitter/tree-sitter-c-sharp` at `cac6d5fb595f5811a076336682d5d595ac1c9e85` | `c1aac67f1ad71de1d6d39708d34811081c26dfa495658de6c14c34200849357c` | Grammar plus upstream `queries/highlights.scm`; the published archive has no injection or locals query, and Runyte authors all structural, indentation, and fold queries |
| `tree-sitter-zig 1.1.2` | MIT | `tree-sitter-grammars/tree-sitter-zig` at `b670c8df85a1568f498aa5c8cae42f51a90473c0` | `ab11fc124851b0db4dd5e55983bbd9631192e93238389dcd44521715e5d53e28` | Grammar; the packaged highlight query is carried locally with `#lua-match?` translated to `#match?`, its unsupported priority property removed, and its unmapped spelling helper omitted, while the packaged indentation/fold node coverage is adapted or adopted locally; the injection query is disabled because its `comment` grammar is not bundled; Runyte authors structural objects and outline |
| `tree-sitter-cmake 0.7.4` | MIT | `uyha/tree-sitter-cmake` at `ca627bb5828616b6246aafdc3c3222789e728e37` | `164e0c4f4236ec5ceff14824a5528615cf462e100467e49826442ff57d327061` | Grammar; the packaged highlight query is carried locally with Lua-pattern predicates translated to supported regular expressions and mapped captures repeated after editor helpers, while packaged indentation/fold node coverage is adapted or adopted locally; the injection query is disabled because its `comment` grammar is not bundled |
| `tree-sitter-proto 0.5.0` | MIT | `coder3101/tree-sitter-proto` at `5a256fe3b6be3bd2ea4d03e1213d847c7093c2e1` | `daf199052df77bd434c30a71c148773c21a3252e3545a8528151fc7bb931a723` | Grammar plus the packaged highlight query carried locally because the Rust crate exports no query constants; packaged indentation/fold node coverage is adapted or adopted locally |
| `tree-sitter-make 1.1.1` | MIT | `tree-sitter-grammars/tree-sitter-make` at `5e9e8f8ff3387b0edcaa90f46ddf3629f4cfeb1d` | `c5998dc7cbcbdab19fae8aefef982bf2d6544513d8d2e69cc44aec4c63810104` | Grammar plus upstream `queries/highlights.scm`; Runyte authors literal-tab-aware recipe indentation and folds |
| `tree-sitter-ini 1.4.0` | Apache-2.0 | `justinmk/tree-sitter-ini` at `d1f6ae18e86de3c21bb6ab634ff9ab549ceb1249` | `387f79682cd53b7c0a5777c96e601a02b9965a787984ef86dbb8952bdab2d62f` | Grammar plus upstream `queries/highlights.scm`, followed by a Runyte-authored comment precedence repair; Runyte authors indentation and carries the packaged section fold query locally |
| `tree-sitter-rust 0.24.2` | MIT | `tree-sitter/tree-sitter-rust` at `e2bee853694a1d3e0f6ef308fe3674542fec95d7` | `439e577dbe07423ec2582ac62c7531120dbfccfa6e5f92406f93dd271a120e45` | Grammar plus `queries/highlights.scm` and `queries/injections.scm` |
| `tree-sitter-python 0.25.0` | MIT | `tree-sitter/tree-sitter-python` at `293fdc02038ee2bf0e2e206711b69c90ac0d413f` | `6bf85fd39652e740bf60f46f4cda9492c3a9ad75880575bf14960f775cb74a1c` | Grammar plus `queries/highlights.scm` |
| `tree-sitter-swift 0.7.3` | MIT | `alex-pinkus/tree-sitter-swift` at `b8b22bffbb3441780e6471665bacfb263741c86a` | `fe36052155b9dd69ca82b3b8f1b4ccfb2d867125ac1a4db1dd7331829242668c` | Grammar and upstream `queries/highlights.scm`, extended by one Runyte-authored comment query; upstream injections are deliberately disabled |
| `tree-sitter-c 0.24.2` | MIT | `tree-sitter/tree-sitter-c` at `b780e47fc780ddc8da13afa35a3f4ed5c157823d` | `a9b2eb57a55fed6b00812912e730b7a275cf4fe98bfd6a5d76263d4438371728` | Grammar plus `queries/highlights.scm`; highlights are also inherited by C++ |
| `tree-sitter-cpp 0.23.4` | MIT | `tree-sitter/tree-sitter-cpp` at `f41e1a044c8a84ea9fa8577fdd2eab92ec96de02` | `df2196ea9d47b4ab4a31b9297eaa5a5d19a0b121dceb9f118f6790ad0ab94743` | Grammar plus `queries/highlights.scm` |
| `tree-sitter-json 0.24.8` | MIT | `tree-sitter/tree-sitter-json` at `ee35a6ebefcef0c5c416c0d1ccec7370cfca5a24` | `4d727acca406c0020cffc6cf35516764f36c8e3dc4408e5ebe2cb35a947ec471` | Grammar plus `queries/highlights.scm` |
| `tree-sitter-toml-ng 0.7.0` | MIT | `tree-sitter-grammars/tree-sitter-toml` at `64b56832c2cffe41758f28e05c756a3a98d16f41` | `e9adc2c898ae49730e857d75be403da3f92bb81d8e37a2f918a08dd10de5ebb1` | Grammar plus `queries/highlights.scm` |
| `tree-sitter-yaml 0.7.2` | MIT | `tree-sitter-grammars/tree-sitter-yaml` at `7708026449bed86239b1cd5bce6e3c34dbca6415` | `53c223db85f05e34794f065454843b0668ebc15d240ada63e2b5939f43ce7c97` | Grammar plus `queries/highlights.scm` |
| `tree-sitter-md 0.5.3` | MIT | `tree-sitter-grammars/tree-sitter-markdown` at `f969cd3ae3f9fbd4e43205431d0ae286014c05b5` | `2efd398be546456c814598ee56c0f51769a77241511b4a58077815d120afa882` | Block grammar plus `tree-sitter-markdown/queries/highlights.scm` and `queries/injections.scm` |

Most upstream highlight, injection, and locals query strings named above are
compiled into their grammar crates and used through crate constants. The
exceptions are the packaged Protobuf highlight query and the Zig and CMake
highlight queries adapted to Runyte's supported predicate set. Selected
upstream indentation and fold queries are also carried locally, either adopted
unchanged or reduced to Runyte's bounded `@indent.begin` and
`@indent.always` dialect. Every copied or adapted file names its exact source
release and retains the applicable SPDX identifier. Other structural,
indentation, fold, and outline files under `src/syntax/queries/` are
Runyte-authored MPL-2.0 material and name their target grammar and version
inline. The owned query compiler rejects unsupported captures and predicates
rather than silently accepting semantics Runyte does not implement. C++
composes the C indentation/fold base, while TypeScript and TSX compose the
JavaScript base (and TSX also composes TypeScript additions). Markdown
intentionally has no root indentation query; all 26 languages have
conservative fold queries.

The Kotlin highlight query has separate Apache-2.0 provenance: its packaged
header says it is based on nvim-treesitter's query at revision
`f8ab59861eed4a1c168505e3433462ed800f2bae`. An exact comparison with that
revision shows broader grammar-specific adaptations than the packaged header's
stated removal of `#lua-match?`: comment, string/interpolation, regex, null,
operator, keyword-node, and related patterns also differ. The full Apache-2.0
terms preserved in `licenses/Apache-2.0.txt` also cover `tree-sitter-ini` and
the adopted INI fold query.

The audited crate archives declare every grammar MIT except
`tree-sitter-ini`, which declares Apache-2.0. Eleven archives
(`tree-sitter-cpp`, `tree-sitter-json`, `tree-sitter-md`, and
`tree-sitter-toml-ng`, plus `tree-sitter-typescript`, `tree-sitter-html`, and
`tree-sitter-java`, plus `tree-sitter-sequel`, `tree-sitter-lua`,
`tree-sitter-zig`, and `tree-sitter-make`) do not include a top-level license
file even though their Cargo metadata declares MIT. Their repository and
packaged revision are therefore retained above and must be checked deliberately
when they are upgraded or assembled into a release license bundle. The exact
HTML revision's repository license is MIT, copyright (c) 2014 Max Brunsfeld.
The exact Java revision's repository license is MIT, copyright (c) 2017 Ayman
Nadeem.
`tree-sitter-javascript 0.25.0` and
`tree-sitter-css 0.25.0` do ship their MIT license files. The exact Go and Bash
archives also ship MIT license files, carrying copyright (c) 2014 and 2017 Max
Brunsfeld respectively. The exact Kotlin SG archive ships its MIT license,
copyright (c) 2019 fwcd, in addition to the Apache provenance header retained
inside its packaged highlight query.

## Review procedure

For any syntax dependency upgrade:

1. change the manifest requirement deliberately and regenerate `Cargo.lock`;
2. compare the resolved package, checksum, repository, packaged revision,
   declared license, shipped license files, grammar ABI, and query paths with
   this baseline;
3. update this inventory, `docs/source-provenance.md`, and
   `THIRD_PARTY_NOTICES.md` as applicable;
4. compile every bundled query and run representative highlighting, injection,
   incremental-parse, and full-reparse equivalence tests; and
5. confirm no `tree-house`, Tree-sitter, or raw grammar type escaped the
   `src/syntax/` boundary.

The standard Rust gates should be run with `--locked`. Once all dependencies
are present locally, `cargo test --locked --offline` additionally checks that
the committed graph can build without registry access.
