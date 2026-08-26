// SPDX-License-Identifier: MPL-2.0

//! Tree-sitter syntax highlighting.
//!
//! This is the only module aware of `tree-house`. Everything above it sees
//! [`Scope`] values and character-offset spans, so a breaking change in the
//! highlighter API stops here.
//!
//! Failure is always local: a grammar whose queries do not compile on first
//! use is recorded in [`Registry::errors`]. Its identity and extension mapping
//! remain available, while affected buffers fall back to plain text rather
//! than taking the editor down.

mod background;
mod grammars;

pub(crate) use background::StaleSyntax;
pub use background::{SyntaxEvent, SyntaxEvents, SyntaxHandle, spawn_background};

use std::{
    cell::Cell,
    collections::HashMap,
    fmt,
    path::Path,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use ropey::RopeSlice;
use tree_house::{
    InjectionLanguageMarker, Language, LanguageConfig, LanguageLoader, Layer, QueryMatchIter,
    QueryMatchIterEvent, Syntax,
    highlighter::{Highlight, Highlighter},
};
use tree_house_bindings::{Grammar, InputEdit, Node, Point, Query, query::InvalidPredicateError};

use crate::text::{Change, Offset, Text, Transaction};

/// Stable identity of a language within a [`Registry`].
///
/// The corresponding tree-house language handle is deliberately kept private
/// to this module. Plain, injection-free parser variants map back to the same
/// identity, so callers do not need to know which parser configuration a
/// particular document size selected.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LanguageId(u32);

/// Revision of a parsed syntax document.
///
/// Structural node paths are only meaningful at the revision that produced
/// them. The first successful parse is revision zero and every successful
/// incremental update advances it.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SyntaxRevision(u64);

impl SyntaxRevision {
    pub fn get(self) -> u64 {
        self.0
    }

    fn advance(&mut self) {
        self.0 = self
            .0
            .checked_add(1)
            .expect("syntax revision counter exhausted");
    }
}

/// A half-open range in Runyte character offsets.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SyntaxRange {
    pub from: Offset,
    pub to: Offset,
}

impl SyntaxRange {
    pub fn new(from: Offset, to: Offset) -> Result<Self, SyntaxError> {
        if from > to {
            return Err(SyntaxError::InvalidRange { from, to });
        }
        Ok(Self { from, to })
    }

    pub fn point(offset: Offset) -> Self {
        Self {
            from: offset,
            to: offset,
        }
    }

    /// Validates this range against the current text revision.
    pub fn checked(self, text: &Text) -> Result<Self, SyntaxError> {
        if self.from > self.to {
            return Err(SyntaxError::InvalidRange {
                from: self.from,
                to: self.to,
            });
        }
        if self.to > text.len_chars() {
            return Err(SyntaxError::CharacterOffsetOutOfBounds {
                offset: self.to,
                len_chars: text.len_chars(),
            });
        }
        Ok(self)
    }

    pub fn is_empty(self) -> bool {
        self.from == self.to
    }
}

/// Grammar-specific node kind owned independently of a parser tree.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SyntaxKind(Box<str>);

impl SyntaxKind {
    pub fn new(value: impl Into<Box<str>>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SyntaxKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Opaque locator for a node in one parsed document revision.
///
/// The path contains only Runyte-owned coordinates and child indices. In
/// particular, it never stores a tree-house node ID or exposes parser-layer
/// handles. Paths are invalidated by a successful syntax update.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct SyntaxPath {
    document: u64,
    revision: SyntaxRevision,
    probe: Offset,
    layer_depth: u32,
    child_indices: Box<[u32]>,
}

impl fmt::Debug for SyntaxPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyntaxPath")
            .field("revision", &self.revision)
            .field("probe", &self.probe)
            .field("layer_depth", &self.layer_depth)
            .field("child_indices", &self.child_indices)
            .finish()
    }
}

impl SyntaxPath {
    pub fn revision(&self) -> SyntaxRevision {
        self.revision
    }
}

/// Owned description of one syntax node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxNodeSummary {
    pub path: SyntaxPath,
    pub range: SyntaxRange,
    pub kind: SyntaxKind,
    pub language: LanguageId,
    pub named: bool,
    pub missing: bool,
    pub extra: bool,
}

/// Grammar-independent structural relationship between syntax nodes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SyntaxRelation {
    Parent,
    FirstNamedChild,
    PreviousNamedSibling,
    NextNamedSibling,
}

/// A selection-range transformation independent of any input grammar.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SyntaxSelectionTransform {
    Expand,
    Parent,
    FirstNamedChild,
    PreviousNamedSibling,
    NextNamedSibling,
}

/// A range tagged with the private document identity and syntax revision that
/// produced it.
///
/// Future pane-local expansion history can store this value without retaining
/// parser nodes. Reusing it after a successful reparse returns
/// [`SyntaxError::StaleRevision`]; using it with another document or a
/// recreated parser returns [`SyntaxError::ForeignDocument`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SyntaxSelectionRange {
    pub range: SyntaxRange,
    pub revision: SyntaxRevision,
    document: u64,
}

/// Grammar-independent structural object requested from a syntax tree.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SyntaxObject {
    Function,
    Class,
    Parameter,
    Section,
    Paragraph,
}

/// Whether a text object includes its delimiters/declaration or only content.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SyntaxObjectPart {
    Around,
    Inside,
}

/// A paired delimiter understood by structural surround selection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DelimiterPair {
    Parentheses,
    SquareBrackets,
    Braces,
    AngleBrackets,
    DoubleQuotes,
    SingleQuotes,
    Backticks,
}

impl DelimiterPair {
    pub const ALL: &'static [Self] = &[
        Self::Parentheses,
        Self::SquareBrackets,
        Self::Braces,
        Self::AngleBrackets,
        Self::DoubleQuotes,
        Self::SingleQuotes,
        Self::Backticks,
    ];

    pub const fn delimiters(self) -> (char, char) {
        match self {
            Self::Parentheses => ('(', ')'),
            Self::SquareBrackets => ('[', ']'),
            Self::Braces => ('{', '}'),
            Self::AngleBrackets => ('<', '>'),
            Self::DoubleQuotes => ('"', '"'),
            Self::SingleQuotes => ('\'', '\''),
            Self::Backticks => ('`', '`'),
        }
    }
}

impl SyntaxObject {
    fn capture_name(self, part: SyntaxObjectPart) -> &'static str {
        match (self, part) {
            (Self::Function, SyntaxObjectPart::Around) => "function.around",
            (Self::Function, SyntaxObjectPart::Inside) => "function.inside",
            (Self::Class, SyntaxObjectPart::Around) => "class.around",
            (Self::Class, SyntaxObjectPart::Inside) => "class.inside",
            (Self::Parameter, SyntaxObjectPart::Around) => "parameter.around",
            (Self::Parameter, SyntaxObjectPart::Inside) => "parameter.inside",
            (Self::Section, SyntaxObjectPart::Around) => "section.around",
            (Self::Section, SyntaxObjectPart::Inside) => "section.inside",
            (Self::Paragraph, SyntaxObjectPart::Around) => "paragraph.around",
            (Self::Paragraph, SyntaxObjectPart::Inside) => "paragraph.inside",
        }
    }

    fn unsupported_capture_name(self) -> &'static str {
        match self {
            Self::Function => "function.unsupported",
            Self::Class => "class.unsupported",
            Self::Parameter => "parameter.unsupported",
            Self::Section => "section.unsupported",
            Self::Paragraph => "paragraph.unsupported",
        }
    }
}

/// One structural query match, including all ranges assigned to its capture.
///
/// A capture can deliberately contain disjoint nodes (for example a parameter
/// and its following comma). Keeping those ranges separate prevents callers
/// from accidentally including unrelated text between them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxCapture {
    pub object: SyntaxObject,
    pub part: SyntaxObjectPart,
    pub language: LanguageId,
    pub revision: SyntaxRevision,
    pub ranges: Vec<SyntaxRange>,
}

/// Presentation-neutral kind of a document-outline entry.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum OutlineKind {
    Module,
    Type,
    Class,
    Struct,
    Enum,
    Actor,
    Extension,
    Alias,
    Concept,
    Interface,
    Function,
    Method,
    Subscript,
    Property,
    Constant,
    Macro,
    Heading,
}

impl OutlineKind {
    fn capture_name(self) -> &'static str {
        match self {
            Self::Module => "outline.module",
            Self::Type => "outline.type",
            Self::Class => "outline.class",
            Self::Struct => "outline.struct",
            Self::Enum => "outline.enum",
            Self::Actor => "outline.actor",
            Self::Extension => "outline.extension",
            Self::Alias => "outline.alias",
            Self::Concept => "outline.concept",
            Self::Interface => "outline.interface",
            Self::Function => "outline.function",
            Self::Method => "outline.method",
            Self::Subscript => "outline.subscript",
            Self::Property => "outline.property",
            Self::Constant => "outline.constant",
            Self::Macro => "outline.macro",
            Self::Heading => "outline.heading",
        }
    }

    fn specificity(self) -> u8 {
        match self {
            Self::Method | Self::Struct | Self::Enum | Self::Interface => 2,
            _ => 1,
        }
    }
}

const OUTLINE_KINDS: [OutlineKind; 17] = [
    OutlineKind::Module,
    OutlineKind::Type,
    OutlineKind::Class,
    OutlineKind::Struct,
    OutlineKind::Enum,
    OutlineKind::Actor,
    OutlineKind::Extension,
    OutlineKind::Alias,
    OutlineKind::Concept,
    OutlineKind::Interface,
    OutlineKind::Function,
    OutlineKind::Method,
    OutlineKind::Subscript,
    OutlineKind::Property,
    OutlineKind::Constant,
    OutlineKind::Macro,
    OutlineKind::Heading,
];

/// One source-ordered entry in a document outline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutlineItem {
    pub name: Box<str>,
    pub kind: OutlineKind,
    /// Full declaration or section range used to derive hierarchy.
    pub range: SyntaxRange,
    /// Revision-safe jump target, normally the declaration name.
    pub target: SyntaxSelectionRange,
    pub language: LanguageId,
    /// Zero for the document grammar, increasing through nested injections.
    pub injection_depth: u32,
    /// Index of the nearest returned item whose range strictly contains this
    /// item, or `None` for a top-level entry.
    pub parent: Option<usize>,
}

/// Explicit degradation encountered while projecting an outline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutlineIssue {
    /// Injection parsing was intentionally disabled by the large-document
    /// policy, so only outer-language entries can be returned.
    InjectionsDisabled { language: LanguageId },
    /// An injected language is parseable but has no truthful outline query.
    UnsupportedInjectedLanguage {
        language: LanguageId,
        injection_depth: u32,
    },
    /// An injected outline query failed independently of parsing/highlights.
    InjectedQueryFailed {
        language: LanguageId,
        injection_depth: u32,
        message: Box<str>,
    },
    /// The injected parser configuration could not produce a tree.
    InjectedParserUnavailable {
        language: LanguageId,
        injection_depth: u32,
        message: Box<str>,
    },
    /// The injected parser produced an error tree. Valid queried declarations
    /// may still be present, but consumers know the result is incomplete.
    IncompleteInjectedParse {
        language: LanguageId,
        injection_depth: u32,
        range: SyntaxRange,
    },
}

/// A bounded, immutable outline projection for one syntax revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Outline {
    pub revision: SyntaxRevision,
    pub items: Vec<OutlineItem>,
    pub issues: Vec<OutlineIssue>,
    /// True when source, item, label, or hierarchy limits omitted results.
    pub truncated: bool,
}

/// Revision-scoped indentation answer for inserting text after one newline.
///
/// `begin_levels` counts captured containers that begin on the newline's row;
/// `always_levels` counts captured containers that span it regardless of their
/// start row. Keeping the two minimal query semantics distinct lets a future
/// editing frontend combine them with its own whitespace policy without
/// exposing Tree-sitter captures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewlineIndent {
    pub newline: SyntaxRange,
    pub revision: SyntaxRevision,
    pub language: LanguageId,
    pub injection_depth: u32,
    pub begin_levels: u16,
    pub always_levels: u16,
    pub issues: Vec<IndentIssue>,
    pub truncated: bool,
    document: u64,
}

/// Explicit degradation encountered while resolving indentation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndentIssue {
    InjectionsDisabled {
        language: LanguageId,
    },
    UnsupportedInjectedLanguage {
        language: LanguageId,
        injection_depth: u32,
    },
    InjectedQueryFailed {
        language: LanguageId,
        injection_depth: u32,
        message: Box<str>,
    },
    InjectedParserUnavailable {
        language: LanguageId,
        injection_depth: u32,
        message: Box<str>,
    },
    IncompleteParse {
        language: LanguageId,
        injection_depth: u32,
    },
}

/// A conservative fold range tied to one parsed document revision.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SyntaxFoldRange {
    pub range: SyntaxRange,
    pub revision: SyntaxRevision,
    document: u64,
}

/// One presentation-neutral fold candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoldItem {
    pub range: SyntaxFoldRange,
    pub language: LanguageId,
    pub injection_depth: u32,
}

/// Explicit degradation encountered while projecting folds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FoldIssue {
    InjectionsDisabled {
        language: LanguageId,
    },
    UnsupportedInjectedLanguage {
        language: LanguageId,
        injection_depth: u32,
    },
    InjectedQueryFailed {
        language: LanguageId,
        injection_depth: u32,
        message: Box<str>,
    },
    InjectedParserUnavailable {
        language: LanguageId,
        injection_depth: u32,
        message: Box<str>,
    },
    IncompleteInjectedParse {
        language: LanguageId,
        injection_depth: u32,
        range: SyntaxRange,
    },
}

/// Bounded, deterministic folds for one syntax revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoldList {
    pub revision: SyntaxRevision,
    pub items: Vec<FoldItem>,
    pub issues: Vec<FoldIssue>,
    pub truncated: bool,
}

/// Coordinate and revision failures at the Runyte syntax boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyntaxError {
    InvalidRange {
        from: Offset,
        to: Offset,
    },
    CharacterOffsetOutOfBounds {
        offset: Offset,
        len_chars: usize,
    },
    ByteOffsetOutOfBounds {
        offset: usize,
        len_bytes: usize,
    },
    DocumentTooLarge {
        len_bytes: usize,
    },
    StaleRevision {
        expected: SyntaxRevision,
        actual: SyntaxRevision,
    },
    ForeignDocument,
    InvalidPath,
    UnknownLanguage,
    UnsupportedTextObject {
        language: LanguageId,
        object: SyntaxObject,
        part: SyntaxObjectPart,
    },
    TextObjectQueryFailed {
        language: LanguageId,
        message: Box<str>,
    },
    UnsupportedOutline {
        language: LanguageId,
    },
    OutlineQueryFailed {
        language: LanguageId,
        message: Box<str>,
    },
    InvalidNewline {
        offset: Offset,
    },
    UnsupportedIndentation {
        language: LanguageId,
    },
    IndentationQueryFailed {
        language: LanguageId,
        message: Box<str>,
    },
    UnsupportedFolds {
        language: LanguageId,
    },
    FoldQueryFailed {
        language: LanguageId,
        message: Box<str>,
    },
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange { from, to } => {
                write!(f, "invalid syntax range {from}..{to}")
            }
            Self::CharacterOffsetOutOfBounds { offset, len_chars } => write!(
                f,
                "character offset {offset} exceeds document length {len_chars}"
            ),
            Self::ByteOffsetOutOfBounds { offset, len_bytes } => {
                write!(
                    f,
                    "byte offset {offset} exceeds document length {len_bytes}"
                )
            }
            Self::DocumentTooLarge { len_bytes } => write!(
                f,
                "document contains {len_bytes} bytes, exceeding the syntax engine limit"
            ),
            Self::StaleRevision { expected, actual } => write!(
                f,
                "syntax result belongs to revision {}, current revision is {}",
                expected.get(),
                actual.get()
            ),
            Self::ForeignDocument => {
                f.write_str("syntax result belongs to a different parsed document")
            }
            Self::InvalidPath => f.write_str("syntax path does not belong to this document"),
            Self::UnknownLanguage => {
                f.write_str("syntax node belongs to an unknown parser language")
            }
            Self::UnsupportedTextObject {
                language,
                object,
                part,
            } => write!(
                f,
                "language {} does not support the {object:?}.{part:?} text object",
                language.0
            ),
            Self::TextObjectQueryFailed { language, message } => write!(
                f,
                "text-object query for language {} failed to compile: {message}",
                language.0
            ),
            Self::UnsupportedOutline { language } => {
                write!(
                    f,
                    "language {} does not support document outlines",
                    language.0
                )
            }
            Self::OutlineQueryFailed { language, message } => write!(
                f,
                "outline query for language {} failed to compile: {message}",
                language.0
            ),
            Self::InvalidNewline { offset } => {
                write!(f, "character offset {offset} does not point at a newline")
            }
            Self::UnsupportedIndentation { language } => write!(
                f,
                "language {} does not support syntax indentation",
                language.0
            ),
            Self::IndentationQueryFailed { language, message } => write!(
                f,
                "indentation query for language {} failed to compile: {message}",
                language.0
            ),
            Self::UnsupportedFolds { language } => {
                write!(f, "language {} does not support syntax folds", language.0)
            }
            Self::FoldQueryFailed { language, message } => write!(
                f,
                "fold query for language {} failed to compile: {message}",
                language.0
            ),
        }
    }
}

impl std::error::Error for SyntaxError {}

/// A grammar or highlight-query failure discovered on first use.
///
/// Registry construction only records static language identities. This owned
/// value lets frontends report a failed lazy configuration without exposing
/// tree-house's grammar or query error types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryError {
    pub language: LanguageId,
    pub language_name: &'static str,
    pub plain: bool,
    pub message: Box<str>,
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}: {}",
            self.language_name,
            if self.plain { " (plain)" } else { "" },
            self.message
        )
    }
}

impl std::error::Error for RegistryError {}

/// How long a single parse may run before tree-sitter gives up.
///
/// Exceeding it means the buffer simply goes unhighlighted. Its value is kept
/// deliberately unchanged while reparsing moves to the background; tuning it
/// is separate follow-up work.
const PARSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Document size above which injection queries are dropped.
///
/// `Syntax::update` re-runs the injection query across the whole document on
/// every edit, which costs roughly 160 ms/MB in a debug build and scales
/// linearly with document size rather than with edit size. That is the single
/// dominant cost of reparsing and it would put every keystroke in a large file
/// far outside a frame budget.
///
/// Below this threshold injections are kept, so fenced code blocks in Markdown
/// and embedded languages in macros highlight normally. Above it they are
/// dropped and only the outer language is highlighted, which keeps typing
/// responsive. The limit is chosen so the query stays inside a frame budget
/// even unoptimised.
const INJECTION_LIMIT_BYTES: usize = 128 * 1024;

const OUTLINE_SOURCE_LIMIT_BYTES: usize = 4 * 1024 * 1024;
/// Only the beginning of the first line can identify a shebang interpreter.
/// Bounding this independently keeps language inference cheap for scratch
/// buffers containing one enormous line.
const SHEBANG_PREFIX_LIMIT_CHARS: usize = 1_024;
const OUTLINE_ITEM_LIMIT: usize = 4096;
const OUTLINE_LABEL_BUDGET_BYTES: usize = 512 * 1024;
const OUTLINE_NAME_LIMIT_CHARS: usize = 256;
const OUTLINE_DEPTH_LIMIT: usize = 64;
const OUTLINE_ISSUE_LIMIT: usize = 128;
const OUTLINE_ISSUE_MESSAGE_LIMIT_CHARS: usize = 512;
const FOLD_ITEM_LIMIT: usize = 4096;
const SYNTAX_CAPABILITY_ISSUE_LIMIT: usize = 128;
const INDENT_LEVEL_LIMIT: usize = 128;

/// Process-local identity for revision-scoped syntax paths. Zero is skipped so
/// an accidentally zero-initialized path can never name a real document.
static NEXT_SYNTAX_DOCUMENT: AtomicU64 = AtomicU64::new(1);

/// Themeable highlight scopes.
///
/// Tree-sitter capture names are hierarchical (`keyword.control.return`). Each
/// capture is mapped to the most specific scope here that prefixes it, so a
/// grammar emitting captures we have never seen still degrades to a sensible
/// colour instead of none.
pub const SCOPES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constructor",
    "function",
    "keyword",
    "label",
    "markup.bold",
    "markup.heading",
    "markup.italic",
    "markup.link.text",
    "markup.link.url",
    "markup.list",
    "markup.quote",
    "markup.raw",
    "namespace",
    "number",
    "operator",
    "property",
    "punctuation",
    "string",
    "tag",
    "type",
    "variable",
];

/// A resolved highlight scope, indexing [`SCOPES`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Scope(u32);

impl Scope {
    /// The scope one of the names in [`SCOPES`] stands for.
    ///
    /// Highlight spans normally come from a parsed tree, but a buffer Runyte
    /// projects itself — the branch list, and anything like it — has no grammar
    /// to ask, while still having parts a reader needs told apart. Those
    /// projections name a scope here so their colour comes from the same theme
    /// entry as everything else, rather than from a second palette.
    pub fn named(name: &str) -> Option<Self> {
        SCOPES
            .iter()
            .position(|scope| *scope == name)
            .map(|index| Self(index as u32))
    }

    pub fn name(self) -> &'static str {
        SCOPES[self.0 as usize]
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Maps a tree-sitter capture name onto a themeable scope by walking from the
/// most specific dotted prefix to the least.
fn scope_for_capture(capture: &str) -> Option<Scope> {
    let capture = match capture {
        // Semantic aliases emitted by the audited Kotlin highlight query.
        // Keeping these aliases at the owned syntax boundary avoids adding
        // grammar-specific theme scopes above this module.
        "boolean" => "constant",
        "character" => "string",
        "conditional" | "exception" | "include" | "repeat" => "keyword",
        "float" => "number",
        // `@none` deliberately clears an earlier capture on the same node.
        // Predicate-only captures beginning with `_` likewise stay neutral.
        "none" => return None,
        capture if capture.starts_with('_') => return None,
        capture => capture,
    };
    let mut candidate = capture;
    loop {
        if let Some(index) = SCOPES.iter().position(|scope| *scope == candidate) {
            return Some(Scope(index as u32));
        }
        let dot = candidate.rfind('.')?;
        candidate = &candidate[..dot];
    }
}

/// Statically linked grammar metadata, lazy query configurations, and extension mapping.
pub struct Registry {
    configs: Vec<LazyLanguageConfig>,
    /// Public identity for every internal parser configuration. Injection-free
    /// variants therefore point back to their canonical language.
    public_languages: Vec<LanguageId>,
    /// Canonical internal parser configuration for each public language.
    internal_languages: Vec<Language>,
    names: Vec<&'static str>,
    /// Line-comment marker for each public language, `None` where the
    /// language has no line comment.
    line_comments: Vec<Option<&'static str>>,
    by_extension: HashMap<&'static str, LanguageId>,
    by_filename: HashMap<&'static str, LanguageId>,
    by_shebang: HashMap<&'static str, LanguageId>,
    by_name: HashMap<&'static str, LanguageId>,
    /// Parser configurations named only by injection queries. They do not
    /// participate in file detection, public language lookup, or LSP setup.
    injection_languages: HashMap<&'static str, Language>,
    /// Injection-free counterpart of each language that has an injection
    /// query, used for documents above [`INJECTION_LIMIT_BYTES`].
    plain_variants: HashMap<LanguageId, Language>,
    text_objects: Vec<HashMap<SyntaxObject, LazyTextObjectCapability>>,
    outlines: Vec<Option<LazyOutlineCapability>>,
    indentations: Vec<Option<LazyIndentationCapability>>,
    folds: Vec<Option<LazyFoldCapability>>,
}

enum TextObjectCapability {
    Ready(Query),
    Failed(Box<str>),
}

enum OutlineCapability {
    Ready(Query),
    Failed(Box<str>),
}

enum IndentationCapability {
    Ready(Query),
    Failed(Box<str>),
}

enum FoldCapability {
    Ready(Query),
    Failed(Box<str>),
}

struct LazyLanguageConfig {
    language: LanguageId,
    definition: &'static grammars::LanguageDefinition,
    include_injections: bool,
    queries: grammars::LanguageQueries,
    value: OnceLock<Result<LanguageConfig, Box<str>>>,
    #[cfg(test)]
    initializations: std::sync::atomic::AtomicUsize,
}

impl LazyLanguageConfig {
    fn new(
        language: LanguageId,
        definition: &'static grammars::LanguageDefinition,
        include_injections: bool,
        queries: grammars::LanguageQueries,
    ) -> Self {
        Self {
            language,
            definition,
            include_injections,
            queries,
            value: OnceLock::new(),
            #[cfg(test)]
            initializations: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn get(&self) -> Result<&LanguageConfig, &str> {
        self.value
            .get_or_init(|| {
                #[cfg(test)]
                self.initializations
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let grammar: Grammar =
                    self.definition.grammar.try_into().map_err(|error| {
                        format!("incompatible grammar ({error})").into_boxed_str()
                    })?;
                let config =
                    compile_language_config(grammar, self.queries, self.include_injections)
                        .map_err(|error| {
                            format!("query failed to compile ({error})").into_boxed_str()
                        })?;
                config.configure(|capture| {
                    scope_for_capture(capture).map(|scope| Highlight::new(scope.0))
                });
                Ok(config)
            })
            .as_ref()
            .map_err(Box::as_ref)
    }

    fn error(&self) -> Option<&str> {
        self.value.get()?.as_ref().err().map(Box::as_ref)
    }

    #[cfg(test)]
    fn initialization_count(&self) -> usize {
        self.initializations
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

struct LazyTextObjectCapability {
    definition: &'static grammars::LanguageDefinition,
    text_object: &'static grammars::TextObjectDefinition,
    override_source: Option<Box<str>>,
    value: OnceLock<TextObjectCapability>,
    #[cfg(test)]
    initializations: std::sync::atomic::AtomicUsize,
}

impl LazyTextObjectCapability {
    fn new(
        definition: &'static grammars::LanguageDefinition,
        text_object: &'static grammars::TextObjectDefinition,
        override_source: Option<Box<str>>,
    ) -> Self {
        Self {
            definition,
            text_object,
            override_source,
            value: OnceLock::new(),
            #[cfg(test)]
            initializations: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn get(&self) -> &TextObjectCapability {
        self.value.get_or_init(|| {
            #[cfg(test)]
            self.initializations
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let grammar: Grammar = match self.definition.grammar.try_into() {
                Ok(grammar) => grammar,
                Err(error) => {
                    return TextObjectCapability::Failed(
                        format!("incompatible grammar ({error})").into(),
                    );
                }
            };
            let composed;
            let source = match self.override_source.as_deref() {
                Some(source) => source,
                None => {
                    composed = self.text_object.query.compose();
                    &composed
                }
            };
            match Query::new(grammar, source, |_, _| Ok(())) {
                Ok(query) => TextObjectCapability::Ready(query),
                Err(error) => TextObjectCapability::Failed(error.to_string().into()),
            }
        })
    }

    #[cfg(test)]
    fn initialization_count(&self) -> usize {
        self.initializations
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

struct LazyOutlineCapability {
    definition: &'static grammars::LanguageDefinition,
    override_source: Option<Box<str>>,
    value: OnceLock<OutlineCapability>,
    #[cfg(test)]
    initializations: std::sync::atomic::AtomicUsize,
}

struct LazyIndentationCapability {
    definition: &'static grammars::LanguageDefinition,
    override_source: Option<Box<str>>,
    value: OnceLock<IndentationCapability>,
    #[cfg(test)]
    initializations: std::sync::atomic::AtomicUsize,
}

impl LazyIndentationCapability {
    fn new(
        definition: &'static grammars::LanguageDefinition,
        override_source: Option<Box<str>>,
    ) -> Self {
        Self {
            definition,
            override_source,
            value: OnceLock::new(),
            #[cfg(test)]
            initializations: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn get(&self) -> &IndentationCapability {
        self.value.get_or_init(|| {
            #[cfg(test)]
            self.initializations
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            match compile_owned_query(
                self.definition,
                self.override_source.as_deref(),
                self.definition.indentation,
                &["indent.begin", "indent.always"],
            ) {
                Ok(query) => IndentationCapability::Ready(query),
                Err(error) => IndentationCapability::Failed(error),
            }
        })
    }

    #[cfg(test)]
    fn initialization_count(&self) -> usize {
        self.initializations
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

struct LazyFoldCapability {
    definition: &'static grammars::LanguageDefinition,
    override_source: Option<Box<str>>,
    value: OnceLock<FoldCapability>,
    #[cfg(test)]
    initializations: std::sync::atomic::AtomicUsize,
}

impl LazyFoldCapability {
    fn new(
        definition: &'static grammars::LanguageDefinition,
        override_source: Option<Box<str>>,
    ) -> Self {
        Self {
            definition,
            override_source,
            value: OnceLock::new(),
            #[cfg(test)]
            initializations: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn get(&self) -> &FoldCapability {
        self.value.get_or_init(|| {
            #[cfg(test)]
            self.initializations
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            match compile_owned_query(
                self.definition,
                self.override_source.as_deref(),
                self.definition.folds,
                &["fold"],
            ) {
                Ok(query) => FoldCapability::Ready(query),
                Err(error) => FoldCapability::Failed(error),
            }
        })
    }

    #[cfg(test)]
    fn initialization_count(&self) -> usize {
        self.initializations
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

fn compile_owned_query(
    definition: &'static grammars::LanguageDefinition,
    override_source: Option<&str>,
    query_source: grammars::QuerySource,
    allowed_captures: &[&str],
) -> Result<Query, Box<str>> {
    let grammar: Grammar = definition
        .grammar
        .try_into()
        .map_err(|error| format!("incompatible grammar ({error})").into_boxed_str())?;
    let composed;
    let source = match override_source {
        Some(source) => source,
        None => {
            composed = query_source.compose();
            &composed
        }
    };
    let query = Query::new(grammar, source, |_, predicate| {
        Err(InvalidPredicateError::Other {
            msg: format!("unsupported syntax capability predicate {predicate}").into(),
        })
    })
    .map_err(|error| error.to_string().into_boxed_str())?;
    if let Some((_, capture)) = query
        .captures()
        .find(|(_, capture)| !allowed_captures.contains(capture))
    {
        return Err(format!("unsupported capture @{capture}").into_boxed_str());
    }
    Ok(query)
}

impl LazyOutlineCapability {
    fn new(
        definition: &'static grammars::LanguageDefinition,
        override_source: Option<Box<str>>,
    ) -> Self {
        Self {
            definition,
            override_source,
            value: OnceLock::new(),
            #[cfg(test)]
            initializations: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn get(&self) -> &OutlineCapability {
        self.value.get_or_init(|| {
            #[cfg(test)]
            self.initializations
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let grammar: Grammar = match self.definition.grammar.try_into() {
                Ok(grammar) => grammar,
                Err(error) => {
                    return OutlineCapability::Failed(
                        format!("incompatible grammar ({error})").into(),
                    );
                }
            };
            let composed;
            let source = match self.override_source.as_deref() {
                Some(source) => source,
                None => {
                    composed = self.definition.outline.compose();
                    &composed
                }
            };
            match Query::new(grammar, source, |_, _| Ok(())) {
                Ok(query) => OutlineCapability::Ready(query),
                Err(error) => OutlineCapability::Failed(error.to_string().into()),
            }
        })
    }

    #[cfg(test)]
    fn initialization_count(&self) -> usize {
        self.initializations
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

fn compile_language_config(
    grammar: Grammar,
    queries: grammars::LanguageQueries,
    include_injections: bool,
) -> Result<LanguageConfig, String> {
    let highlights = queries.highlights.compose();
    let injections = if include_injections {
        queries.injections.compose()
    } else {
        Default::default()
    };
    let locals = queries.locals.compose();
    LanguageConfig::new(grammar, &highlights, &injections, &locals)
        .map_err(|error| error.to_string())
}

impl Registry {
    /// Builds the registry, skipping any grammar that fails to load or whose
    /// queries fail to compile. Never panics and never touches the network.
    pub fn new() -> Self {
        Self::new_with_text_object_override(None)
    }

    fn new_with_text_object_override(override_query: Option<(&str, SyntaxObject, &str)>) -> Self {
        Self::new_with_overrides(override_query, None, None, None, None)
    }

    #[cfg(test)]
    fn new_with_config_override(
        override_config: Option<(&str, bool, grammars::LanguageQueries)>,
    ) -> Self {
        Self::new_with_overrides(None, override_config, None, None, None)
    }

    #[cfg(test)]
    fn new_with_outline_override(override_query: Option<(&str, &str)>) -> Self {
        Self::new_with_overrides(None, None, override_query, None, None)
    }

    #[cfg(test)]
    fn new_with_indentation_override(override_query: Option<(&str, &str)>) -> Self {
        Self::new_with_overrides(None, None, None, override_query, None)
    }

    #[cfg(test)]
    fn new_with_fold_override(override_query: Option<(&str, &str)>) -> Self {
        Self::new_with_overrides(None, None, None, None, override_query)
    }

    #[cfg(test)]
    pub(crate) fn new_with_broken_config_for_test(language: &str, plain: bool) -> Self {
        const INVALID_HIGHLIGHT: grammars::QueryFragment = grammars::QueryFragment::new(
            "(runyte_invalid_node) @keyword",
            "invalid lazy configuration test fixture",
        );
        let queries = grammars::LanguageQueries {
            highlights: grammars::QuerySource::new(&[INVALID_HIGHLIGHT]),
            injections: grammars::QuerySource::EMPTY,
            locals: grammars::QuerySource::EMPTY,
        };
        Self::new_with_config_override(Some((language, plain, queries)))
    }

    fn new_with_overrides(
        override_query: Option<(&str, SyntaxObject, &str)>,
        override_config: Option<(&str, bool, grammars::LanguageQueries)>,
        override_outline: Option<(&str, &str)>,
        override_indentation: Option<(&str, &str)>,
        override_folds: Option<(&str, &str)>,
    ) -> Self {
        let mut registry = Self {
            configs: Vec::new(),
            public_languages: Vec::new(),
            internal_languages: Vec::new(),
            names: Vec::new(),
            line_comments: Vec::new(),
            by_extension: HashMap::new(),
            by_filename: HashMap::new(),
            by_shebang: HashMap::new(),
            by_name: HashMap::new(),
            injection_languages: HashMap::new(),
            plain_variants: HashMap::new(),
            text_objects: Vec::new(),
            outlines: Vec::new(),
            indentations: Vec::new(),
            folds: Vec::new(),
        };

        for definition in grammars::BUILTIN_LANGUAGES {
            let language_id = LanguageId(registry.internal_languages.len() as u32);
            let language = Language::new(registry.configs.len() as u32);
            let queries = override_config
                .filter(|(name, plain, _)| *name == definition.name && !*plain)
                .map_or(definition.queries, |(_, _, queries)| queries);
            registry.configs.push(LazyLanguageConfig::new(
                language_id,
                definition,
                true,
                queries,
            ));
            registry.public_languages.push(language_id);
            registry.internal_languages.push(language);
            registry.names.push(definition.name);
            registry.line_comments.push(definition.line_comment);
            let text_objects = definition
                .text_objects
                .iter()
                .map(|text_object| {
                    let object = text_object.object;
                    let override_source = override_query
                        .filter(|(language, target, _)| {
                            *language == definition.name && *target == object
                        })
                        .map(|(_, _, source)| source.into());
                    let capability =
                        LazyTextObjectCapability::new(definition, text_object, override_source);
                    (object, capability)
                })
                .collect();
            registry.text_objects.push(text_objects);
            registry.outlines.push(
                (!definition.outline.fragments.is_empty()
                    || override_outline.is_some_and(|(name, _)| name == definition.name))
                .then(|| {
                    let source = override_outline
                        .filter(|(name, _)| *name == definition.name)
                        .map(|(_, source)| source.into());
                    LazyOutlineCapability::new(definition, source)
                }),
            );
            registry.indentations.push(
                (!definition.indentation.fragments.is_empty()
                    || override_indentation.is_some_and(|(name, _)| name == definition.name))
                .then(|| {
                    let source = override_indentation
                        .filter(|(name, _)| *name == definition.name)
                        .map(|(_, source)| source.into());
                    LazyIndentationCapability::new(definition, source)
                }),
            );
            registry.folds.push(
                (!definition.folds.fragments.is_empty()
                    || override_folds.is_some_and(|(name, _)| name == definition.name))
                .then(|| {
                    let source = override_folds
                        .filter(|(name, _)| *name == definition.name)
                        .map(|(_, source)| source.into());
                    LazyFoldCapability::new(definition, source)
                }),
            );
            registry.by_name.insert(definition.name, language_id);
            for extension in definition.extensions {
                registry.by_extension.insert(*extension, language_id);
            }
            for filename in definition.filenames {
                registry.by_filename.insert(*filename, language_id);
            }
            for shebang in definition.shebangs {
                registry.by_shebang.insert(*shebang, language_id);
            }

            // Build the injection-free counterpart used for large documents.
            // It is deliberately absent from the name and extension maps: it is
            // an internal substitution, not a language a caller can ask for.
            if !definition.queries.injections.fragments.is_empty() {
                let plain_language = Language::new(registry.configs.len() as u32);
                let queries = override_config
                    .filter(|(name, plain, _)| *name == definition.name && *plain)
                    .map_or(definition.queries, |(_, _, queries)| queries);
                registry.configs.push(LazyLanguageConfig::new(
                    language_id,
                    definition,
                    false,
                    queries,
                ));
                registry.public_languages.push(language_id);
                registry.plain_variants.insert(language_id, plain_language);
            }
        }

        // Markdown has distinct block and inline grammars. The block query
        // reaches this parser through `markdown_inline`, but it remains an
        // implementation layer of the public Markdown language rather than a
        // document language of its own.
        if let Some(markdown) = registry.by_name.get("markdown").copied() {
            let inline = Language::new(registry.configs.len() as u32);
            registry.configs.push(LazyLanguageConfig::new(
                markdown,
                &grammars::MARKDOWN_INLINE,
                true,
                grammars::MARKDOWN_INLINE.queries,
            ));
            registry.public_languages.push(markdown);
            registry
                .injection_languages
                .insert(grammars::MARKDOWN_INLINE.name, inline);
        }
        registry
    }

    /// The language to actually parse `bytes` of source with, dropping
    /// injections when the document is large enough for the injection query to
    /// dominate reparsing.
    fn language_for_size(&self, language: LanguageId, bytes: usize) -> Option<Language> {
        if bytes <= INJECTION_LIMIT_BYTES {
            return self.internal_language(language);
        }
        self.plain_variants
            .get(&language)
            .copied()
            .or_else(|| self.internal_language(language))
    }

    fn internal_language(&self, language: LanguageId) -> Option<Language> {
        self.internal_languages.get(language.0 as usize).copied()
    }

    fn public_language(&self, language: Language) -> Option<LanguageId> {
        self.public_languages.get(language.idx()).copied()
    }

    fn text_object_capability(
        &self,
        language: Language,
        object: SyntaxObject,
    ) -> Option<&TextObjectCapability> {
        let public = self.public_language(language)?;
        Some(
            self.text_objects
                .get(public.0 as usize)?
                .get(&object)?
                .get(),
        )
    }

    fn outline_capability(&self, language: Language) -> Option<&OutlineCapability> {
        let public = self.public_language(language)?;
        Some(self.outlines.get(public.0 as usize)?.as_ref()?.get())
    }

    fn indentation_capability(&self, language: Language) -> Option<&IndentationCapability> {
        let public = self.public_language(language)?;
        Some(self.indentations.get(public.0 as usize)?.as_ref()?.get())
    }

    fn fold_capability(&self, language: Language) -> Option<&FoldCapability> {
        let public = self.public_language(language)?;
        Some(self.folds.get(public.0 as usize)?.as_ref()?.get())
    }

    fn parser_error(&self, language: Language) -> Option<&str> {
        self.configs.get(language.idx())?.error()
    }

    fn language_has_injections(&self, language: LanguageId) -> bool {
        self.internal_language(language)
            .and_then(|internal| self.configs.get(internal.idx()))
            .is_some_and(|config| !config.definition.queries.injections.fragments.is_empty())
    }

    /// Language configurations that failed during their first use. The editor
    /// stays usable and the affected canonical or injection-free variant is
    /// simply unhighlighted. Unused languages have no errors to report because
    /// their grammar and queries have not been compiled yet.
    pub fn errors(&self) -> Vec<RegistryError> {
        self.configs
            .iter()
            .filter_map(|config| {
                config.error().map(|error| RegistryError {
                    language: config.language,
                    language_name: config.definition.name,
                    plain: !config.include_injections,
                    message: error.into(),
                })
            })
            .collect()
    }

    pub fn language_name(&self, language: LanguageId) -> &'static str {
        self.names[language.0 as usize]
    }

    /// The marker that comments out the rest of a line in this language, or
    /// `None` where the language has no line comment.
    pub fn line_comment(&self, language: LanguageId) -> Option<&'static str> {
        self.line_comments[language.0 as usize]
    }

    pub fn language_for_path(&self, path: &Path) -> Option<LanguageId> {
        if let Some(filename) = path.file_name().and_then(|name| name.to_str())
            && let Some(language) = self.by_filename.get(filename)
        {
            return Some(*language);
        }
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        self.by_extension.get(extension.as_str()).copied()
    }

    /// Infers a document language using bounded, editor-owned inputs.
    ///
    /// Exact file names win over case-insensitive extensions, and both win
    /// over a first-line shebang. The shebang scan reads at most 1,024
    /// characters, so a pathless or unknown one-line buffer cannot make
    /// inference allocate in proportion to its size.
    pub fn language_for_document(&self, path: Option<&Path>, source: &Text) -> Option<LanguageId> {
        path.and_then(|path| self.language_for_path(path))
            .or_else(|| {
                let prefix = source
                    .line(0)
                    .chars()
                    .take(SHEBANG_PREFIX_LIMIT_CHARS)
                    .take_while(|character| *character != '\n')
                    .collect::<String>();
                if prefix.starts_with("#!") {
                    self.language_for_shebang(&prefix)
                } else {
                    None
                }
            })
    }

    pub fn language_for_name(&self, name: &str) -> Option<LanguageId> {
        self.by_name.get(name).copied()
    }
}

impl Registry {
    /// Resolves a language named inside the document, such as the info string
    /// of a Markdown code fence.
    ///
    /// Documents name languages loosely — `rs` and `rust`, `yml` and `yaml` —
    /// so this accepts both the canonical name and any registered extension.
    fn language_for_text(&self, text: &str) -> Option<LanguageId> {
        let key = text.trim().to_ascii_lowercase();
        if key.is_empty() {
            return None;
        }
        self.by_name
            .get(key.as_str())
            .or_else(|| self.by_extension.get(key.as_str()))
            .copied()
    }

    fn language_for_injection_text(&self, text: &str) -> Option<Language> {
        let key = text.trim().to_ascii_lowercase();
        if key.is_empty() {
            return None;
        }
        self.injection_languages
            .get(key.as_str())
            .copied()
            .or_else(|| {
                self.language_for_text(&key)
                    .and_then(|language| self.internal_language(language))
            })
    }

    fn language_for_shebang(&self, text: &str) -> Option<LanguageId> {
        let interpreter = shebang_interpreter(text)?;
        self.by_shebang.get(interpreter).copied()
    }
}

impl LanguageLoader for Registry {
    fn language_for_marker(&self, marker: InjectionLanguageMarker) -> Option<Language> {
        match marker {
            InjectionLanguageMarker::Name(name) => self.language_for_injection_text(name),
            InjectionLanguageMarker::Match(text) => {
                self.language_for_injection_text(&text.to_string())
            }
            InjectionLanguageMarker::Filename(path) => self
                .language_for_path(Path::new(&path.to_string()))
                .and_then(|language| self.internal_language(language)),
            InjectionLanguageMarker::Shebang(text) => {
                let marker = text
                    .chars()
                    .take(SHEBANG_PREFIX_LIMIT_CHARS)
                    .collect::<String>();
                self.language_for_shebang(&marker)
                    .and_then(|language| self.internal_language(language))
            }
        }
    }

    fn get_config(&self, language: Language) -> Option<&LanguageConfig> {
        self.configs.get(language.idx())?.get().ok()
    }
}

fn shebang_interpreter(text: &str) -> Option<&str> {
    let command = match text.strip_prefix("#!") {
        Some(command) => command.trim_start(),
        // Tree-house's injection marker already extracts the interpreter
        // token, so it reaches this helper without a shebang prefix.
        None if !text.starts_with(char::is_whitespace) => text.trim_end(),
        None => return None,
    };
    let mut words = command.split_ascii_whitespace();
    let executable = words.next()?;
    let executable = executable.rsplit(['/', '\\']).next()?;
    if executable != "env" {
        return Some(executable);
    }

    let mut word = words.next()?;
    while word.starts_with('-') || is_env_assignment(word) {
        word = words.next()?;
    }
    word.rsplit(['/', '\\']).next()
}

fn is_env_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

/// A parsed syntax tree for one buffer.
#[derive(Clone, Debug)]
pub struct DocumentSyntax {
    document: u64,
    language: LanguageId,
    revision: SyntaxRevision,
    syntax: Syntax,
}

/// A highlighted span, in character offsets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub from: Offset,
    pub to: Offset,
    pub scope: Scope,
}

impl DocumentSyntax {
    /// Parses a buffer. Returns `None` when the parse fails, which leaves the
    /// caller rendering plain text.
    pub fn new(text: &Text, language: LanguageId, registry: &Registry) -> Option<Self> {
        checked_char_to_byte(text, text.len_chars()).ok()?;
        let parser_language = registry.language_for_size(language, text.rope().len_bytes())?;
        debug_assert_eq!(registry.public_language(parser_language), Some(language));
        let syntax =
            Syntax::new(rope_slice(text), parser_language, PARSE_TIMEOUT, registry).ok()?;
        let document = NEXT_SYNTAX_DOCUMENT.fetch_add(1, Ordering::Relaxed);
        assert_ne!(document, u64::MAX, "syntax document identity exhausted");
        Some(Self {
            document,
            language,
            revision: SyntaxRevision::default(),
            syntax,
        })
    }

    pub fn language(&self) -> LanguageId {
        self.language
    }

    pub fn revision(&self) -> SyntaxRevision {
        self.revision
    }

    /// Reparses incrementally after a transaction.
    ///
    /// `before` must be the text as it was when the transaction's offsets were
    /// computed, and `text` the result of applying it. Returns `false` when the
    /// reparse failed, in which case the caller should drop this tree.
    pub fn update(
        &mut self,
        before: &Text,
        text: &Text,
        transaction: &Transaction,
        registry: &Registry,
    ) -> bool {
        let Some(parser_language) =
            registry.language_for_size(self.language, text.rope().len_bytes())
        else {
            return false;
        };
        let current_language = self.syntax.layer(self.syntax.root()).language;
        if parser_language != current_language {
            let Ok(syntax) =
                Syntax::new(rope_slice(text), parser_language, PARSE_TIMEOUT, registry)
            else {
                return false;
            };
            self.syntax = syntax;
            self.revision.advance();
            return true;
        }
        let Ok(edits) = input_edits(before, transaction) else {
            return false;
        };
        let updated = self
            .syntax
            .update(rope_slice(text), PARSE_TIMEOUT, &edits, registry)
            .is_ok();
        if updated {
            self.revision.advance();
        }
        updated
    }

    /// Smallest named node covering the character at `offset`.
    ///
    /// A caret at EOF is biased to the final character. An empty document has
    /// no character to bias toward, so its zero-width root node is returned.
    pub fn node_at(
        &self,
        text: &Text,
        registry: &Registry,
        offset: Offset,
    ) -> Result<Option<SyntaxNodeSummary>, SyntaxError> {
        checked_char_to_byte(text, offset)?;
        let range = if text.len_chars() == 0 {
            SyntaxRange::point(0)
        } else if offset == text.len_chars() {
            SyntaxRange {
                from: offset - 1,
                to: offset,
            }
        } else {
            SyntaxRange {
                from: offset,
                to: offset + 1,
            }
        };
        self.node_covering(text, registry, range)
    }

    /// Smallest named node in the deepest parser layer covering `range`.
    pub fn node_covering(
        &self,
        text: &Text,
        registry: &Registry,
        range: SyntaxRange,
    ) -> Result<Option<SyntaxNodeSummary>, SyntaxError> {
        let bytes = checked_range_to_bytes(text, range)?;
        let preferred_probe = range_probe(text, range);
        let Some((layer_depth, layer, node)) = self.deepest_named_node(bytes) else {
            return Ok(None);
        };
        self.summary(text, registry, layer_depth, layer, node, preferred_probe)
            .map(Some)
    }

    /// Named ancestors from the outer document root to `path`, inclusive.
    ///
    /// Every parser layer is walked independently and the chains are then
    /// joined outer-to-inner. This deliberately does not use tree-house's
    /// cross-layer `TreeCursor::goto_parent`, which jumps from an injected root
    /// to the outer tree root and skips the enclosing injection node.
    pub fn ancestors(
        &self,
        text: &Text,
        registry: &Registry,
        path: &SyntaxPath,
    ) -> Result<Vec<SyntaxNodeSummary>, SyntaxError> {
        let resolved = self.resolve_path(text, path)?;
        let probe_bytes = character_probe_bytes(text, path.probe)?;
        let layers = self.layers_covering(probe_bytes.clone());
        let mut summaries = Vec::new();

        for (depth, layer) in layers
            .into_iter()
            .enumerate()
            .take(resolved.layer_depth + 1)
        {
            let layer_data = self.syntax.layer(layer);
            let Some(tree) = layer_data.tree() else {
                continue;
            };
            let node = if depth == resolved.layer_depth {
                resolved.node.clone()
            } else {
                tree.root_node()
                    .named_descendant_for_byte_range(probe_bytes.start, probe_bytes.end)
                    .unwrap_or_else(|| tree.root_node())
            };
            for ancestor in named_ancestor_chain(node) {
                summaries.push(self.summary(text, registry, depth, layer, ancestor, path.probe)?);
            }
        }
        Ok(summaries)
    }

    pub fn related(
        &self,
        text: &Text,
        registry: &Registry,
        path: &SyntaxPath,
        relation: SyntaxRelation,
    ) -> Result<Option<SyntaxNodeSummary>, SyntaxError> {
        match relation {
            SyntaxRelation::Parent => self.parent(text, registry, path),
            SyntaxRelation::FirstNamedChild => self.first_named_child(text, registry, path),
            SyntaxRelation::PreviousNamedSibling => {
                self.previous_named_sibling(text, registry, path)
            }
            SyntaxRelation::NextNamedSibling => self.next_named_sibling(text, registry, path),
        }
    }

    pub fn parent(
        &self,
        text: &Text,
        registry: &Registry,
        path: &SyntaxPath,
    ) -> Result<Option<SyntaxNodeSummary>, SyntaxError> {
        let ancestors = self.ancestors(text, registry, path)?;
        Ok(ancestors
            .len()
            .checked_sub(2)
            .and_then(|index| ancestors.get(index).cloned()))
    }

    pub fn first_named_child(
        &self,
        text: &Text,
        registry: &Registry,
        path: &SyntaxPath,
    ) -> Result<Option<SyntaxNodeSummary>, SyntaxError> {
        let resolved = self.resolve_path(text, path)?;
        if let Some(child) = resolved.node.named_child(0) {
            return self
                .summary(
                    text,
                    registry,
                    resolved.layer_depth,
                    resolved.layer,
                    child,
                    path.probe,
                )
                .map(Some);
        }

        // A leaf in an outer grammar may be the content node into which the
        // next language is injected. Enter that layer only after exhausting
        // ordinary named children in the current tree.
        let probe_bytes = character_probe_bytes(text, path.probe)?;
        let layers = self.layers_covering(probe_bytes);
        let next_depth = resolved.layer_depth + 1;
        let Some(&layer) = layers.get(next_depth) else {
            return Ok(None);
        };
        let Some(tree) = self.syntax.layer(layer).tree() else {
            return Ok(None);
        };
        self.summary(
            text,
            registry,
            next_depth,
            layer,
            tree.root_node(),
            path.probe,
        )
        .map(Some)
    }

    pub fn previous_named_sibling(
        &self,
        text: &Text,
        registry: &Registry,
        path: &SyntaxPath,
    ) -> Result<Option<SyntaxNodeSummary>, SyntaxError> {
        let resolved = self.resolve_path(text, path)?;
        let Some(sibling) = resolved.node.prev_named_sibling() else {
            return Ok(None);
        };
        self.summary(
            text,
            registry,
            resolved.layer_depth,
            resolved.layer,
            sibling,
            path.probe,
        )
        .map(Some)
    }

    pub fn next_named_sibling(
        &self,
        text: &Text,
        registry: &Registry,
        path: &SyntaxPath,
    ) -> Result<Option<SyntaxNodeSummary>, SyntaxError> {
        let resolved = self.resolve_path(text, path)?;
        let Some(sibling) = resolved.node.next_named_sibling() else {
            return Ok(None);
        };
        self.summary(
            text,
            registry,
            resolved.layer_depth,
            resolved.layer,
            sibling,
            path.probe,
        )
        .map(Some)
    }

    /// Tags an editor range with the current syntax revision.
    pub fn selection_range(
        &self,
        text: &Text,
        range: SyntaxRange,
    ) -> Result<SyntaxSelectionRange, SyntaxError> {
        Ok(SyntaxSelectionRange {
            range: range.checked(text)?,
            revision: self.revision,
            document: self.document,
        })
    }

    /// Resolves a range saved by future expansion history without retaining
    /// parser nodes. This is the history-independent primitive shrink needs:
    /// pane state chooses which prior range to restore, while syntax validates
    /// that the document has not changed underneath it.
    pub fn resolve_selection_range(
        &self,
        text: &Text,
        selection: SyntaxSelectionRange,
    ) -> Result<SyntaxRange, SyntaxError> {
        if selection.document != self.document {
            return Err(SyntaxError::ForeignDocument);
        }
        if selection.revision != self.revision {
            return Err(SyntaxError::StaleRevision {
                expected: selection.revision,
                actual: self.revision,
            });
        }
        selection.range.checked(text)
    }

    /// Applies a structural selection transform to a revision-tagged range.
    ///
    /// Expansion and parent always choose the nearest *strict* visual
    /// superset. Equal-range grammar wrappers are skipped, so a successful
    /// transform can never appear to do nothing.
    pub fn transform_selection_range(
        &self,
        text: &Text,
        registry: &Registry,
        selection: SyntaxSelectionRange,
        transform: SyntaxSelectionTransform,
    ) -> Result<Option<SyntaxSelectionRange>, SyntaxError> {
        let range = self.resolve_selection_range(text, selection)?;
        let range = match transform {
            SyntaxSelectionTransform::Expand | SyntaxSelectionTransform::Parent => {
                self.strict_parent_selection_range(text, registry, range)?
            }
            SyntaxSelectionTransform::FirstNamedChild => {
                self.first_named_child_selection_range(text, registry, range)?
            }
            SyntaxSelectionTransform::PreviousNamedSibling => {
                self.named_sibling_selection_range(text, registry, range, false)?
            }
            SyntaxSelectionTransform::NextNamedSibling => {
                self.named_sibling_selection_range(text, registry, range, true)?
            }
        };
        Ok(range.map(|range| SyntaxSelectionRange {
            range,
            revision: self.revision,
            document: self.document,
        }))
    }

    /// Applies a structural transform against the current revision and
    /// returns only the presentation-neutral character range.
    pub fn transform_range(
        &self,
        text: &Text,
        registry: &Registry,
        range: SyntaxRange,
        transform: SyntaxSelectionTransform,
    ) -> Result<Option<SyntaxRange>, SyntaxError> {
        let selection = self.selection_range(text, range)?;
        Ok(self
            .transform_selection_range(text, registry, selection, transform)?
            .map(|selection| selection.range))
    }

    pub fn expand_range(
        &self,
        text: &Text,
        registry: &Registry,
        range: SyntaxRange,
    ) -> Result<Option<SyntaxRange>, SyntaxError> {
        self.transform_range(text, registry, range, SyntaxSelectionTransform::Expand)
    }

    pub fn parent_range(
        &self,
        text: &Text,
        registry: &Registry,
        range: SyntaxRange,
    ) -> Result<Option<SyntaxRange>, SyntaxError> {
        self.transform_range(text, registry, range, SyntaxSelectionTransform::Parent)
    }

    pub fn first_named_child_range(
        &self,
        text: &Text,
        registry: &Registry,
        range: SyntaxRange,
    ) -> Result<Option<SyntaxRange>, SyntaxError> {
        self.transform_range(
            text,
            registry,
            range,
            SyntaxSelectionTransform::FirstNamedChild,
        )
    }

    pub fn previous_named_sibling_range(
        &self,
        text: &Text,
        registry: &Registry,
        range: SyntaxRange,
    ) -> Result<Option<SyntaxRange>, SyntaxError> {
        self.transform_range(
            text,
            registry,
            range,
            SyntaxSelectionTransform::PreviousNamedSibling,
        )
    }

    pub fn next_named_sibling_range(
        &self,
        text: &Text,
        registry: &Registry,
        range: SyntaxRange,
    ) -> Result<Option<SyntaxRange>, SyntaxError> {
        self.transform_range(
            text,
            registry,
            range,
            SyntaxSelectionTransform::NextNamedSibling,
        )
    }

    /// Runs one structural text-object capability across all parser layers.
    ///
    /// Matches are bounded by tree-house's fixed 256 in-progress-match limit.
    /// That limit constrains pathological query ambiguity, not the number of
    /// sequential results: ordinary documents can return more than 256
    /// captures. Query compilation is isolated per language, so a broken Rust
    /// text-object query does not affect highlighting or another language's
    /// structural captures.
    pub fn text_object_captures(
        &self,
        text: &Text,
        registry: &Registry,
        object: SyntaxObject,
        part: SyntaxObjectPart,
        search: SyntaxRange,
    ) -> Result<Vec<SyntaxCapture>, SyntaxError> {
        let checked = checked_range_to_bytes(text, search)?;
        let byte_range = if search.is_empty() {
            character_probe_bytes(text, search.from)?
        } else {
            checked
        };
        let capture_name = object.capture_name(part);
        let unsupported_capture_name = object.unsupported_capture_name();
        let saw_supported = Cell::new(false);
        let failed_language = Cell::new(None);
        let unsupported_language = Cell::new(None);
        let loader = |language: Language| {
            let public = registry.public_language(language)?;
            match registry.text_object_capability(language, object)? {
                TextObjectCapability::Ready(query) => {
                    if query.get_capture(capture_name).is_some() {
                        saw_supported.set(true);
                        Some(query)
                    } else {
                        None
                    }
                }
                TextObjectCapability::Failed(_) => {
                    failed_language.set(Some(public));
                    None
                }
            }
        };
        let mut iter =
            QueryMatchIter::<_, ()>::new(&self.syntax, rope_slice(text), loader, byte_range);
        let mut captures = Vec::new();

        while let Some(event) = iter.next() {
            let QueryMatchIterEvent::Match(captured_match) = event else {
                continue;
            };
            let language = iter.current_language();
            let Some(public) = registry.public_language(language) else {
                return Err(SyntaxError::UnknownLanguage);
            };
            let Some(TextObjectCapability::Ready(query)) =
                registry.text_object_capability(language, object)
            else {
                continue;
            };
            let Some(capture) = query.get_capture(capture_name) else {
                continue;
            };
            if let Some(unsupported) = query.get_capture(unsupported_capture_name)
                && captured_match
                    .nodes_for_capture(unsupported)
                    .next()
                    .is_some()
            {
                unsupported_language.set(Some(public));
            }
            let ranges = captured_match
                .nodes_for_capture(capture)
                .map(|node| {
                    let from = checked_byte_to_char(text, node.start_byte() as usize)?;
                    let to = checked_byte_to_char(text, node.end_byte() as usize)?;
                    SyntaxRange::new(from, to)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if !ranges.is_empty() {
                captures.push(SyntaxCapture {
                    object,
                    part,
                    language: public,
                    revision: self.revision,
                    ranges,
                });
            }
        }
        drop(iter);

        if !captures.is_empty() {
            return Ok(captures);
        }
        if let Some(language) = unsupported_language.get() {
            return Err(SyntaxError::UnsupportedTextObject {
                language,
                object,
                part,
            });
        }
        if saw_supported.get() {
            return Ok(captures);
        }
        if let Some(language) = failed_language.get()
            && let Some(TextObjectCapability::Failed(message)) = registry
                .text_objects
                .get(language.0 as usize)
                .and_then(|capabilities| capabilities.get(&object))
                .map(LazyTextObjectCapability::get)
        {
            return Err(SyntaxError::TextObjectQueryFailed {
                language,
                message: message.clone(),
            });
        }
        Err(SyntaxError::UnsupportedTextObject {
            language: self.language,
            object,
            part,
        })
    }

    /// Resolves the minimal indentation captures applying at one newline.
    ///
    /// The deepest parser layer covering the newline with a usable query wins;
    /// an unsupported injected language therefore degrades to the nearest
    /// supported outer language rather than disabling indentation entirely.
    pub fn newline_indent(
        &self,
        text: &Text,
        registry: &Registry,
        newline: Offset,
    ) -> Result<NewlineIndent, SyntaxError> {
        let len_bytes = text.rope().len_bytes();
        if len_bytes > OUTLINE_SOURCE_LIMIT_BYTES {
            return Err(SyntaxError::DocumentTooLarge { len_bytes });
        }
        if text.char_at(newline) != Some('\n') {
            return Err(SyntaxError::InvalidNewline { offset: newline });
        }
        let newline_range = SyntaxRange::new(newline, newline + 1)?;
        let byte_range = checked_range_to_bytes(text, newline_range)?;
        let newline_byte = byte_range.start;
        let newline_row = text.position_of(newline).row;
        let mut issues = Vec::new();
        let mut truncated = false;
        if len_bytes > INJECTION_LIMIT_BYTES && registry.language_has_injections(self.language) {
            push_indent_issue(
                &mut issues,
                IndentIssue::InjectionsDisabled {
                    language: self.language,
                },
                &mut truncated,
            );
        }

        let layers = self.layers_covering(byte_range.clone());
        let mut root_failure = None;
        for (depth, layer) in layers.into_iter().enumerate().rev() {
            let parser_language = self.syntax.layer(layer).language;
            let language = registry
                .public_language(parser_language)
                .ok_or(SyntaxError::UnknownLanguage)?;
            let capability = registry.indentation_capability(parser_language);
            let query = match capability {
                Some(IndentationCapability::Ready(query)) => query,
                Some(IndentationCapability::Failed(message)) => {
                    if root_failure.is_none() {
                        root_failure = Some((language, message.clone()));
                    }
                    if depth > 0 {
                        push_indent_issue(
                            &mut issues,
                            IndentIssue::InjectedQueryFailed {
                                language,
                                injection_depth: depth as u32,
                                message: bounded_outline_issue_message(message),
                            },
                            &mut truncated,
                        );
                    }
                    continue;
                }
                None => {
                    if depth > 0 {
                        push_indent_issue(
                            &mut issues,
                            IndentIssue::UnsupportedInjectedLanguage {
                                language,
                                injection_depth: depth as u32,
                            },
                            &mut truncated,
                        );
                    }
                    continue;
                }
            };
            let Some(tree) = self.syntax.layer(layer).tree() else {
                let message = registry
                    .parser_error(parser_language)
                    .unwrap_or("parser did not produce a syntax tree");
                if depth > 0 {
                    push_indent_issue(
                        &mut issues,
                        IndentIssue::InjectedParserUnavailable {
                            language,
                            injection_depth: depth as u32,
                            message: bounded_outline_issue_message(message),
                        },
                        &mut truncated,
                    );
                }
                continue;
            };
            if tree_has_parse_error(tree.root_node()) {
                push_indent_issue(
                    &mut issues,
                    IndentIssue::IncompleteParse {
                        language,
                        injection_depth: depth as u32,
                    },
                    &mut truncated,
                );
            }

            let begin = query.get_capture("indent.begin");
            let always = query.get_capture("indent.always");
            let scan_end = u32::try_from(len_bytes)
                .map_err(|_| SyntaxError::DocumentTooLarge { len_bytes })?;
            let loader = |candidate: Language| (candidate == parser_language).then_some(query);
            let mut iter =
                QueryMatchIter::<_, ()>::new(&self.syntax, rope_slice(text), loader, 0..scan_end);
            let mut seen = std::collections::HashSet::new();
            let mut begin_levels = 0usize;
            let mut always_levels = 0usize;
            let mut current_depth = 0usize;
            while let Some(event) = iter.next() {
                let captured_match = match event {
                    QueryMatchIterEvent::EnterInjection(_) => {
                        current_depth = current_depth.saturating_add(1);
                        continue;
                    }
                    QueryMatchIterEvent::ExitInjection { .. } => {
                        current_depth = current_depth.saturating_sub(1);
                        continue;
                    }
                    QueryMatchIterEvent::Match(captured_match)
                        if current_depth == depth && iter.current_language() == parser_language =>
                    {
                        captured_match
                    }
                    QueryMatchIterEvent::Match(_) => continue,
                };
                if let Some(capture) = begin {
                    for node in captured_match.nodes_for_capture(capture) {
                        let key = (0u8, node.start_byte(), node.end_byte());
                        let starts_on_newline_row =
                            checked_byte_to_char(text, node.start_byte() as usize)
                                .map(|offset| text.position_of(offset).row == newline_row)?;
                        if node.start_byte() <= newline_byte
                            && node.end_byte() > newline_byte
                            && starts_on_newline_row
                            && seen.insert(key)
                        {
                            begin_levels += 1;
                        }
                    }
                }
                if let Some(capture) = always {
                    for node in captured_match.nodes_for_capture(capture) {
                        let key = (1u8, node.start_byte(), node.end_byte());
                        if node.start_byte() <= newline_byte
                            && node.end_byte() > newline_byte
                            && seen.insert(key)
                        {
                            always_levels += 1;
                        }
                    }
                }
            }
            drop(iter);
            if begin_levels > INDENT_LEVEL_LIMIT || always_levels > INDENT_LEVEL_LIMIT {
                truncated = true;
            }
            return Ok(NewlineIndent {
                newline: newline_range,
                revision: self.revision,
                language,
                injection_depth: depth as u32,
                begin_levels: begin_levels.min(INDENT_LEVEL_LIMIT) as u16,
                always_levels: always_levels.min(INDENT_LEVEL_LIMIT) as u16,
                issues,
                truncated,
                document: self.document,
            });
        }

        if let Some((language, message)) = root_failure {
            return Err(SyntaxError::IndentationQueryFailed { language, message });
        }
        Err(SyntaxError::UnsupportedIndentation {
            language: self.language,
        })
    }

    /// Validates that an indentation answer still belongs to this text tree.
    pub fn resolve_newline_indent(
        &self,
        text: &Text,
        indent: &NewlineIndent,
    ) -> Result<SyntaxRange, SyntaxError> {
        if indent.document != self.document {
            return Err(SyntaxError::ForeignDocument);
        }
        if indent.revision != self.revision {
            return Err(SyntaxError::StaleRevision {
                expected: indent.revision,
                actual: self.revision,
            });
        }
        let range = indent.newline.checked(text)?;
        if range.to != range.from + 1 || text.char_at(range.from) != Some('\n') {
            return Err(SyntaxError::InvalidNewline { offset: range.from });
        }
        Ok(range)
    }

    /// Builds conservative, source-ordered folds across all parser layers.
    pub fn folds(&self, text: &Text, registry: &Registry) -> Result<FoldList, SyntaxError> {
        let len_bytes = text.rope().len_bytes();
        if len_bytes > OUTLINE_SOURCE_LIMIT_BYTES {
            return Err(SyntaxError::DocumentTooLarge { len_bytes });
        }
        let scan_end =
            u32::try_from(len_bytes).map_err(|_| SyntaxError::DocumentTooLarge { len_bytes })?;
        let root_language = self.syntax.layer(self.syntax.root()).language;
        match registry.fold_capability(root_language) {
            Some(FoldCapability::Failed(message)) => {
                return Err(SyntaxError::FoldQueryFailed {
                    language: self.language,
                    message: message.clone(),
                });
            }
            None => {
                return Err(SyntaxError::UnsupportedFolds {
                    language: self.language,
                });
            }
            Some(FoldCapability::Ready(_)) => {}
        }

        let loader = |language: Language| match registry.fold_capability(language)? {
            FoldCapability::Ready(query) => Some(query),
            FoldCapability::Failed(_) => None,
        };
        let mut iter =
            QueryMatchIter::<_, ()>::new(&self.syntax, rope_slice(text), loader, 0..scan_end);
        let mut candidates = Vec::new();
        let mut issues = Vec::new();
        let mut truncated = false;
        if len_bytes > INJECTION_LIMIT_BYTES && registry.language_has_injections(self.language) {
            push_fold_issue(
                &mut issues,
                FoldIssue::InjectionsDisabled {
                    language: self.language,
                },
                &mut truncated,
            );
        }
        let mut injection_depth = 0u32;
        while let Some(event) = iter.next() {
            let captured_match = match event {
                QueryMatchIterEvent::EnterInjection(injection) => {
                    injection_depth = injection_depth.saturating_add(1);
                    let parser_language = self.syntax.layer(injection.layer).language;
                    let language = registry
                        .public_language(parser_language)
                        .ok_or(SyntaxError::UnknownLanguage)?;
                    let issue = if self.syntax.layer(injection.layer).tree().is_none() {
                        let message = registry
                            .parser_error(parser_language)
                            .unwrap_or("parser did not produce a syntax tree");
                        Some(FoldIssue::InjectedParserUnavailable {
                            language,
                            injection_depth,
                            message: bounded_outline_issue_message(message),
                        })
                    } else {
                        match registry.fold_capability(parser_language) {
                            None => Some(FoldIssue::UnsupportedInjectedLanguage {
                                language,
                                injection_depth,
                            }),
                            Some(FoldCapability::Failed(message)) => {
                                Some(FoldIssue::InjectedQueryFailed {
                                    language,
                                    injection_depth,
                                    message: bounded_outline_issue_message(message),
                                })
                            }
                            Some(FoldCapability::Ready(_)) => self
                                .syntax
                                .layer(injection.layer)
                                .tree()
                                .filter(|tree| tree_has_parse_error(tree.root_node()))
                                .map(|_| {
                                    let from =
                                        checked_byte_to_char(text, injection.range.start as usize)?;
                                    let to =
                                        checked_byte_to_char(text, injection.range.end as usize)?;
                                    Ok(FoldIssue::IncompleteInjectedParse {
                                        language,
                                        injection_depth,
                                        range: SyntaxRange::new(from, to)?,
                                    })
                                })
                                .transpose()?,
                        }
                    };
                    if let Some(issue) = issue {
                        push_fold_issue(&mut issues, issue, &mut truncated);
                    }
                    continue;
                }
                QueryMatchIterEvent::ExitInjection { .. } => {
                    injection_depth = injection_depth.saturating_sub(1);
                    continue;
                }
                QueryMatchIterEvent::Match(captured_match) => captured_match,
            };
            let parser_language = iter.current_language();
            let language = registry
                .public_language(parser_language)
                .ok_or(SyntaxError::UnknownLanguage)?;
            let Some(FoldCapability::Ready(query)) = registry.fold_capability(parser_language)
            else {
                continue;
            };
            let Some(capture) = query.get_capture("fold") else {
                continue;
            };
            for node in captured_match.nodes_for_capture(capture) {
                if let Some(range) = conservative_fold_range(text, node)? {
                    candidates.push((range, language, injection_depth));
                    if candidates.len() >= FOLD_ITEM_LIMIT * 2 {
                        truncated = true;
                        break;
                    }
                }
            }
            if candidates.len() >= FOLD_ITEM_LIMIT * 2 {
                break;
            }
        }
        drop(iter);

        candidates.sort_by(|left, right| {
            left.0
                .from
                .cmp(&right.0.from)
                .then_with(|| right.0.to.cmp(&left.0.to))
                .then_with(|| left.2.cmp(&right.2))
                .then_with(|| left.1.cmp(&right.1))
        });
        candidates.dedup();
        if candidates.len() > FOLD_ITEM_LIMIT {
            candidates.truncate(FOLD_ITEM_LIMIT);
            truncated = true;
        }
        let items = candidates
            .into_iter()
            .map(|(range, language, injection_depth)| FoldItem {
                range: SyntaxFoldRange {
                    range,
                    revision: self.revision,
                    document: self.document,
                },
                language,
                injection_depth,
            })
            .collect();
        Ok(FoldList {
            revision: self.revision,
            items,
            issues,
            truncated,
        })
    }

    /// Validates a saved fold range after possible document changes.
    pub fn resolve_fold_range(
        &self,
        text: &Text,
        fold: SyntaxFoldRange,
    ) -> Result<SyntaxRange, SyntaxError> {
        if fold.document != self.document {
            return Err(SyntaxError::ForeignDocument);
        }
        if fold.revision != self.revision {
            return Err(SyntaxError::StaleRevision {
                expected: fold.revision,
                actual: self.revision,
            });
        }
        fold.range.checked(text)
    }

    /// Builds a bounded, source-ordered document outline across every parser
    /// layer. Query compilation is independent from highlighting and text
    /// objects, so requesting an outline initializes only the languages
    /// actually present in this document.
    pub fn outline(&self, text: &Text, registry: &Registry) -> Result<Outline, SyntaxError> {
        #[derive(Clone)]
        struct Candidate {
            name: Box<str>,
            kind: OutlineKind,
            range: SyntaxRange,
            target: SyntaxRange,
            language: LanguageId,
            injection_depth: u32,
        }

        const RAW_MATCH_LIMIT: usize = OUTLINE_ITEM_LIMIT * 2;

        let len_bytes = text.rope().len_bytes();
        let scan_end = len_bytes.min(OUTLINE_SOURCE_LIMIT_BYTES);
        let scan_end =
            u32::try_from(scan_end).map_err(|_| SyntaxError::DocumentTooLarge { len_bytes })?;
        let root_language = self.syntax.layer(self.syntax.root()).language;
        if let Some(OutlineCapability::Failed(message)) = registry.outline_capability(root_language)
        {
            return Err(SyntaxError::OutlineQueryFailed {
                language: self.language,
                message: message.clone(),
            });
        }
        let saw_supported = Cell::new(false);
        let failed_language = Cell::new(None);
        let loader = |language: Language| match registry.outline_capability(language)? {
            OutlineCapability::Ready(query) => {
                saw_supported.set(true);
                Some(query)
            }
            OutlineCapability::Failed(_) => {
                failed_language.set(registry.public_language(language));
                None
            }
        };
        let mut iter =
            QueryMatchIter::<_, ()>::new(&self.syntax, rope_slice(text), loader, 0..scan_end);
        let mut candidates = Vec::new();
        let mut truncated = len_bytes > OUTLINE_SOURCE_LIMIT_BYTES;
        let mut issues = Vec::new();
        if len_bytes > INJECTION_LIMIT_BYTES && registry.language_has_injections(self.language) {
            push_outline_issue(
                &mut issues,
                OutlineIssue::InjectionsDisabled {
                    language: self.language,
                },
                &mut truncated,
            );
        }
        let mut label_bytes = 0usize;
        let mut injection_depth = 0u32;
        let mut projection_limit_reached = false;

        while let Some(event) = iter.next() {
            let captured_match = match event {
                QueryMatchIterEvent::EnterInjection(injection) => {
                    injection_depth = injection_depth.saturating_add(1);
                    let parser_language = self.syntax.layer(injection.layer).language;
                    let Some(language) = registry.public_language(parser_language) else {
                        return Err(SyntaxError::UnknownLanguage);
                    };
                    let issue = if self.syntax.layer(injection.layer).tree().is_none() {
                        let message = registry
                            .parser_error(parser_language)
                            .unwrap_or("parser did not produce a syntax tree");
                        Some(OutlineIssue::InjectedParserUnavailable {
                            language,
                            injection_depth,
                            message: bounded_outline_issue_message(message),
                        })
                    } else {
                        match registry.outline_capability(parser_language) {
                            None => Some(OutlineIssue::UnsupportedInjectedLanguage {
                                language,
                                injection_depth,
                            }),
                            Some(OutlineCapability::Failed(message)) => {
                                Some(OutlineIssue::InjectedQueryFailed {
                                    language,
                                    injection_depth,
                                    message: bounded_outline_issue_message(message),
                                })
                            }
                            Some(OutlineCapability::Ready(_)) => self
                                .syntax
                                .layer(injection.layer)
                                .tree()
                                .filter(|tree| tree_has_parse_error(tree.root_node()))
                                .map(|_| {
                                    let from =
                                        checked_byte_to_char(text, injection.range.start as usize)?;
                                    let to =
                                        checked_byte_to_char(text, injection.range.end as usize)?;
                                    Ok(OutlineIssue::IncompleteInjectedParse {
                                        language,
                                        injection_depth,
                                        range: SyntaxRange::new(from, to)?,
                                    })
                                })
                                .transpose()?,
                        }
                    };
                    if let Some(issue) = issue {
                        push_outline_issue(&mut issues, issue, &mut truncated);
                    }
                    continue;
                }
                QueryMatchIterEvent::ExitInjection { .. } => {
                    injection_depth = injection_depth.saturating_sub(1);
                    continue;
                }
                QueryMatchIterEvent::Match(captured_match) => captured_match,
            };
            let language = iter.current_language();
            let Some(public) = registry.public_language(language) else {
                return Err(SyntaxError::UnknownLanguage);
            };
            let Some(OutlineCapability::Ready(query)) = registry.outline_capability(language)
            else {
                continue;
            };
            let Some(name_capture) = query.get_capture("outline.name") else {
                continue;
            };
            let name_nodes = captured_match
                .nodes_for_capture(name_capture)
                .collect::<Vec<_>>();
            if name_nodes.is_empty() {
                continue;
            }
            let Some((kind, item_node)) = OUTLINE_KINDS.iter().find_map(|kind| {
                let capture = query.get_capture(kind.capture_name())?;
                captured_match
                    .nodes_for_capture(capture)
                    .next()
                    .map(|node| (*kind, node))
            }) else {
                continue;
            };

            let from = checked_byte_to_char(text, item_node.start_byte() as usize)?;
            let to = checked_byte_to_char(text, item_node.end_byte() as usize)?;
            let range = SyntaxRange::new(from, to)?;
            for name_node in name_nodes {
                let target_from = checked_byte_to_char(text, name_node.start_byte() as usize)?;
                let target_to = checked_byte_to_char(text, name_node.end_byte() as usize)?;
                let target = SyntaxRange::new(target_from, target_to)?;
                let (name, name_truncated) = bounded_outline_name(text, target);
                truncated |= name_truncated;
                if name.is_empty() {
                    continue;
                }
                if label_bytes.saturating_add(name.len()) > OUTLINE_LABEL_BUDGET_BYTES {
                    truncated = true;
                    projection_limit_reached = true;
                    break;
                }
                label_bytes += name.len();
                candidates.push(Candidate {
                    name,
                    kind,
                    range,
                    target,
                    language: public,
                    injection_depth,
                });
                if candidates.len() == RAW_MATCH_LIMIT {
                    truncated = true;
                    projection_limit_reached = true;
                    break;
                }
            }
            if projection_limit_reached {
                break;
            }
        }
        drop(iter);

        if candidates.is_empty() && !saw_supported.get() {
            if let Some(language) = failed_language.get()
                && let Some(OutlineCapability::Failed(message)) = registry
                    .outlines
                    .get(language.0 as usize)
                    .and_then(Option::as_ref)
                    .map(LazyOutlineCapability::get)
            {
                return Err(SyntaxError::OutlineQueryFailed {
                    language,
                    message: message.clone(),
                });
            }
            return Err(SyntaxError::UnsupportedOutline {
                language: self.language,
            });
        }

        candidates.sort_by(|left, right| {
            left.range
                .from
                .cmp(&right.range.from)
                .then_with(|| right.range.to.cmp(&left.range.to))
                .then_with(|| left.target.from.cmp(&right.target.from))
                .then_with(|| left.kind.cmp(&right.kind))
        });

        // Composed queries intentionally allow a base language and its
        // derived grammar to recognize the same declaration. Keep one entry,
        // preferring the more specific method classification.
        let mut seen = HashMap::<(SyntaxRange, LanguageId, u32), usize>::new();
        let mut deduplicated: Vec<Candidate> = Vec::new();
        for candidate in candidates {
            let key = (
                candidate.target,
                candidate.language,
                candidate.injection_depth,
            );
            if let Some(&index) = seen.get(&key) {
                let existing = &deduplicated[index];
                if strictly_contains(candidate.range, existing.range)
                    || (candidate.range == existing.range
                        && candidate.kind.specificity() > existing.kind.specificity())
                {
                    deduplicated[index] = candidate;
                }
                continue;
            }
            seen.insert(key, deduplicated.len());
            deduplicated.push(candidate);
        }
        deduplicated.sort_by(|left, right| {
            left.range
                .from
                .cmp(&right.range.from)
                .then_with(|| right.range.to.cmp(&left.range.to))
                .then_with(|| left.injection_depth.cmp(&right.injection_depth))
                .then_with(|| left.target.from.cmp(&right.target.from))
        });
        if deduplicated.len() > OUTLINE_ITEM_LIMIT {
            deduplicated.truncate(OUTLINE_ITEM_LIMIT);
            truncated = true;
        }

        let mut items: Vec<OutlineItem> = Vec::with_capacity(deduplicated.len());
        let mut ancestors: Vec<usize> = Vec::new();
        for candidate in deduplicated {
            while ancestors
                .last()
                .is_some_and(|&index| !strictly_contains(items[index].range, candidate.range))
            {
                ancestors.pop();
            }
            if ancestors.len() >= OUTLINE_DEPTH_LIMIT {
                truncated = true;
                continue;
            }
            let parent = ancestors.last().copied();
            let kind = if candidate.kind == OutlineKind::Function
                && parent.is_some_and(|index| {
                    matches!(
                        items[index].kind,
                        OutlineKind::Class
                            | OutlineKind::Struct
                            | OutlineKind::Enum
                            | OutlineKind::Actor
                            | OutlineKind::Extension
                            | OutlineKind::Interface
                    )
                }) {
                OutlineKind::Method
            } else {
                candidate.kind
            };
            let index = items.len();
            items.push(OutlineItem {
                name: candidate.name,
                kind,
                range: candidate.range,
                target: SyntaxSelectionRange {
                    range: candidate.target,
                    revision: self.revision,
                    document: self.document,
                },
                language: candidate.language,
                injection_depth: candidate.injection_depth,
                parent,
            });
            ancestors.push(index);
        }

        Ok(Outline {
            revision: self.revision,
            items,
            issues,
            truncated,
        })
    }

    /// Highlight spans covering the character range `[from, to)`.
    ///
    /// Spans are returned in order and never overlap: where tree-sitter nests
    /// captures, the innermost active highlight wins, which is what produces
    /// the expected colour for things like a string inside an attribute.
    pub fn spans(&self, text: &Text, registry: &Registry, from: Offset, to: Offset) -> Vec<Span> {
        let source = rope_slice(text);
        let Ok(byte_range) = SyntaxRange::new(from.min(text.len_chars()), to.min(text.len_chars()))
            .and_then(|range| checked_range_to_bytes(text, range))
        else {
            return Vec::new();
        };
        let start_byte = byte_range.start;
        let end_byte = byte_range.end;

        let mut highlighter: Highlighter<'_, '_, Registry> =
            Highlighter::new(&self.syntax, source, registry, start_byte..end_byte);
        let mut spans = Vec::new();
        let mut position = start_byte;

        while position < end_byte {
            // The innermost active highlight wins, which is what produces the
            // expected colour for a nested capture such as a string inside an
            // attribute.
            let scope = highlighter
                .active_highlights()
                .next_back()
                .map(|highlight| Scope(highlight.get()));
            let next = highlighter.next_event_offset();
            if next == u32::MAX || next > end_byte {
                push_span(&mut spans, text, scope, position, end_byte);
                break;
            }
            let next = next.max(start_byte);
            if next > position {
                push_span(&mut spans, text, scope, position, next);
                position = next;
            }
            highlighter.advance();
        }
        spans
    }
}

impl DocumentSyntax {
    fn strict_parent_selection_range(
        &self,
        text: &Text,
        registry: &Registry,
        range: SyntaxRange,
    ) -> Result<Option<SyntaxRange>, SyntaxError> {
        let Some(node) = self.node_covering(text, registry, range)? else {
            return Ok(None);
        };
        Ok(self
            .ancestors(text, registry, &node.path)?
            .into_iter()
            .rev()
            .map(|ancestor| ancestor.range)
            .find(|candidate| strictly_contains(*candidate, range)))
    }

    fn first_named_child_selection_range(
        &self,
        text: &Text,
        registry: &Registry,
        range: SyntaxRange,
    ) -> Result<Option<SyntaxRange>, SyntaxError> {
        let Some(mut node) = self.node_covering(text, registry, range)? else {
            return Ok(None);
        };
        loop {
            let Some(child) = self.first_named_child(text, registry, &node.path)? else {
                return Ok(None);
            };
            if child.range != range {
                return Ok(Some(child.range));
            }
            node = child;
        }
    }

    fn named_sibling_selection_range(
        &self,
        text: &Text,
        registry: &Registry,
        range: SyntaxRange,
        next: bool,
    ) -> Result<Option<SyntaxRange>, SyntaxError> {
        let Some(mut node) = self.node_covering(text, registry, range)? else {
            return Ok(None);
        };
        loop {
            let mut sibling = if next {
                self.next_named_sibling(text, registry, &node.path)?
            } else {
                self.previous_named_sibling(text, registry, &node.path)?
            };
            while let Some(candidate) = sibling {
                if candidate.range != range {
                    return Ok(Some(candidate.range));
                }
                sibling = if next {
                    self.next_named_sibling(text, registry, &candidate.path)?
                } else {
                    self.previous_named_sibling(text, registry, &candidate.path)?
                };
            }

            let Some(parent) = self.parent(text, registry, &node.path)? else {
                return Ok(None);
            };
            if parent.range != node.range {
                return Ok(None);
            }
            node = parent;
        }
    }
}

fn strictly_contains(candidate: SyntaxRange, range: SyntaxRange) -> bool {
    candidate != range && candidate.from <= range.from && candidate.to >= range.to
}

fn bounded_outline_name(text: &Text, range: SyntaxRange) -> (Box<str>, bool) {
    let available = range.to.saturating_sub(range.from);
    let take = available.min(OUTLINE_NAME_LIMIT_CHARS);
    let raw = text.slice_string(range.from, range.from + take);
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    (normalized.into_boxed_str(), available > take)
}

fn bounded_outline_issue_message(message: &str) -> Box<str> {
    message
        .chars()
        .take(OUTLINE_ISSUE_MESSAGE_LIMIT_CHARS)
        .collect::<String>()
        .into_boxed_str()
}

fn push_outline_issue(issues: &mut Vec<OutlineIssue>, issue: OutlineIssue, truncated: &mut bool) {
    if issues.contains(&issue) {
        return;
    }
    if issues.len() < OUTLINE_ISSUE_LIMIT {
        issues.push(issue);
    } else {
        *truncated = true;
    }
}

fn push_indent_issue(issues: &mut Vec<IndentIssue>, issue: IndentIssue, truncated: &mut bool) {
    if issues.contains(&issue) {
        return;
    }
    if issues.len() < SYNTAX_CAPABILITY_ISSUE_LIMIT {
        issues.push(issue);
    } else {
        *truncated = true;
    }
}

fn push_fold_issue(issues: &mut Vec<FoldIssue>, issue: FoldIssue, truncated: &mut bool) {
    if issues.contains(&issue) {
        return;
    }
    if issues.len() < SYNTAX_CAPABILITY_ISSUE_LIMIT {
        issues.push(issue);
    } else {
        *truncated = true;
    }
}

fn conservative_fold_range(
    text: &Text,
    node: &Node<'_>,
) -> Result<Option<SyntaxRange>, SyntaxError> {
    let node_from = checked_byte_to_char(text, node.start_byte() as usize)?;
    let node_to = checked_byte_to_char(text, node.end_byte() as usize)?;
    let start = text.position_of(node_from);
    let end = text.position_of(node_to);
    let final_row = if end.col == 0 && end.row > start.row {
        end.row - 1
    } else {
        end.row
    };
    if final_row <= start.row {
        return Ok(None);
    }

    // A structural closing line remains useful context, but an ordinary final
    // body line belongs inside the fold. Only consume that final row when the
    // node ends at a row boundary; otherwise syntax following the node on the
    // same row must remain visible.
    let from = text.line_to_offset(start.row) + text.line_len(start.row);
    let ends_at_row_boundary = end.col == 0 || end.col == text.line_len(end.row);
    let to = if ends_at_row_boundary && !has_trailing_closing_delimiter(text, node, final_row)? {
        node_to
    } else {
        text.line_to_offset(final_row)
    };
    if from >= to {
        return Ok(None);
    }
    Ok(Some(SyntaxRange::new(from, to)?))
}

fn has_trailing_closing_delimiter(
    text: &Text,
    node: &Node<'_>,
    final_row: usize,
) -> Result<bool, SyntaxError> {
    let Some(mut tail) = node.child(node.child_count().saturating_sub(1)) else {
        return Ok(false);
    };
    // HTML represents `</tag>` as a named direct child rather than exposing
    // its final `>` directly on the element. Do not otherwise descend into a
    // final body statement: a Python call ending in `)` is content, not the
    // function's own closing delimiter.
    if tail.kind() == "end_tag"
        && let Some(child) = tail.child(tail.child_count().saturating_sub(1))
    {
        tail = child;
    }
    let from = checked_byte_to_char(text, tail.start_byte() as usize)?;
    let to = checked_byte_to_char(text, tail.end_byte() as usize)?;
    if text.position_of(from).row != final_row {
        return Ok(false);
    }
    let token = text.slice_string(from, to);
    Ok(matches!(
        token.as_str(),
        "}" | "]" | ")" | ">" | "/>" | "fi" | "done" | "esac"
    ) || token.len() >= 3
        && token
            .chars()
            .all(|character| character == '`' || character == '~'))
}

fn tree_has_parse_error(root: Node<'_>) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "ERROR" || node.is_missing() {
            return true;
        }
        stack.extend((0..node.child_count()).filter_map(|index| node.child(index)));
    }
    false
}

struct ResolvedPath<'tree> {
    layer_depth: usize,
    layer: Layer,
    node: Node<'tree>,
}

impl DocumentSyntax {
    fn layers_covering(&self, bytes: std::ops::Range<u32>) -> Vec<Layer> {
        let inclusive_end = if bytes.is_empty() {
            bytes.end
        } else {
            bytes.end - 1
        };
        self.syntax
            .layers_for_byte_range(bytes.start, inclusive_end)
            .collect()
    }

    fn deepest_named_node(&self, bytes: std::ops::Range<u32>) -> Option<(usize, Layer, Node<'_>)> {
        let layers = self.layers_covering(bytes.clone());
        layers
            .into_iter()
            .enumerate()
            .rev()
            .find_map(|(depth, layer)| {
                let tree = self.syntax.layer(layer).tree()?;
                let root = tree.root_node();
                let node = root
                    .named_descendant_for_byte_range(bytes.start, bytes.end)
                    .or_else(|| node_covers(&root, &bytes).then_some(root))?;
                Some((depth, layer, node))
            })
    }

    fn summary(
        &self,
        text: &Text,
        registry: &Registry,
        layer_depth: usize,
        layer: Layer,
        node: Node<'_>,
        preferred_probe: Offset,
    ) -> Result<SyntaxNodeSummary, SyntaxError> {
        let from = checked_byte_to_char(text, node.start_byte() as usize)?;
        let to = checked_byte_to_char(text, node.end_byte() as usize)?;
        let range = SyntaxRange::new(from, to)?;
        let probe = node_probe(text, &node, preferred_probe)?;
        let language = registry
            .public_language(self.syntax.layer(layer).language)
            .ok_or(SyntaxError::UnknownLanguage)?;
        let path = SyntaxPath {
            document: self.document,
            revision: self.revision,
            probe,
            layer_depth: u32::try_from(layer_depth).map_err(|_| SyntaxError::InvalidPath)?,
            child_indices: child_indices(node.clone()).into_boxed_slice(),
        };
        Ok(SyntaxNodeSummary {
            path,
            range,
            kind: SyntaxKind::new(node.kind()),
            language,
            named: node.is_named(),
            missing: node.is_missing(),
            extra: node.is_extra(),
        })
    }

    fn resolve_path<'tree>(
        &'tree self,
        text: &Text,
        path: &SyntaxPath,
    ) -> Result<ResolvedPath<'tree>, SyntaxError> {
        if path.document != self.document {
            return Err(SyntaxError::InvalidPath);
        }
        if path.revision != self.revision {
            return Err(SyntaxError::StaleRevision {
                expected: path.revision,
                actual: self.revision,
            });
        }
        let bytes = character_probe_bytes(text, path.probe)?;
        let layers = self.layers_covering(bytes);
        let layer_depth = path.layer_depth as usize;
        let &layer = layers.get(layer_depth).ok_or(SyntaxError::InvalidPath)?;
        let tree = self
            .syntax
            .layer(layer)
            .tree()
            .ok_or(SyntaxError::InvalidPath)?;
        let mut node = tree.root_node();
        for &child_index in path.child_indices.iter() {
            node = node.child(child_index).ok_or(SyntaxError::InvalidPath)?;
        }
        Ok(ResolvedPath {
            layer_depth,
            layer,
            node,
        })
    }
}

fn node_covers(node: &Node<'_>, bytes: &std::ops::Range<u32>) -> bool {
    node.start_byte() <= bytes.start && node.end_byte() >= bytes.end
}

fn child_indices(mut node: Node<'_>) -> Vec<u32> {
    let mut path = Vec::new();
    while let Some(parent) = node.parent() {
        let index = (0..parent.child_count())
            .find(|&index| parent.child(index).as_ref() == Some(&node))
            .expect("a tree-sitter parent contains its child");
        path.push(index);
        node = parent;
    }
    path.reverse();
    path
}

fn named_ancestor_chain(mut node: Node<'_>) -> Vec<Node<'_>> {
    let mut ancestors = Vec::new();
    loop {
        if node.is_named() {
            ancestors.push(node.clone());
        }
        let Some(parent) = node.parent() else {
            break;
        };
        node = parent;
    }
    ancestors.reverse();
    ancestors
}

fn range_probe(text: &Text, range: SyntaxRange) -> Offset {
    if range.from < text.len_chars() {
        range.from
    } else {
        text.len_chars().saturating_sub(1)
    }
}

fn character_probe_bytes(text: &Text, offset: Offset) -> Result<std::ops::Range<u32>, SyntaxError> {
    checked_char_to_byte(text, offset)?;
    if text.len_chars() == 0 {
        return Ok(0..0);
    }
    let from = if offset == text.len_chars() {
        offset - 1
    } else {
        offset
    };
    checked_range_to_bytes(text, SyntaxRange { from, to: from + 1 })
}

fn node_probe(text: &Text, node: &Node<'_>, preferred: Offset) -> Result<Offset, SyntaxError> {
    let preferred_bytes = character_probe_bytes(text, preferred)?;
    if node_covers(node, &preferred_bytes) {
        return Ok(preferred);
    }
    checked_byte_to_char(text, node.start_byte() as usize)
}

fn push_span(
    spans: &mut Vec<Span>,
    text: &Text,
    scope: Option<Scope>,
    from_byte: u32,
    to_byte: u32,
) {
    if to_byte <= from_byte {
        return;
    }
    let Some(scope) = scope else {
        return;
    };
    let from = byte_to_char(text, from_byte);
    let to = byte_to_char(text, to_byte);
    if let Some(previous) = spans.last_mut()
        && previous.to == from
        && previous.scope == scope
    {
        previous.to = to;
        return;
    }
    spans.push(Span { from, to, scope });
}

fn rope_slice(text: &Text) -> RopeSlice<'_> {
    text.rope().slice(..)
}

/// Converts a Runyte character offset to tree-sitter's byte coordinates.
///
/// New structural APIs use this checked boundary. The small clamping wrappers
/// below remain for the existing highlight and bracket paths, whose callers
/// already rely on graceful degradation rather than coordinate errors.
fn checked_char_to_byte(text: &Text, offset: Offset) -> Result<u32, SyntaxError> {
    if offset > text.len_chars() {
        return Err(SyntaxError::CharacterOffsetOutOfBounds {
            offset,
            len_chars: text.len_chars(),
        });
    }
    let byte = text.rope().char_to_byte(offset);
    u32::try_from(byte).map_err(|_| SyntaxError::DocumentTooLarge { len_bytes: byte })
}

/// Converts a tree-sitter byte offset to a Runyte character offset.
///
/// Ropey maps a byte in the middle of a UTF-8 codepoint to the character that
/// contains it. Tree-sitter node endpoints should be codepoint boundaries, but
/// retaining Ropey's defined behavior makes injection probes safe as well.
fn checked_byte_to_char(text: &Text, offset: usize) -> Result<Offset, SyntaxError> {
    if offset > text.rope().len_bytes() {
        return Err(SyntaxError::ByteOffsetOutOfBounds {
            offset,
            len_bytes: text.rope().len_bytes(),
        });
    }
    Ok(text.rope().byte_to_char(offset))
}

fn checked_range_to_bytes(
    text: &Text,
    range: SyntaxRange,
) -> Result<std::ops::Range<u32>, SyntaxError> {
    let range = range.checked(text)?;
    Ok(checked_char_to_byte(text, range.from)?..checked_char_to_byte(text, range.to)?)
}

fn char_to_byte(text: &Text, offset: Offset) -> u32 {
    checked_char_to_byte(text, offset.min(text.len_chars()))
        .expect("a successfully parsed document fits in tree-sitter coordinates")
}

fn byte_to_char(text: &Text, byte: u32) -> Offset {
    let byte = (byte as usize).min(text.rope().len_bytes());
    checked_byte_to_char(text, byte)
        .expect("a tree-sitter byte coordinate is within its source document")
}

/// Converts a transaction into the byte-and-point edits tree-sitter needs.
///
/// Tree-sitter works in bytes while transactions work in characters, so every
/// position is converted against the pre-edit text.
///
/// Every edit stays in *original* coordinates with no cumulative shift.
/// `Syntax::update` applies the slice in reverse (`edits.iter().rev()`) so that
/// an earlier edit cannot disturb the position of a later one, which means each
/// edit must describe the document as it was before any of them were applied.
/// Shifting them here instead produces stale nodes on multi-range edits.
fn input_edits(before: &Text, transaction: &Transaction) -> Result<Vec<InputEdit>, SyntaxError> {
    transaction
        .changes()
        .iter()
        .map(|change| {
            let start_byte = checked_char_to_byte(before, change.from)?;
            let inserted_bytes =
                u32::try_from(change.text.len()).map_err(|_| SyntaxError::DocumentTooLarge {
                    len_bytes: change.text.len(),
                })?;
            let new_end_byte =
                start_byte
                    .checked_add(inserted_bytes)
                    .ok_or(SyntaxError::DocumentTooLarge {
                        len_bytes: start_byte as usize + change.text.len(),
                    })?;
            Ok(InputEdit {
                start_byte,
                old_end_byte: checked_char_to_byte(before, change.to)?,
                new_end_byte,
                start_point: point_of(before, change.from)?,
                old_end_point: point_of(before, change.to)?,
                new_end_point: new_point(before, change)?,
            })
        })
        .collect()
}

fn point_of(text: &Text, offset: Offset) -> Result<Point, SyntaxError> {
    checked_char_to_byte(text, offset)?;
    let position = text.position_of(offset);
    let row_start = text.line_to_offset(position.row);
    let column = checked_char_to_byte(text, offset)? - checked_char_to_byte(text, row_start)?;
    let row = u32::try_from(position.row).map_err(|_| SyntaxError::DocumentTooLarge {
        len_bytes: text.rope().len_bytes(),
    })?;
    Ok(Point { row, col: column })
}

/// Where the end of a change lands once its replacement text is in place.
fn new_point(before: &Text, change: &Change) -> Result<Point, SyntaxError> {
    let start = point_of(before, change.from)?;
    let newlines = u32::try_from(change.text.matches('\n').count()).map_err(|_| {
        SyntaxError::DocumentTooLarge {
            len_bytes: change.text.len(),
        }
    })?;
    if newlines == 0 {
        let inserted_bytes =
            u32::try_from(change.text.len()).map_err(|_| SyntaxError::DocumentTooLarge {
                len_bytes: change.text.len(),
            })?;
        return Ok(Point {
            row: start.row,
            col: start
                .col
                .checked_add(inserted_bytes)
                .ok_or(SyntaxError::DocumentTooLarge {
                    len_bytes: start.col as usize + change.text.len(),
                })?,
        });
    }
    let last_line = change.text.rsplit('\n').next().unwrap_or_default();
    let row = start
        .row
        .checked_add(newlines)
        .ok_or(SyntaxError::DocumentTooLarge {
            len_bytes: change.text.len(),
        })?;
    let col = u32::try_from(last_line.len()).map_err(|_| SyntaxError::DocumentTooLarge {
        len_bytes: change.text.len(),
    })?;
    Ok(Point { row, col })
}

/// Bracket pairs recognised by [`DocumentSyntax::matching_bracket`].
const BRACKETS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')];

impl DocumentSyntax {
    /// Smallest syntax node enclosing `range` whose edge characters are the
    /// requested delimiter pair. `None` accepts the closest known pair.
    pub fn enclosing_delimiter(
        &self,
        text: &Text,
        registry: &Registry,
        range: SyntaxRange,
        pair: Option<DelimiterPair>,
        part: SyntaxObjectPart,
    ) -> Result<Option<SyntaxRange>, SyntaxError> {
        let range = range.checked(text)?;
        let Some(node) = self.node_covering(text, registry, range)? else {
            return Ok(None);
        };
        let ancestors = self.ancestors(text, registry, &node.path)?;
        for ancestor in ancestors.iter().rev() {
            if ancestor.range.to.saturating_sub(ancestor.range.from) < 2 {
                continue;
            }
            let first = text.char_at(ancestor.range.from);
            let last = text.char_at(ancestor.range.to - 1);
            let matched = match pair {
                Some(requested) => {
                    let (open, close) = requested.delimiters();
                    first == Some(open) && last == Some(close)
                }
                None => DelimiterPair::ALL.iter().copied().any(|requested| {
                    let (open, close) = requested.delimiters();
                    first == Some(open) && last == Some(close)
                }),
            };
            if !matched {
                continue;
            }
            let selected = match part {
                SyntaxObjectPart::Around => ancestor.range,
                SyntaxObjectPart::Inside => SyntaxRange {
                    from: ancestor.range.from + 1,
                    to: ancestor.range.to - 1,
                },
            };
            if selected != range {
                return Ok(Some(selected));
            }
        }

        // Markdown's block/inline trees deliberately leave ordinary prose
        // punctuation as text, so there is no delimiter-shaped ancestor for
        // `(notes)` or `[labels]`. Keep code-fence injections structural, but
        // inside the outer Markdown layer fall back to balanced pairs within
        // the smallest enclosing Markdown node that contains one.
        if registry.language_name(self.language) == "markdown" && node.language == self.language {
            for ancestor in ancestors
                .iter()
                .rev()
                .filter(|ancestor| ancestor.language == self.language)
            {
                if let Some(selected) =
                    lexical_enclosing_delimiter(text, ancestor.range, range, pair, part)
                {
                    return Ok(Some(selected));
                }
            }
        }
        Ok(None)
    }

    /// Offset of the bracket matching the one at `offset`, if any.
    ///
    /// This resolves through the syntax tree rather than by counting
    /// characters, so a brace inside a string or comment does not confuse it.
    /// Returns `None` when the caret is not on a bracket, or when the tree has
    /// no node covering it.
    pub fn matching_bracket(&self, text: &Text, offset: Offset) -> Option<Offset> {
        let character = text.char_at(offset)?;
        let opening = BRACKETS.iter().find(|(open, _)| *open == character);
        let closing = BRACKETS.iter().find(|(_, close)| *close == character);
        if opening.is_none() && closing.is_none() {
            return None;
        }

        let byte = char_to_byte(text, offset);
        // Walk outward until a node actually spans the bracket, since the
        // bracket token itself is a leaf whose range is just the bracket.
        let mut node = self.syntax.descendant_for_byte_range(byte, byte + 1)?;
        loop {
            let start = node.start_byte();
            let end = node.end_byte();
            if end > start + 1 {
                let first = byte_to_char(text, start);
                let last = byte_to_char(text, end.saturating_sub(1));
                if text.char_at(first) == Some(character) && opening.is_some() {
                    return Some(last);
                }
                if text.char_at(last) == Some(character) && closing.is_some() {
                    return Some(first);
                }
            }
            node = node.parent()?;
        }
    }
}

fn lexical_enclosing_delimiter(
    text: &Text,
    bounds: SyntaxRange,
    range: SyntaxRange,
    requested: Option<DelimiterPair>,
    part: SyntaxObjectPart,
) -> Option<SyntaxRange> {
    DelimiterPair::ALL
        .iter()
        .copied()
        .filter(|pair| requested.is_none_or(|requested| requested == *pair))
        .flat_map(|pair| lexical_delimiter_pairs(text, bounds, pair))
        .filter(|around| {
            if range.is_empty() {
                around.from <= range.from && range.from < around.to
            } else {
                around.from <= range.from && range.to <= around.to
            }
        })
        .map(|around| match part {
            SyntaxObjectPart::Around => around,
            SyntaxObjectPart::Inside => SyntaxRange {
                from: around.from + 1,
                to: around.to - 1,
            },
        })
        .filter(|selected| *selected != range)
        .min_by_key(|selected| selected.to.saturating_sub(selected.from))
}

fn lexical_delimiter_pairs(
    text: &Text,
    bounds: SyntaxRange,
    pair: DelimiterPair,
) -> Vec<SyntaxRange> {
    let (open, close) = pair.delimiters();
    let characters = text
        .slice_string(bounds.from, bounds.to)
        .chars()
        .enumerate()
        .map(|(relative, character)| (bounds.from + relative, character))
        .collect::<Vec<_>>();
    let mut pairs = Vec::new();

    if open == close {
        let mut opening = None;
        for (index, (offset, character)) in characters.iter().copied().enumerate() {
            if character != open || markdown_character_is_escaped(&characters, index) {
                continue;
            }
            if let Some(from) = opening.take() {
                pairs.push(SyntaxRange {
                    from,
                    to: offset + 1,
                });
            } else {
                opening = Some(offset);
            }
        }
        return pairs;
    }

    let mut openings = Vec::new();
    for (index, (offset, character)) in characters.iter().copied().enumerate() {
        if markdown_character_is_escaped(&characters, index) {
            continue;
        }
        if character == open {
            openings.push(offset);
        } else if character == close
            && let Some(from) = openings.pop()
        {
            pairs.push(SyntaxRange {
                from,
                to: offset + 1,
            });
        }
    }
    pairs
}

fn markdown_character_is_escaped(characters: &[(Offset, char)], index: usize) -> bool {
    characters[..index]
        .iter()
        .rev()
        .take_while(|(_, character)| *character == '\\')
        .count()
        % 2
        == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The marker is an `Option` so that a language added later cannot
    /// silently inherit somebody else's comment syntax. This pins the current
    /// answer for each one, so adding a language fails here until its comment
    /// syntax has actually been decided.
    #[test]
    fn every_built_in_language_declares_its_line_comment() {
        let registry = Registry::new();
        let mut markers: Vec<(&str, Option<&str>)> = grammars::BUILTIN_LANGUAGES
            .iter()
            .map(|definition| {
                let language = registry
                    .language_for_name(definition.name)
                    .expect("every built-in language is addressable by name");
                (definition.name, registry.line_comment(language))
            })
            .collect();
        markers.sort_unstable();

        let mut expected = vec![
            ("bash", Some("#")),
            ("c", Some("//")),
            ("cpp", Some("//")),
            ("css", None),
            ("go", Some("//")),
            ("html", None),
            ("java", Some("//")),
            ("javascript", Some("//")),
            ("json", None),
            ("kotlin", Some("//")),
            ("markdown", None),
            ("python", Some("#")),
            ("rust", Some("//")),
            ("swift", Some("//")),
            ("toml", Some("#")),
            ("tsx", Some("//")),
            ("typescript", Some("//")),
            ("yaml", Some("#")),
        ];
        expected.sort_unstable();
        assert_eq!(markers, expected);
    }

    #[test]
    fn every_scope_name_is_unique_and_sorted() {
        let mut sorted = SCOPES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.as_slice(),
            SCOPES,
            "SCOPES must be sorted and unique"
        );
    }

    #[test]
    fn captures_fall_back_to_their_most_specific_known_prefix() {
        assert_eq!(scope_for_capture("keyword").unwrap().name(), "keyword");
        assert_eq!(
            scope_for_capture("keyword.control.return").unwrap().name(),
            "keyword"
        );
        assert_eq!(
            scope_for_capture("function.method.builtin").unwrap().name(),
            "function"
        );
        assert_eq!(
            scope_for_capture("markup.link.url.fragment")
                .unwrap()
                .name(),
            "markup.link.url"
        );
        for (capture, expected) in [
            ("boolean", "constant"),
            ("character", "string"),
            ("conditional", "keyword"),
            ("exception", "keyword"),
            ("float", "number"),
            ("include", "keyword"),
            ("repeat", "keyword"),
        ] {
            assert_eq!(scope_for_capture(capture).unwrap().name(), expected);
        }
        assert!(scope_for_capture("none").is_none());
        assert!(scope_for_capture("_predicate_only").is_none());
        assert!(scope_for_capture("nonsense").is_none());
        assert!(scope_for_capture("").is_none());
    }

    #[test]
    fn tree_house_shebang_markers_use_the_same_exact_interpreter_registry() {
        let registry = Registry::new();
        let bash = registry.language_for_name("bash").unwrap();
        let marker = Text::from_str("bash");
        assert_eq!(
            registry
                .language_for_marker(InjectionLanguageMarker::Shebang(marker.rope().slice(..)))
                .and_then(|language| registry.public_language(language)),
            Some(bash)
        );

        let wrong_case = Text::from_str("BASH");
        assert!(
            registry
                .language_for_marker(InjectionLanguageMarker::Shebang(
                    wrong_case.rope().slice(..)
                ))
                .is_none()
        );
    }

    #[test]
    fn markdown_inline_is_an_internal_layer_of_the_public_markdown_language() {
        let registry = Registry::new();
        let markdown = registry.language_for_name("markdown").unwrap();

        assert!(registry.language_for_name("markdown_inline").is_none());
        let inline = registry
            .language_for_marker(InjectionLanguageMarker::Name("markdown_inline"))
            .expect("the block grammar must be able to reach the inline parser");
        assert_eq!(registry.public_language(inline), Some(markdown));

        let text = Text::from_str("*text*\n");
        let syntax = DocumentSyntax::new(&text, markdown, &registry).unwrap();
        let layers = syntax
            .syntax
            .layers_for_byte_range(1, 4)
            .collect::<Vec<_>>();
        assert_eq!(
            layers.len(),
            2,
            "Markdown prose must enter the inline layer"
        );
        assert!(registry.errors().is_empty(), "{:?}", registry.errors());
        let highlighted = syntax
            .spans(&text, &registry, 0, text.len_chars())
            .into_iter()
            .map(|span| (text.slice_string(span.from, span.to), span.scope.name()))
            .collect::<Vec<_>>();
        assert!(
            highlighted
                .iter()
                .any(|(content, scope)| content == "text" && *scope == "markup.italic"),
            "inline layer did not color emphasis: {highlighted:?}; root: {:?}",
            syntax.syntax.layer(layers[1]).tree().unwrap().root_node()
        );
    }

    #[test]
    fn registry_construction_registers_every_identity_without_compiling_queries() {
        let registry = Registry::new();
        for definition in grammars::BUILTIN_LANGUAGES {
            assert!(
                registry.language_for_name(definition.name).is_some(),
                "{} missing",
                definition.name
            );
        }
        assert!(registry.errors().is_empty());
        assert!(
            registry
                .configs
                .iter()
                .all(|config| config.initialization_count() == 0)
        );
        assert!(registry.text_objects.iter().all(|capabilities| {
            capabilities
                .values()
                .all(|capability| capability.initialization_count() == 0)
        }));
        assert!(
            registry
                .outlines
                .iter()
                .flatten()
                .all(|capability| { capability.initialization_count() == 0 })
        );
        assert!(
            registry
                .indentations
                .iter()
                .flatten()
                .all(|capability| capability.initialization_count() == 0)
        );
        assert!(
            registry
                .folds
                .iter()
                .flatten()
                .all(|capability| capability.initialization_count() == 0)
        );
    }

    #[test]
    fn first_config_use_returns_one_stable_reference() {
        let registry = Registry::new();
        let rust = registry.language_for_name("rust").unwrap();
        let internal = registry.internal_language(rust).unwrap();
        let slot = &registry.configs[internal.idx()];
        assert_eq!(slot.initialization_count(), 0);

        let first = registry.get_config(internal).unwrap() as *const LanguageConfig;
        let second = registry.get_config(internal).unwrap() as *const LanguageConfig;

        assert_eq!(first, second);
        assert_eq!(slot.initialization_count(), 1);
        assert_eq!(
            registry
                .configs
                .iter()
                .map(LazyLanguageConfig::initialization_count)
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn java_config_structural_queries_and_outline_initialize_independently() {
        let registry = Registry::new();
        let java = registry.language_for_name("java").unwrap();
        let internal = registry.internal_language(java).unwrap();
        let text = Text::from_str("class Demo { int value(int input) { return input; } }");

        let syntax = DocumentSyntax::new(&text, java, &registry).unwrap();
        assert_eq!(registry.configs[internal.idx()].initialization_count(), 1);
        assert_eq!(
            registry
                .configs
                .iter()
                .map(LazyLanguageConfig::initialization_count)
                .sum::<usize>(),
            1,
            "parsing Java must not initialize another bundled language"
        );
        assert!(
            registry.text_objects[java.0 as usize]
                .values()
                .all(|capability| capability.initialization_count() == 0)
        );
        assert_eq!(
            registry.outlines[java.0 as usize]
                .as_ref()
                .unwrap()
                .initialization_count(),
            0
        );

        assert!(
            syntax
                .text_object_captures(
                    &text,
                    &registry,
                    SyntaxObject::Function,
                    SyntaxObjectPart::Around,
                    SyntaxRange::new(0, text.len_chars()).unwrap(),
                )
                .is_ok()
        );
        assert_eq!(
            registry.text_objects[java.0 as usize][&SyntaxObject::Function].initialization_count(),
            1
        );
        for object in [SyntaxObject::Class, SyntaxObject::Parameter] {
            assert_eq!(
                registry.text_objects[java.0 as usize][&object].initialization_count(),
                0
            );
        }
        assert_eq!(
            registry.outlines[java.0 as usize]
                .as_ref()
                .unwrap()
                .initialization_count(),
            0
        );

        assert!(syntax.outline(&text, &registry).is_ok());
        assert_eq!(
            registry.outlines[java.0 as usize]
                .as_ref()
                .unwrap()
                .initialization_count(),
            1
        );
        assert!(registry.errors().is_empty());
    }

    #[test]
    fn kotlin_config_structural_queries_and_outline_initialize_independently() {
        let registry = Registry::new();
        let kotlin = registry.language_for_name("kotlin").unwrap();
        let internal = registry.internal_language(kotlin).unwrap();
        let text = Text::from_str("class Demo { fun value(input: Int) = input }");

        let syntax = DocumentSyntax::new(&text, kotlin, &registry).unwrap();
        assert_eq!(registry.configs[internal.idx()].initialization_count(), 1);
        assert_eq!(
            registry
                .configs
                .iter()
                .map(LazyLanguageConfig::initialization_count)
                .sum::<usize>(),
            1,
            "parsing Kotlin must not initialize another bundled language"
        );
        assert!(
            registry.text_objects[kotlin.0 as usize]
                .values()
                .all(|capability| capability.initialization_count() == 0)
        );
        assert_eq!(
            registry.outlines[kotlin.0 as usize]
                .as_ref()
                .unwrap()
                .initialization_count(),
            0
        );

        for object in [
            SyntaxObject::Function,
            SyntaxObject::Class,
            SyntaxObject::Parameter,
        ] {
            assert!(
                syntax
                    .text_object_captures(
                        &text,
                        &registry,
                        object,
                        SyntaxObjectPart::Around,
                        SyntaxRange::new(0, text.len_chars()).unwrap(),
                    )
                    .is_ok()
            );
            assert_eq!(
                registry.text_objects[kotlin.0 as usize][&object].initialization_count(),
                1
            );
        }
        assert_eq!(
            registry.outlines[kotlin.0 as usize]
                .as_ref()
                .unwrap()
                .initialization_count(),
            0
        );
        assert!(syntax.outline(&text, &registry).is_ok());
        assert_eq!(
            registry.outlines[kotlin.0 as usize]
                .as_ref()
                .unwrap()
                .initialization_count(),
            1
        );
        assert!(registry.errors().is_empty());
    }

    #[test]
    fn concurrent_first_use_initializes_each_lazy_syntax_capability_once() {
        let registry = Registry::new();
        let rust = registry.language_for_name("rust").unwrap();
        let internal = registry.internal_language(rust).unwrap();
        let pointers = std::thread::scope(|scope| {
            (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        registry.get_config(internal).unwrap() as *const LanguageConfig as usize
                    })
                })
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(pointers.iter().all(|pointer| *pointer == pointers[0]));
        assert_eq!(registry.configs[internal.idx()].initialization_count(), 1);

        let capability = &registry.text_objects[rust.0 as usize][&SyntaxObject::Function];
        let pointers = std::thread::scope(|scope| {
            (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        registry
                            .text_object_capability(internal, SyntaxObject::Function)
                            .unwrap() as *const TextObjectCapability
                            as usize
                    })
                })
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(pointers.iter().all(|pointer| *pointer == pointers[0]));
        assert_eq!(capability.initialization_count(), 1);

        let capability = registry.indentations[rust.0 as usize].as_ref().unwrap();
        let pointers = std::thread::scope(|scope| {
            (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        registry.indentation_capability(internal).unwrap()
                            as *const IndentationCapability as usize
                    })
                })
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(pointers.iter().all(|pointer| *pointer == pointers[0]));
        assert_eq!(capability.initialization_count(), 1);

        let capability = registry.folds[rust.0 as usize].as_ref().unwrap();
        let pointers = std::thread::scope(|scope| {
            (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        registry.fold_capability(internal).unwrap() as *const FoldCapability
                            as usize
                    })
                })
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(pointers.iter().all(|pointer| *pointer == pointers[0]));
        assert_eq!(capability.initialization_count(), 1);

        let capability = registry.outlines[rust.0 as usize].as_ref().unwrap();
        let pointers = std::thread::scope(|scope| {
            (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        registry.outline_capability(internal).unwrap() as *const OutlineCapability
                            as usize
                    })
                })
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(pointers.iter().all(|pointer| *pointer == pointers[0]));
        assert_eq!(capability.initialization_count(), 1);
    }

    #[test]
    fn canonical_and_plain_configs_initialize_independently() {
        let registry = Registry::new();
        let rust = registry.language_for_name("rust").unwrap();
        let canonical = registry.language_for_size(rust, 1).unwrap();
        let plain = registry
            .language_for_size(rust, INJECTION_LIMIT_BYTES + 1)
            .unwrap();

        assert!(registry.get_config(canonical).is_some());
        assert_eq!(registry.configs[canonical.idx()].initialization_count(), 1);
        assert_eq!(registry.configs[plain.idx()].initialization_count(), 0);
        assert!(registry.get_config(plain).is_some());
        assert_eq!(registry.configs[plain.idx()].initialization_count(), 1);
        assert_eq!(registry.public_language(plain), Some(rust));
    }

    #[test]
    fn lazy_config_failure_is_typed_observable_and_language_local() {
        const INVALID_HIGHLIGHT: grammars::QueryFragment = grammars::QueryFragment::new(
            "(not_a_rust_node) @keyword",
            "invalid lazy configuration test fixture",
        );
        let queries = grammars::LanguageQueries {
            highlights: grammars::QuerySource::new(&[INVALID_HIGHLIGHT]),
            injections: grammars::QuerySource::EMPTY,
            locals: grammars::QuerySource::EMPTY,
        };
        let registry = Registry::new_with_config_override(Some(("rust", false, queries)));
        let rust = registry.language_for_name("rust").unwrap();
        let rust_internal = registry.internal_language(rust).unwrap();

        assert!(registry.errors().is_empty());
        assert!(registry.get_config(rust_internal).is_none());
        let errors = registry.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].language, rust);
        assert_eq!(errors[0].language_name, "rust");
        assert!(!errors[0].plain);
        assert!(errors[0].message.contains("query failed to compile"));
        assert!(
            registry
                .language_for_path(Path::new("still-known.rs"))
                .is_some()
        );

        let python = registry.language_for_name("python").unwrap();
        assert!(
            registry
                .get_config(registry.internal_language(python).unwrap())
                .is_some(),
            "one failed language must not disable another language"
        );
        assert_eq!(registry.errors(), errors);
    }

    #[test]
    fn failed_plain_config_does_not_disable_the_canonical_language() {
        let registry = Registry::new_with_broken_config_for_test("rust", true);
        let rust = registry.language_for_name("rust").unwrap();
        let canonical = registry.language_for_size(rust, 1).unwrap();
        let plain = registry
            .language_for_size(rust, INJECTION_LIMIT_BYTES + 1)
            .unwrap();

        assert!(registry.get_config(canonical).is_some());
        assert!(registry.errors().is_empty());
        assert!(registry.get_config(plain).is_none());
        let errors = registry.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].language, rust);
        assert!(errors[0].plain);
        assert!(registry.get_config(canonical).is_some());
    }

    #[test]
    fn injected_language_configs_compile_only_when_the_fence_is_parsed() {
        let registry = Registry::new();
        let markdown = registry.language_for_name("markdown").unwrap();
        let rust = registry.language_for_name("rust").unwrap();
        let markdown_internal = registry.internal_language(markdown).unwrap();
        let rust_internal = registry.internal_language(rust).unwrap();
        let text = Text::from_str("```rust\nfn injected() {}\n```\n");

        assert_eq!(
            registry.configs[markdown_internal.idx()].initialization_count(),
            0
        );
        assert_eq!(
            registry.configs[rust_internal.idx()].initialization_count(),
            0
        );
        assert!(DocumentSyntax::new(&text, markdown, &registry).is_some());
        assert_eq!(
            registry.configs[markdown_internal.idx()].initialization_count(),
            1
        );
        assert_eq!(
            registry.configs[rust_internal.idx()].initialization_count(),
            1
        );
        assert!(registry.errors().is_empty());
    }

    #[test]
    fn html_initializes_only_its_reached_javascript_and_css_injections() {
        let registry = Registry::new();
        let html = registry.language_for_name("html").unwrap();
        let javascript = registry.language_for_name("javascript").unwrap();
        let css = registry.language_for_name("css").unwrap();
        let html_internal = registry.internal_language(html).unwrap();
        let javascript_internal = registry.internal_language(javascript).unwrap();
        let css_internal = registry.internal_language(css).unwrap();
        let text = Text::from_str(
            "<main><script>const answer = 42;</script><style>main { color: red; }</style></main>",
        );

        assert!(
            registry
                .configs
                .iter()
                .all(|config| config.initialization_count() == 0)
        );
        assert!(DocumentSyntax::new(&text, html, &registry).is_some());
        for language in [html_internal, javascript_internal, css_internal] {
            assert_eq!(registry.configs[language.idx()].initialization_count(), 1);
        }
        assert_eq!(
            registry
                .configs
                .iter()
                .map(LazyLanguageConfig::initialization_count)
                .sum::<usize>(),
            3,
            "unrelated language configurations must stay lazy"
        );
        assert!(registry.errors().is_empty());
    }

    #[test]
    fn html_leaves_an_unreached_registered_injection_target_lazy() {
        for (source, reached, unreached) in [
            (
                "<script>const reached = true;</script>",
                "javascript",
                "css",
            ),
            ("<style>main { color: red; }</style>", "css", "javascript"),
        ] {
            let registry = Registry::new();
            let html = registry.language_for_name("html").unwrap();
            let html_internal = registry.internal_language(html).unwrap();
            let reached = registry
                .internal_language(registry.language_for_name(reached).unwrap())
                .unwrap();
            let unreached = registry
                .internal_language(registry.language_for_name(unreached).unwrap())
                .unwrap();
            let text = Text::from_str(source);

            assert!(DocumentSyntax::new(&text, html, &registry).is_some());
            assert_eq!(
                registry.configs[html_internal.idx()].initialization_count(),
                1
            );
            assert_eq!(registry.configs[reached.idx()].initialization_count(), 1);
            assert_eq!(
                registry.configs[unreached.idx()].initialization_count(),
                0,
                "a registered but unreached injection target must stay lazy"
            );
        }
    }

    #[test]
    fn failed_injected_config_is_reported_without_disabling_the_outer_document() {
        let registry = Registry::new_with_broken_config_for_test("rust", false);
        let markdown = registry.language_for_name("markdown").unwrap();
        let rust = registry.language_for_name("rust").unwrap();
        let text = Text::from_str("# Outer\n\n```rust\nfn unavailable() {}\n```\n");

        let syntax = DocumentSyntax::new(&text, markdown, &registry)
            .expect("an injected-language failure must not disable Markdown");
        assert!(
            !syntax
                .spans(&text, &registry, 0, text.len_chars())
                .is_empty(),
            "the outer Markdown layer must remain highlightable"
        );
        let errors = registry.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].language, rust);
        assert!(!errors[0].plain);
    }

    #[test]
    fn lazy_registry_uses_safe_standard_one_time_cells() {
        let source = include_str!("mod.rs");
        assert!(source.contains("OnceLock<Result<LanguageConfig"));
        assert!(source.contains("OnceLock<TextObjectCapability>"));
        assert!(source.contains("OnceLock<OutlineCapability>"));
        assert!(source.contains("OnceLock<IndentationCapability>"));
        assert!(source.contains("OnceLock<FoldCapability>"));
        assert!(!source.contains(&["un", "safe"].concat()));
    }

    #[test]
    fn every_canonical_plain_and_owned_capability_query_compiles() {
        let registry = Registry::new();
        let plain_count = grammars::BUILTIN_LANGUAGES
            .iter()
            .filter(|definition| !definition.queries.injections.compose().is_empty())
            .count();
        assert_eq!(
            registry.configs.len(),
            grammars::BUILTIN_LANGUAGES.len() + plain_count + 1,
            "public languages need canonical/plain configurations and Markdown needs one internal inline configuration"
        );
        assert_eq!(
            plain_count, 3,
            "Rust, HTML, and Markdown have plain variants"
        );

        for definition in grammars::BUILTIN_LANGUAGES
            .iter()
            .chain(std::iter::once(&grammars::MARKDOWN_INLINE))
        {
            let grammar: Grammar = definition.grammar.try_into().unwrap();
            compile_language_config(grammar, definition.queries, true)
                .unwrap_or_else(|error| panic!("{} canonical queries: {error}", definition.name));
            if !definition.queries.injections.compose().is_empty() {
                compile_language_config(grammar, definition.queries, false)
                    .unwrap_or_else(|error| panic!("{} plain queries: {error}", definition.name));
            }
            for text_object in definition.text_objects {
                let source = text_object.query.compose();
                Query::new(grammar, &source, |_, _| Ok(())).unwrap_or_else(|error| {
                    panic!(
                        "{} {:?} text-object query: {error}",
                        definition.name, text_object.object
                    )
                });
            }
            let source = definition.outline.compose();
            if !source.is_empty() {
                Query::new(grammar, &source, |_, _| Ok(()))
                    .unwrap_or_else(|error| panic!("{} outline query: {error}", definition.name));
            }
            if !definition.indentation.fragments.is_empty() {
                compile_owned_query(
                    definition,
                    None,
                    definition.indentation,
                    &["indent.begin", "indent.always"],
                )
                .unwrap_or_else(|error| panic!("{} indentation query: {error}", definition.name));
            }
            if !definition.folds.fragments.is_empty() {
                compile_owned_query(definition, None, definition.folds, &["fold"])
                    .unwrap_or_else(|error| panic!("{} fold query: {error}", definition.name));
            }
        }
    }

    #[test]
    fn indentation_and_fold_queries_reject_unowned_dialects() {
        let indentation = Registry::new_with_indentation_override(Some((
            "rust",
            "((block) @indent.begin (#set! indent.scope \"all\"))",
        )));
        let rust = indentation.language_for_name("rust").unwrap();
        let text = Text::from_str("fn main() {\n}\n");
        let syntax = DocumentSyntax::new(&text, rust, &indentation).unwrap();
        assert!(matches!(
            syntax.newline_indent(&text, &indentation, 11),
            Err(SyntaxError::IndentationQueryFailed { language, .. }) if language == rust
        ));

        let folds = Registry::new_with_fold_override(Some(("rust", "(block) @fold.extra")));
        let syntax = DocumentSyntax::new(&text, rust, &folds).unwrap();
        assert!(matches!(
            syntax.folds(&text, &folds),
            Err(SyntaxError::FoldQueryFailed { language, .. }) if language == rust
        ));
    }

    #[test]
    fn indentation_and_folds_are_lazy_independent_and_explicitly_supported() {
        let registry = Registry::new();
        let rust = registry.language_for_name("rust").unwrap();
        let internal = registry.internal_language(rust).unwrap();
        let text = Text::from_str("fn main() {\n    call();\n}\n");
        let syntax = DocumentSyntax::new(&text, rust, &registry).unwrap();
        let indentation = registry.indentations[rust.0 as usize].as_ref().unwrap();
        let folds = registry.folds[rust.0 as usize].as_ref().unwrap();
        assert_eq!(indentation.initialization_count(), 0);
        assert_eq!(folds.initialization_count(), 0);

        assert!(syntax.newline_indent(&text, &registry, 11).is_ok());
        assert_eq!(indentation.initialization_count(), 1);
        assert_eq!(folds.initialization_count(), 0);
        assert!(syntax.folds(&text, &registry).is_ok());
        assert_eq!(indentation.initialization_count(), 1);
        assert_eq!(folds.initialization_count(), 1);
        assert_eq!(registry.configs[internal.idx()].initialization_count(), 1);

        let markdown = registry.language_for_name("markdown").unwrap();
        let markdown_text = Text::from_str("# Heading\n\nbody\n");
        let markdown_syntax = DocumentSyntax::new(&markdown_text, markdown, &registry).unwrap();
        assert!(matches!(
            markdown_syntax.newline_indent(&markdown_text, &registry, 9),
            Err(SyntaxError::UnsupportedIndentation { language }) if language == markdown
        ));
        assert!(markdown_syntax.folds(&markdown_text, &registry).is_ok());
    }

    #[test]
    fn syntax_capability_issue_lists_are_strictly_bounded() {
        let rust = LanguageId(0);
        let mut truncated = false;
        let mut indent = Vec::new();
        let mut folds = Vec::new();
        for depth in 0..=SYNTAX_CAPABILITY_ISSUE_LIMIT {
            push_indent_issue(
                &mut indent,
                IndentIssue::UnsupportedInjectedLanguage {
                    language: rust,
                    injection_depth: depth as u32,
                },
                &mut truncated,
            );
            push_fold_issue(
                &mut folds,
                FoldIssue::UnsupportedInjectedLanguage {
                    language: rust,
                    injection_depth: depth as u32,
                },
                &mut truncated,
            );
        }
        assert_eq!(indent.len(), 128);
        assert_eq!(folds.len(), 128);
        assert!(truncated);
    }

    #[test]
    fn broken_injected_indent_and_fold_queries_degrade_to_the_outer_language() {
        let source = "<script>function main() {\n    call();\n}</script>\n";
        let text = Text::from_str(source);

        let indentation = Registry::new_with_indentation_override(Some((
            "javascript",
            "(runyte_invalid_javascript_node) @indent.always",
        )));
        let html = indentation.language_for_name("html").unwrap();
        let javascript = indentation.language_for_name("javascript").unwrap();
        let syntax = DocumentSyntax::new(&text, html, &indentation).unwrap();
        let newline = source.find('\n').unwrap();
        let indent = syntax.newline_indent(&text, &indentation, newline).unwrap();
        assert_eq!(indent.language, html);
        assert_eq!(indent.injection_depth, 0);
        assert!(indent.issues.iter().any(|issue| matches!(
            issue,
            IndentIssue::InjectedQueryFailed {
                language,
                injection_depth: 1,
                ..
            } if *language == javascript
        )));

        let folds = Registry::new_with_fold_override(Some((
            "javascript",
            "(runyte_invalid_javascript_node) @fold",
        )));
        let html = folds.language_for_name("html").unwrap();
        let javascript = folds.language_for_name("javascript").unwrap();
        let syntax = DocumentSyntax::new(&text, html, &folds).unwrap();
        let projection = syntax.folds(&text, &folds).unwrap();
        assert!(
            projection
                .items
                .iter()
                .all(|item| item.language == html && item.injection_depth == 0)
        );
        assert!(projection.issues.iter().any(|issue| matches!(
            issue,
            FoldIssue::InjectedQueryFailed {
                language,
                injection_depth: 1,
                ..
            } if *language == javascript
        )));
    }

    #[test]
    fn a_broken_outline_query_is_typed_and_does_not_disable_highlighting() {
        let registry = Registry::new_with_outline_override(Some((
            "rust",
            "(not_a_rust_node) @outline.function",
        )));
        let rust = registry.language_for_name("rust").unwrap();
        let text = Text::from_str("fn main() {}\n");
        let syntax = DocumentSyntax::new(&text, rust, &registry).unwrap();

        assert!(matches!(
            syntax.outline(&text, &registry),
            Err(SyntaxError::OutlineQueryFailed { language, .. }) if language == rust
        ));
        assert!(
            !syntax
                .spans(&text, &registry, 0, text.len_chars())
                .is_empty()
        );
        assert!(registry.errors().is_empty());
    }

    #[test]
    fn injected_outline_query_failure_is_reported_without_hiding_outer_items() {
        let registry = Registry::new_with_outline_override(Some((
            "rust",
            "(not_a_rust_node) @outline.function",
        )));
        let markdown = registry.language_for_name("markdown").unwrap();
        let rust = registry.language_for_name("rust").unwrap();
        let text = Text::from_str("# Outer\n\n```rust\nfn unavailable() {}\n```\n");
        let syntax = DocumentSyntax::new(&text, markdown, &registry).unwrap();
        let outline = syntax.outline(&text, &registry).unwrap();

        assert_eq!(outline.items[0].name.as_ref(), "Outer");
        assert!(outline.issues.iter().any(|issue| matches!(
            issue,
            OutlineIssue::InjectedQueryFailed {
                language,
                injection_depth: 1,
                ..
            } if *language == rust
        )));
    }

    #[test]
    fn injected_parser_failure_is_reported_without_hiding_outer_items() {
        let registry = Registry::new_with_broken_config_for_test("rust", false);
        let markdown = registry.language_for_name("markdown").unwrap();
        let rust = registry.language_for_name("rust").unwrap();
        let text = Text::from_str("# Outer\n\n```rust\nfn unavailable() {}\n```\n");
        let syntax = DocumentSyntax::new(&text, markdown, &registry).unwrap();
        let outline = syntax.outline(&text, &registry).unwrap();

        assert_eq!(outline.items[0].name.as_ref(), "Outer");
        assert_eq!(
            outline
                .issues
                .iter()
                .filter(|issue| matches!(
                    issue,
                    OutlineIssue::InjectedParserUnavailable {
                        language,
                        injection_depth: 1,
                        ..
                    } if *language == rust
                ))
                .count(),
            1,
            "the iterator's real injection layer must be reported exactly once"
        );
    }

    #[test]
    fn equal_range_outer_and_injected_items_have_deterministic_depth_order() {
        let registry = Registry::new_with_outline_override(Some((
            "html",
            "(script_element (raw_text) @outline.name @outline.heading)",
        )));
        let html = registry.language_for_name("html").unwrap();
        let text = Text::from_str("<script>function boot(){}</script>");
        let syntax = DocumentSyntax::new(&text, html, &registry).unwrap();
        let outline = syntax.outline(&text, &registry).unwrap();

        assert_eq!(outline.items.len(), 2);
        assert_eq!(outline.items[0].range, outline.items[1].range);
        assert_eq!(
            outline
                .items
                .iter()
                .map(|item| item.injection_depth)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(outline.items[0].parent, None);
        assert_eq!(outline.items[1].parent, None);
    }

    #[test]
    fn outline_issue_projection_is_bounded() {
        let registry = Registry::new_with_outline_override(Some((
            "html",
            "(element (start_tag (tag_name) @outline.name)) @outline.type",
        )));
        let html = registry.language_for_name("html").unwrap();
        let source = "<script>function broken(</script>".repeat(OUTLINE_ISSUE_LIMIT + 10);
        let text = Text::from_str(&source);
        let syntax = DocumentSyntax::new(&text, html, &registry).unwrap();
        let outline = syntax.outline(&text, &registry).unwrap();

        assert_eq!(outline.issues.len(), OUTLINE_ISSUE_LIMIT);
        assert!(outline.truncated);
    }

    #[test]
    fn locals_are_compiled_through_the_language_config_boundary() {
        const INVALID_LOCALS: grammars::QueryFragment = grammars::QueryFragment::new(
            "(not_a_rust_node) @local.scope",
            "invalid locals test fixture",
        );
        let queries = grammars::LanguageQueries {
            highlights: grammars::QuerySource::EMPTY,
            injections: grammars::QuerySource::EMPTY,
            locals: grammars::QuerySource::new(&[INVALID_LOCALS]),
        };
        let grammar: Grammar = tree_sitter_rust::LANGUAGE.try_into().unwrap();

        assert!(
            compile_language_config(grammar, queries, true).is_err(),
            "an invalid locals query must not be silently dropped"
        );
    }

    #[test]
    fn raw_grammar_definitions_are_private_to_syntax() {
        let module = include_str!("mod.rs");
        let definitions = include_str!("grammars.rs");
        let private_declaration = ["mod", "grammars;"].join(" ");
        let public_declaration = ["pub", "mod", "grammars;"].join(" ");

        assert!(
            module
                .lines()
                .any(|line| line.trim() == private_declaration)
        );
        assert!(!module.lines().any(|line| line.trim() == public_declaration));
        assert!(!definitions.contains("pub struct LanguageDefinition"));
        assert!(!definitions.contains("pub const BUILTIN_LANGUAGES"));
    }

    #[test]
    fn a_broken_text_object_query_degrades_only_that_language_capability() {
        let registry = Registry::new_with_text_object_override(Some((
            "rust",
            SyntaxObject::Class,
            "(not_a_rust_node) @x",
        )));
        assert!(registry.errors().is_empty());
        let rust = registry.language_for_name("rust").unwrap();
        let text = Text::from_str("fn main() {}\n");
        let syntax = DocumentSyntax::new(&text, rust, &registry).unwrap();
        assert!(
            !syntax
                .spans(&text, &registry, 0, text.len_chars())
                .is_empty()
        );
        assert!(matches!(
            syntax.text_object_captures(
                &text,
                &registry,
                SyntaxObject::Class,
                SyntaxObjectPart::Around,
                SyntaxRange::new(0, text.len_chars()).unwrap(),
            ),
            Err(SyntaxError::TextObjectQueryFailed { language, .. }) if language == rust
        ));

        assert_eq!(
            syntax
                .text_object_captures(
                    &text,
                    &registry,
                    SyntaxObject::Function,
                    SyntaxObjectPart::Around,
                    SyntaxRange::new(0, text.len_chars()).unwrap(),
                )
                .unwrap()
                .len(),
            1,
            "a broken class query must not disable Rust functions"
        );

        let python = registry.language_for_name("python").unwrap();
        let text = Text::from_str("def ok():\n    pass\n");
        let syntax = DocumentSyntax::new(&text, python, &registry).unwrap();
        assert_eq!(
            syntax
                .text_object_captures(
                    &text,
                    &registry,
                    SyntaxObject::Function,
                    SyntaxObjectPart::Around,
                    SyntaxRange::new(0, text.len_chars()).unwrap(),
                )
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_broken_javascript_function_query_leaves_classes_and_typescript_available() {
        let registry = Registry::new_with_text_object_override(Some((
            "javascript",
            SyntaxObject::Function,
            "(not_a_javascript_node) @function.around",
        )));
        assert!(registry.errors().is_empty());

        let javascript = registry.language_for_name("javascript").unwrap();
        let text = Text::from_str("class View {}\nfunction render() {}\n");
        let syntax = DocumentSyntax::new(&text, javascript, &registry).unwrap();
        assert!(matches!(
            syntax.text_object_captures(
                &text,
                &registry,
                SyntaxObject::Function,
                SyntaxObjectPart::Around,
                SyntaxRange::new(0, text.len_chars()).unwrap(),
            ),
            Err(SyntaxError::TextObjectQueryFailed { language, .. }) if language == javascript
        ));
        assert_eq!(
            syntax
                .text_object_captures(
                    &text,
                    &registry,
                    SyntaxObject::Class,
                    SyntaxObjectPart::Around,
                    SyntaxRange::new(0, text.len_chars()).unwrap(),
                )
                .unwrap()
                .len(),
            1,
            "a broken JavaScript function query must not disable JavaScript classes"
        );

        let typescript = registry.language_for_name("typescript").unwrap();
        let text = Text::from_str("function typed(value: string): string { return value; }\n");
        let syntax = DocumentSyntax::new(&text, typescript, &registry).unwrap();
        assert_eq!(
            syntax
                .text_object_captures(
                    &text,
                    &registry,
                    SyntaxObject::Function,
                    SyntaxObjectPart::Around,
                    SyntaxRange::new(0, text.len_chars()).unwrap(),
                )
                .unwrap()
                .len(),
            1,
            "a broken JavaScript capability must not disable TypeScript"
        );
    }

    #[test]
    fn checked_coordinates_preserve_unicode_character_offsets() {
        let text = Text::from_str("a🦀β");
        assert_eq!(checked_char_to_byte(&text, 0), Ok(0));
        assert_eq!(checked_char_to_byte(&text, 1), Ok(1));
        assert_eq!(checked_char_to_byte(&text, 2), Ok(5));
        assert_eq!(checked_char_to_byte(&text, 3), Ok(7));

        assert_eq!(checked_byte_to_char(&text, 0), Ok(0));
        assert_eq!(checked_byte_to_char(&text, 2), Ok(1));
        assert_eq!(checked_byte_to_char(&text, 5), Ok(2));
        assert_eq!(checked_byte_to_char(&text, 7), Ok(3));
        assert_eq!(
            checked_range_to_bytes(&text, SyntaxRange::new(1, 3).unwrap()),
            Ok(1..7)
        );
    }

    #[test]
    fn checked_coordinates_reject_invalid_ranges_and_offsets() {
        let text = Text::from_str("abc");
        assert_eq!(
            SyntaxRange::new(3, 2),
            Err(SyntaxError::InvalidRange { from: 3, to: 2 })
        );
        assert_eq!(
            SyntaxRange::new(0, 4).unwrap().checked(&text),
            Err(SyntaxError::CharacterOffsetOutOfBounds {
                offset: 4,
                len_chars: 3,
            })
        );
        assert_eq!(
            checked_char_to_byte(&text, 4),
            Err(SyntaxError::CharacterOffsetOutOfBounds {
                offset: 4,
                len_chars: 3,
            })
        );
        assert_eq!(
            checked_byte_to_char(&text, 4),
            Err(SyntaxError::ByteOffsetOutOfBounds {
                offset: 4,
                len_bytes: 3,
            })
        );
    }

    #[test]
    fn empty_and_eof_ranges_are_valid_coordinates() {
        let empty = Text::new();
        assert_eq!(
            checked_range_to_bytes(&empty, SyntaxRange::point(0)),
            Ok(0..0)
        );

        let text = Text::from_str("🦀");
        assert_eq!(
            checked_range_to_bytes(&text, SyntaxRange::point(text.len_chars())),
            Ok(4..4)
        );
    }

    #[test]
    fn syntax_kind_owns_its_name() {
        let kind = {
            let name = String::from("function_item");
            SyntaxKind::new(name.as_str())
        };
        assert_eq!(kind.as_str(), "function_item");
        assert_eq!(kind.to_string(), "function_item");
    }

    #[test]
    fn plain_parser_variants_keep_the_canonical_language_identity() {
        let registry = Registry::new();
        let rust = registry.language_for_name("rust").unwrap();
        let canonical = registry.language_for_size(rust, 1).unwrap();
        let plain = registry
            .language_for_size(rust, INJECTION_LIMIT_BYTES + 1)
            .unwrap();

        assert_ne!(canonical, plain);
        assert_eq!(registry.public_language(canonical), Some(rust));
        assert_eq!(registry.public_language(plain), Some(rust));
        assert_eq!(registry.language_name(rust), "rust");
    }

    #[test]
    fn successful_updates_advance_the_document_revision() {
        let registry = Registry::new();
        let language = registry.language_for_name("rust").unwrap();
        let mut text = Text::from_str("fn main() {}\n");
        let mut syntax = DocumentSyntax::new(&text, language, &registry).unwrap();
        assert_eq!(syntax.revision().get(), 0);

        let before = text.clone();
        let transaction = Transaction::insert(3, "async ");
        text.apply(&transaction);
        assert!(syntax.update(&before, &text, &transaction, &registry));
        assert_eq!(syntax.revision().get(), 1);
        assert_eq!(syntax.language(), language);
    }

    #[test]
    fn updates_switch_parser_variants_across_the_injection_limit() {
        let registry = Registry::new();
        let language = registry.language_for_name("rust").unwrap();
        let canonical = registry.language_for_size(language, 1).unwrap();
        let plain = registry
            .language_for_size(language, INJECTION_LIMIT_BYTES + 1)
            .unwrap();
        let mut text = Text::from_str("fn main() {}\n");
        let mut syntax = DocumentSyntax::new(&text, language, &registry).unwrap();
        assert_eq!(
            syntax.syntax.layer(syntax.syntax.root()).language,
            canonical
        );

        let before = text.clone();
        let padding = " ".repeat(INJECTION_LIMIT_BYTES + 1);
        let grow = Transaction::insert(text.len_chars(), padding);
        text.apply(&grow);
        assert!(syntax.update(&before, &text, &grow, &registry));
        assert_eq!(syntax.syntax.layer(syntax.syntax.root()).language, plain);

        let before = text.clone();
        let shrink = Transaction::delete("fn main() {}\n".chars().count(), text.len_chars());
        text.apply(&shrink);
        assert!(syntax.update(&before, &text, &shrink, &registry));
        assert_eq!(
            syntax.syntax.layer(syntax.syntax.root()).language,
            canonical
        );
        assert_eq!(syntax.revision().get(), 2);
    }
}
