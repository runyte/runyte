---
title: "SQL, Lua, C#, Zig, CMake, Protobuf, Make, and INI open without syntax capabilities"
status: resolved
reported: 2026-08-29
resolved: 2026-08-29
commit: 1f4187e
---

## Resolution

Commit `1f4187e` (`Add eight statically linked language grammars`) resolves
the issue. `BUILTIN_LANGUAGES` had no definitions for these file types, so
path detection could not choose a language and none of the lazy parser-owned
highlight, indentation, fold, outline, or text-object capabilities could be
constructed.

The commit exact-pins the eight audited grammar crates and registers their
extensions, exact filenames, Lua shebang, and explicit line-comment markers.
Each language now has highlighting, newline indentation, and folds. SQL, Lua,
C#, and Zig have document outlines; Lua, C#, and Zig also have function,
class-like, and parameter text objects. Protobuf's packaged highlight query is
carried locally because its Rust crate exports no query constant. Zig and CMake
highlight queries are carried locally to translate unsupported
`#lua-match?` predicates, and Zig's unsupported priority property is omitted.
Upstream indentation files that use captures beyond Runyte's bounded
`@indent.begin`, `@indent.always`, and syntax-required `@indent.tab` dialect
are reduced to the same node coverage within Runyte's owned semantics. Every
copied or adapted query records its exact upstream version and license.

INI deliberately uses `;` for comments inserted by `toggle-comments`; both
`;` and `#` continue to parse as comments. CMake, Protobuf, Make, and INI
retain the issue's conservative two-capability structural shape and do not
invent outlines or text objects. A stripped LTO release build grew from
34,244,352 bytes at baseline `06e859d` to 43,168,320 bytes, an increase of
8,923,968 bytes (8.51 MiB, 26.1%).

An independent review then found three integration gaps. SQL, Zig, CMake, and
INI comment captures ended with unmapped spelling helpers, while CMake command
and shebang patterns could likewise let helper captures replace their semantic
scope. Later mapped captures now preserve those highlights. Zig and CMake no
longer register upstream injections for the unbundled `comment` grammar, so
they do not create unusable injection-free parser variants or report false
large-file degradation. Finally, Make rules use the owned `@indent.tab`
capture, and smart newline realizes that result as the literal tab required by
ordinary recipe lines rather than as the configured number of spaces.

Coverage lives in `tests/syntax.rs`:
`filename_extension_and_bounded_shebang_inference_have_stable_precedence`,
`extensions_map_to_languages_case_insensitively`,
`additional_language_grammars_highlight_representative_documents`,
`additional_language_highlights_keep_semantic_captures_after_helper_captures`,
`indentation_and_folds_cover_the_truthful_language_matrix`,
`outline_queries_cover_the_supported_language_inventory`,
`lua_c_sharp_and_zig_structural_objects_match_real_declarations`, and
`every_bundled_grammar_loads_without_error`. Query compilation and explicit
comment markers are covered by
`src/syntax/mod.rs::every_canonical_plain_and_owned_capability_query_compiles`
and
`src/syntax/mod.rs::every_built_in_language_declares_its_line_comment`. Make's
literal recipe indentation is covered by
`src/app/tests/editing_and_buffers.rs::smart_newline_uses_the_required_make_recipe_tab`.

Known limitation: CMake functions/macros and Make targets do not yet appear in
the document outline; adding those capabilities remains a separate product
choice rather than being inferred from this grammar-registration change.
Makefiles that override `.RECIPEPREFIX` still receive the standard tab prefix;
Runyte does not currently evaluate Make directives when choosing indentation.

## Report

# Statically linked grammars for SQL, Lua, C#, Zig, CMake, Protobuf, Make, and INI

Runyte compiles tree-sitter grammars into the binary from `tree-sitter-*`
crates rather than loading them from shared libraries at runtime. The languages
currently registered in `src/syntax/grammars.rs` are Bash, C, C++, CSS, Go,
HTML, Java, JavaScript, JSON, Kotlin, Markdown, Python, Rust, Swift,
TypeScript, TSX, TOML, and YAML.

SQL, Lua, C#, Zig, CMake, Protobuf, Make, and INI are not registered. Documents
in those languages open as plain text: no highlighting, no syntax-aware
indentation, no folds, no outline, and no structural text objects. Language
detection has no entry to match, so `Registry::language_for_path` and
`language_for_document` return `None` for them.

## Expected behavior

Each of the eight languages is registered as a `LanguageDefinition` in
`BUILTIN_LANGUAGES` and detected from its file extensions, and from exact file
names where the language has them. Detected documents receive highlighting,
indentation, and folds. Languages with a meaningful declaration structure also
receive an outline and function, class, and parameter text objects.

Grammar handles and raw query sources remain private to `src/syntax`, as the
module documentation requires. No grammar is downloaded or read from a runtime
path.

## Grammar crates

The eight crates below were resolved against the pinned `tree-sitter-language`
`0.1.7` and compiled together. Every one exposes `pub const LANGUAGE:
LanguageFn`, so each fits the existing `grammar:` field without an adapter.
Every crate declares `tree-sitter-language 0.1`.

| Language | Crate | Version | License |
| --- | --- | --- | --- |
| SQL | `tree-sitter-sequel` | 0.3.11 | MIT |
| Lua | `tree-sitter-lua` | 0.5.0 | MIT |
| C# | `tree-sitter-c-sharp` | 0.23.5 | MIT |
| Zig | `tree-sitter-zig` | 1.1.2 | MIT |
| CMake | `tree-sitter-cmake` | 0.7.4 | MIT |
| Protobuf | `tree-sitter-proto` | 0.5.0 | MIT |
| Make | `tree-sitter-make` | 1.1.1 | MIT |
| INI | `tree-sitter-ini` | 1.4.0 | Apache-2.0 |

`tree-sitter-sql` is a separate and unrelated crate last published as 0.0.2 in
2021. `tree-sitter-sequel` is the maintained SQL grammar and is the crate named
above.

### Exported query constants

The set of query constants a crate exposes is narrower than the set of `.scm`
files it ships. Several crates gate the constants behind `cfg` flags their
build script sets from the files present in the package, and several others
carry the declarations commented out. The constants that exist are:

| Crate | `HIGHLIGHTS_QUERY` | `INJECTIONS_QUERY` | `LOCALS_QUERY` |
| --- | --- | --- | --- |
| `tree-sitter-sequel` | yes | no | no |
| `tree-sitter-lua` | yes | yes | yes |
| `tree-sitter-c-sharp` | yes | no | no |
| `tree-sitter-zig` | yes | yes | no |
| `tree-sitter-cmake` | yes | yes | no |
| `tree-sitter-proto` | no | no | no |
| `tree-sitter-make` | yes | no | no |
| `tree-sitter-ini` | yes | no | no |

`tree-sitter-proto` exports `LANGUAGE` and `NODE_TYPES` only. Its
`queries/highlights.scm` is packaged in the crate but has no constant, so the
Protobuf highlight query has to be carried as a Runyte-owned file under
`src/syntax/queries/proto/` and attributed to the upstream crate and version,
in the way `SWIFT_COMMENT_HIGHLIGHTS` already records a Runyte-authored query
against a specific upstream release.

`tree-sitter-c-sharp` packages only `highlights.scm` and `tags.scm`. Its
`INJECTIONS_QUERY` and `LOCALS_QUERY` declarations are `cfg`-gated on files
absent from the published package, so referring to either is a compile error
rather than an empty query.

## Query work

Existing languages fall into two shapes. Data and markup formats — CSS, HTML,
JSON, TOML, TSX, YAML — carry two Runyte-authored queries, `folds.scm` and
`indentation.scm`. Full programming languages — Go, Java, JavaScript, Kotlin,
Python, Rust, TypeScript — carry six, adding `functions.scm`, `classes.scm`,
`parameters.scm`, and `outline.scm`. C, C++, Swift, and Bash sit between the
two.

Runyte's indentation queries use the `@indent.always`, `@indent.begin`,
`@indent.branch`, `@indent.end`, and `@indent.ignore` captures. Four of the
crates ship indent or fold queries written against the same captures, so those
files can be adopted as Runyte-owned queries with attribution rather than
written from the grammar's node types:

| Crate | Reusable files |
| --- | --- |
| `tree-sitter-sequel` | `indents.scm` |
| `tree-sitter-zig` | `indents.scm`, `folds.scm` |
| `tree-sitter-cmake` | `indents.scm`, `folds.scm` |
| `tree-sitter-proto` | `indents.scm`, `folds.scm` |
| `tree-sitter-ini` | `folds.scm` |

`tree-sitter-lua`, `tree-sitter-c-sharp`, and `tree-sitter-make` ship no indent
or fold queries; theirs are written from the grammar node types.

The suggested target shape per language is:

- **Six queries** for C#, Lua, and Zig: these have functions, types, and
  parameter lists that outline and text objects can address.
- **Three queries** for SQL: `folds.scm`, `indentation.scm`, and `outline.scm`,
  the outline addressing statements and named definitions.
- **Two queries** for CMake, Protobuf, Make, and INI: `folds.scm` and
  `indentation.scm`. CMake and Make may additionally warrant an outline over
  function, macro, and target definitions.

## Language definition values

| Language | `name` | `extensions` | `filenames` | `line_comment` |
| --- | --- | --- | --- | --- |
| SQL | `sql` | `sql` | — | `--` |
| Lua | `lua` | `lua` | — | `--` |
| C# | `c-sharp` | `cs`, `csx` | — | `//` |
| Zig | `zig` | `zig`, `zon` | — | `//` |
| CMake | `cmake` | `cmake` | `CMakeLists.txt` | `#` |
| Protobuf | `proto` | `proto` | — | `//` |
| Make | `make` | `mk`, `mak` | `Makefile`, `makefile`, `GNUmakefile` | `#` |
| INI | `ini` | `ini` | — | see below |

Extensions are matched case-insensitively and exact file names win over them,
so extension entries stay lowercase while file names are written as they appear
on disk.

The INI grammar accepts both `;` and `#` as comment introducers, matching
`[;#]` at the start of a comment. `line_comment` holds a single marker and is
what `toggle-comments` inserts, so one has to be chosen; both parse correctly
in either direction, and the choice only decides which marker new comments
receive.

Lua may also warrant a `lua` shebang entry, consistent with the existing use of
`shebangs` for interpreted languages.

## Constraints

Grammar crates are third-party material and require entries in
`THIRD_PARTY_NOTICES.md`, together with any query file adopted from a crate,
recorded against the crate name and exact version the query was taken from.
Seven of the eight crates are MIT and one, `tree-sitter-ini`, is Apache-2.0;
both are compatible with MPL-2.0 for this use.

`Cargo.toml` pins several existing grammar crates to exact versions with `=`
and leaves others on caret requirements. The new entries follow whichever
convention applies, and an exact pin is appropriate wherever a Runyte-authored
query is written against a specific upstream node set, since a grammar update
can rename or restructure nodes and silently empty a query.

Parser tables are the dominant contribution to binary size, and two of these
grammars are large relative to what is already linked. Measured as generated C
source, with the already-shipped Rust grammar at 6.5 MB as the reference point:
C# is 29.7 MB, SQL 17.4 MB, Zig 5.8 MB, Make 1.0 MB, CMake 0.54 MB, Lua
0.36 MB, Protobuf 0.28 MB, and INI 0.03 MB. Generated source size is not
linked size — the release profile applies `lto = true` and `strip = true` — so
the actual growth is measured from a release build, and the C# and SQL entries
are the two worth measuring before and after.

The `include` list in `Cargo.toml` governs what the published crate carries.
New query files under `src/syntax/queries/` are covered by the existing
`/src/**` entry, but the list is confirmed rather than assumed.

## Existing tests that change

Query compilation is lazy. `Registry::new` registers identities only, and
`LazyLanguageConfig::get` compiles a language's queries through a `OnceLock` on
first use, so a query that fails to compile surfaces when a document in that
language is first opened rather than at startup. Three existing tests in
`src/syntax/mod.rs` therefore carry the inventory and have to be updated as part
of the change rather than after it:

- `every_built_in_language_declares_its_line_comment` compares every registered
  language against a hardcoded list of expected markers. It fails until each new
  language's `line_comment` is added, which is its intended behavior — the
  comment marker has to be decided rather than inherited.
- `every_canonical_plain_and_owned_capability_query_compiles` compiles every
  canonical and plain configuration, and is where a malformed query is actually
  caught. It asserts `plain_count == 3` with the message "Rust, HTML, and
  Markdown have plain variants". A plain variant is generated for any language
  whose injections query is non-empty, so wiring the available
  `INJECTIONS_QUERY` for Lua, Zig, and CMake raises that count and both the
  assertion and its message need updating. It also asserts
  `registry.configs.len() == BUILTIN_LANGUAGES.len() + plain_count + 1`, which
  follows automatically once `plain_count` is correct.
- `registry_construction_registers_every_identity_without_compiling_queries`
  iterates the inventory and needs no edit, but confirms the new languages are
  addressable by name.

`is_builtin_language_name` draws on the same inventory, so each added name also
becomes valid as a language-server configuration key. Language-server
configuration for these languages is out of scope here; only the name becoming
valid is in scope.

## Validation

Each language needs coverage at the behavior boundary in `tests/syntax.rs`:
detection from every declared extension and file name, a highlight query that
produces scopes on a representative document, indentation applied on a newline,
and folds where they are defined. Languages given an outline or text objects
need those exercised too, since a query that compiles but matches nothing is
otherwise indistinguishable from one that works.

Documentation updates: the language count and list in `README.md`, and the
explicit language list in `docs/user-guide.md`.

Then:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
