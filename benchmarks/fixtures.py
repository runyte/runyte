# SPDX-License-Identifier: MPL-2.0

"""Deterministic documents for the startup benchmark.

Fixtures are generated rather than committed, and generated from a fixed seed
rather than copied out of the repository. A benchmark whose inputs are Runyte's
own source files would report a different number every time that source
changed, which is the opposite of what a regression benchmark is for.

Each fixture isolates one cost:

``small.rs``      fixed startup cost; the document is too small to matter.
``small.txt``     the same document with no language. Against ``small.rs`` this
                  isolates the cost of compiling one language's queries.
``medium.rs``     a realistic working file.
``large.rs``      a document large enough that parsing dominates.
``large.txt``     the same bytes with no language, isolating file reading from
                  parsing.
``large.lua``     source code which every measured editor parses with one
                  tree-sitter grammar.
``large.md``      Markdown, which every measured editor parses with tree-sitter,
                  making it a second fixture where a cross-editor comparison
                  is between editors doing the same work.
``minified.json`` one very long line, which stresses everything that works
                  outward from the start of a line rather than per row.
"""

from __future__ import annotations

import random
from pathlib import Path

SMALL_LINES = 200
MEDIUM_LINES = 5_000
LARGE_LINES = 200_000
LUA_LINES = 30_000
MARKDOWN_LINES = 30_000
MINIFIED_KEYS = 200_000
SEED = 20260829

FIXTURES = (
    "small.rs",
    "small.txt",
    "medium.rs",
    "large.rs",
    "large.txt",
    "large.lua",
    "large.md",
    "minified.json",
)


def _rust_source(lines: int, seed: int = SEED) -> str:
    """Rust with enough shape to exercise a highlighter: items, strings,
    comments, generics, attributes and nesting."""
    rng = random.Random(seed)
    names = ["parse", "render", "collect", "resolve", "encode", "merge", "scan", "apply"]
    types = ["usize", "u64", "i32", "String", "Vec<u8>", "Option<usize>"]
    out: list[str] = ["// SPDX-License-Identifier: MPL-2.0", "//! Generated benchmark fixture.", ""]
    index = 0
    while len(out) < lines:
        name = f"{rng.choice(names)}_{index}"
        kind = rng.choice(types)
        out += [
            f"/// Documentation for `{name}`.",
            "#[inline]",
            f"pub fn {name}(value: {kind}, label: &str) -> {kind} {{",
            f'    let tag = format!("{name}: {{label}}");',
            "    if tag.len() > 8 {",
            f"        // narrow the {name} path",
            "        return value;",
            "    }",
            "    value",
            "}",
            "",
        ]
        index += 1
    return "\n".join(out[:lines]) + "\n"


def _markdown(lines: int, seed: int = SEED) -> str:
    """Markdown exercising both of its grammars: block structure and inline spans.

    Fenced code blocks deliberately carry no info string. A tagged fence injects
    another language, and each editor injects only the languages it actually
    has, so a tagged fence would measure the editors' differing grammar
    inventories rather than their Markdown parsing. Untagged fences inject
    nothing anywhere and keep the comparison about Markdown.
    """
    rng = random.Random(seed)
    words = [
        "buffer", "selection", "grammar", "viewport", "transaction", "register",
        "pane", "workspace", "offset", "revision", "gutter", "overlay",
    ]

    def sentence(count: int) -> str:
        body = " ".join(rng.choice(words) for _ in range(count))
        return body.capitalize() + "."

    out: list[str] = ["# Generated benchmark fixture", ""]
    section = 0
    while len(out) < lines:
        section += 1
        out += [
            f"## Section {section}",
            "",
            f"{sentence(12)} With *emphasis*, **strong emphasis**, and `inline code`.",
            "",
            f"See [the reference](https://example.invalid/{section}) for detail.",
            "",
            "- First item with `code`",
            "- Second item with *emphasis*",
            f"- Third item referring to section {section}",
            "",
            "> A block quote holding one sentence.",
            f"> {sentence(8)}",
            "",
            "```",
            f"plain fenced block {section}",
            "no info string, so nothing is injected",
            "```",
            "",
            "| Column | Meaning |",
            "| --- | --- |",
            f"| `{rng.choice(words)}` | {sentence(4)} |",
            "",
        ]
    return "\n".join(out[:lines]) + "\n"


def _lua_source(lines: int, seed: int = SEED) -> str:
    """Lua source with functions, tables, loops, branches and calls.

    Comments are deliberately absent because Helix injects its comment grammar
    into every Lua comment. Long strings and the special calls recognized by
    Neovim's and tree-sitter-lua's injection queries are absent too. Keeping
    those constructs out makes every editor parse only the Lua grammar.
    """
    rng = random.Random(seed)
    verbs = [
        "collect", "encode", "merge", "parse", "render", "resolve", "scan", "visit",
    ]
    out: list[str] = []
    index = 0
    while len(out) < lines:
        name = f"{rng.choice(verbs)}_{index}"
        values = [rng.randint(1, 99) for _ in range(4)]
        scale = rng.randint(2, 9)
        literal = ", ".join(str(value) for value in values)
        out += [
            f"local function {name}(values, scale)",
            f'    local state = {{ total = 0, label = "{name}", enabled = true }}',
            "    for item_index, value in ipairs(values) do",
            "        local adjusted = value * scale + item_index",
            "        if adjusted % 2 == 0 then",
            "            state.total = state.total + adjusted",
            "        elseif adjusted > 10 then",
            "            state.total = state.total - math.floor(adjusted / 3)",
            "        else",
            "            state.total = state.total + 1",
            "        end",
            "    end",
            "    state.label = string.upper(state.label)",
            "    return state",
            "end",
            "",
            f"local result_{index} = {name}({{{literal}}}, {scale})",
            f"result_{index}.total = math.max(result_{index}.total, 0)",
            f"assert(result_{index}.enabled and result_{index}.total ~= nil)",
            "",
        ]
        index += 1
    return "\n".join(out[:lines]) + "\n"


def _minified_json(keys: int, seed: int = SEED) -> str:
    rng = random.Random(seed)
    pairs = ",".join(f'"key{i}":{rng.randint(0, 1_000_000)}' for i in range(keys))
    return "{" + pairs + "}"


def ensure(directory: Path, names=FIXTURES) -> dict[str, Path]:
    """Generate any missing fixture in `directory` and return their paths."""
    directory.mkdir(parents=True, exist_ok=True)
    paths: dict[str, Path] = {}
    large_source: str | None = None

    for name in names:
        path = directory / name
        paths[name] = path
        if path.exists():
            continue
        if name == "small.rs":
            path.write_text(_rust_source(SMALL_LINES))
        elif name == "small.txt":
            # Byte-identical to small.rs so the only variable is the extension.
            path.write_text(_rust_source(SMALL_LINES))
        elif name == "medium.rs":
            path.write_text(_rust_source(MEDIUM_LINES))
        elif name in ("large.rs", "large.txt"):
            if large_source is None:
                large_source = _rust_source(LARGE_LINES)
            path.write_text(large_source)
        elif name == "large.lua":
            path.write_text(_lua_source(LUA_LINES))
        elif name == "large.md":
            path.write_text(_markdown(MARKDOWN_LINES))
        elif name == "minified.json":
            path.write_text(_minified_json(MINIFIED_KEYS))
        else:
            raise ValueError(f"unknown fixture {name}")
    return paths


def describe(path: Path) -> str:
    size = path.stat().st_size
    if size >= 1_000_000:
        return f"{size / 1_000_000:.1f} MB"
    return f"{size / 1_000:.0f} KB"
