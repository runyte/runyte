// SPDX-License-Identifier: MPL-2.0

//! Phase 1 gate: syntax highlighting.
//!
//! Covers per-language correctness for the bundled grammars, incremental
//! reparse cost on a large file, and graceful degradation when a buffer has no
//! usable grammar.

use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use runyte::{
    selection::Range as SelectionRange,
    syntax::{
        DocumentSyntax, LanguageId, Outline, OutlineIssue, OutlineKind, Registry, RegistryError,
        Span, SyntaxCapture, SyntaxError, SyntaxKind, SyntaxObject, SyntaxObjectPart, SyntaxRange,
        SyntaxRelation, SyntaxRevision, SyntaxSelectionTransform,
    },
    text::{Text, Transaction},
};

/// The frame budget for an incremental reparse. Documented here because the
/// phase gate refers to it: a reparse must fit comfortably inside a redraw so
/// typing never stalls.
///
/// Generous enough to hold in an unoptimised debug build on a slow machine;
/// the ratio assertion alongside it is the sharper check.
const REPARSE_BUDGET: Duration = Duration::from_millis(50);

fn parse(source: &str, language: &str) -> (Registry, Text, DocumentSyntax) {
    let registry = Registry::new();
    let id = registry
        .language_for_name(language)
        .unwrap_or_else(|| panic!("{language} grammar missing"));
    let text = Text::from_str(source);
    let syntax = DocumentSyntax::new(&text, id, &registry)
        .unwrap_or_else(|| panic!("{language} failed to parse"));
    (registry, text, syntax)
}

fn scopes(source: &str, language: &str) -> Vec<(String, &'static str)> {
    let (registry, text, syntax) = parse(source, language);
    spans_of(&syntax, &text, &registry)
        .into_iter()
        .map(|span| (text.slice_string(span.from, span.to), span.scope.name()))
        .collect()
}

fn spans_of(syntax: &DocumentSyntax, text: &Text, registry: &Registry) -> Vec<Span> {
    syntax.spans(text, registry, 0, text.len_chars())
}

fn char_offset(source: &str, needle: &str) -> usize {
    let byte = source
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} missing from fixture"));
    source[..byte].chars().count()
}

fn text_object_captures(
    source: &str,
    language: &str,
    object: SyntaxObject,
    part: SyntaxObjectPart,
) -> (Text, Registry, Vec<SyntaxCapture>) {
    let (registry, text, syntax) = parse(source, language);
    let captures = syntax
        .text_object_captures(
            &text,
            &registry,
            object,
            part,
            SyntaxRange::new(0, text.len_chars()).unwrap(),
        )
        .unwrap();
    (text, registry, captures)
}

fn capture_text(text: &Text, capture: &SyntaxCapture) -> Vec<String> {
    capture
        .ranges
        .iter()
        .map(|range| text.slice_string(range.from, range.to))
        .collect()
}

fn document_outline(source: &str, language: &str) -> (Registry, Text, DocumentSyntax, Outline) {
    let (registry, text, syntax) = parse(source, language);
    let outline = syntax.outline(&text, &registry).expect("outline");
    (registry, text, syntax, outline)
}

fn outline_entries(outline: &Outline) -> Vec<(&str, OutlineKind)> {
    outline
        .items
        .iter()
        .map(|item| (item.name.as_ref(), item.kind))
        .collect()
}

fn expansion_sequence(
    syntax: &DocumentSyntax,
    text: &Text,
    registry: &Registry,
    start: SyntaxRange,
) -> Vec<SyntaxRange> {
    let mut ranges = vec![start];
    while let Some(next) = syntax
        .expand_range(text, registry, *ranges.last().unwrap())
        .unwrap()
    {
        assert!(
            next.from <= ranges.last().unwrap().from
                && next.to >= ranges.last().unwrap().to
                && next != *ranges.last().unwrap(),
            "expansion must be a strict visual superset: {ranges:?} -> {next:?}"
        );
        ranges.push(next);
        assert!(ranges.len() < 64, "expansion must terminate at the root");
    }
    ranges
}

fn preserve_selection_direction(original: SelectionRange, range: SyntaxRange) -> SelectionRange {
    if original.anchor <= original.head {
        SelectionRange::new(range.from, range.to)
    } else {
        SelectionRange::new(range.to, range.from)
    }
}

#[test]
fn public_syntax_boundary_uses_runyte_owned_values() {
    let _lookup: fn(&Registry, &str) -> Option<LanguageId> = Registry::language_for_name;
    let _infer: fn(&Registry, Option<&Path>, &Text) -> Option<LanguageId> =
        Registry::language_for_document;
    let _errors: fn(&Registry) -> Vec<RegistryError> = Registry::errors;
    let _constructor: fn(&Text, LanguageId, &Registry) -> Option<DocumentSyntax> =
        DocumentSyntax::new;
    let _language: fn(&DocumentSyntax) -> LanguageId = DocumentSyntax::language;
    let _revision: fn(&DocumentSyntax) -> SyntaxRevision = DocumentSyntax::revision;
    let kind = SyntaxKind::new("identifier");
    let range = SyntaxRange::new(0, 0).unwrap();
    let error: Box<dyn std::error::Error> = Box::new(SyntaxError::StaleRevision {
        expected: SyntaxRevision::default(),
        actual: SyntaxRevision::default(),
    });
    assert_eq!(kind.as_str(), "identifier");
    assert!(range.is_empty());
    assert!(error.to_string().contains("revision"));
}

#[test]
fn filename_extension_and_bounded_shebang_inference_have_stable_precedence() {
    let registry = Registry::new();
    let name = |language| registry.language_name(language);

    for (filename, language) in [
        (".bashrc", "bash"),
        (".bash_profile", "bash"),
        ("CMakeLists.txt", "cmake"),
        ("Makefile", "make"),
        ("makefile", "make"),
        ("GNUmakefile", "make"),
    ] {
        assert_eq!(
            registry.language_for_document(Some(Path::new(filename)), &Text::from_str("")),
            registry.language_for_name(language),
            "{filename}"
        );
    }

    assert_eq!(
        name(
            registry
                .language_for_document(Some(Path::new("main.GO")), &Text::from_str("#!/bin/bash\n"))
                .unwrap()
        ),
        "go",
        "an extension wins over conflicting source metadata"
    );
    assert_eq!(
        name(
            registry
                .language_for_document(
                    Some(Path::new(".bashrc")),
                    &Text::from_str("#!/usr/bin/python\n")
                )
                .unwrap()
        ),
        "bash",
        "an exact filename wins over conflicting source metadata"
    );
    assert!(
        registry
            .language_for_document(
                Some(Path::new("go.mod")),
                &Text::from_str("module example\n")
            )
            .is_none()
    );
    assert!(
        registry
            .language_for_document(Some(Path::new("bashrc")), &Text::from_str(""))
            .is_none()
    );

    for source in [
        "#!/bin/bash\necho ok\n",
        "#!/bin/sh\r\necho ok\r\n",
        "#!/bin/dash\necho ok\n",
        "#!/usr/bin/env bash\necho ok\n",
        "#!/usr/bin/env -S bash -eu\necho ok\n",
        "#!/usr/bin/env RUNYTE=1 bash\necho ok\n",
        "#!/usr/bin/env lua\nprint('ok')\n",
    ] {
        let language = registry
            .language_for_document(None, &Text::from_str(source))
            .expect(source);
        let expected = if source.contains("lua") {
            "lua"
        } else {
            "bash"
        };
        assert_eq!(name(language), expected, "{source:?}");
    }

    for source in [
        "#!/bin/zsh\necho no\n",
        "#!/bin/ksh\necho no\n",
        "#!/bin/notbash\necho no\n",
        "#!/bin/BASH\necho no\n",
        "#!/usr/bin/env python bash\n",
        " #!/bin/bash\necho no\n",
        "echo first\n#!/bin/bash\n",
        "bash\necho no\n",
        "not a shebang\n",
    ] {
        assert!(
            registry
                .language_for_document(None, &Text::from_str(source))
                .is_none(),
            "{source:?}"
        );
    }

    let beyond_bound = format!("#!{}bash\n", " ".repeat(1_100));
    assert!(
        registry
            .language_for_document(None, &Text::from_str(&beyond_bound))
            .is_none()
    );
}

// -- Document outlines ----------------------------------------------------

#[test]
fn outline_queries_cover_the_supported_language_inventory() {
    let cases = [
        (
            "rust",
            "mod api { fn helper() {} struct Model; trait Store { fn load(&self); } }\nfn main() {}\nconst LIMIT: usize = 1;\nmacro_rules! trace { () => {} }\n",
            vec![
                ("api", OutlineKind::Module),
                ("helper", OutlineKind::Function),
                ("Model", OutlineKind::Type),
                ("Store", OutlineKind::Interface),
                ("load", OutlineKind::Method),
                ("main", OutlineKind::Function),
                ("LIMIT", OutlineKind::Constant),
                ("trace", OutlineKind::Macro),
            ],
        ),
        (
            "python",
            "LIMIT = 1\nclass Model:\n    def load(self):\n        pass\ndef main():\n    pass\n",
            vec![
                ("LIMIT", OutlineKind::Constant),
                ("Model", OutlineKind::Class),
                ("load", OutlineKind::Method),
                ("main", OutlineKind::Function),
            ],
        ),
        (
            "markdown",
            "# One\n\ntext\n\n## Two\n\nmore\n",
            vec![("One", OutlineKind::Heading), ("Two", OutlineKind::Heading)],
        ),
        (
            "c",
            "int prototype(int x);\nint *pointer_result(void) { return 0; }\ntypedef int Count;\ntypedef int *CountPtr;\nstruct Model { int x; };\nenum State { ON };\nint main(void) { return 0; }\n",
            vec![
                ("prototype", OutlineKind::Function),
                ("pointer_result", OutlineKind::Function),
                ("Count", OutlineKind::Type),
                ("CountPtr", OutlineKind::Type),
                ("Model", OutlineKind::Type),
                ("State", OutlineKind::Type),
                ("main", OutlineKind::Function),
            ],
        ),
        (
            "cpp",
            "using Count = int;\ntemplate<typename T> T id(T value) { return value; }\ntemplate<typename T> using Ptr = T*;\ntemplate<typename T> concept Number = true;\nclass Model { public: Model() {} ~Model() {} int operator+(int x) { return x; } };\nModel::~Model() {}\nint Model::operator-(int x) { return x; }\n",
            vec![
                ("Count", OutlineKind::Alias),
                ("id", OutlineKind::Function),
                ("Ptr", OutlineKind::Alias),
                ("Number", OutlineKind::Concept),
                ("Model", OutlineKind::Class),
                ("Model", OutlineKind::Method),
                ("~Model", OutlineKind::Method),
                ("operator+", OutlineKind::Method),
                ("~Model", OutlineKind::Method),
                ("operator-", OutlineKind::Method),
            ],
        ),
        (
            "swift",
            "class C { var member = 1; subscript(i: Int) -> Int { i } }\nstruct S { let value = 1 }\nenum E { case one }\nactor A {}\nextension C { var extra: Int { 2 } }\ntypealias ID = Int\nfunc top() { let local = 1 }\nlet global = 2\n",
            vec![
                ("C", OutlineKind::Class),
                ("member", OutlineKind::Property),
                ("subscript", OutlineKind::Subscript),
                ("S", OutlineKind::Struct),
                ("value", OutlineKind::Property),
                ("E", OutlineKind::Enum),
                ("A", OutlineKind::Actor),
                ("C", OutlineKind::Extension),
                ("extra", OutlineKind::Property),
                ("ID", OutlineKind::Alias),
                ("top", OutlineKind::Function),
                ("global", OutlineKind::Property),
            ],
        ),
        (
            "javascript",
            "class Model { load() {} }\nfunction main() {}\nconst render = () => {};\nconst PublicFn = function Inner() {};\nconst PublicClass = class Internal {};\nexports.boot = function Hidden() {};\nconst registry = { \"quoted\": function Nested() {}, Plain: class NestedClass {} };\n(function Bare() {});\n(class BareClass {});\n",
            vec![
                ("Model", OutlineKind::Class),
                ("load", OutlineKind::Method),
                ("main", OutlineKind::Function),
                ("render", OutlineKind::Function),
                ("PublicFn", OutlineKind::Function),
                ("PublicClass", OutlineKind::Class),
                ("boot", OutlineKind::Function),
                ("quoted", OutlineKind::Function),
                ("Plain", OutlineKind::Class),
            ],
        ),
        (
            "typescript",
            "interface Store { load(): void; }\ntype Id = string;\nabstract class Model { abstract save(): void; }\nfunction main(): void;\nconst PublicFn = function Inner() {};\nconst PublicClass = class Internal {};\n(function Bare() {});\n",
            vec![
                ("Store", OutlineKind::Interface),
                ("load", OutlineKind::Method),
                ("Id", OutlineKind::Type),
                ("Model", OutlineKind::Class),
                ("save", OutlineKind::Method),
                ("main", OutlineKind::Function),
                ("PublicFn", OutlineKind::Function),
                ("PublicClass", OutlineKind::Class),
            ],
        ),
        (
            "tsx",
            "interface Props { title: string }\nconst View = (props: Props) => <h1>{props.title}</h1>;\nconst Factory = class Internal {};\n(class Bare {});\n",
            vec![
                ("Props", OutlineKind::Interface),
                ("View", OutlineKind::Function),
                ("Factory", OutlineKind::Class),
            ],
        ),
        (
            "go",
            "package main\ntype (\nModel struct { value int }\nStore interface { Load() error }\nCount int\nID = string\n)\nconst (\nFirst, Second, Third = 1, 2, 3\nTyped string = \"x\"\nFromValue = First\n)\nfunc main() {}\nfunc (m *Model) Load() error { return nil }\n",
            vec![
                ("main", OutlineKind::Module),
                ("Model", OutlineKind::Struct),
                ("Store", OutlineKind::Interface),
                ("Count", OutlineKind::Type),
                ("ID", OutlineKind::Alias),
                ("First", OutlineKind::Constant),
                ("Second", OutlineKind::Constant),
                ("Third", OutlineKind::Constant),
                ("Typed", OutlineKind::Constant),
                ("FromValue", OutlineKind::Constant),
                ("main", OutlineKind::Function),
                ("Load", OutlineKind::Method),
            ],
        ),
        (
            "bash",
            "#!/usr/bin/env bash\nfunction build { echo build; }\nrelease() ( echo release )\n",
            vec![
                ("build", OutlineKind::Function),
                ("release", OutlineKind::Function),
            ],
        ),
        (
            "java",
            "open module com.example.app { requires java.base; }\nsealed class Base permits Impl { Base(int value) {} int load() { return 1; } }\nfinal class Impl extends Base { Impl() { super(1); } }\nrecord Point(int x, int y) { Point { } int sum() { return x + y; } }\ninterface Store { void save(); }\nenum State { ON, OFF }\n@interface Marker {}\n",
            vec![
                ("com.example.app", OutlineKind::Module),
                ("Base", OutlineKind::Class),
                ("Base", OutlineKind::Method),
                ("load", OutlineKind::Method),
                ("Impl", OutlineKind::Class),
                ("Impl", OutlineKind::Method),
                ("Point", OutlineKind::Class),
                ("Point", OutlineKind::Method),
                ("sum", OutlineKind::Method),
                ("Store", OutlineKind::Interface),
                ("save", OutlineKind::Method),
                ("State", OutlineKind::Enum),
                ("Marker", OutlineKind::Interface),
            ],
        ),
        (
            "kotlin",
            "typealias Id = String\ninterface Store {\n    fun load(): Id\n}\ndata class Model(val id: Id) : Store {\n    override fun load(): Id = id\n    val label = \"model\"\n    constructor(): this(\"default\")\n    companion object Factory {\n        fun create() = Model(\"new\")\n    }\n}\nenum class State {\n    ON,\n    OFF;\n    fun active() = this == ON\n}\nobject Registry {\n    val item = 1\n    fun get() = item\n}\nfun top() = Unit\nval global = 1\n",
            vec![
                ("Id", OutlineKind::Alias),
                ("Store", OutlineKind::Interface),
                ("load", OutlineKind::Method),
                ("Model", OutlineKind::Class),
                ("load", OutlineKind::Method),
                ("label", OutlineKind::Property),
                ("constructor", OutlineKind::Method),
                ("Factory", OutlineKind::Class),
                ("create", OutlineKind::Method),
                ("State", OutlineKind::Enum),
                ("ON", OutlineKind::Constant),
                ("OFF", OutlineKind::Constant),
                ("active", OutlineKind::Method),
                ("Registry", OutlineKind::Class),
                ("item", OutlineKind::Property),
                ("get", OutlineKind::Method),
                ("top", OutlineKind::Function),
                ("global", OutlineKind::Property),
            ],
        ),
        (
            "sql",
            "CREATE TABLE users (id INT);\nCREATE VIEW active_users AS SELECT id FROM users;\nCREATE FUNCTION count_users() RETURNS INT AS 'SELECT 1';\n",
            vec![
                ("users", OutlineKind::Type),
                ("active_users", OutlineKind::Type),
                ("count_users", OutlineKind::Function),
            ],
        ),
        (
            "lua",
            "function top(value) return value end\nfunction Model:load(id) return id end\nhelper = function() return 1 end\n",
            vec![
                ("top", OutlineKind::Function),
                ("load", OutlineKind::Method),
                ("helper", OutlineKind::Function),
            ],
        ),
        (
            "c-sharp",
            "namespace Demo { class Model { int Value { get; set; } Model() {} int Load(int id) { return id; } } struct Point {} interface Store {} enum State { On } record Item(int Id); }\n",
            vec![
                ("Demo", OutlineKind::Module),
                ("Model", OutlineKind::Class),
                ("Value", OutlineKind::Property),
                ("Model", OutlineKind::Method),
                ("Load", OutlineKind::Method),
                ("Point", OutlineKind::Struct),
                ("Store", OutlineKind::Interface),
                ("State", OutlineKind::Enum),
                ("Item", OutlineKind::Class),
            ],
        ),
        (
            "zig",
            "const Model = struct { value: i32, fn load(self: Model) i32 { return self.value; } };\nconst State = enum { on, off };\nfn main() void {}\n",
            vec![
                ("Model", OutlineKind::Struct),
                ("load", OutlineKind::Method),
                ("State", OutlineKind::Enum),
                ("main", OutlineKind::Function),
            ],
        ),
    ];

    for (language, source, expected) in cases {
        let (_, _, _, outline) = document_outline(source, language);
        assert_eq!(outline_entries(&outline), expected, "{language}");
        assert!(!outline.truncated, "small {language} outline was truncated");
    }
}

#[test]
fn decorated_python_and_templated_cpp_keep_their_full_item_ranges() {
    let python = "@trace\ndef top():\n    pass\n\nclass C:\n    @trace\n    def method(self):\n        pass\n";
    let (_, text, _, outline) = document_outline(python, "python");
    assert_eq!(
        outline_entries(&outline),
        [
            ("top", OutlineKind::Function),
            ("C", OutlineKind::Class),
            ("method", OutlineKind::Method),
        ]
    );
    for name in ["top", "method"] {
        let item = outline
            .items
            .iter()
            .find(|item| item.name.as_ref() == name)
            .unwrap();
        assert!(
            text.slice_string(item.range.from, item.range.to)
                .trim_start()
                .starts_with("@trace")
        );
        assert_eq!(
            text.slice_string(item.target.range.from, item.target.range.to),
            name
        );
    }

    let cpp = "template<typename T> T id(T value) { return value; }\ntemplate<typename T> using Ptr = T*;\ntemplate<typename T> concept Number = true;\n";
    let (_, text, _, outline) = document_outline(cpp, "cpp");
    assert_eq!(
        outline_entries(&outline),
        [
            ("id", OutlineKind::Function),
            ("Ptr", OutlineKind::Alias),
            ("Number", OutlineKind::Concept),
        ]
    );
    assert!(outline.items.iter().all(|item| {
        text.slice_string(item.range.from, item.range.to)
            .starts_with("template")
    }));
}

#[test]
fn outline_hierarchy_is_preorder_and_uses_nearest_strict_container() {
    let source = "class Outer:\n    class Inner:\n        def method(self):\n            pass\n    def sibling(self):\n        pass\ndef top():\n    pass\n";
    let (_, _, _, outline) = document_outline(source, "python");

    assert_eq!(
        outline_entries(&outline),
        [
            ("Outer", OutlineKind::Class),
            ("Inner", OutlineKind::Class),
            ("method", OutlineKind::Method),
            ("sibling", OutlineKind::Method),
            ("top", OutlineKind::Function),
        ]
    );
    assert_eq!(
        outline
            .items
            .iter()
            .map(|item| item.parent)
            .collect::<Vec<_>>(),
        [None, Some(0), Some(1), Some(0), None]
    );
}

#[test]
fn outline_targets_are_owned_revision_safe_character_ranges() {
    let source = "def café():\n    pass\n";
    let (registry, mut text, mut syntax, first) = document_outline(source, "python");
    let item = &first.items[0];
    assert_eq!(
        text.slice_string(item.target.range.from, item.target.range.to),
        "café"
    );
    assert_eq!(
        syntax.resolve_selection_range(&text, item.target),
        Ok(item.target.range)
    );

    let before = text.clone();
    let transaction = Transaction::insert(0, "# moved\n");
    text.apply(&transaction);
    assert!(syntax.update(&before, &text, &transaction, &registry));
    assert!(matches!(
        syntax.resolve_selection_range(&text, item.target),
        Err(SyntaxError::StaleRevision { .. })
    ));
    let updated = syntax.outline(&text, &registry).unwrap();
    assert_eq!(updated.revision, syntax.revision());
    assert_eq!(updated.items[0].name.as_ref(), "café");
}

#[test]
fn java_outline_hierarchy_names_and_targets_are_revision_safe() {
    let source =
        "class Café { Café() {} int méthode() { return 1; } class Nested { void run() {} } }\n";
    let (registry, mut text, mut syntax, outline) = document_outline(source, "java");
    assert_eq!(
        outline_entries(&outline),
        [
            ("Café", OutlineKind::Class),
            ("Café", OutlineKind::Method),
            ("méthode", OutlineKind::Method),
            ("Nested", OutlineKind::Class),
            ("run", OutlineKind::Method),
        ]
    );
    assert_eq!(
        outline
            .items
            .iter()
            .map(|item| item.parent)
            .collect::<Vec<_>>(),
        [None, Some(0), Some(0), Some(0), Some(3)]
    );
    let method = outline
        .items
        .iter()
        .find(|item| item.name.as_ref() == "méthode")
        .unwrap();
    assert_eq!(
        text.slice_string(method.target.range.from, method.target.range.to),
        "méthode"
    );
    assert_eq!(
        syntax.resolve_selection_range(&text, method.target),
        Ok(method.target.range)
    );

    let before = text.clone();
    let transaction = Transaction::insert(0, "// changed\n");
    text.apply(&transaction);
    assert!(syntax.update(&before, &text, &transaction, &registry));
    assert!(matches!(
        syntax.resolve_selection_range(&text, method.target),
        Err(SyntaxError::StaleRevision { .. })
    ));
}

#[test]
fn kotlin_outline_hierarchy_unicode_names_and_targets_are_revision_safe() {
    let source = "class Cafe {\n    val étiquette = \"x\"\n    fun méthode() = étiquette\n    class Nested {\n        fun run() {}\n    }\n}\n";
    let (registry, mut text, mut syntax, outline) = document_outline(source, "kotlin");
    assert_eq!(
        outline_entries(&outline),
        [
            ("Cafe", OutlineKind::Class),
            ("étiquette", OutlineKind::Property),
            ("méthode", OutlineKind::Method),
            ("Nested", OutlineKind::Class),
            ("run", OutlineKind::Method),
        ]
    );
    assert_eq!(
        outline
            .items
            .iter()
            .map(|item| item.parent)
            .collect::<Vec<_>>(),
        [None, Some(0), Some(0), Some(0), Some(3)]
    );
    let method = outline
        .items
        .iter()
        .find(|item| item.name.as_ref() == "méthode")
        .unwrap();
    assert_eq!(
        text.slice_string(method.target.range.from, method.target.range.to),
        "méthode"
    );
    assert_eq!(
        syntax.resolve_selection_range(&text, method.target),
        Ok(method.target.range)
    );

    let before = text.clone();
    let transaction = Transaction::insert(0, "// changed\n");
    text.apply(&transaction);
    assert!(syntax.update(&before, &text, &transaction, &registry));
    assert!(matches!(
        syntax.resolve_selection_range(&text, method.target),
        Err(SyntaxError::StaleRevision { .. })
    ));
}

#[test]
fn outline_incremental_result_matches_a_fresh_parse_and_survives_malformed_input() {
    let registry = Registry::new();
    let language = registry.language_for_name("rust").unwrap();
    let mut text = Text::from_str("fn before() {}\nfn broken(\nfn after() {}\n");
    let mut syntax = DocumentSyntax::new(&text, language, &registry).unwrap();
    let malformed = syntax.outline(&text, &registry).unwrap();
    assert!(
        malformed
            .items
            .iter()
            .any(|item| item.name.as_ref() == "before")
    );

    let before = text.clone();
    let at = char_offset(&text.to_string(), "before");
    let transaction = Transaction::change(at, at + "before".chars().count(), "renamed");
    text.apply(&transaction);
    assert!(syntax.update(&before, &text, &transaction, &registry));
    let incremental = syntax.outline(&text, &registry).unwrap();
    let fresh = DocumentSyntax::new(&text, language, &registry).unwrap();
    let fresh = fresh.outline(&text, &registry).unwrap();
    assert_eq!(outline_entries(&incremental), outline_entries(&fresh));
    assert_eq!(
        incremental
            .items
            .iter()
            .map(|item| (item.range, item.target.range, item.parent, item.language))
            .collect::<Vec<_>>(),
        fresh
            .items
            .iter()
            .map(|item| (item.range, item.target.range, item.parent, item.language))
            .collect::<Vec<_>>()
    );
    assert_eq!(incremental.truncated, fresh.truncated);
}

#[test]
fn unsupported_languages_do_not_gain_invented_outlines() {
    for (language, source) in [
        ("html", "<main>text</main>"),
        ("css", "main { color: red; }"),
        ("json", "{\"name\": \"runyte\"}"),
        ("toml", "name = \"runyte\""),
        ("yaml", "name: runyte"),
    ] {
        let (registry, text, syntax) = parse(source, language);
        assert!(matches!(
            syntax.outline(&text, &registry),
            Err(SyntaxError::UnsupportedOutline { language: id })
                if registry.language_name(id) == language
        ));
    }
}

#[test]
fn outline_includes_supported_injections_without_exposing_parser_layers() {
    let source = "<main><script>class View { render() {} } function boot() {}</script></main>";
    let (registry, _, _, outline) = document_outline(source, "html");
    assert_eq!(
        outline_entries(&outline),
        [
            ("View", OutlineKind::Class),
            ("render", OutlineKind::Method),
            ("boot", OutlineKind::Function),
        ]
    );
    assert!(
        outline
            .items
            .iter()
            .all(|item| registry.language_name(item.language) == "javascript")
    );
    assert!(outline.items.iter().all(|item| item.injection_depth == 1));
    assert!(outline.issues.is_empty());
}

#[test]
fn markdown_rust_error_degradation_does_not_invent_an_injected_symbol() {
    let source = "# Outer\n\n```rust\nfn unavailable() {}\n```\n";
    let (registry, _, _, outline) = document_outline(source, "markdown");
    assert!(
        outline
            .items
            .iter()
            .any(|item| item.name.as_ref() == "Outer")
    );
    assert!(
        !outline
            .items
            .iter()
            .any(|item| item.name.as_ref() == "unavailable"),
        "tree-house 0.4 parses nonzero Markdown Rust ranges as ERROR; outline must degrade truthfully"
    );
    assert!(
        outline
            .items
            .iter()
            .all(|item| registry.language_name(item.language) == "markdown")
    );
    assert!(outline.issues.iter().any(|issue| matches!(
        issue,
        OutlineIssue::IncompleteInjectedParse {
            language,
            injection_depth: 1,
            ..
        } if registry.language_name(*language) == "rust"
    )));
}

#[test]
fn unsupported_injected_outline_is_an_owned_issue_while_supported_items_survive() {
    let source = "<script>function boot() {}</script><style>main { color: red; }</style>";
    let (registry, _, _, outline) = document_outline(source, "html");
    assert_eq!(outline_entries(&outline), [("boot", OutlineKind::Function)]);
    assert!(outline.issues.iter().any(|issue| matches!(
        issue,
        OutlineIssue::UnsupportedInjectedLanguage {
            language,
            injection_depth: 1,
        } if registry.language_name(*language) == "css"
    )));
}

#[test]
fn large_document_outline_reports_disabled_injections() {
    let mut source = String::from("# Visible\n\n");
    source.push_str(&"plain text\n".repeat(14_000));
    assert!(source.len() > 128 * 1024);
    let (registry, _, _, outline) = document_outline(&source, "markdown");
    assert_eq!(
        outline_entries(&outline)[0],
        ("Visible", OutlineKind::Heading)
    );
    assert!(outline.issues.iter().any(|issue| matches!(
        issue,
        OutlineIssue::InjectionsDisabled { language }
            if registry.language_name(*language) == "markdown"
    )));
}

#[test]
fn outline_limits_names_item_count_and_hierarchy_depth() {
    let long_name = "x".repeat(300);
    let source = format!("# {long_name}\n");
    let (_, _, _, outline) = document_outline(&source, "markdown");
    assert!(outline.truncated);
    assert_eq!(outline.items[0].name.chars().count(), 256);

    let many = (0..4_200)
        .map(|index| format!("def f{index}():\n    pass\n"))
        .collect::<String>();
    let (_, _, _, outline) = document_outline(&many, "python");
    assert!(outline.truncated);
    assert_eq!(outline.items.len(), 4096);

    let many_long = (0..2_200)
        .map(|index| format!("def n{index}_{}():\n    pass\n", "x".repeat(300)))
        .collect::<String>();
    let (_, _, _, outline) = document_outline(&many_long, "python");
    assert!(outline.truncated);
    assert!(outline.items.len() < 2_200);
    assert!(
        outline
            .items
            .iter()
            .map(|item| item.name.len())
            .sum::<usize>()
            <= 512 * 1024
    );

    let mut deep = String::new();
    for depth in 0..70 {
        deep.push_str(&"    ".repeat(depth));
        deep.push_str(&format!("class C{depth}:\n"));
    }
    deep.push_str(&"    ".repeat(70));
    deep.push_str("pass\n");
    let (_, _, _, outline) = document_outline(&deep, "python");
    assert!(outline.truncated);
    assert_eq!(outline.items.len(), 64);
    assert_eq!(outline.items.last().unwrap().parent, Some(62));
}

#[test]
fn outline_scans_only_the_bounded_source_prefix() {
    let mut source = String::from("fn visible() {}\n/*");
    source.push_str(&"x".repeat(4 * 1024 * 1024));
    source.push_str("*/\nfn hidden() {}\n");
    let (_, _, _, outline) = document_outline(&source, "rust");
    assert!(outline.truncated);
    assert!(
        outline
            .items
            .iter()
            .any(|item| item.name.as_ref() == "visible")
    );
    assert!(
        !outline
            .items
            .iter()
            .any(|item| item.name.as_ref() == "hidden")
    );
}

// -- Indentation and folds -----------------------------------------------

#[test]
fn indentation_and_folds_cover_the_truthful_language_matrix() {
    let supported = [
        ("rust", "fn main() {\n    one();\n}\n"),
        ("python", "def main():\n    one()\n    two()\n"),
        ("swift", "func main() {\n    one()\n}\n"),
        ("c", "int main() {\n    return 0;\n}\n"),
        ("cpp", "namespace demo {\nint value;\n}\n"),
        ("javascript", "function main() {\n    one();\n}\n"),
        ("typescript", "interface Demo {\n    value: string;\n}\n"),
        ("tsx", "<Demo>\n    <Child />\n</Demo>\n"),
        ("html", "<main>\n    text\n</main>\n"),
        ("css", "main {\n    color: red;\n}\n"),
        ("go", "func main() {\n    println()\n}\n"),
        ("bash", "if true; then\n    echo yes\nfi\n"),
        ("java", "class Demo {\n    int value;\n}\n"),
        ("kotlin", "class Demo {\n    val value = 1\n}\n"),
        ("sql", "CREATE TABLE demo (\n    id INT\n);\n"),
        ("lua", "function main()\n    return 1\nend\n"),
        ("c-sharp", "class Demo {\n    int Value;\n}\n"),
        ("zig", "fn main() void {\n    return;\n}\n"),
        ("cmake", "if(TRUE)\n    message(STATUS ok)\nendif()\n"),
        ("proto", "message Demo {\n    string name = 1;\n}\n"),
        ("make", "all:\n\t@echo ok\n\t@echo done\n"),
        ("ini", "[editor]\nname=runyte\ncolor=blue\n"),
        ("json", "{\n    \"value\": 1\n}\n"),
        ("toml", "values = [\n    1,\n    2,\n]\n"),
        ("yaml", "root:\n  child: value\n  other: value\n"),
    ];

    for (language, source) in supported {
        let (registry, text, syntax) = parse(source, language);
        let newline = char_offset(source, "\n");
        let indent = syntax
            .newline_indent(&text, &registry, newline)
            .unwrap_or_else(|error| panic!("{language} indentation failed: {error}"));
        assert!(
            indent.begin_levels + indent.always_levels + indent.tab_levels > 0,
            "{language} returned no indentation captures: {indent:?}"
        );
        assert_eq!(registry.language_name(indent.language), language);
        assert!(syntax.resolve_newline_indent(&text, &indent).is_ok());

        let folds = syntax
            .folds(&text, &registry)
            .unwrap_or_else(|error| panic!("{language} folds failed: {error}"));
        assert!(!folds.items.is_empty(), "{language} produced no fold");
        assert!(folds.items.windows(2).all(|pair| {
            (
                pair[0].range.range.from,
                std::cmp::Reverse(pair[0].range.range.to),
            ) <= (
                pair[1].range.range.from,
                std::cmp::Reverse(pair[1].range.range.to),
            )
        }));
        assert!(folds.items.iter().all(|item| {
            !item.range.range.is_empty() && syntax.resolve_fold_range(&text, item.range).is_ok()
        }));
    }

    let markdown_source = "# Heading\n\nbody\n\nmore\n";
    let (registry, text, syntax) = parse(markdown_source, "markdown");
    let markdown = registry.language_for_name("markdown").unwrap();
    assert!(matches!(
        syntax.newline_indent(&text, &registry, char_offset(markdown_source, "\n")),
        Err(SyntaxError::UnsupportedIndentation { language }) if language == markdown
    ));
    assert!(!syntax.folds(&text, &registry).unwrap().items.is_empty());
}

#[test]
fn java_and_kotlin_indent_and_fold_nested_modern_bodies() {
    for (language, source, nested) in [
        (
            "java",
            "record Demo(int value) {\n    int read() {\n        return value;\n    }\n}\n",
            "int read() {\n",
        ),
        (
            "kotlin",
            "data class Demo(val value: Int) {\n    fun read() = when (value) {\n        0 -> 1\n        else -> value\n    }\n}\n",
            "fun read() = when (value) {\n",
        ),
    ] {
        let (registry, text, syntax) = parse(source, language);
        let newline = char_offset(source, nested) + nested.chars().count() - 1;
        let indent = syntax.newline_indent(&text, &registry, newline).unwrap();
        assert!(indent.always_levels >= 2, "{language}: {indent:?}");
        let folds = syntax.folds(&text, &registry).unwrap();
        assert!(folds.items.len() >= 2, "{language}: {folds:?}");
    }
}

#[test]
fn toml_indents_truthful_containers_and_folds_tables_table_arrays_and_arrays() {
    let source = "[server]\nvalues = [\n    1,\n    2,\n]\n\n[[users]]\nname = \"Ada\"\nroles = [\n    \"admin\",\n]\n";
    let (registry, text, syntax) = parse(source, "toml");
    for marker in ["values = [\n", "roles = [\n"] {
        let newline = char_offset(source, marker) + marker.chars().count() - 1;
        let indent = syntax.newline_indent(&text, &registry, newline).unwrap();
        assert!(indent.begin_levels > 0, "{marker}: {indent:?}");
    }

    let folds = syntax.folds(&text, &registry).unwrap();
    let folded = folds
        .items
        .iter()
        .map(|item| item.range.range.from)
        .collect::<Vec<_>>();
    assert!(folded.len() >= 4, "{folds:?}");
    for header in ["[server]", "values = [", "[[users]]", "roles = ["] {
        let expected = char_offset(source, header) + header.chars().count();
        assert!(
            folded.contains(&expected),
            "missing TOML fold for {header}: {folds:?}"
        );
    }
}

#[test]
fn injected_indentation_uses_the_deepest_supported_language_and_folds_include_it() {
    let source = "<script>function main() {\n    const value = 1;\n}</script>\n";
    let (registry, text, syntax) = parse(source, "html");
    let javascript = registry.language_for_name("javascript").unwrap();
    let newline = char_offset(source, "function main() {\n") + "function main() {".chars().count();
    let indent = syntax.newline_indent(&text, &registry, newline).unwrap();
    assert_eq!(indent.language, javascript);
    assert_eq!(indent.injection_depth, 1);
    assert!(indent.always_levels > 0, "{indent:?}");

    let folds = syntax.folds(&text, &registry).unwrap();
    assert!(
        folds
            .items
            .iter()
            .any(|item| { item.language == javascript && item.injection_depth == 1 })
    );
}

#[test]
fn fold_ranges_keep_a_structural_closing_line_and_its_suffix_visible() {
    let source = "fn main() {\n    one();\n} // suffix must remain visible\n";
    let (registry, text, syntax) = parse(source, "rust");
    let folds = syntax.folds(&text, &registry).unwrap();
    let fold = folds.items.first().expect("function body fold");
    assert_eq!(
        fold.range.range.to,
        text.line_to_offset(2),
        "fold must stop before the complete closing line"
    );
    assert!(
        text.slice_string(fold.range.range.to, text.len_chars())
            .starts_with("} // suffix")
    );
}

#[test]
fn indentation_and_folds_match_fresh_after_malformed_incremental_edits_and_go_stale() {
    let registry = Registry::new();
    let rust = registry.language_for_name("rust").unwrap();
    let source = "fn main() {\n    if true {\n        one();\n    }\n}\n";
    let mut text = Text::from_str(source);
    let mut incremental = DocumentSyntax::new(&text, rust, &registry).unwrap();
    let newline = char_offset(source, "if true {\n") + "if true {".chars().count();
    let old_indent = incremental
        .newline_indent(&text, &registry, newline)
        .unwrap();
    let old_fold = incremental.folds(&text, &registry).unwrap().items[0].range;
    let unrelated = DocumentSyntax::new(&text, rust, &registry).unwrap();
    assert!(matches!(
        unrelated.resolve_newline_indent(&text, &old_indent),
        Err(SyntaxError::ForeignDocument)
    ));
    assert!(matches!(
        unrelated.resolve_fold_range(&text, old_fold),
        Err(SyntaxError::ForeignDocument)
    ));

    let before = text.clone();
    let closing = char_offset(source, "    }\n}\n");
    let transaction = Transaction::delete(closing, text.len_chars());
    text.apply(&transaction);
    assert!(incremental.update(&before, &text, &transaction, &registry));
    assert!(matches!(
        incremental.resolve_newline_indent(&text, &old_indent),
        Err(SyntaxError::StaleRevision { .. })
    ));
    assert!(matches!(
        incremental.resolve_fold_range(&text, old_fold),
        Err(SyntaxError::StaleRevision { .. })
    ));

    let fresh = DocumentSyntax::new(&text, rust, &registry).unwrap();
    let incremental_indent = incremental
        .newline_indent(&text, &registry, newline)
        .unwrap();
    let fresh_indent = fresh.newline_indent(&text, &registry, newline).unwrap();
    assert_eq!(
        (
            incremental_indent.language,
            incremental_indent.injection_depth,
            incremental_indent.begin_levels,
            incremental_indent.always_levels,
            incremental_indent.tab_levels,
            incremental_indent.issues,
            incremental_indent.truncated,
        ),
        (
            fresh_indent.language,
            fresh_indent.injection_depth,
            fresh_indent.begin_levels,
            fresh_indent.always_levels,
            fresh_indent.tab_levels,
            fresh_indent.issues,
            fresh_indent.truncated,
        )
    );
    let ranges = |syntax: &DocumentSyntax| {
        syntax
            .folds(&text, &registry)
            .unwrap()
            .items
            .into_iter()
            .map(|item| (item.range.range, item.language, item.injection_depth))
            .collect::<Vec<_>>()
    };
    assert_eq!(ranges(&incremental), ranges(&fresh));
}

#[test]
fn indentation_and_fold_projection_enforce_source_depth_and_item_bounds() {
    let mut nested = "[".repeat(140);
    nested.push('\n');
    nested.push('0');
    nested.push_str(&"]".repeat(140));
    let (registry, text, syntax) = parse(&nested, "json");
    let indent = syntax
        .newline_indent(&text, &registry, char_offset(&nested, "\n"))
        .unwrap();
    assert_eq!(indent.always_levels, 128);
    assert!(indent.truncated);

    let mut many = String::from("[\n");
    for value in 0..5_000 {
        many.push_str(&format!("[\n{value}\n],\n"));
    }
    many.push_str("]\n");
    let (registry, text, syntax) = parse(&many, "json");
    let folds = syntax.folds(&text, &registry).unwrap();
    assert_eq!(folds.items.len(), 4096);
    assert!(folds.truncated);

    let too_large = " ".repeat(4 * 1024 * 1024 + 1);
    let (registry, text, syntax) = parse(&too_large, "json");
    assert!(matches!(
        syntax.folds(&text, &registry),
        Err(SyntaxError::DocumentTooLarge { .. })
    ));
    assert!(matches!(
        syntax.newline_indent(&text, &registry, 0),
        Err(SyntaxError::DocumentTooLarge { .. })
    ));
}

// -- Structural traversal -------------------------------------------------

#[test]
fn structural_lookup_uses_character_offsets_and_owns_node_values() {
    let source = "// 🦀\nfn αβ() {}\n";
    let (registry, text, syntax) = parse(source, "rust");
    let offset = char_offset(source, "αβ");
    let node = syntax.node_at(&text, &registry, offset).unwrap().unwrap();

    assert_eq!(node.kind.as_str(), "identifier");
    assert_eq!(text.slice_string(node.range.from, node.range.to), "αβ");
    assert_eq!(registry.language_name(node.language), "rust");
    assert!(node.named);
}

#[test]
fn structural_lookup_handles_eof_and_an_empty_document() {
    let (registry, text, syntax) = parse("let value = 1;", "rust");
    let eof = syntax
        .node_at(&text, &registry, text.len_chars())
        .unwrap()
        .unwrap();
    assert_eq!(eof.range.to, text.len_chars());

    let (registry, text, syntax) = parse("", "rust");
    let root = syntax.node_at(&text, &registry, 0).unwrap().unwrap();
    assert_eq!(root.kind.as_str(), "source_file");
    assert_eq!(root.range, SyntaxRange::point(0));
}

#[test]
fn malformed_comments_and_whitespace_remain_structurally_navigable() {
    let source = "// note\nfn broken( {\n";
    let (registry, text, syntax) = parse(source, "rust");

    let comment = syntax.node_at(&text, &registry, 2).unwrap().unwrap();
    assert!(comment.kind.as_str().contains("comment"));

    let whitespace = char_offset(source, " broken");
    let whitespace = syntax
        .node_at(&text, &registry, whitespace)
        .unwrap()
        .unwrap();
    assert!(whitespace.range.from <= char_offset(source, "broken"));
    assert!(whitespace.range.to <= text.len_chars());
}

#[test]
fn traversal_preserves_equal_range_wrappers() {
    let (registry, text, syntax) = parse("x", "rust");
    let node = syntax.node_at(&text, &registry, 0).unwrap().unwrap();
    let ancestors = syntax.ancestors(&text, &registry, &node.path).unwrap();

    assert!(
        ancestors
            .windows(2)
            .any(|pair| pair[0].range == pair[1].range),
        "expected equal-range grammar wrappers, got {ancestors:?}"
    );
}

#[test]
fn named_child_and_siblings_stay_in_their_parser_layer() {
    let source = "fn first() {}\nfn second() {}\n";
    let (registry, text, syntax) = parse(source, "rust");
    let root = syntax
        .node_covering(
            &text,
            &registry,
            SyntaxRange::new(0, text.len_chars()).unwrap(),
        )
        .unwrap()
        .unwrap();
    let first = syntax
        .related(
            &text,
            &registry,
            &root.path,
            SyntaxRelation::FirstNamedChild,
        )
        .unwrap()
        .unwrap();
    let second = syntax
        .next_named_sibling(&text, &registry, &first.path)
        .unwrap()
        .unwrap();
    let previous = syntax
        .previous_named_sibling(&text, &registry, &second.path)
        .unwrap()
        .unwrap();

    assert!(
        text.slice_string(first.range.from, first.range.to)
            .starts_with("fn first")
    );
    assert!(
        text.slice_string(second.range.from, second.range.to)
            .starts_with("fn second")
    );
    assert_eq!(previous.path, first.path);
}

#[test]
fn markdown_fence_boundaries_choose_the_correct_language_layer() {
    let source = "# Title\n\n```rust\nfn main() {}\n```\n";
    let (registry, text, syntax) = parse(source, "markdown");
    let opening = syntax.node_at(&text, &registry, char_offset(source, "```rust"));
    let code = syntax.node_at(&text, &registry, char_offset(source, "fn main"));
    let closing_offset = source[..source.rfind("```").unwrap()].chars().count();
    let closing = syntax.node_at(&text, &registry, closing_offset);

    assert_eq!(
        registry.language_name(opening.unwrap().unwrap().language),
        "markdown"
    );
    assert_eq!(
        registry.language_name(code.unwrap().unwrap().language),
        "rust"
    );
    assert_eq!(
        registry.language_name(closing.unwrap().unwrap().language),
        "markdown"
    );
}

#[test]
fn markdown_fences_resolve_javascript_typescript_and_tsx_markers() {
    let source = "```js\nconst jsValue = 1;\n```\n\
                  ```typescript\nconst tsValue: number = 2;\n```\n\
                  ```tsx\nconst View = () => <div />;\n```\n\
                  ```html\n<runyte-card data-ready=\"yes\"></runyte-card>\n```\n\
                  ```css\n.card { color: red; }\n```\n";
    let (registry, text, syntax) = parse(source, "markdown");

    for (needle, expected) in [
        ("jsValue", "javascript"),
        ("tsValue", "typescript"),
        ("View", "tsx"),
        ("data-ready", "html"),
        ("color", "css"),
    ] {
        let node = syntax
            .node_at(&text, &registry, char_offset(source, needle))
            .unwrap()
            .unwrap();
        assert_eq!(
            registry.language_name(node.language),
            expected,
            "at {needle}"
        );
    }
}

#[test]
fn html_script_and_style_raw_text_use_registered_language_layers() {
    let source = "<main data-kind=\"demo\">\n\
                  <script>const answer = 42;</script>\n\
                  <style>main { color: red; }</style>\n\
                  </main>\n";
    let (registry, text, syntax) = parse(source, "html");

    for (needle, expected) in [
        ("data-kind", "html"),
        ("answer", "javascript"),
        ("color", "css"),
        ("</main>", "html"),
    ] {
        let node = syntax
            .node_at(&text, &registry, char_offset(source, needle))
            .unwrap()
            .unwrap();
        assert_eq!(
            registry.language_name(node.language),
            expected,
            "at {needle}"
        );
    }
    let highlighted = spans_of(&syntax, &text, &registry)
        .into_iter()
        .map(|span| (text.slice_string(span.from, span.to), span.scope.name()))
        .collect::<Vec<_>>();
    assert_scope(&highlighted, "const", "keyword");
    assert_scope(&highlighted, "color", "property");
}

#[test]
fn html_upstream_injection_is_unconditional_for_script_type() {
    let source = "<script type=\"application/json\">{\"answer\": 42}</script>";
    let (registry, text, syntax) = parse(source, "html");
    let node = syntax
        .node_at(&text, &registry, char_offset(source, "answer"))
        .unwrap()
        .unwrap();

    assert_eq!(
        registry.language_name(node.language),
        "javascript",
        "tree-sitter-html 0.23.2 does not inspect the script type attribute"
    );
}

#[test]
fn injected_ancestors_ascend_into_the_enclosing_markdown_node() {
    let source = "# Title\n\n```rust\nfn main() {}\n```\n";
    let (registry, text, syntax) = parse(source, "markdown");
    let code_offset = char_offset(source, "main");
    let leaf = syntax
        .node_at(&text, &registry, code_offset)
        .unwrap()
        .unwrap();
    let ancestors = syntax.ancestors(&text, &registry, &leaf.path).unwrap();
    let first_rust = ancestors
        .iter()
        .position(|node| registry.language_name(node.language) == "rust")
        .expect("injected Rust chain");
    let markdown_parent = &ancestors[first_rust - 1];

    assert_eq!(registry.language_name(markdown_parent.language), "markdown");
    assert!(markdown_parent.range.from > 0);
    assert!(markdown_parent.range.to < text.len_chars());
    assert!(markdown_parent.range.from <= code_offset && code_offset < markdown_parent.range.to);

    let mut current = leaf;
    while registry.language_name(current.language) == "rust" {
        current = syntax
            .parent(&text, &registry, &current.path)
            .unwrap()
            .expect("Rust node must ascend into Markdown");
    }
    assert_eq!(current.path, markdown_parent.path);
}

#[test]
fn structural_paths_are_stale_after_an_incremental_update() {
    let registry = Registry::new();
    let language = registry.language_for_name("rust").unwrap();
    let mut text = Text::from_str("fn main() {}\n");
    let mut syntax = DocumentSyntax::new(&text, language, &registry).unwrap();
    let node = syntax.node_at(&text, &registry, 3).unwrap().unwrap();
    let before = text.clone();
    let transaction = Transaction::insert(0, "pub ");
    text.apply(&transaction);
    assert!(syntax.update(&before, &text, &transaction, &registry));

    assert!(matches!(
        syntax.parent(&text, &registry, &node.path),
        Err(SyntaxError::StaleRevision { .. })
    ));
}

// -- Structural selection ------------------------------------------------

#[test]
fn repeated_expansion_reaches_the_root_through_strict_supersets() {
    let source = "fn main() { let value = call(argument); }\n";
    let (registry, text, syntax) = parse(source, "rust");
    let start = SyntaxRange::point(char_offset(source, "argument"));
    let ranges = expansion_sequence(&syntax, &text, &registry, start);

    assert!(ranges.len() >= 5, "expected syntax levels, got {ranges:?}");
    assert_eq!(
        ranges.last(),
        Some(&SyntaxRange::new(0, text.len_chars()).unwrap())
    );
}

#[test]
fn equal_range_wrappers_are_skipped_instead_of_returning_no_op_steps() {
    let (registry, text, syntax) = parse("x", "rust");
    let identifier = syntax
        .expand_range(&text, &registry, SyntaxRange::point(0))
        .unwrap()
        .unwrap();
    assert_eq!(identifier, SyntaxRange::new(0, 1).unwrap());
    assert_eq!(syntax.expand_range(&text, &registry, identifier), Ok(None));
    assert_eq!(syntax.parent_range(&text, &registry, identifier), Ok(None));
}

#[test]
fn parent_child_and_sibling_ranges_are_grammar_independent() {
    let source = "fn first() {}\nfn second() {}\n";
    let (registry, text, syntax) = parse(source, "rust");
    let root = SyntaxRange::new(0, text.len_chars()).unwrap();
    let first = syntax
        .first_named_child_range(&text, &registry, root)
        .unwrap()
        .unwrap();
    let second = syntax
        .next_named_sibling_range(&text, &registry, first)
        .unwrap()
        .unwrap();

    assert!(
        text.slice_string(first.from, first.to)
            .starts_with("fn first")
    );
    assert!(
        text.slice_string(second.from, second.to)
            .starts_with("fn second")
    );
    assert_eq!(
        syntax.previous_named_sibling_range(&text, &registry, second),
        Ok(Some(first))
    );
    assert_eq!(syntax.parent_range(&text, &registry, first), Ok(Some(root)));
}

#[test]
fn structural_range_tokens_are_stale_after_update() {
    let registry = Registry::new();
    let language = registry.language_for_name("rust").unwrap();
    let mut text = Text::from_str("fn main() { value(); }\n");
    let mut syntax = DocumentSyntax::new(&text, language, &registry).unwrap();
    let selection = syntax
        .selection_range(&text, SyntaxRange::point(12))
        .unwrap();
    let before = text.clone();
    let transaction = Transaction::insert(0, "// changed\n");
    text.apply(&transaction);
    assert!(syntax.update(&before, &text, &transaction, &registry));

    assert!(matches!(
        syntax.resolve_selection_range(&text, selection),
        Err(SyntaxError::StaleRevision { .. })
    ));
    assert!(matches!(
        syntax.transform_selection_range(
            &text,
            &registry,
            selection,
            SyntaxSelectionTransform::Expand,
        ),
        Err(SyntaxError::StaleRevision { .. })
    ));
}

#[test]
fn structural_range_tokens_cannot_cross_documents_at_the_same_revision() {
    let (registry, first_text, first_syntax) = parse("fn first() {}\n", "rust");
    let language = registry.language_for_name("rust").unwrap();
    let second_text = Text::from_str("fn second() {}\n");
    let second_syntax = DocumentSyntax::new(&second_text, language, &registry).unwrap();
    let selection = first_syntax
        .selection_range(&first_text, SyntaxRange::point(3))
        .unwrap();
    assert_eq!(first_syntax.revision(), second_syntax.revision());

    assert_eq!(
        second_syntax.resolve_selection_range(&second_text, selection),
        Err(SyntaxError::ForeignDocument)
    );
}

#[test]
fn recreating_a_parser_invalidates_old_structural_range_tokens() {
    let (registry, text, syntax) = parse("fn main() {}\n", "rust");
    let selection = syntax
        .selection_range(&text, SyntaxRange::point(3))
        .unwrap();
    let language = syntax.language();
    let recreated = DocumentSyntax::new(&text, language, &registry).unwrap();
    assert_eq!(syntax.revision(), recreated.revision());

    assert_eq!(
        recreated.transform_selection_range(
            &text,
            &registry,
            selection,
            SyntaxSelectionTransform::Expand,
        ),
        Err(SyntaxError::ForeignDocument)
    );
}

#[test]
fn incremental_and_full_parses_produce_equivalent_expansion_ranges() {
    let registry = Registry::new();
    let language = registry.language_for_name("rust").unwrap();
    let original = "fn main() { let value = call(argument); }\n";
    let mut text = Text::from_str(original);
    let mut incremental = DocumentSyntax::new(&text, language, &registry).unwrap();
    let before = text.clone();
    let inserted = "// lead\n";
    let transaction = Transaction::insert(0, inserted);
    text.apply(&transaction);
    assert!(incremental.update(&before, &text, &transaction, &registry));
    let fresh = DocumentSyntax::new(&text, language, &registry).unwrap();
    let start = SyntaxRange::point(inserted.chars().count() + char_offset(original, "argument"));

    assert_eq!(
        expansion_sequence(&incremental, &text, &registry, start),
        expansion_sequence(&fresh, &text, &registry, start)
    );
}

#[test]
fn malformed_unicode_source_still_expands_in_character_offsets() {
    let source = "fn привет( { let 🦀 = ; ] }\n";
    let (registry, text, syntax) = parse(source, "rust");
    let ranges = expansion_sequence(
        &syntax,
        &text,
        &registry,
        SyntaxRange::point(char_offset(source, "привет")),
    );

    assert!(ranges.iter().all(|range| range.to <= text.len_chars()));
    assert_eq!(ranges.last().unwrap().to, text.len_chars());
}

#[test]
fn expansion_crosses_from_an_injected_layer_back_to_markdown() {
    let source = "# Title\n\n```rust\nfn main() {}\n```\n";
    let (registry, text, syntax) = parse(source, "markdown");
    let ranges = expansion_sequence(
        &syntax,
        &text,
        &registry,
        SyntaxRange::point(char_offset(source, "main")),
    );
    let languages: Vec<_> = ranges
        .iter()
        .filter_map(|range| syntax.node_covering(&text, &registry, *range).unwrap())
        .map(|node| registry.language_name(node.language))
        .collect();

    assert!(languages.contains(&"rust"), "languages: {languages:?}");
    assert_eq!(languages.last(), Some(&"markdown"));
    assert_eq!(ranges.last().unwrap().to, text.len_chars());
}

#[test]
fn multiple_range_mapping_preserves_reversed_direction_at_the_boundary() {
    let source = "fn first() {}\nfn second() {}\n";
    let (registry, text, syntax) = parse(source, "rust");
    let first = char_offset(source, "first");
    let second = char_offset(source, "second");
    let selections = [
        SelectionRange::new(first, first + "first".chars().count()),
        SelectionRange::new(second + "second".chars().count(), second),
    ];
    let mapped: Vec<_> = selections
        .into_iter()
        .map(|selection| {
            let range = SyntaxRange::new(selection.from(), selection.to()).unwrap();
            let expanded = syntax
                .expand_range(&text, &registry, range)
                .unwrap()
                .unwrap();
            preserve_selection_direction(selection, expanded)
        })
        .collect();

    assert_eq!(mapped.len(), 2);
    assert!(mapped[0].anchor < mapped[0].head);
    assert!(mapped[1].anchor > mapped[1].head);
}

#[test]
fn large_markdown_expansion_explicitly_uses_the_injection_free_tree() {
    let mut source = String::from("# Title\n\n```rust\nfn embedded() {}\n```\n");
    while source.len() <= 128 * 1024 {
        source.push_str("ordinary markdown paragraph\n\n");
    }
    let (registry, text, syntax) = parse(&source, "markdown");
    let offset = char_offset(&source, "embedded");
    let node = syntax.node_at(&text, &registry, offset).unwrap().unwrap();
    assert_eq!(registry.language_name(node.language), "markdown");

    let ranges = expansion_sequence(&syntax, &text, &registry, SyntaxRange::point(offset));
    assert_eq!(ranges.last().unwrap().to, text.len_chars());
}

// -- Structural text objects ---------------------------------------------

#[test]
fn rust_functions_classes_and_parameters_are_owned_captures() {
    let source =
        "struct Café { value: usize }\nfn greet(α: usize, beta: &str) { println!(\"{α}\"); }\n";
    let (text, registry, functions) = text_object_captures(
        source,
        "rust",
        SyntaxObject::Function,
        SyntaxObjectPart::Around,
    );
    assert_eq!(functions.len(), 1);
    assert!(capture_text(&text, &functions[0])[0].starts_with("fn greet"));
    assert_eq!(registry.language_name(functions[0].language), "rust");
    assert_eq!(functions[0].revision, SyntaxRevision::default());

    let (text, _, classes) = text_object_captures(
        source,
        "rust",
        SyntaxObject::Class,
        SyntaxObjectPart::Around,
    );
    assert_eq!(
        capture_text(&text, &classes[0]),
        ["struct Café { value: usize }"]
    );

    let (text, _, parameters) = text_object_captures(
        source,
        "rust",
        SyntaxObject::Parameter,
        SyntaxObjectPart::Inside,
    );
    let parameters: Vec<_> = parameters
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect();
    assert_eq!(parameters, ["α: usize", "beta: &str"]);
}

#[test]
fn grouped_parameter_captures_keep_disjoint_ranges() {
    let source = "fn greet(first: usize, second: usize, third: usize) {}\n";
    let (text, _, captures) = text_object_captures(
        source,
        "rust",
        SyntaxObject::Parameter,
        SyntaxObjectPart::Around,
    );
    let grouped = captures
        .iter()
        .find(|capture| capture.ranges.len() == 2)
        .expect("parameter and comma must remain one grouped match");

    assert_eq!(capture_text(&text, grouped), ["first: usize", ","]);
    assert!(grouped.ranges[0].to < grouped.ranges[1].to);
    let capture_texts: Vec<_> = captures
        .iter()
        .map(|capture| capture_text(&text, capture))
        .collect();
    assert_eq!(
        capture_texts,
        [
            vec!["first: usize".to_owned(), ",".to_owned()],
            vec!["second: usize".to_owned(), ",".to_owned()],
            vec!["third: usize".to_owned()],
        ]
    );
}

#[test]
fn rust_self_parameters_use_the_same_owned_capability() {
    let source = "impl Value { fn get(&self, fallback: usize) -> usize { fallback } }\n";
    let (text, _, captures) = text_object_captures(
        source,
        "rust",
        SyntaxObject::Parameter,
        SyntaxObjectPart::Inside,
    );
    let parameters: Vec<_> = captures
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect();
    assert_eq!(parameters, ["&self", "fallback: usize"]);
}

#[test]
fn python_functions_classes_and_unicode_parameters_are_captured() {
    let source = "class Café:\n    def привет(self, имя: str = \"мир\"):\n        return имя\n";
    let (text, _, classes) = text_object_captures(
        source,
        "python",
        SyntaxObject::Class,
        SyntaxObjectPart::Around,
    );
    assert!(capture_text(&text, &classes[0])[0].starts_with("class Café"));

    let (text, _, functions) = text_object_captures(
        source,
        "python",
        SyntaxObject::Function,
        SyntaxObjectPart::Around,
    );
    assert!(capture_text(&text, &functions[0])[0].contains("def привет"));

    let (text, _, parameters) = text_object_captures(
        source,
        "python",
        SyntaxObject::Parameter,
        SyntaxObjectPart::Inside,
    );
    let parameters: Vec<_> = parameters
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect();
    assert_eq!(parameters, ["self", "имя: str = \"мир\""]);
}

#[test]
fn javascript_functions_classes_and_unicode_parameters_are_captured() {
    let source = r#"class Café {
    méthode(имя, suffix = "!") { return имя + suffix; }
}
const greet = (世界, ...rest) => `${世界}${rest.length}`;
function outer(α, β = 2) { return α + β; }
"#;
    let (text, _, classes) = text_object_captures(
        source,
        "javascript",
        SyntaxObject::Class,
        SyntaxObjectPart::Around,
    );
    assert_eq!(classes.len(), 1);
    assert!(capture_text(&text, &classes[0])[0].starts_with("class Café"));

    let (text, _, functions) = text_object_captures(
        source,
        "javascript",
        SyntaxObject::Function,
        SyntaxObjectPart::Around,
    );
    let functions: Vec<_> = functions
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect();
    assert_eq!(functions.len(), 3, "functions: {functions:?}");
    for expected in ["méthode(", "(世界, ...rest) =>", "function outer("] {
        assert!(
            functions.iter().any(|text| text.contains(expected)),
            "missing {expected:?} in {functions:?}"
        );
    }

    let (text, _, parameters) = text_object_captures(
        source,
        "javascript",
        SyntaxObject::Parameter,
        SyntaxObjectPart::Inside,
    );
    let parameters: Vec<_> = parameters
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect();
    for expected in ["имя", "suffix = \"!\"", "世界", "...rest", "α", "β = 2"] {
        assert!(
            parameters.iter().any(|text| text == expected),
            "missing {expected:?} in {parameters:?}"
        );
    }
}

#[test]
fn typescript_inherits_function_objects_and_adds_signatures_and_abstract_classes() {
    let source = r#"abstract class Repository<T> {
    abstract load(id: string): Promise<T>;
}
declare function map<T>(value: T, fn: (item: T) => T): T;
const transform = (値: number = 1): number => 値 + 1;
"#;
    let (text, _, classes) = text_object_captures(
        source,
        "typescript",
        SyntaxObject::Class,
        SyntaxObjectPart::Around,
    );
    assert_eq!(classes.len(), 1);
    assert!(capture_text(&text, &classes[0])[0].starts_with("abstract class Repository"));

    let (text, _, functions) = text_object_captures(
        source,
        "typescript",
        SyntaxObject::Function,
        SyntaxObjectPart::Around,
    );
    let functions: Vec<_> = functions
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect();
    for expected in ["abstract load(", "function map", "(値: number = 1)"] {
        assert!(
            functions.iter().any(|text| text.contains(expected)),
            "missing {expected:?} in {functions:?}"
        );
    }

    let (text, _, parameters) = text_object_captures(
        source,
        "typescript",
        SyntaxObject::Parameter,
        SyntaxObjectPart::Inside,
    );
    let parameters: Vec<_> = parameters
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect();
    for expected in [
        "id: string",
        "value: T",
        "fn: (item: T) => T",
        "item: T",
        "値: number = 1",
    ] {
        assert!(
            parameters.iter().any(|text| text == expected),
            "missing {expected:?} in {parameters:?}"
        );
    }
}

#[test]
fn go_functions_class_like_types_and_grouped_parameters_are_truthful() {
    let source = r#"package demo
type Model struct { value int }
type Store interface { Load(key string) (string, error) }
type Empty interface {}
func top(x, y int, label string) {}
func (m *Model) Load(key string) (string, error) { return "", nil }
var callback = func(value int) int { return value }
"#;

    let (text, _, functions) = text_object_captures(
        source,
        "go",
        SyntaxObject::Function,
        SyntaxObjectPart::Around,
    );
    let functions = functions
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    assert_eq!(functions.len(), 3, "{functions:?}");
    for expected in ["func top(", "func (m *Model) Load(", "func(value int)"] {
        assert!(
            functions.iter().any(|text| text.contains(expected)),
            "missing {expected:?} in {functions:?}"
        );
    }

    let (text, _, function_bodies) = text_object_captures(
        source,
        "go",
        SyntaxObject::Function,
        SyntaxObjectPart::Inside,
    );
    let function_bodies = function_bodies
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    assert_eq!(function_bodies.len(), 3, "{function_bodies:?}");
    assert!(function_bodies.iter().all(|body| body.starts_with('{')));
    assert!(function_bodies.iter().all(|body| body.ends_with('}')));

    let (text, _, classes) =
        text_object_captures(source, "go", SyntaxObject::Class, SyntaxObjectPart::Around);
    let classes = classes
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    assert_eq!(classes.len(), 3, "{classes:?}");
    assert!(classes.iter().any(|text| text.starts_with("Model struct")));
    assert!(
        classes
            .iter()
            .any(|text| text.starts_with("Store interface"))
    );
    assert!(
        classes
            .iter()
            .any(|text| text.starts_with("Empty interface"))
    );

    let (text, _, class_bodies) =
        text_object_captures(source, "go", SyntaxObject::Class, SyntaxObjectPart::Inside);
    let class_bodies = class_bodies
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    assert_eq!(
        class_bodies.len(),
        2,
        "an empty interface has no invented inside object: {class_bodies:?}"
    );
    assert!(class_bodies.iter().any(|body| body.contains("value int")));
    assert!(
        class_bodies
            .iter()
            .any(|body| body.contains("Load(key string)"))
    );

    let (text, _, parameters) = text_object_captures(
        source,
        "go",
        SyntaxObject::Parameter,
        SyntaxObjectPart::Inside,
    );
    let parameters = parameters
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    assert!(
        parameters.iter().any(|text| text == "x, y int"),
        "a grouped declaration must remain one parameter object: {parameters:?}"
    );
    for expected in ["label string", "m *Model", "key string", "string", "error"] {
        assert!(
            parameters.iter().any(|text| text == expected),
            "missing {expected:?} in {parameters:?}"
        );
    }
}

#[test]
fn bash_claims_function_objects_and_explicitly_rejects_class_and_parameter_objects() {
    let source =
        "#!/usr/bin/env bash\nbuild() { echo build; }\nfunction release { echo release; }\n";
    let (text, registry, functions) = text_object_captures(
        source,
        "bash",
        SyntaxObject::Function,
        SyntaxObjectPart::Around,
    );
    let functions = functions
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    assert_eq!(functions.len(), 2, "{functions:?}");
    assert!(functions[0].starts_with("build()"));
    assert!(functions[1].starts_with("function release"));

    let language = registry.language_for_name("bash").unwrap();
    let syntax = DocumentSyntax::new(&text, language, &registry).unwrap();
    for object in [SyntaxObject::Class, SyntaxObject::Parameter] {
        assert!(matches!(
            syntax.text_object_captures(
                &text,
                &registry,
                object,
                SyntaxObjectPart::Around,
                SyntaxRange::new(0, text.len_chars()).unwrap(),
            ),
            Err(SyntaxError::UnsupportedTextObject { language: id, object: unsupported, .. })
                if id == language && unsupported == object
        ));
    }
}

#[test]
fn java_structural_objects_cover_modern_declarations_and_parameter_forms() {
    let source = r#"sealed class Café permits Child {
    Café(Café this, int count, String... names) {}
    int méthode(int value) { return value; }
}
record Child(int x, int y) implements Store {
    Child { }
    public void save() {}
}
interface Store { void save(); }
enum State { ON, OFF }
@interface Marker {}
class Lambdas {
    java.util.function.Function<String, String> one = имя -> имя.trim();
    java.util.function.BiFunction<Integer, Integer, Integer> two = (left, right) -> left + right;
}
"#;

    let (text, _, classes) = text_object_captures(
        source,
        "java",
        SyntaxObject::Class,
        SyntaxObjectPart::Around,
    );
    let classes = classes
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    assert_eq!(classes.len(), 6, "{classes:?}");
    for expected in [
        "sealed class Café",
        "record Child",
        "interface Store",
        "enum State",
        "@interface Marker",
        "class Lambdas",
    ] {
        assert!(
            classes.iter().any(|text| text.starts_with(expected)),
            "missing {expected:?} in {classes:?}"
        );
    }

    let (text, _, class_bodies) = text_object_captures(
        source,
        "java",
        SyntaxObject::Class,
        SyntaxObjectPart::Inside,
    );
    assert_eq!(class_bodies.len(), 6);
    assert!(class_bodies.iter().all(|capture| {
        capture_text(&text, capture)
            .iter()
            .all(|body| body.starts_with('{') && body.ends_with('}'))
    }));

    let (text, _, functions) = text_object_captures(
        source,
        "java",
        SyntaxObject::Function,
        SyntaxObjectPart::Around,
    );
    let functions = functions
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    for expected in [
        "Café(Café this",
        "int méthode(",
        "Child {",
        "void save()",
        "имя ->",
        "(left, right) ->",
    ] {
        assert!(
            functions.iter().any(|text| text.contains(expected)),
            "missing {expected:?} in {functions:?}"
        );
    }

    let (text, _, parameters) = text_object_captures(
        source,
        "java",
        SyntaxObject::Parameter,
        SyntaxObjectPart::Inside,
    );
    let parameters = parameters
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    for expected in [
        "Café this",
        "int count",
        "String... names",
        "int value",
        "int x",
        "int y",
        "имя",
        "left",
        "right",
    ] {
        assert!(
            parameters.iter().any(|text| text == expected),
            "missing {expected:?} in {parameters:?}"
        );
    }
}

#[test]
fn kotlin_structural_objects_cover_declarations_constructors_and_lambda_parameters() {
    let source = r#"data class Café(val id: Int, private var имя: String) {
    constructor(id: Int): this(id, "default")
    fun méthode(value: Int, transform: (Int) -> Int = { it }) = transform(value)
    companion object Factory { fun create() = Café(1, "new") }
}
interface Store {
    fun load(key: String): String
}
enum class State {
    ON,
    OFF
}
object Registry {
    val mapper = fun(input: Int): Int { return input }
}
val pair = { left: Int, right: Int -> left + right }
val destructured = { (left, right): Pair<Int, Int> -> left + right }
"#;

    let (text, _, classes) = text_object_captures(
        source,
        "kotlin",
        SyntaxObject::Class,
        SyntaxObjectPart::Around,
    );
    let classes = classes
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    assert_eq!(classes.len(), 5, "{classes:?}");
    for expected in [
        "data class Café",
        "companion object Factory",
        "interface Store",
        "enum class State",
        "object Registry",
    ] {
        assert!(
            classes.iter().any(|text| text.starts_with(expected)),
            "missing {expected:?} in {classes:?}"
        );
    }

    let (text, _, functions) = text_object_captures(
        source,
        "kotlin",
        SyntaxObject::Function,
        SyntaxObjectPart::Around,
    );
    let functions = functions
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    for expected in [
        "constructor(id: Int)",
        "fun méthode(",
        "fun create()",
        "fun load(",
        "fun(input: Int)",
        "{ left: Int, right: Int ->",
        "{ (left, right): Pair<Int, Int> ->",
    ] {
        assert!(
            functions.iter().any(|text| text.contains(expected)),
            "missing {expected:?} in {functions:?}"
        );
    }

    let (text, _, parameters) = text_object_captures(
        source,
        "kotlin",
        SyntaxObject::Parameter,
        SyntaxObjectPart::Inside,
    );
    let parameters = parameters
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    for expected in [
        "val id: Int",
        "private var имя: String",
        "id: Int",
        "value: Int",
        "transform: (Int) -> Int",
        "key: String",
        "input: Int",
        "left: Int",
        "right: Int",
        "(left, right)",
    ] {
        assert!(
            parameters.iter().any(|text| text == expected),
            "missing {expected:?} in {parameters:?}"
        );
    }
}

#[test]
fn kotlin_defaulted_and_typed_destructured_parameters_have_truthful_around_ranges() {
    let source = r#"fun configure(
    first: String = "α",
    middle: Int = listOf(1, 2).sum(),
    last: String = factory("世界", mapOf("κ" to 1))
) = first + middle + last
val typedFirst = { (left, right): Pair<Int, Int>, suffix: String -> suffix }
val typedLast = { prefix: String, (α, β): Pair<String, String> -> prefix }
"#;

    let (text, _, inside) = text_object_captures(
        source,
        "kotlin",
        SyntaxObject::Parameter,
        SyntaxObjectPart::Inside,
    );
    let inside = inside
        .iter()
        .map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    for expected in [
        vec!["first: String"],
        vec!["middle: Int"],
        vec!["last: String"],
        vec!["(left, right)"],
        vec!["(α, β)"],
    ] {
        assert!(
            inside.iter().any(|capture| capture == &expected),
            "missing inside capture {expected:?} in {inside:?}"
        );
    }

    let (text, _, around) = text_object_captures(
        source,
        "kotlin",
        SyntaxObject::Parameter,
        SyntaxObjectPart::Around,
    );
    let around = around
        .iter()
        .map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    for expected in [
        vec!["first: String", "=", "\"α\"", ","],
        vec!["middle: Int", "=", "listOf(1, 2).sum()", ","],
        vec!["last: String", "=", "factory(\"世界\", mapOf(\"κ\" to 1))"],
        vec!["(left, right)", ":", "Pair<Int, Int>", ","],
        vec!["(α, β)", ":", "Pair<String, String>"],
    ] {
        assert!(
            around.iter().any(|capture| capture == &expected),
            "missing around capture {expected:?} in {around:?}"
        );
    }

    assert!(
        around.iter().flatten().all(|range| range.trim() == range),
        "grouped ranges must not absorb inter-node whitespace: {around:?}"
    );
}

#[test]
fn kotlin_context_receiver_prefix_is_an_explicit_structural_limitation() {
    let source = "context(Logger)\nfun String.render(value: Int) = this + value\n";
    let (text, _, functions) = text_object_captures(
        source,
        "kotlin",
        SyntaxObject::Function,
        SyntaxObjectPart::Around,
    );
    let functions = functions
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect::<Vec<_>>();
    assert_eq!(functions, ["fun String.render(value: Int) = this + value"]);
    assert!(
        !functions[0].contains("context(Logger)"),
        "tree-sitter-kotlin-sg 0.4.1 parses a context receiver as a separate call"
    );
}

#[test]
fn lua_c_sharp_and_zig_structural_objects_match_real_declarations() {
    let cases = [
        (
            "lua",
            "local Model = { value = 1 }\nfunction Model:load(id, fallback) return id or fallback end\n",
            "function Model:load",
            "{ value = 1 }",
            "id",
        ),
        (
            "c-sharp",
            "class Model { int Load(int id, int fallback) { return id; } }\n",
            "int Load",
            "class Model",
            "int id",
        ),
        (
            "zig",
            "const Model = struct { value: i32 };\nfn load(id: i32, fallback: i32) i32 { return id; }\n",
            "fn load",
            "const Model = struct",
            "id: i32",
        ),
    ];

    for (language, source, function_needle, class_needle, parameter_needle) in cases {
        for (object, needle) in [
            (SyntaxObject::Function, function_needle),
            (SyntaxObject::Class, class_needle),
            (SyntaxObject::Parameter, parameter_needle),
        ] {
            let (text, _, captures) =
                text_object_captures(source, language, object, SyntaxObjectPart::Around);
            let captured = captures
                .iter()
                .map(|capture| capture_text(&text, capture).join(""))
                .collect::<Vec<_>>();
            assert!(
                captured.iter().any(|text| text.contains(needle)),
                "{language} {object:?} did not capture {needle:?}: {captured:?}"
            );
        }
    }
}

#[test]
fn markdown_sections_and_paragraphs_are_captured() {
    let source = "# One\n\nFirst paragraph.\n\n## Two\n\nSecond paragraph.\n";
    let (text, _, sections) = text_object_captures(
        source,
        "markdown",
        SyntaxObject::Section,
        SyntaxObjectPart::Around,
    );
    assert!(sections.len() >= 2, "sections: {sections:?}");
    let section_text: Vec<_> = sections
        .iter()
        .map(|capture| capture_text(&text, capture).join(""))
        .collect();
    assert!(section_text.iter().any(|text| text.contains("# One")));
    assert!(section_text.iter().any(|text| text.contains("## Two")));

    let (text, _, paragraphs) = text_object_captures(
        source,
        "markdown",
        SyntaxObject::Paragraph,
        SyntaxObjectPart::Around,
    );
    let paragraph_text: Vec<_> = paragraphs
        .iter()
        .flat_map(|capture| capture_text(&text, capture))
        .collect();
    assert!(
        paragraph_text
            .iter()
            .any(|text| text.trim_end() == "First paragraph.")
    );
    assert!(
        paragraph_text
            .iter()
            .any(|text| text.trim_end() == "Second paragraph.")
    );
}

#[test]
fn injected_rust_functions_degrade_instead_of_over_selecting_the_fence() {
    let source = "# Notes\n\n```rust\nfn first() {}\nfn second() {}\n```\n";
    let (registry, text, syntax) = parse(source, "markdown");
    let rust = registry.language_for_name("rust").unwrap();
    let injected_errors: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|name| {
            let leaf = syntax
                .node_at(&text, &registry, char_offset(source, name))
                .unwrap()
                .unwrap();
            syntax
                .ancestors(&text, &registry, &leaf.path)
                .unwrap()
                .into_iter()
                .find(|node| node.language == rust && node.kind.as_str() == "ERROR")
                .expect("pinned tree-house produces an injected Rust ERROR node")
        })
        .collect();
    assert_eq!(
        injected_errors[0].range, injected_errors[1].range,
        "the pinned parser does not distinguish the two injected functions"
    );
    let first_end = char_offset(source, "}\nfn second") + 1;
    let second_end = char_offset(source, "}\n```") + 1;
    assert!(
        injected_errors[0].range.to > first_end && injected_errors[0].range.to < second_end,
        "the shared ERROR range must cross one function boundary and stop inside another"
    );
    assert!(matches!(
        syntax.text_object_captures(
            &text,
            &registry,
            SyntaxObject::Function,
            SyntaxObjectPart::Around,
            SyntaxRange::new(0, text.len_chars()).unwrap(),
        ),
        Err(SyntaxError::UnsupportedTextObject { language, .. }) if language == rust
    ));
}

#[test]
fn unsupported_text_objects_and_invalid_search_ranges_are_explicit() {
    let (registry, text, syntax) = parse("let value = 1\n", "swift");
    assert!(matches!(
        syntax.text_object_captures(
            &text,
            &registry,
            SyntaxObject::Function,
            SyntaxObjectPart::Around,
            SyntaxRange::new(0, text.len_chars()).unwrap(),
        ),
        Err(SyntaxError::UnsupportedTextObject { .. })
    ));
    assert!(matches!(
        syntax.text_object_captures(
            &text,
            &registry,
            SyntaxObject::Function,
            SyntaxObjectPart::Around,
            SyntaxRange {
                from: 2,
                to: text.len_chars() + 1,
            },
        ),
        Err(SyntaxError::CharacterOffsetOutOfBounds { .. })
    ));
}

#[test]
fn html_and_css_do_not_invent_declaration_text_objects() {
    for (language, source) in [
        ("html", "<section class=\"card\"></section>"),
        ("css", ".card { color: var(--accent); }"),
    ] {
        let (registry, text, syntax) = parse(source, language);
        for object in [
            SyntaxObject::Function,
            SyntaxObject::Class,
            SyntaxObject::Parameter,
        ] {
            assert!(matches!(
                syntax.text_object_captures(
                    &text,
                    &registry,
                    object,
                    SyntaxObjectPart::Around,
                    SyntaxRange::new(0, text.len_chars()).unwrap(),
                ),
                Err(SyntaxError::UnsupportedTextObject { .. })
            ));
        }
    }
}

#[test]
fn match_limit_is_not_a_total_capture_limit() {
    let source: String = (0..300)
        .map(|index| format!("fn f{index}() {{}}\n"))
        .collect();
    let (_, _, captures) = text_object_captures(
        &source,
        "rust",
        SyntaxObject::Function,
        SyntaxObjectPart::Around,
    );
    assert_eq!(captures.len(), 300);
}

fn assert_scope(scopes: &[(String, &str)], text: &str, scope: &str) {
    assert!(
        scopes
            .iter()
            .any(|(content, name)| content == text && *name == scope),
        "expected {text:?} to be {scope:?}, got {scopes:?}"
    );
}

// -- Per-language correctness ---------------------------------------------

#[test]
fn rust_highlights_keywords_comments_strings_and_types() {
    let scopes = scopes(
        "// note\nfn main() {\n    let s: String = \"hi\".into();\n}\n",
        "rust",
    );
    assert_scope(&scopes, "// note", "comment");
    assert_scope(&scopes, "fn", "keyword");
    assert_scope(&scopes, "let", "keyword");
    assert_scope(&scopes, "main", "function");
    assert_scope(&scopes, "String", "type");
    assert!(
        scopes
            .iter()
            .any(|(content, name)| content.contains("hi") && *name == "string"),
        "expected a string highlight, got {scopes:?}"
    );
}

#[test]
fn python_highlights_keywords_comments_strings_and_functions() {
    let scopes = scopes(
        "# note\ndef greet(name: str) -> str:\n    return f\"Hello, {name}\"\n",
        "python",
    );
    assert_scope(&scopes, "# note", "comment");
    assert_scope(&scopes, "def", "keyword");
    assert_scope(&scopes, "greet", "function");
    assert_scope(&scopes, "return", "keyword");
    assert!(
        scopes.iter().any(|(_, name)| *name == "string"),
        "expected a string highlight, got {scopes:?}"
    );
}

#[test]
fn javascript_highlights_modern_syntax_jsx_and_unicode() {
    let scopes = scopes(
        r#"// привет
const greet = async (name = "世界") => await Promise.resolve(name);
class View { render() { return <section data-id="界">{greet("мир")}</section>; } }
"#,
        "javascript",
    );
    assert_scope(&scopes, "// привет", "comment");
    assert_scope(&scopes, "const", "keyword");
    assert_scope(&scopes, "async", "keyword");
    assert_scope(&scopes, "await", "keyword");
    assert_scope(&scopes, "greet", "function");
    assert_scope(&scopes, "section", "tag");
    assert_scope(&scopes, "data-id", "attribute");
}

#[test]
fn typescript_inherits_javascript_and_adds_type_syntax() {
    let scopes = scopes(
        "interface Greeting<T> { greet(name?: string): Promise<T>; }\n\
         abstract class Greeter<T> implements Greeting<T> {\n\
             abstract greet(name?: string): Promise<T>;\n\
         }\n",
        "typescript",
    );
    for keyword in ["interface", "abstract", "class", "implements"] {
        assert_scope(&scopes, keyword, "keyword");
    }
    assert_scope(&scopes, "Greeting", "type");
    assert_scope(&scopes, "string", "type");
}

#[test]
fn tsx_composes_jsx_and_typescript_highlights() {
    let scopes = scopes(
        "type Props = { title: string };\n\
         export const Card = ({ title }: Props) => (\n\
             <article aria-label={title}>{title}</article>\n\
         );\n",
        "tsx",
    );
    assert_scope(&scopes, "type", "keyword");
    assert_scope(&scopes, "Props", "type");
    // The TypeScript addition follows the JavaScript base by design, so its
    // uppercase-identifier type capture wins for this component name.
    assert_scope(&scopes, "Card", "type");
    assert_scope(&scopes, "article", "tag");
    assert_scope(&scopes, "aria-label", "attribute");
}

#[test]
fn html_highlights_document_structure_attributes_and_unicode() {
    let scopes = scopes(
        "<!doctype html>\n<!-- привет -->\n<runyte-card data-title=\"世界\">Hello</runyte-card>\n",
        "html",
    );
    assert!(
        scopes
            .iter()
            .any(|(content, name)| content.contains("doctype") && *name == "constant"),
        "expected the doctype to be highlighted, got {scopes:?}"
    );
    assert_scope(&scopes, "<!-- привет -->", "comment");
    assert_scope(&scopes, "runyte-card", "tag");
    assert_scope(&scopes, "data-title", "attribute");
    assert_scope(&scopes, "世界", "string");
}

#[test]
fn css_highlights_modern_selectors_custom_properties_and_functions() {
    let source = ":root { --accent: #c0ffee; }\n\
         @scope (.card) to (.panel) {\n\
           & > .title:where([lang=\"pl\"]) { color: var(--accent); }\n\
         }\n\
         @container layout (inline-size > 30rem) { .card { display: grid; } }\n";
    let scopes = scopes(source, "css");
    assert_scope(&scopes, "--accent", "variable");
    assert_scope(&scopes, "var", "function");
    assert_scope(&scopes, "&", "tag");
    assert_scope(&scopes, "color", "property");
    assert_scope(&scopes, "@container", "keyword");

    let (registry, text, syntax) = parse(source, "css");
    for (needle, expected_ancestor) in [(".card", "scope_statement"), ("@container", "at_rule")] {
        let node = syntax
            .node_at(&text, &registry, char_offset(source, needle))
            .unwrap()
            .unwrap();
        let ancestors = syntax.ancestors(&text, &registry, &node.path).unwrap();
        assert!(
            ancestors
                .iter()
                .any(|ancestor| ancestor.kind.as_str() == expected_ancestor),
            "expected {needle:?} beneath {expected_ancestor}, got {ancestors:?}"
        );
    }
}

#[test]
fn go_highlights_functions_methods_types_strings_and_comments() {
    let scopes = scopes(
        "// note\npackage main\ntype Café struct{}\nfunc (c Café) Greet(name string) string { return \"hello\" }\n",
        "go",
    );
    assert_scope(&scopes, "// note", "comment");
    assert_scope(&scopes, "package", "keyword");
    assert_scope(&scopes, "func", "keyword");
    assert_scope(&scopes, "Greet", "function");
    assert_scope(&scopes, "Café", "type");
    assert_scope(&scopes, "\"hello\"", "string");
}

#[test]
fn bash_highlights_functions_keywords_strings_and_comments() {
    let scopes = scopes(
        "#!/usr/bin/env bash\n# note\nbuild() { if true; then echo \"hello\"; fi; }\n",
        "bash",
    );
    assert!(
        scopes
            .iter()
            .any(|(text, scope)| text.contains("note") && *scope == "comment"),
        "{scopes:?}"
    );
    assert_scope(&scopes, "build", "function");
    assert_scope(&scopes, "if", "keyword");
    assert_scope(&scopes, "then", "keyword");
    assert_scope(&scopes, "\"hello\"", "string");
}

#[test]
fn java_highlights_modern_declarations_patterns_unicode_and_comments() {
    let scopes = scopes(
        "// привет\nsealed interface Shape permits Point {}\nrecord Point(int x, int y) implements Shape {}\nfinal class Demo { String label = \"世界\"; int area(Object value) { return switch (value) { case Point(int x, int y) when x > 0 -> x * y; default -> 0; }; } }\n",
        "java",
    );
    assert_scope(&scopes, "// привет", "comment");
    for keyword in [
        "sealed",
        "interface",
        "permits",
        "record",
        "implements",
        "switch",
        "case",
        "when",
    ] {
        assert_scope(&scopes, keyword, "keyword");
    }
    assert_scope(&scopes, "Shape", "type");
    assert_scope(&scopes, "Point", "type");
    assert_scope(&scopes, "area", "function");
    assert_scope(&scopes, "\"世界\"", "string");
}

#[test]
fn kotlin_highlights_kotlin2_strings_when_guards_and_owned_scope_aliases() {
    let scopes = scopes(
        r#"// привет
import kotlin.math.abs
data class Point(val x: Int, val y: Int)
fun render(name: String): String = $$"""
    literal $name and interpolation $$name and expression $${name.uppercase()}
""".trimIndent()
fun guarded(value: Any) = when (value) {
    is String if value.isNotEmpty() -> value
    else -> null
}
val ratio = 1.5
val marker = '界'
val ready = true
"#,
        "kotlin",
    );
    assert_scope(&scopes, "// привет", "comment");
    assert_scope(&scopes, "import", "keyword");
    assert_scope(&scopes, "data", "keyword");
    assert_scope(&scopes, "class", "keyword");
    assert_scope(&scopes, "fun", "keyword");
    assert_scope(&scopes, "when", "keyword");
    assert_scope(&scopes, "if", "keyword");
    assert_scope(&scopes, "1.5", "number");
    assert_scope(&scopes, "'界'", "string");
    assert_scope(&scopes, "true", "constant");
    assert!(
        scopes
            .iter()
            .any(|(content, scope)| content.contains("literal $name") && *scope == "string"),
        "expected Kotlin 2 multi-dollar string highlighting, got {scopes:?}"
    );
}

#[test]
fn swift_highlights_keywords_comments_strings_and_types() {
    let scopes = scopes(
        "// note\nstruct Greeting {\n    let text: String\n    func render() -> String { text }\n}\n",
        "swift",
    );
    assert_scope(&scopes, "// note", "comment");
    assert_scope(&scopes, "struct", "keyword");
    assert_scope(&scopes, "func", "keyword");
    assert_scope(&scopes, "Greeting", "type");
    assert_scope(&scopes, "String", "type");
}

#[test]
fn c_highlights_keywords_comments_strings_and_functions() {
    let scopes = scopes(
        "// note\nconst char *greet(void) { return \"hello\"; }\n",
        "c",
    );
    assert_scope(&scopes, "// note", "comment");
    assert_scope(&scopes, "const", "keyword");
    assert_scope(&scopes, "return", "keyword");
    assert_scope(&scopes, "greet", "function");
    assert_scope(&scopes, "\"hello\"", "string");
}

#[test]
fn cpp_inherits_c_highlights_and_adds_cpp_constructs() {
    let scopes = scopes(
        "// note\nclass Greeting {\npublic:\n    std::string render() const { return \"hello\"; }\n};\n",
        "cpp",
    );
    assert_scope(&scopes, "// note", "comment");
    assert_scope(&scopes, "class", "keyword");
    assert_scope(&scopes, "public", "keyword");
    assert_scope(&scopes, "return", "keyword");
    assert_scope(&scopes, "\"hello\"", "string");
}

#[test]
fn additional_language_grammars_highlight_representative_documents() {
    for (language, source, text, scope) in [
        (
            "sql",
            "-- note\nSELECT name FROM users;\n",
            "SELECT",
            "keyword",
        ),
        (
            "lua",
            "-- note\nlocal function greet(name) return name end\n",
            "greet",
            "function",
        ),
        (
            "c-sharp",
            "// note\nclass Greeting { string Text = \"hi\"; }\n",
            "Greeting",
            "type",
        ),
        (
            "zig",
            "// note\nfn greet() void { return; }\n",
            "greet",
            "function",
        ),
        ("cmake", "# note\nproject(runyte)\n", "project", "function"),
        (
            "proto",
            "// note\nmessage Greeting { string text = 1; }\n",
            "message",
            "keyword",
        ),
        ("make", "# note\nall:\n\t@echo ok\n", "# note", "comment"),
        ("ini", "; note\n[editor]\nname=runyte\n", "name", "property"),
    ] {
        let scopes = scopes(source, language);
        assert_scope(&scopes, text, scope);
    }
}

#[test]
fn additional_language_highlights_keep_semantic_captures_after_helper_captures() {
    for (language, source, comment) in [
        ("sql", "-- sql\nSELECT 1;\n", "-- sql"),
        ("zig", "// zig\nconst value = 1;\n", "// zig"),
        ("cmake", "# cmake\nproject(runyte)\n", "# cmake"),
        ("ini", "; ini\nname=runyte\n", "; ini\n"),
    ] {
        assert_scope(&scopes(source, language), comment, "comment");
    }

    let cmake = scopes(
        "#!/usr/bin/env cmake\nset(VAR value)\nlist(APPEND VAR next)\n",
        "cmake",
    );
    assert_scope(&cmake, "#!/usr/bin/env cmake", "keyword");
    assert_scope(&cmake, "set", "function");
    assert_scope(&cmake, "list", "function");
}

#[test]
fn json_highlights_keys_strings_and_numbers() {
    let scopes = scopes(r#"{"name": "runyte", "count": 42, "ok": true}"#, "json");
    assert!(
        scopes.iter().any(|(_, name)| *name == "string"),
        "expected string highlights, got {scopes:?}"
    );
    assert!(
        scopes
            .iter()
            .any(|(content, name)| content == "42" && (*name == "number" || *name == "constant")),
        "expected 42 to be a number or constant, got {scopes:?}"
    );
}

#[test]
fn toml_highlights_tables_and_values() {
    let scopes = scopes("# comment\n[package]\nname = \"runyte\"\n", "toml");
    assert_scope(&scopes, "# comment", "comment");
    assert!(
        scopes.iter().any(|(_, name)| *name == "string"),
        "expected a string highlight, got {scopes:?}"
    );
}

#[test]
fn yaml_highlights_keys_and_comments() {
    let scopes = scopes("# comment\neditor:\n  tab_width: 4\n", "yaml");
    assert_scope(&scopes, "# comment", "comment");
    assert!(
        !scopes.is_empty(),
        "expected yaml highlights, got {scopes:?}"
    );
}

#[test]
fn markdown_highlights_structure() {
    let scopes = scopes(
        "# Title\n\nSetext\n======\n\n> quoted\n\n- item\n\nSome *italic*, _also italic_, **bold**, __also bold__, `code`, and [label](https://example.com).\n",
        "markdown",
    );

    assert_scope(&scopes, "Title", "markup.heading");
    assert_scope(&scopes, "Setext\n======", "markup.heading");
    assert_scope(&scopes, "> ", "markup.quote");
    assert_scope(&scopes, "- ", "markup.list");
    assert_scope(&scopes, "italic", "markup.italic");
    assert_scope(&scopes, "also italic", "markup.italic");
    assert_scope(&scopes, "bold", "markup.bold");
    assert_scope(&scopes, "also bold", "markup.bold");
    assert_scope(&scopes, "code", "markup.raw");
    assert_scope(&scopes, "label", "markup.link.text");
    assert_scope(&scopes, "https://example.com", "markup.link.url");
}

#[test]
fn large_markdown_keeps_block_color_and_drops_inline_color_with_injections() {
    let mut source = "ordinary prose\n\n".repeat(9_000);
    assert!(source.len() > 128 * 1024);
    source.push_str("# Heading\n\n*inline* and `code`\n");

    let scopes = scopes(&source, "markdown");
    assert_scope(&scopes, "Heading", "markup.heading");
    assert!(
        scopes
            .iter()
            .all(|(_, scope)| *scope != "markup.italic" && *scope != "markup.raw"),
        "large Markdown must use the documented injection-free tree, got {scopes:?}"
    );
}

#[test]
fn every_bundled_grammar_loads_without_error() {
    let registry = Registry::new();
    for language in [
        "rust",
        "python",
        "swift",
        "c",
        "cpp",
        "javascript",
        "typescript",
        "tsx",
        "html",
        "css",
        "go",
        "bash",
        "java",
        "kotlin",
        "sql",
        "lua",
        "c-sharp",
        "zig",
        "cmake",
        "proto",
        "make",
        "ini",
        "json",
        "toml",
        "yaml",
        "markdown",
    ] {
        let id = registry
            .language_for_name(language)
            .unwrap_or_else(|| panic!("{language} missing"));
        assert!(
            DocumentSyntax::new(&Text::new(), id, &registry).is_some(),
            "{language} failed on first use"
        );
    }
    assert!(
        registry.errors().is_empty(),
        "grammar load errors: {:?}",
        registry.errors()
    );
}

#[test]
fn extensions_map_to_languages_case_insensitively() {
    let registry = Registry::new();
    for (path, expected) in [
        ("src/main.rs", "rust"),
        ("tools/check.py", "python"),
        ("Sources/App.swift", "swift"),
        ("src/native.c", "c"),
        ("include/widget.hpp", "cpp"),
        ("src/widget.CPP", "cpp"),
        ("web/app.js", "javascript"),
        ("web/view.JSX", "javascript"),
        ("web/server.mjs", "javascript"),
        ("web/config.cjs", "javascript"),
        ("web/types.ts", "typescript"),
        ("web/types.mts", "typescript"),
        ("web/types.cts", "typescript"),
        ("web/view.tsx", "tsx"),
        ("web/index.html", "html"),
        ("web/theme.CSS", "css"),
        ("cmd/runyte/main.go", "go"),
        ("scripts/release.SH", "bash"),
        ("packages/runyte.ebuild", "bash"),
        ("classes/runyte.eclass", "bash"),
        ("src/Main.JAVA", "java"),
        ("src/Main.KT", "kotlin"),
        ("build/settings.KTS", "kotlin"),
        ("db/schema.SQL", "sql"),
        ("scripts/plugin.LUA", "lua"),
        ("src/Program.CS", "c-sharp"),
        ("scripts/sample.CSX", "c-sharp"),
        ("src/main.ZIG", "zig"),
        ("build/package.ZON", "zig"),
        ("cmake/helpers.CMAKE", "cmake"),
        ("proto/model.PROTO", "proto"),
        ("build/rules.MK", "make"),
        ("build/rules.MAK", "make"),
        ("config/settings.INI", "ini"),
        ("CMakeLists.txt", "cmake"),
        ("Makefile", "make"),
        ("makefile", "make"),
        ("GNUmakefile", "make"),
        ("Cargo.toml", "toml"),
        ("data.JSON", "json"),
        ("config.yml", "yaml"),
        ("README.md", "markdown"),
    ] {
        let language = registry
            .language_for_path(std::path::Path::new(path))
            .unwrap_or_else(|| panic!("no language for {path}"));
        assert_eq!(registry.language_name(language), expected, "for {path}");
    }
}

// -- Degradation -----------------------------------------------------------

#[test]
fn unknown_extensions_have_no_language() {
    let registry = Registry::new();
    assert!(
        registry
            .language_for_path(std::path::Path::new("notes.xyz"))
            .is_none()
    );
    assert!(
        registry
            .language_for_path(std::path::Path::new("no-extension"))
            .is_none()
    );
}

#[test]
fn malformed_source_still_parses_and_never_panics() {
    // Deliberately broken Rust: the parser must produce a tree with error
    // nodes rather than failing, so the buffer stays highlighted.
    let (registry, text, syntax) = parse("fn main( { let ; ] } unclosed \"", "rust");
    let spans = spans_of(&syntax, &text, &registry);
    assert!(spans.iter().all(|span| span.to <= text.len_chars()));
}

#[test]
fn malformed_javascript_family_updates_match_full_unicode_reparses() {
    for (language, source, inserted) in [
        (
            "javascript",
            "const greet = (name) => `Hello, ${name}`;\nfunction broken(α",
            ") { return α; }\n",
        ),
        (
            "typescript",
            "interface Box<T> { value: T }\nconst box: Box<string> = { value: \"界\" };\nfunction broken(α: string",
            ") { return α; }\n",
        ),
        (
            "tsx",
            "type Props = { title: string };\nconst Card = (p: Props) => <section>{p.title}</section>;\nconst broken = <div>界",
            "</div>;\n",
        ),
    ] {
        let registry = Registry::new();
        let language_id = registry.language_for_name(language).unwrap();
        let mut text = Text::from_str(source);
        let mut incremental = DocumentSyntax::new(&text, language_id, &registry).unwrap();
        let before = text.clone();
        let transaction = Transaction::insert(text.len_chars(), inserted);
        text.apply(&transaction);
        assert!(incremental.update(&before, &text, &transaction, &registry));

        let fresh = DocumentSyntax::new(&text, language_id, &registry).unwrap();
        assert_eq!(
            spans_of(&incremental, &text, &registry),
            spans_of(&fresh, &text, &registry),
            "incremental {language} highlights diverged from a full reparse"
        );
    }
}

#[test]
fn malformed_html_and_css_updates_match_full_unicode_reparses() {
    for (language, source, inserted) in [
        (
            "html",
            "<main data-title=\"世界\"><script>const greet = (name) => `Hello, ${name}`;",
            "</script><style>.card { color: var(--accent); }</style></main>\n",
        ),
        (
            "css",
            "@scope (.card) { & > .title { --label: \"世界\"; color: var(--accent); }",
            " }\n",
        ),
    ] {
        let registry = Registry::new();
        let language_id = registry.language_for_name(language).unwrap();
        let mut text = Text::from_str(source);
        let mut incremental = DocumentSyntax::new(&text, language_id, &registry).unwrap();
        let before = text.clone();
        let transaction = Transaction::insert(text.len_chars(), inserted);
        text.apply(&transaction);
        assert!(incremental.update(&before, &text, &transaction, &registry));

        let fresh = DocumentSyntax::new(&text, language_id, &registry).unwrap();
        assert_eq!(
            spans_of(&incremental, &text, &registry),
            spans_of(&fresh, &text, &registry),
            "incremental {language} highlights diverged from a full reparse"
        );
        assert!(
            spans_of(&incremental, &text, &registry)
                .iter()
                .all(|span| span.to <= text.len_chars())
        );
    }
}

#[test]
fn malformed_go_and_bash_updates_match_full_unicode_reparses_and_outlines() {
    for (language, source, inserted) in [
        (
            "go",
            "package main\ntype Café struct { value string\nfunc greet(name string",
            ") string { return name }\n",
        ),
        (
            "bash",
            "#!/usr/bin/env bash\nbuild() { if true; then echo \"界\"",
            "; fi; }\n",
        ),
    ] {
        let registry = Registry::new();
        let language_id = registry.language_for_name(language).unwrap();
        let mut text = Text::from_str(source);
        let mut incremental = DocumentSyntax::new(&text, language_id, &registry).unwrap();
        let malformed = incremental.outline(&text, &registry).unwrap();
        assert!(
            malformed
                .items
                .iter()
                .all(|item| item.range.to <= text.len_chars()),
            "malformed {language} produced an invalid outline range"
        );

        let before = text.clone();
        let transaction = Transaction::insert(text.len_chars(), inserted);
        text.apply(&transaction);
        assert!(incremental.update(&before, &text, &transaction, &registry));
        let fresh = DocumentSyntax::new(&text, language_id, &registry).unwrap();
        assert_eq!(
            spans_of(&incremental, &text, &registry),
            spans_of(&fresh, &text, &registry),
            "incremental {language} highlights diverged from a full reparse"
        );
        assert_eq!(
            outline_entries(&incremental.outline(&text, &registry).unwrap()),
            outline_entries(&fresh.outline(&text, &registry).unwrap()),
            "incremental {language} outline diverged from a full reparse"
        );
    }
}

#[test]
fn malformed_java_updates_match_fresh_unicode_reparses_and_outlines() {
    let registry = Registry::new();
    let language = registry.language_for_name("java").unwrap();
    let mut text =
        Text::from_str("sealed class Café permits Child { String value = \"世界\"; int broken(имя");
    let mut incremental = DocumentSyntax::new(&text, language, &registry).unwrap();
    let malformed = incremental.outline(&text, &registry).unwrap();
    assert!(
        malformed
            .items
            .iter()
            .all(|item| item.range.to <= text.len_chars())
    );

    let before = text.clone();
    let transaction = Transaction::insert(
        text.len_chars(),
        ") { return switch (имя) { default -> 1; }; } }\nfinal class Child extends Café {}\n",
    );
    text.apply(&transaction);
    assert!(incremental.update(&before, &text, &transaction, &registry));
    let fresh = DocumentSyntax::new(&text, language, &registry).unwrap();
    assert_eq!(
        spans_of(&incremental, &text, &registry),
        spans_of(&fresh, &text, &registry)
    );
    assert_eq!(
        outline_entries(&incremental.outline(&text, &registry).unwrap()),
        outline_entries(&fresh.outline(&text, &registry).unwrap())
    );
}

#[test]
fn malformed_kotlin_updates_match_fresh_unicode_reparses_and_outlines() {
    let registry = Registry::new();
    let language = registry.language_for_name("kotlin").unwrap();
    let mut text =
        Text::from_str("sealed class Café { val label = \"世界\"; fun broken(имя: String");
    let mut incremental = DocumentSyntax::new(&text, language, &registry).unwrap();
    let malformed = incremental.outline(&text, &registry).unwrap();
    assert!(
        malformed
            .items
            .iter()
            .all(|item| item.range.to <= text.len_chars())
    );

    let before = text.clone();
    let transaction = Transaction::insert(
        text.len_chars(),
        ") = when (имя) { \"界\" -> 1 else -> 0 } }\nobject Registry { fun value() = 1 }\n",
    );
    text.apply(&transaction);
    assert!(incremental.update(&before, &text, &transaction, &registry));
    let fresh = DocumentSyntax::new(&text, language, &registry).unwrap();
    assert_eq!(
        spans_of(&incremental, &text, &registry),
        spans_of(&fresh, &text, &registry)
    );
    assert_eq!(
        outline_entries(&incremental.outline(&text, &registry).unwrap()),
        outline_entries(&fresh.outline(&text, &registry).unwrap())
    );
}

#[test]
fn renaming_html_script_tags_removes_the_injected_layer_incrementally() {
    let source = "<script>const answer = 42;</script>";
    let registry = Registry::new();
    let html = registry.language_for_name("html").unwrap();
    let mut text = Text::from_str(source);
    let mut incremental = DocumentSyntax::new(&text, html, &registry).unwrap();
    let open = char_offset(source, "script");
    let close = source[..source.rfind("script").unwrap()].chars().count();
    let before = text.clone();
    let transaction = Transaction::new(vec![
        runyte::text::Change::new(open, open + "script".len(), "section"),
        runyte::text::Change::new(close, close + "script".len(), "section"),
    ]);
    text.apply(&transaction);
    assert!(incremental.update(&before, &text, &transaction, &registry));

    let fresh = DocumentSyntax::new(&text, html, &registry).unwrap();
    assert_eq!(
        spans_of(&incremental, &text, &registry),
        spans_of(&fresh, &text, &registry)
    );
    let updated = text.to_string();
    let answer = incremental
        .node_at(&text, &registry, char_offset(&updated, "answer"))
        .unwrap()
        .unwrap();
    assert_eq!(registry.language_name(answer.language), "html");
}

#[test]
fn highlighting_an_empty_document_yields_no_spans() {
    let (registry, text, syntax) = parse("", "rust");
    assert!(spans_of(&syntax, &text, &registry).is_empty());
}

#[test]
fn spans_never_overlap_and_stay_in_order() {
    let (registry, text, syntax) = parse(
        "fn f() -> Result<String, Error> { let v = vec![1, 2, 3]; }",
        "rust",
    );
    let spans = spans_of(&syntax, &text, &registry);
    for pair in spans.windows(2) {
        assert!(
            pair[0].to <= pair[1].from,
            "overlapping spans {:?} and {:?}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn multibyte_source_produces_character_offsets_not_byte_offsets() {
    let source = "// αβγ 🦀\nfn main() {}\n";
    let (registry, text, syntax) = parse(source, "rust");
    let spans = spans_of(&syntax, &text, &registry);
    for span in &spans {
        assert!(
            span.to <= text.len_chars(),
            "span {span:?} exceeds {} characters",
            text.len_chars()
        );
    }
    // `fn` sits after the multibyte comment; a byte/char confusion moves it.
    let named: Vec<_> = spans
        .iter()
        .map(|span| (text.slice_string(span.from, span.to), span.scope.name()))
        .collect();
    assert!(
        named.iter().any(|(c, n)| c == "fn" && *n == "keyword"),
        "got {named:?}"
    );
}

// -- Incremental reparse ---------------------------------------------------

/// A realistic ~1 MB Rust fixture built from this crate's largest sources.
///
/// Fixture choice matters more than size here. Tree-sitter's subtree reuse
/// depends on structural diversity, and repetitive source is pathological for
/// it — reuse degrades as the number of near-identical copies grows:
///
/// | Fixture (release build)          | Full   | Incremental |
/// | -------------------------------- | ------ | ----------- |
/// | real source, 245 KB              |  24 ms |     0.3 ms  |
/// | real source x16, 3.9 MB          | 312 ms |     3.0 ms  |
/// | generated, varied, 3.1 MB        | 169 ms |    48 ms    |
/// | 20k byte-identical fns, 1.26 MB  |  65 ms |   921 ms    |
///
/// So this concatenates the largest, most varied sources available and repeats
/// them only a handful of times. A fixture of small files repeated 60x measures
/// the degenerate corner instead of the property under test.
fn realistic_source() -> String {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = vec![
        source_root.join("app.rs"),
        source_root.join("ui.rs"),
        source_root.join("keymap.rs"),
    ];
    let mut app_modules = fs::read_dir(source_root.join("app"))
        .expect("application module directory")
        .map(|entry| entry.expect("application module entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .collect::<Vec<_>>();
    app_modules.sort();
    paths.extend(app_modules);

    let sample = paths
        .iter()
        .map(|path| {
            fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut source = String::new();
    while source.len() < 1_000_000 {
        source.push_str(&sample);
        source.push('\n');
    }
    source
}

/// Phase 1 gate: an incremental reparse must be far cheaper than a full parse.
///
/// The ratio is asserted rather than a wall-clock figure, so the test means the
/// same thing on a slow machine and in a debug build: it is checking that
/// tree-sitter's subtree reuse is actually engaged, which is the property that
/// makes typing in a large file viable.
#[test]
fn incremental_reparse_is_far_cheaper_than_a_full_parse() {
    let source = realistic_source();
    let registry = Registry::new();
    let language = registry.language_for_name("rust").unwrap();
    let mut text = Text::from_str(&source);

    let started = Instant::now();
    let mut syntax = DocumentSyntax::new(&text, language, &registry).expect("initial parse");
    let full = started.elapsed();

    // Edit early in the document, which leaves the most text to reuse.
    let at = text.len_chars() / 20;
    let mut worst = Duration::ZERO;
    for index in 0..10 {
        let before = text.clone();
        let transaction = Transaction::insert(at + index, "x");
        text.apply(&transaction);

        let started = Instant::now();
        assert!(
            syntax.update(&before, &text, &transaction, &registry),
            "reparse failed"
        );
        worst = worst.max(started.elapsed());
    }

    assert!(
        worst * 5 < full,
        "incremental reparse {worst:?} was not meaningfully cheaper than the \
         {full:?} full parse; subtree reuse is probably not engaged"
    );
    assert!(
        worst <= REPARSE_BUDGET,
        "worst incremental reparse {worst:?} exceeded the {REPARSE_BUDGET:?} budget"
    );
}

#[test]
fn incremental_reparse_matches_a_full_reparse() {
    let registry = Registry::new();
    let language = registry.language_for_name("rust").unwrap();
    let mut text = Text::from_str("fn main() {\n    let x = 1;\n}\n");
    let mut syntax = DocumentSyntax::new(&text, language, &registry).unwrap();

    // A sequence of edits, including one that introduces a new line.
    for (at, insert) in [(11usize, "\n    // c"), (5, "oo"), (0, "// lead\n")] {
        let before = text.clone();
        let transaction = Transaction::insert(at, insert);
        text.apply(&transaction);
        assert!(syntax.update(&before, &text, &transaction, &registry));
    }

    let incremental = spans_of(&syntax, &text, &registry);
    let fresh_syntax = DocumentSyntax::new(&text, language, &registry).unwrap();
    let fresh = spans_of(&fresh_syntax, &text, &registry);
    assert_eq!(
        incremental, fresh,
        "incremental reparse diverged from a full reparse"
    );
}

#[test]
fn a_multi_range_transaction_reparses_correctly() {
    let registry = Registry::new();
    let language = registry.language_for_name("rust").unwrap();
    let mut text = Text::from_str("let a = 1;\nlet b = 2;\nlet c = 3;\n");
    let mut syntax = DocumentSyntax::new(&text, language, &registry).unwrap();

    // Replace all three identifiers at once, as a multi-cursor edit would.
    let before = text.clone();
    let transaction = Transaction::new(vec![
        runyte::text::Change::new(4, 5, "alpha"),
        runyte::text::Change::new(15, 16, "beta"),
        runyte::text::Change::new(26, 27, "gamma"),
    ]);
    text.apply(&transaction);
    assert!(syntax.update(&before, &text, &transaction, &registry));

    let fresh = DocumentSyntax::new(&text, language, &registry).unwrap();
    assert_eq!(
        spans_of(&syntax, &text, &registry),
        spans_of(&fresh, &text, &registry)
    );
}

// -- Injection handling ----------------------------------------------------

#[test]
fn markdown_fenced_code_is_highlighted_through_an_injection() {
    let scopes = scopes(
        "# Title\n\n```rust\nfn main() { let x = 1; }\n```\n",
        "markdown",
    );
    assert!(
        scopes
            .iter()
            .any(|(content, name)| content == "fn" && *name == "keyword"),
        "expected Rust inside the fence to be highlighted, got {scopes:?}"
    );
}

#[test]
fn markdown_fences_resolve_go_and_bash_without_global_language_logic() {
    let source = "```go\npackage main\nfunc greet() {}\n```\n\
                  ```bash\nbuild() { echo ok; }\n```\n";
    let (registry, text, syntax) = parse(source, "markdown");

    for (needle, expected) in [("greet", "go"), ("build", "bash")] {
        let node = syntax
            .node_at(&text, &registry, char_offset(source, needle))
            .unwrap()
            .unwrap();
        assert_eq!(
            registry.language_name(node.language),
            expected,
            "at {needle}"
        );
    }

    let highlighted = spans_of(&syntax, &text, &registry)
        .into_iter()
        .map(|span| (text.slice_string(span.from, span.to), span.scope.name()))
        .collect::<Vec<_>>();
    assert_scope(&highlighted, "func", "keyword");
    assert_scope(&highlighted, "build", "function");
}

#[test]
fn large_documents_drop_injections_but_still_highlight() {
    // Above the injection threshold the outer language must still highlight;
    // only embedded languages are given up.
    let source = realistic_source();
    assert!(
        source.len() > 128 * 1024,
        "fixture must exceed the threshold"
    );

    let registry = Registry::new();
    let language = registry.language_for_name("rust").unwrap();
    let text = Text::from_str(&source);
    let syntax = DocumentSyntax::new(&text, language, &registry).expect("parse");
    assert_eq!(
        syntax.language(),
        language,
        "the injection-free parser variant must retain its canonical identity"
    );

    let spans = syntax.spans(&text, &registry, 0, 4_000);
    assert!(
        spans.iter().any(|span| span.scope.name() == "keyword"),
        "large documents must still highlight the outer language"
    );
}

#[test]
fn large_html_drops_script_injection_but_keeps_outer_highlights() {
    let mut source = String::from("<script>const dropped = 1;</script>\n");
    while source.len() <= 128 * 1024 {
        source.push_str("<runyte-card data-title=\"世界\">content</runyte-card>\n");
    }

    let registry = Registry::new();
    let html = registry.language_for_name("html").unwrap();
    let text = Text::from_str(&source);
    let syntax = DocumentSyntax::new(&text, html, &registry).expect("large HTML parse");
    let dropped = syntax
        .node_at(&text, &registry, char_offset(&source, "dropped"))
        .unwrap()
        .unwrap();
    assert_eq!(
        registry.language_name(dropped.language),
        "html",
        "the large-document plain variant must not create a JavaScript layer"
    );

    let spans = syntax.spans(&text, &registry, 0, 4_000);
    assert!(
        spans.iter().any(|span| span.scope.name() == "tag"),
        "large HTML must retain outer-language highlighting"
    );
}

#[test]
fn small_documents_keep_injections() {
    // The same content below the threshold keeps full fidelity.
    let source = "//! Doc.\nfn main() {}\n";
    assert!(source.len() < 128 * 1024);
    let (registry, text, syntax) = parse(source, "rust");
    let spans = syntax.spans(&text, &registry, 0, text.len_chars());
    assert!(!spans.is_empty());
}

// -- Match brackets --------------------------------------------------------

#[test]
fn match_bracket_resolves_through_the_syntax_tree() {
    let (_, text, syntax) = parse("fn main() { let v = vec![1]; }", "rust");
    let open_brace = text.to_string().find('{').unwrap();
    let close_brace = text.to_string().rfind('}').unwrap();
    assert_eq!(
        syntax.matching_bracket(&text, open_brace),
        Some(close_brace)
    );
    assert_eq!(
        syntax.matching_bracket(&text, close_brace),
        Some(open_brace)
    );
}

#[test]
fn match_bracket_ignores_brackets_inside_strings() {
    let source = "fn main() { let s = \"a { b\"; }";
    let (_, text, syntax) = parse(source, "rust");
    let outer_open = source.find('{').unwrap();
    let outer_close = source.rfind('}').unwrap();
    assert_eq!(
        syntax.matching_bracket(&text, outer_open),
        Some(outer_close),
        "the brace inside the string must not capture the match"
    );
}

#[test]
fn match_bracket_returns_none_off_a_bracket() {
    let (_, text, syntax) = parse("fn main() {}", "rust");
    assert_eq!(syntax.matching_bracket(&text, 0), None);
}
