// SPDX-License-Identifier: MPL-2.0

//! Declarative definitions for Runyte's statically linked languages.
//!
//! Grammars are compiled into the binary from the `tree-sitter-*` crates rather
//! than loaded from shared libraries at runtime. Static linking keeps startup
//! offline and avoids loading parser code from an untrusted runtime path.
//!
//! These definitions deliberately remain private to `syntax`: grammar handles
//! and raw query sources are parser implementation details. Callers discover
//! languages and use their capabilities through Runyte-owned [`super::Registry`]
//! and structural values instead.

use std::borrow::Cow;

use tree_sitter_language::LanguageFn;

use super::SyntaxObject;

/// Runyte-authored query targeting `tree-sitter-swift 0.7.3`.
///
/// The upstream Swift query captures comments as both `@comment` and `@spell`.
/// `tree-house` does not preserve the mapped capture when the same node also
/// has an unmapped capture, so repeat the semantic capture last.
const SWIFT_COMMENT_HIGHLIGHTS: &str = "[(comment) (multiline_comment)] @comment";

const JAVASCRIPT_HIGHLIGHTS: QueryFragment = upstream(
    tree_sitter_javascript::HIGHLIGHT_QUERY,
    "tree-sitter-javascript 0.25.0 highlights",
);
const JAVASCRIPT_JSX_HIGHLIGHTS: QueryFragment = upstream(
    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
    "tree-sitter-javascript 0.25.0 JSX highlights",
);
const JAVASCRIPT_LOCALS: QueryFragment = upstream(
    tree_sitter_javascript::LOCALS_QUERY,
    "tree-sitter-javascript 0.25.0 locals",
);
const TYPESCRIPT_HIGHLIGHTS: QueryFragment = upstream(
    tree_sitter_typescript::HIGHLIGHTS_QUERY,
    "tree-sitter-typescript 0.23.2 highlights",
);
const TYPESCRIPT_LOCALS: QueryFragment = upstream(
    tree_sitter_typescript::LOCALS_QUERY,
    "tree-sitter-typescript 0.23.2 locals",
);

const JAVASCRIPT_FUNCTIONS: QueryFragment = runyte(
    include_str!("queries/javascript/functions.scm"),
    "Runyte JavaScript function text objects",
);
const JAVASCRIPT_CLASSES: QueryFragment = runyte(
    include_str!("queries/javascript/classes.scm"),
    "Runyte JavaScript class text objects",
);
const JAVASCRIPT_PARAMETERS: QueryFragment = runyte(
    include_str!("queries/javascript/parameters.scm"),
    "Runyte JavaScript parameter text objects",
);
const TYPESCRIPT_FUNCTIONS: QueryFragment = runyte(
    include_str!("queries/typescript/functions.scm"),
    "Runyte TypeScript function text-object additions",
);
const TYPESCRIPT_CLASSES: QueryFragment = runyte(
    include_str!("queries/typescript/classes.scm"),
    "Runyte TypeScript class text-object additions",
);
const TYPESCRIPT_PARAMETERS: QueryFragment = runyte(
    include_str!("queries/typescript/parameters.scm"),
    "Runyte TypeScript parameter text objects",
);
const JAVASCRIPT_OUTLINE: QueryFragment = runyte(
    include_str!("queries/javascript/outline.scm"),
    "Runyte JavaScript outline",
);
const TYPESCRIPT_OUTLINE: QueryFragment = runyte(
    include_str!("queries/typescript/outline.scm"),
    "Runyte TypeScript outline additions",
);
const C_OUTLINE: QueryFragment = runyte(include_str!("queries/c/outline.scm"), "Runyte C outline");
const CPP_OUTLINE: QueryFragment = runyte(
    include_str!("queries/cpp/outline.scm"),
    "Runyte C++ outline additions",
);
const GO_FUNCTIONS: QueryFragment = runyte(
    include_str!("queries/go/functions.scm"),
    "Runyte Go function text objects",
);
const GO_CLASSES: QueryFragment = runyte(
    include_str!("queries/go/classes.scm"),
    "Runyte Go class-like text objects",
);
const GO_PARAMETERS: QueryFragment = runyte(
    include_str!("queries/go/parameters.scm"),
    "Runyte Go parameter text objects",
);
const GO_OUTLINE: QueryFragment =
    runyte(include_str!("queries/go/outline.scm"), "Runyte Go outline");
const BASH_FUNCTIONS: QueryFragment = runyte(
    include_str!("queries/bash/functions.scm"),
    "Runyte Bash function text objects",
);
const BASH_OUTLINE: QueryFragment = runyte(
    include_str!("queries/bash/outline.scm"),
    "Runyte Bash outline",
);
const JAVA_FUNCTIONS: QueryFragment = runyte(
    include_str!("queries/java/functions.scm"),
    "Runyte Java function text objects",
);
const JAVA_CLASSES: QueryFragment = runyte(
    include_str!("queries/java/classes.scm"),
    "Runyte Java class text objects",
);
const JAVA_PARAMETERS: QueryFragment = runyte(
    include_str!("queries/java/parameters.scm"),
    "Runyte Java parameter text objects",
);
const JAVA_OUTLINE: QueryFragment = runyte(
    include_str!("queries/java/outline.scm"),
    "Runyte Java outline",
);
const KOTLIN_FUNCTIONS: QueryFragment = runyte(
    include_str!("queries/kotlin/functions.scm"),
    "Runyte Kotlin function text objects",
);
const KOTLIN_CLASSES: QueryFragment = runyte(
    include_str!("queries/kotlin/classes.scm"),
    "Runyte Kotlin class text objects",
);
const KOTLIN_PARAMETERS: QueryFragment = runyte(
    include_str!("queries/kotlin/parameters.scm"),
    "Runyte Kotlin parameter text objects",
);
const KOTLIN_OUTLINE: QueryFragment = runyte(
    include_str!("queries/kotlin/outline.scm"),
    "Runyte Kotlin outline",
);
const C_INDENTATION: QueryFragment = runyte(
    include_str!("queries/c/indentation.scm"),
    "Runyte C indentation",
);
const C_FOLDS: QueryFragment = runyte(include_str!("queries/c/folds.scm"), "Runyte C folds");
const CPP_INDENTATION: QueryFragment = runyte(
    include_str!("queries/cpp/indentation.scm"),
    "Runyte C++ indentation additions",
);
const CPP_FOLDS: QueryFragment = runyte(
    include_str!("queries/cpp/folds.scm"),
    "Runyte C++ fold additions",
);
const JAVASCRIPT_INDENTATION: QueryFragment = runyte(
    include_str!("queries/javascript/indentation.scm"),
    "Runyte JavaScript indentation",
);
const JAVASCRIPT_FOLDS: QueryFragment = runyte(
    include_str!("queries/javascript/folds.scm"),
    "Runyte JavaScript folds",
);
const TYPESCRIPT_INDENTATION: QueryFragment = runyte(
    include_str!("queries/typescript/indentation.scm"),
    "Runyte TypeScript indentation additions",
);
const TYPESCRIPT_FOLDS: QueryFragment = runyte(
    include_str!("queries/typescript/folds.scm"),
    "Runyte TypeScript fold additions",
);
const TSX_INDENTATION: QueryFragment = runyte(
    include_str!("queries/tsx/indentation.scm"),
    "Runyte TSX indentation additions",
);
const TSX_FOLDS: QueryFragment = runyte(
    include_str!("queries/tsx/folds.scm"),
    "Runyte TSX fold additions",
);

/// Runyte-authored precedence repair for `tree-sitter-go 0.25.0`.
///
/// The upstream query captures method names as functions before a blanket
/// field-identifier property rule. Tree-house keeps the later mapped capture,
/// so repeat these semantic captures after the upstream fragment.
const GO_METHOD_HIGHLIGHTS: &str = r#"
(method_declaration name: (field_identifier) @function.method)
(call_expression
  function: (selector_expression
    field: (field_identifier) @function.method))
"#;

/// One query fragment and the source whose license/provenance covers it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct QueryFragment {
    pub source: &'static str,
    pub provenance: &'static str,
}

impl QueryFragment {
    pub const fn new(source: &'static str, provenance: &'static str) -> Self {
        Self { source, provenance }
    }
}

/// Ordered query fragments compiled as one Tree-sitter query.
///
/// Listing a base language's fragments before a derived language's fragments
/// is the declarative inheritance mechanism. It already models C -> C++ and
/// can model JavaScript -> TypeScript -> TSX without teaching the registry
/// about any of those language names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct QuerySource {
    pub fragments: &'static [QueryFragment],
}

impl QuerySource {
    pub const EMPTY: Self = Self { fragments: &[] };

    pub const fn new(fragments: &'static [QueryFragment]) -> Self {
        Self { fragments }
    }

    /// Composes fragments using the same separator as the pre-definition
    /// registry. Empty and single-fragment queries stay borrowed.
    pub fn compose(self) -> Cow<'static, str> {
        match self.fragments {
            [] => Cow::Borrowed(""),
            [fragment] => Cow::Borrowed(fragment.source),
            fragments => Cow::Owned(
                fragments
                    .iter()
                    .map(|fragment| fragment.source)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        }
    }
}

/// Parser query capabilities compiled together for one language.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LanguageQueries {
    pub highlights: QuerySource,
    pub injections: QuerySource,
    /// Tree-house's locals query, passed to both highlighting and injections.
    pub locals: QuerySource,
}

/// One independently compiled structural text-object capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TextObjectDefinition {
    pub object: SyntaxObject,
    pub query: QuerySource,
}

/// One statically linked built-in language and all parser-owned metadata.
pub(super) struct LanguageDefinition {
    pub name: &'static str,
    /// Lowercase file extensions, without the dot.
    pub extensions: &'static [&'static str],
    /// Exact, case-sensitive file names recognized before extensions.
    pub filenames: &'static [&'static str],
    /// Lowercase interpreter basenames recognized in a first-line shebang.
    pub shebangs: &'static [&'static str],
    /// Marker that comments out the rest of a line, or `None` where the
    /// language has no line comment at all. Deliberately an `Option` rather
    /// than a default so that adding a language is an explicit decision.
    pub line_comment: Option<&'static str>,
    pub grammar: LanguageFn,
    pub queries: LanguageQueries,
    pub text_objects: &'static [TextObjectDefinition],
    /// Independently compiled document-outline capability.
    pub outline: QuerySource,
    /// Independently compiled newline-indentation capability.
    pub indentation: QuerySource,
    /// Independently compiled fold-range capability.
    pub folds: QuerySource,
}

const fn upstream(source: &'static str, provenance: &'static str) -> QueryFragment {
    QueryFragment::new(source, provenance)
}

const fn runyte(source: &'static str, provenance: &'static str) -> QueryFragment {
    QueryFragment::new(source, provenance)
}

pub(super) const BUILTIN_LANGUAGES: &[LanguageDefinition] = &[
    LanguageDefinition {
        name: "rust",
        extensions: &["rs"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_rust::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_rust::HIGHLIGHTS_QUERY,
                "tree-sitter-rust 0.24.2 highlights",
            )]),
            injections: QuerySource::new(&[upstream(
                tree_sitter_rust::INJECTIONS_QUERY,
                "tree-sitter-rust 0.24.2 injections",
            )]),
            locals: QuerySource::EMPTY,
        },
        text_objects: &[
            TextObjectDefinition {
                object: SyntaxObject::Function,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/rust/functions.scm"),
                    "Runyte Rust function text objects",
                )]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Class,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/rust/classes.scm"),
                    "Runyte Rust class text objects",
                )]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Parameter,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/rust/parameters.scm"),
                    "Runyte Rust parameter text objects",
                )]),
            },
        ],
        outline: QuerySource::new(&[runyte(
            include_str!("queries/rust/outline.scm"),
            "Runyte Rust outline",
        )]),
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/rust/indentation.scm"),
            "Runyte Rust indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/rust/folds.scm"),
            "Runyte Rust folds",
        )]),
    },
    LanguageDefinition {
        name: "python",
        extensions: &["py", "pyi", "py3", "pyw"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("#"),
        grammar: tree_sitter_python::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_python::HIGHLIGHTS_QUERY,
                "tree-sitter-python 0.25.0 highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[
            TextObjectDefinition {
                object: SyntaxObject::Function,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/python/functions.scm"),
                    "Runyte Python function text objects",
                )]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Class,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/python/classes.scm"),
                    "Runyte Python class text objects",
                )]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Parameter,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/python/parameters.scm"),
                    "Runyte Python parameter text objects",
                )]),
            },
        ],
        outline: QuerySource::new(&[runyte(
            include_str!("queries/python/outline.scm"),
            "Runyte Python outline",
        )]),
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/python/indentation.scm"),
            "Runyte Python indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/python/folds.scm"),
            "Runyte Python folds",
        )]),
    },
    LanguageDefinition {
        name: "swift",
        extensions: &["swift", "swiftinterface"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_swift::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[
                upstream(
                    tree_sitter_swift::HIGHLIGHTS_QUERY,
                    "tree-sitter-swift 0.7.3 highlights",
                ),
                runyte(
                    SWIFT_COMMENT_HIGHLIGHTS,
                    "Runyte Swift comment highlight compatibility query",
                ),
            ]),
            // The upstream query injects separate `comment` and `regex`
            // grammars. Neither is bundled, so enabling it would mask Swift
            // comments without producing a replacement highlight layer.
            injections: QuerySource::EMPTY,
            // Keeping this empty preserves existing highlighting behavior.
            // Future definitions can opt into upstream locals explicitly.
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::new(&[runyte(
            include_str!("queries/swift/outline.scm"),
            "Runyte Swift outline",
        )]),
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/swift/indentation.scm"),
            "Runyte Swift indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/swift/folds.scm"),
            "Runyte Swift folds",
        )]),
    },
    LanguageDefinition {
        name: "c",
        extensions: &["c"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_c::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_c::HIGHLIGHT_QUERY,
                "tree-sitter-c 0.24.2 highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::new(&[C_OUTLINE]),
        indentation: QuerySource::new(&[C_INDENTATION]),
        folds: QuerySource::new(&[C_FOLDS]),
    },
    LanguageDefinition {
        name: "cpp",
        extensions: &[
            "cc", "cpp", "cxx", "c++", "h", "hh", "hpp", "hxx", "h++", "ipp", "tpp", "inl", "ixx",
            "cppm", "ino", "cu", "cuh",
        ],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_cpp::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[
                upstream(
                    tree_sitter_c::HIGHLIGHT_QUERY,
                    "tree-sitter-c 0.24.2 inherited highlights",
                ),
                upstream(
                    tree_sitter_cpp::HIGHLIGHT_QUERY,
                    "tree-sitter-cpp 0.23.4 highlights",
                ),
            ]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::new(&[C_OUTLINE, CPP_OUTLINE]),
        indentation: QuerySource::new(&[C_INDENTATION, CPP_INDENTATION]),
        folds: QuerySource::new(&[C_FOLDS, CPP_FOLDS]),
    },
    LanguageDefinition {
        name: "javascript",
        extensions: &["js", "jsx", "mjs", "cjs"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_javascript::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[JAVASCRIPT_HIGHLIGHTS, JAVASCRIPT_JSX_HIGHLIGHTS]),
            // The upstream injections require unbundled regex and JSDoc
            // grammars, infer languages from template tags, and use #offset!
            // for Handlebars templates. Keep them disabled until each target
            // and predicate has an explicit, tested Runyte boundary.
            injections: QuerySource::EMPTY,
            locals: QuerySource::new(&[JAVASCRIPT_LOCALS]),
        },
        text_objects: &[
            TextObjectDefinition {
                object: SyntaxObject::Function,
                query: QuerySource::new(&[JAVASCRIPT_FUNCTIONS]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Class,
                query: QuerySource::new(&[JAVASCRIPT_CLASSES]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Parameter,
                query: QuerySource::new(&[JAVASCRIPT_PARAMETERS]),
            },
        ],
        outline: QuerySource::new(&[JAVASCRIPT_OUTLINE]),
        indentation: QuerySource::new(&[JAVASCRIPT_INDENTATION]),
        folds: QuerySource::new(&[JAVASCRIPT_FOLDS]),
    },
    LanguageDefinition {
        name: "typescript",
        extensions: &["ts", "mts", "cts"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[JAVASCRIPT_HIGHLIGHTS, TYPESCRIPT_HIGHLIGHTS]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::new(&[JAVASCRIPT_LOCALS, TYPESCRIPT_LOCALS]),
        },
        text_objects: &[
            TextObjectDefinition {
                object: SyntaxObject::Function,
                query: QuerySource::new(&[JAVASCRIPT_FUNCTIONS, TYPESCRIPT_FUNCTIONS]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Class,
                query: QuerySource::new(&[JAVASCRIPT_CLASSES, TYPESCRIPT_CLASSES]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Parameter,
                query: QuerySource::new(&[TYPESCRIPT_PARAMETERS]),
            },
        ],
        outline: QuerySource::new(&[JAVASCRIPT_OUTLINE, TYPESCRIPT_OUTLINE]),
        indentation: QuerySource::new(&[JAVASCRIPT_INDENTATION, TYPESCRIPT_INDENTATION]),
        folds: QuerySource::new(&[JAVASCRIPT_FOLDS, TYPESCRIPT_FOLDS]),
    },
    LanguageDefinition {
        name: "tsx",
        extensions: &["tsx"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_typescript::LANGUAGE_TSX,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[
                JAVASCRIPT_HIGHLIGHTS,
                JAVASCRIPT_JSX_HIGHLIGHTS,
                TYPESCRIPT_HIGHLIGHTS,
            ]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::new(&[JAVASCRIPT_LOCALS, TYPESCRIPT_LOCALS]),
        },
        text_objects: &[
            TextObjectDefinition {
                object: SyntaxObject::Function,
                query: QuerySource::new(&[JAVASCRIPT_FUNCTIONS, TYPESCRIPT_FUNCTIONS]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Class,
                query: QuerySource::new(&[JAVASCRIPT_CLASSES, TYPESCRIPT_CLASSES]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Parameter,
                query: QuerySource::new(&[TYPESCRIPT_PARAMETERS]),
            },
        ],
        outline: QuerySource::new(&[JAVASCRIPT_OUTLINE, TYPESCRIPT_OUTLINE]),
        indentation: QuerySource::new(&[
            JAVASCRIPT_INDENTATION,
            TYPESCRIPT_INDENTATION,
            TSX_INDENTATION,
        ]),
        folds: QuerySource::new(&[JAVASCRIPT_FOLDS, TYPESCRIPT_FOLDS, TSX_FOLDS]),
    },
    LanguageDefinition {
        name: "html",
        // This is the exact file-type inventory published by the grammar.
        // Additional web-template extensions require their own reviewed
        // injection and language-detection policy.
        extensions: &["html"],
        filenames: &[],
        shebangs: &[],
        line_comment: None,
        grammar: tree_sitter_html::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_html::HIGHLIGHTS_QUERY,
                "tree-sitter-html 0.23.2 highlights",
            )]),
            // The upstream query is already the reviewed Runyte subset: it
            // injects only the registered JavaScript and CSS languages into
            // script/style raw text. Tags remain in the outer HTML layer.
            injections: QuerySource::new(&[upstream(
                tree_sitter_html::INJECTIONS_QUERY,
                "tree-sitter-html 0.23.2 JavaScript/CSS injections",
            )]),
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::EMPTY,
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/html/indentation.scm"),
            "Runyte HTML indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/html/folds.scm"),
            "Runyte HTML folds",
        )]),
    },
    LanguageDefinition {
        name: "css",
        extensions: &["css"],
        filenames: &[],
        shebangs: &[],
        line_comment: None,
        grammar: tree_sitter_css::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_css::HIGHLIGHTS_QUERY,
                "tree-sitter-css 0.25.0 highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::EMPTY,
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/css/indentation.scm"),
            "Runyte CSS indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/css/folds.scm"),
            "Runyte CSS folds",
        )]),
    },
    LanguageDefinition {
        name: "go",
        extensions: &["go"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_go::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[
                upstream(
                    tree_sitter_go::HIGHLIGHTS_QUERY,
                    "tree-sitter-go 0.25.0 highlights",
                ),
                runyte(
                    GO_METHOD_HIGHLIGHTS,
                    "Runyte Go method highlight precedence query",
                ),
            ]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[
            TextObjectDefinition {
                object: SyntaxObject::Function,
                query: QuerySource::new(&[GO_FUNCTIONS]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Class,
                query: QuerySource::new(&[GO_CLASSES]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Parameter,
                query: QuerySource::new(&[GO_PARAMETERS]),
            },
        ],
        outline: QuerySource::new(&[GO_OUTLINE]),
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/go/indentation.scm"),
            "Runyte Go indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/go/folds.scm"),
            "Runyte Go folds",
        )]),
    },
    LanguageDefinition {
        name: "bash",
        extensions: &["sh", "bash", "ebuild", "eclass"],
        filenames: &[".bashrc", ".bash_profile"],
        shebangs: &["sh", "bash", "dash"],
        line_comment: Some("#"),
        grammar: tree_sitter_bash::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_bash::HIGHLIGHT_QUERY,
                "tree-sitter-bash 0.25.1 highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[TextObjectDefinition {
            object: SyntaxObject::Function,
            query: QuerySource::new(&[BASH_FUNCTIONS]),
        }],
        outline: QuerySource::new(&[BASH_OUTLINE]),
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/bash/indentation.scm"),
            "Runyte Bash indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/bash/folds.scm"),
            "Runyte Bash folds",
        )]),
    },
    LanguageDefinition {
        name: "java",
        extensions: &["java"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_java::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_java::HIGHLIGHTS_QUERY,
                "tree-sitter-java 0.23.5 highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[
            TextObjectDefinition {
                object: SyntaxObject::Function,
                query: QuerySource::new(&[JAVA_FUNCTIONS]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Class,
                query: QuerySource::new(&[JAVA_CLASSES]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Parameter,
                query: QuerySource::new(&[JAVA_PARAMETERS]),
            },
        ],
        outline: QuerySource::new(&[JAVA_OUTLINE]),
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/java/indentation.scm"),
            "Runyte Java indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/java/folds.scm"),
            "Runyte Java folds",
        )]),
    },
    LanguageDefinition {
        name: "kotlin",
        extensions: &["kt", "kts"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_kotlin_sg::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY,
                "tree-sitter-kotlin-sg 0.4.1 highlights derived from nvim-treesitter f8ab59861eed4a1c168505e3433462ed800f2bae",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[
            TextObjectDefinition {
                object: SyntaxObject::Function,
                query: QuerySource::new(&[KOTLIN_FUNCTIONS]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Class,
                query: QuerySource::new(&[KOTLIN_CLASSES]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Parameter,
                query: QuerySource::new(&[KOTLIN_PARAMETERS]),
            },
        ],
        outline: QuerySource::new(&[KOTLIN_OUTLINE]),
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/kotlin/indentation.scm"),
            "Runyte Kotlin indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/kotlin/folds.scm"),
            "Runyte Kotlin folds",
        )]),
    },
    LanguageDefinition {
        name: "sql",
        extensions: &["sql"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("--"),
        grammar: tree_sitter_sequel::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_sequel::HIGHLIGHTS_QUERY,
                "tree-sitter-sequel 0.3.11 highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::new(&[runyte(
            include_str!("queries/sql/outline.scm"),
            "Runyte SQL outline",
        )]),
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/sql/indentation.scm"),
            "tree-sitter-sequel 0.3.11 indentation adapted for Runyte",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/sql/folds.scm"),
            "Runyte SQL folds",
        )]),
    },
    LanguageDefinition {
        name: "lua",
        extensions: &["lua"],
        filenames: &[],
        shebangs: &["lua"],
        line_comment: Some("--"),
        grammar: tree_sitter_lua::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_lua::HIGHLIGHTS_QUERY,
                "tree-sitter-lua 0.5.0 highlights",
            )]),
            injections: QuerySource::new(&[upstream(
                tree_sitter_lua::INJECTIONS_QUERY,
                "tree-sitter-lua 0.5.0 C injections",
            )]),
            locals: QuerySource::new(&[upstream(
                tree_sitter_lua::LOCALS_QUERY,
                "tree-sitter-lua 0.5.0 locals",
            )]),
        },
        text_objects: &[
            TextObjectDefinition {
                object: SyntaxObject::Function,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/lua/functions.scm"),
                    "Runyte Lua function text objects",
                )]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Class,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/lua/classes.scm"),
                    "Runyte Lua class-like table text objects",
                )]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Parameter,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/lua/parameters.scm"),
                    "Runyte Lua parameter text objects",
                )]),
            },
        ],
        outline: QuerySource::new(&[runyte(
            include_str!("queries/lua/outline.scm"),
            "Runyte Lua outline",
        )]),
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/lua/indentation.scm"),
            "Runyte Lua indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/lua/folds.scm"),
            "Runyte Lua folds",
        )]),
    },
    LanguageDefinition {
        name: "c-sharp",
        extensions: &["cs", "csx"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_c_sharp::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_c_sharp::HIGHLIGHTS_QUERY,
                "tree-sitter-c-sharp 0.23.5 highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[
            TextObjectDefinition {
                object: SyntaxObject::Function,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/c-sharp/functions.scm"),
                    "Runyte C# function text objects",
                )]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Class,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/c-sharp/classes.scm"),
                    "Runyte C# class-like text objects",
                )]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Parameter,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/c-sharp/parameters.scm"),
                    "Runyte C# parameter text objects",
                )]),
            },
        ],
        outline: QuerySource::new(&[runyte(
            include_str!("queries/c-sharp/outline.scm"),
            "Runyte C# outline",
        )]),
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/c-sharp/indentation.scm"),
            "Runyte C# indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/c-sharp/folds.scm"),
            "Runyte C# folds",
        )]),
    },
    LanguageDefinition {
        name: "zig",
        extensions: &["zig", "zon"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_zig::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[runyte(
                include_str!("queries/zig/highlights.scm"),
                "tree-sitter-zig 1.1.2 highlights adapted for Runyte predicates",
            )]),
            injections: QuerySource::new(&[upstream(
                tree_sitter_zig::INJECTIONS_QUERY,
                "tree-sitter-zig 1.1.2 comment injections",
            )]),
            locals: QuerySource::EMPTY,
        },
        text_objects: &[
            TextObjectDefinition {
                object: SyntaxObject::Function,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/zig/functions.scm"),
                    "Runyte Zig function text objects",
                )]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Class,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/zig/classes.scm"),
                    "Runyte Zig class-like container text objects",
                )]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Parameter,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/zig/parameters.scm"),
                    "Runyte Zig parameter text objects",
                )]),
            },
        ],
        outline: QuerySource::new(&[runyte(
            include_str!("queries/zig/outline.scm"),
            "Runyte Zig outline",
        )]),
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/zig/indentation.scm"),
            "tree-sitter-zig 1.1.2 indentation adapted for Runyte",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/zig/folds.scm"),
            "tree-sitter-zig 1.1.2 folds",
        )]),
    },
    LanguageDefinition {
        name: "cmake",
        extensions: &["cmake"],
        filenames: &["CMakeLists.txt"],
        shebangs: &[],
        line_comment: Some("#"),
        grammar: tree_sitter_cmake::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[runyte(
                include_str!("queries/cmake/highlights.scm"),
                "tree-sitter-cmake 0.7.4 highlights adapted for Runyte predicates",
            )]),
            injections: QuerySource::new(&[upstream(
                tree_sitter_cmake::INJECTIONS_QUERY,
                "tree-sitter-cmake 0.7.4 comment injections",
            )]),
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::EMPTY,
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/cmake/indentation.scm"),
            "tree-sitter-cmake 0.7.4 indentation adapted for Runyte",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/cmake/folds.scm"),
            "tree-sitter-cmake 0.7.4 folds",
        )]),
    },
    LanguageDefinition {
        name: "proto",
        extensions: &["proto"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("//"),
        grammar: tree_sitter_proto::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[runyte(
                include_str!("queries/proto/highlights.scm"),
                "tree-sitter-proto 0.5.0 packaged highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::EMPTY,
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/proto/indentation.scm"),
            "tree-sitter-proto 0.5.0 indentation adapted for Runyte",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/proto/folds.scm"),
            "tree-sitter-proto 0.5.0 folds",
        )]),
    },
    LanguageDefinition {
        name: "make",
        extensions: &["mk", "mak"],
        filenames: &["Makefile", "makefile", "GNUmakefile"],
        shebangs: &[],
        line_comment: Some("#"),
        grammar: tree_sitter_make::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_make::HIGHLIGHTS_QUERY,
                "tree-sitter-make 1.1.1 highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::EMPTY,
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/make/indentation.scm"),
            "Runyte Make indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/make/folds.scm"),
            "Runyte Make folds",
        )]),
    },
    LanguageDefinition {
        name: "ini",
        extensions: &["ini"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some(";"),
        grammar: tree_sitter_ini::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_ini::HIGHLIGHTS_QUERY,
                "tree-sitter-ini 1.4.0 highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::EMPTY,
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/ini/indentation.scm"),
            "Runyte INI indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/ini/folds.scm"),
            "tree-sitter-ini 1.4.0 folds",
        )]),
    },
    LanguageDefinition {
        name: "json",
        extensions: &["json"],
        filenames: &[],
        shebangs: &[],
        line_comment: None,
        grammar: tree_sitter_json::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_json::HIGHLIGHTS_QUERY,
                "tree-sitter-json 0.24.8 highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::EMPTY,
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/json/indentation.scm"),
            "Runyte JSON indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/json/folds.scm"),
            "Runyte JSON folds",
        )]),
    },
    LanguageDefinition {
        name: "toml",
        extensions: &["toml"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("#"),
        grammar: tree_sitter_toml_ng::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
                "tree-sitter-toml-ng 0.7.0 highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::EMPTY,
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/toml/indentation.scm"),
            "Runyte TOML indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/toml/folds.scm"),
            "Runyte TOML folds",
        )]),
    },
    LanguageDefinition {
        name: "yaml",
        extensions: &["yaml", "yml"],
        filenames: &[],
        shebangs: &[],
        line_comment: Some("#"),
        grammar: tree_sitter_yaml::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[upstream(
                tree_sitter_yaml::HIGHLIGHTS_QUERY,
                "tree-sitter-yaml 0.7.2 highlights",
            )]),
            injections: QuerySource::EMPTY,
            locals: QuerySource::EMPTY,
        },
        text_objects: &[],
        outline: QuerySource::EMPTY,
        indentation: QuerySource::new(&[runyte(
            include_str!("queries/yaml/indentation.scm"),
            "Runyte YAML indentation",
        )]),
        folds: QuerySource::new(&[runyte(
            include_str!("queries/yaml/folds.scm"),
            "Runyte YAML folds",
        )]),
    },
    LanguageDefinition {
        name: "markdown",
        extensions: &["md", "markdown"],
        filenames: &[],
        shebangs: &[],
        line_comment: None,
        grammar: tree_sitter_md::LANGUAGE,
        queries: LanguageQueries {
            highlights: QuerySource::new(&[runyte(
                include_str!("queries/markdown/highlights.scm"),
                "Runyte semantic Markdown block highlights for tree-sitter-md 0.5.3",
            )]),
            injections: QuerySource::new(&[runyte(
                include_str!("queries/markdown/injections.scm"),
                "Runyte Markdown injections for tree-sitter-md 0.5.3",
            )]),
            locals: QuerySource::EMPTY,
        },
        text_objects: &[
            TextObjectDefinition {
                object: SyntaxObject::Section,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/markdown/sections.scm"),
                    "Runyte Markdown section text objects",
                )]),
            },
            TextObjectDefinition {
                object: SyntaxObject::Paragraph,
                query: QuerySource::new(&[runyte(
                    include_str!("queries/markdown/paragraphs.scm"),
                    "Runyte Markdown paragraph text objects",
                )]),
            },
        ],
        outline: QuerySource::new(&[runyte(
            include_str!("queries/markdown/outline.scm"),
            "Runyte Markdown outline",
        )]),
        indentation: QuerySource::EMPTY,
        folds: QuerySource::new(&[runyte(
            include_str!("queries/markdown/folds.scm"),
            "Runyte Markdown folds",
        )]),
    },
];

/// Parser configurations reachable only through an injection marker.
///
/// Keeping these outside [`BUILTIN_LANGUAGES`] prevents an implementation
/// grammar from becoming a file-detection or language-server identity.
pub(super) const MARKDOWN_INLINE: LanguageDefinition = LanguageDefinition {
    name: "markdown_inline",
    extensions: &[],
    filenames: &[],
    shebangs: &[],
    line_comment: None,
    grammar: tree_sitter_md::INLINE_LANGUAGE,
    queries: LanguageQueries {
        highlights: QuerySource::new(&[runyte(
            include_str!("queries/markdown/inline_highlights.scm"),
            "Runyte semantic Markdown inline highlights for tree-sitter-md 0.5.3",
        )]),
        injections: QuerySource::EMPTY,
        locals: QuerySource::EMPTY,
    },
    text_objects: &[],
    outline: QuerySource::EMPTY,
    indentation: QuerySource::EMPTY,
    folds: QuerySource::EMPTY,
};

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn built_in_detection_inventory_is_exact_and_unique() {
        let inventory = BUILTIN_LANGUAGES
            .iter()
            .map(|definition| (definition.name, definition.extensions))
            .collect::<Vec<_>>();
        assert_eq!(
            inventory,
            [
                ("rust", &["rs"][..]),
                ("python", &["py", "pyi", "py3", "pyw"][..]),
                ("swift", &["swift", "swiftinterface"][..]),
                ("c", &["c"][..]),
                (
                    "cpp",
                    &[
                        "cc", "cpp", "cxx", "c++", "h", "hh", "hpp", "hxx", "h++", "ipp", "tpp",
                        "inl", "ixx", "cppm", "ino", "cu", "cuh",
                    ][..],
                ),
                ("javascript", &["js", "jsx", "mjs", "cjs"][..]),
                ("typescript", &["ts", "mts", "cts"][..]),
                ("tsx", &["tsx"][..]),
                ("html", &["html"][..]),
                ("css", &["css"][..]),
                ("go", &["go"][..]),
                ("bash", &["sh", "bash", "ebuild", "eclass"][..]),
                ("java", &["java"][..]),
                ("kotlin", &["kt", "kts"][..]),
                ("sql", &["sql"][..]),
                ("lua", &["lua"][..]),
                ("c-sharp", &["cs", "csx"][..]),
                ("zig", &["zig", "zon"][..]),
                ("cmake", &["cmake"][..]),
                ("proto", &["proto"][..]),
                ("make", &["mk", "mak"][..]),
                ("ini", &["ini"][..]),
                ("json", &["json"][..]),
                ("toml", &["toml"][..]),
                ("yaml", &["yaml", "yml"][..]),
                ("markdown", &["md", "markdown"][..]),
            ]
        );

        let mut names = HashSet::new();
        let mut extensions = HashSet::new();
        let mut filenames = HashSet::new();
        let mut shebangs = HashSet::new();
        for definition in BUILTIN_LANGUAGES
            .iter()
            .chain(std::iter::once(&MARKDOWN_INLINE))
        {
            assert!(names.insert(definition.name), "duplicate language name");
            assert_eq!(definition.name, definition.name.to_ascii_lowercase());
            for extension in definition.extensions {
                assert!(
                    extensions.insert(*extension),
                    "duplicate extension {extension}"
                );
                assert_eq!(*extension, extension.to_ascii_lowercase());
            }
            for filename in definition.filenames {
                assert!(
                    filenames.insert(*filename),
                    "duplicate exact filename {filename}"
                );
            }
            for shebang in definition.shebangs {
                assert!(
                    shebangs.insert(*shebang),
                    "duplicate shebang interpreter {shebang}"
                );
                assert_eq!(*shebang, shebang.to_ascii_lowercase());
            }
        }

        let bash = BUILTIN_LANGUAGES
            .iter()
            .find(|definition| definition.name == "bash")
            .unwrap();
        assert_eq!(bash.filenames, &[".bashrc", ".bash_profile"]);
        assert_eq!(bash.shebangs, &["sh", "bash", "dash"]);

        let lua = BUILTIN_LANGUAGES
            .iter()
            .find(|definition| definition.name == "lua")
            .unwrap();
        assert_eq!(lua.shebangs, &["lua"]);

        let cmake = BUILTIN_LANGUAGES
            .iter()
            .find(|definition| definition.name == "cmake")
            .unwrap();
        assert_eq!(cmake.filenames, &["CMakeLists.txt"]);

        let make = BUILTIN_LANGUAGES
            .iter()
            .find(|definition| definition.name == "make")
            .unwrap();
        assert_eq!(make.filenames, &["Makefile", "makefile", "GNUmakefile"]);
    }

    #[test]
    fn query_composition_is_ordered_and_borrows_simple_sources() {
        const FIRST: QueryFragment = QueryFragment::new("(first) @one", "first fixture");
        const SECOND: QueryFragment = QueryFragment::new("(second) @two", "second fixture");

        assert!(matches!(QuerySource::EMPTY.compose(), Cow::Borrowed("")));
        assert!(matches!(
            QuerySource::new(&[FIRST]).compose(),
            Cow::Borrowed("(first) @one")
        ));
        assert_eq!(
            QuerySource::new(&[FIRST, SECOND]).compose(),
            "(first) @one\n(second) @two"
        );
    }

    #[test]
    fn javascript_family_query_inheritance_is_ordered() {
        let javascript = BUILTIN_LANGUAGES
            .iter()
            .find(|definition| definition.name == "javascript")
            .unwrap();
        let typescript = BUILTIN_LANGUAGES
            .iter()
            .find(|definition| definition.name == "typescript")
            .unwrap();
        let tsx = BUILTIN_LANGUAGES
            .iter()
            .find(|definition| definition.name == "tsx")
            .unwrap();

        assert_eq!(
            javascript.queries.highlights.fragments,
            &[JAVASCRIPT_HIGHLIGHTS, JAVASCRIPT_JSX_HIGHLIGHTS]
        );
        assert!(javascript.queries.injections.fragments.is_empty());
        assert_eq!(javascript.queries.locals.fragments, &[JAVASCRIPT_LOCALS]);
        assert_eq!(javascript.outline.fragments, &[JAVASCRIPT_OUTLINE]);
        assert_eq!(
            typescript.queries.highlights.fragments,
            &[JAVASCRIPT_HIGHLIGHTS, TYPESCRIPT_HIGHLIGHTS]
        );
        assert!(typescript.queries.injections.fragments.is_empty());
        assert_eq!(
            typescript.queries.locals.fragments,
            &[JAVASCRIPT_LOCALS, TYPESCRIPT_LOCALS]
        );
        assert_eq!(
            typescript.outline.fragments,
            &[JAVASCRIPT_OUTLINE, TYPESCRIPT_OUTLINE]
        );
        assert_eq!(
            tsx.queries.highlights.fragments,
            &[
                JAVASCRIPT_HIGHLIGHTS,
                JAVASCRIPT_JSX_HIGHLIGHTS,
                TYPESCRIPT_HIGHLIGHTS,
            ]
        );
        assert!(tsx.queries.injections.fragments.is_empty());
        assert_eq!(
            tsx.queries.locals.fragments,
            &[JAVASCRIPT_LOCALS, TYPESCRIPT_LOCALS]
        );
        assert_eq!(
            tsx.outline.fragments,
            &[JAVASCRIPT_OUTLINE, TYPESCRIPT_OUTLINE]
        );
    }

    #[test]
    fn html_injections_are_the_audited_registered_subset() {
        let html = BUILTIN_LANGUAGES
            .iter()
            .find(|definition| definition.name == "html")
            .unwrap();
        let injection = html.queries.injections.compose();

        assert_eq!(html.queries.injections.fragments.len(), 1);
        assert_eq!(injection, tree_sitter_html::INJECTIONS_QUERY);
        assert!(injection.contains("script_element"));
        assert!(injection.contains("style_element"));
        assert!(injection.contains("injection.language \"javascript\""));
        assert!(injection.contains("injection.language \"css\""));
        assert_eq!(injection.matches("#set! injection.language").count(), 2);
        assert!(!injection.contains("@injection.language"));
        assert!(!injection.contains("injection.filename"));
        assert!(!injection.contains("injection.shebang"));
    }

    #[test]
    fn every_query_fragment_has_explicit_provenance() {
        for definition in BUILTIN_LANGUAGES
            .iter()
            .chain(std::iter::once(&MARKDOWN_INLINE))
        {
            let sources = [
                definition.queries.highlights,
                definition.queries.injections,
                definition.queries.locals,
            ]
            .into_iter()
            .chain(definition.text_objects.iter().map(|object| object.query))
            .chain([definition.outline, definition.indentation, definition.folds]);
            for source in sources {
                for fragment in source.fragments {
                    assert!(
                        !fragment.source.trim().is_empty(),
                        "{} has an empty query",
                        definition.name
                    );
                    assert!(
                        !fragment.provenance.trim().is_empty(),
                        "{} has a query without provenance",
                        definition.name
                    );
                }
            }
        }
    }
}
