# SPDX-License-Identifier: MPL-2.0

"""Deterministic documents for the startup benchmark.

Fixtures are generated rather than committed, and generated from a fixed seed
rather than copied out of the repository. A benchmark whose inputs were Runyte's
own source files would report a different number every time that source changed,
which is the opposite of what a regression benchmark is for.

The matrix is one document at three sizes, written twice: once as ``.lua``,
which every measured editor parses with the same single tree-sitter grammar, and
once as ``.txt``, which no editor claims a language for. The two files of a size
are byte-identical, so the difference between them is the whole cost of treating
a document as a language and nothing else. The difference along the size axis is
how each of those costs scales.

``short.txt``   500 lines with no language: reading and drawing alone.
``medium.txt``  5,000 lines with no language.
``long.txt``    50,000 lines with no language.
``short.lua``   the same 500 lines parsed with the Lua grammar. Against
                ``short.txt`` this is dominated by compiling one language's
                queries, since the document is too small to matter.
``medium.lua``  the same 5,000 lines parsed with the Lua grammar.
``long.lua``    the same 50,000 lines parsed with the Lua grammar. Against
                ``long.txt`` this is parsing.
"""

from __future__ import annotations

import random
from pathlib import Path

# Lines per size. Every count is a multiple of the generator's block length, so
# no fixture ends in a truncated Lua function.
SIZES = {
    "short": 500,
    "medium": 5_000,
    "long": 50_000,
}

# The extension decides whether an editor claims a language for the document.
SUFFIXES = ("txt", "lua")

SEED = 20260829

FIXTURES = tuple(f"{size}.{suffix}" for suffix in SUFFIXES for size in SIZES)


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


def split(name: str) -> tuple[str, str]:
    """Return the size and suffix of a fixture name, rejecting anything else."""
    size, _, suffix = name.partition(".")
    if size not in SIZES or suffix not in SUFFIXES:
        raise ValueError(f"unknown fixture {name}")
    return size, suffix


def ensure(directory: Path, names=FIXTURES) -> dict[str, Path]:
    """Generate any missing fixture in `directory` and return their paths."""
    directory.mkdir(parents=True, exist_ok=True)
    paths: dict[str, Path] = {}
    # One document per size, shared by both extensions so the pair stays
    # byte-identical however few of them this run was asked for.
    sources: dict[str, str] = {}

    for name in names:
        path = directory / name
        paths[name] = path
        size, _ = split(name)
        if path.exists():
            continue
        if size not in sources:
            sources[size] = _lua_source(SIZES[size])
        path.write_text(sources[size])
    return paths


def describe(path: Path) -> str:
    size = path.stat().st_size
    if size >= 1_000_000:
        return f"{size / 1_000_000:.1f} MB"
    return f"{size / 1_000:.0f} kB"
