# SPDX-License-Identifier: MPL-2.0

"""Deterministic path corpora for the fuzzy-matching benchmark.

The candidates are generated from a fixed seed rather than taken from a real
checkout, for the reason `fixtures.py` gives about documents: a benchmark whose
input is Runyte's own tree reports a different number every time that tree
changes. A fixed seed also means the corpus a recorded result was measured on
can be rebuilt exactly, on another machine and a year later.

What is generated is meant to look like a source repository rather than like
random strings, because both matchers here are tuned for paths. Segments come
from a small vocabulary, so names repeat across directories and a short query
has many plausible answers to choose between — which is the case a ranking
disagreement shows up in. Depth is weighted towards two and three segments, and
the extension distribution is weighted towards source files.

Sizes are candidate counts, spanning the range a file picker actually sees: a
small project, a large one, and a tree big enough that per-candidate cost is the
only thing left in the measurement.
"""

from __future__ import annotations

import random
from pathlib import Path

SIZES = {
    "small": 1_000,
    "medium": 10_000,
    "large": 100_000,
}

SEED = 20260904

# Directory segments. Deliberately short and repetitive: real trees reuse the
# same handful of names at every level, which is what makes a query like `src`
# ambiguous rather than decisive.
DIRECTORIES = (
    "src", "lib", "app", "core", "docs", "tests", "test", "internal",
    "pkg", "cmd", "api", "web", "ui", "server", "client", "common",
    "utils", "util", "config", "scripts", "tools", "vendor", "examples",
    "handlers", "models", "views", "services", "workers", "parser",
    "runtime", "protocol", "storage", "network", "graphics", "audio",
    "platform", "backend", "frontend", "shared", "legacy", "migrations",
)

# Basename stems. The overlap with DIRECTORIES is intentional: `parser/parser.rs`
# is a real shape, and a query that matches both a directory and a basename is
# exactly where a path-aware ranking differs from a flat one.
STEMS = (
    "main", "index", "parser", "lexer", "token", "buffer", "window",
    "picker", "matcher", "scanner", "render", "layout", "session",
    "client", "server", "handler", "worker", "queue", "cache", "config",
    "loader", "writer", "reader", "stream", "socket", "packet", "codec",
    "helper", "logger", "metrics", "tracing", "registry", "factory",
    "builder", "adapter", "context", "manager", "resolver", "validator",
    "file_picker", "key_map", "text_buffer", "undo_stack", "diff_view",
    "line_index", "syntax_tree", "event_loop", "path_utils", "test_helper",
)

# Extensions, weighted towards source. A picker's corpus is mostly code.
EXTENSIONS = (
    (".rs", 30), (".py", 14), (".ts", 12), (".go", 10), (".c", 6),
    (".h", 6), (".js", 6), (".md", 5), (".json", 4), (".yaml", 3),
    (".toml", 2), (".txt", 2),
)

# Suffixes appended to a stem, so that one stem yields a family of related
# names rather than a single candidate.
QUALIFIERS = ("", "", "", "", "_test", "_impl", "_v2", "_old", "2", "_inner")


def _extension(generator: random.Random) -> str:
    names = [name for name, _ in EXTENSIONS]
    weights = [weight for _, weight in EXTENSIONS]
    return generator.choices(names, weights=weights, k=1)[0]


# Files per directory. Runyte's own tree has roughly ten tracked files for
# every directory, and a picker's corpus is mostly files; a generator that gave
# each file its own fresh directory chain would invert that.
FILES_PER_DIRECTORY = 10

# How deep the generated tree goes. Beyond this a directory holds files only.
MAX_DEPTH = 4


def _tree(generator: random.Random, count: int) -> list[str]:
    """`count` directory paths, grown as a tree rather than drawn independently.

    Real directories are reused: hundreds of files share a handful of them, and
    the same segment name appears at several levels. Growing a tree by
    extending directories that already exist reproduces that, where drawing a
    fresh chain per file would give almost every file a directory of its own.
    """
    directories: list[str] = []
    seen: set[str] = set()
    while len(directories) < count:
        # Extending an existing directory is what makes the tree deep and its
        # names shared; starting again at the root keeps it broad.
        parent = (
            generator.choice(directories)
            if directories and generator.random() < 0.65
            else ""
        )
        if parent.count("/") + 1 >= MAX_DEPTH:
            continue
        candidate = generator.choice(DIRECTORIES)
        path = f"{parent}/{candidate}" if parent else candidate
        if path in seen:
            continue
        seen.add(path)
        directories.append(path)
    return directories


def paths(count: int, seed: int = SEED) -> list[str]:
    """`count` unique repository-shaped candidates, in a fixed shuffled order.

    Both files and the directories that contain them, because both are what the
    picker ranks. A directory is a candidate in its own right: typing a folder
    name and being offered the folder is the ordinary way to reach what is
    inside it, and a corpus of files alone measures a picker nobody uses.
    Leaving them out once produced a recorded result claiming Runyte ranked
    `src` badly, when the editor puts the `src` directory first.

    Directories are spelled without a trailing separator, which is how
    `path_text` spells them for the matcher; the trailing slash in the picker
    is added when the row is drawn.

    The result is shuffled rather than sorted: corpus order should not be
    correlated with anything a ranking might key on, and a matcher that quietly
    depended on sorted input would go unnoticed otherwise.
    """
    if count < 1:
        raise ValueError("a corpus needs at least one candidate")
    generator = random.Random(seed)
    directories = _tree(
        generator, max(1, round(count / (FILES_PER_DIRECTORY + 1)))
    )
    candidates = set(directories)
    # The root is a placement option so that top-level files exist, and is not
    # itself a candidate.
    holders = directories + [""]
    while len(candidates) < count:
        holder = generator.choice(holders)
        stem = generator.choice(STEMS) + generator.choice(QUALIFIERS)
        name = stem + _extension(generator)
        candidates.add(f"{holder}/{name}" if holder else name)
    ordered = sorted(candidates)
    generator.shuffle(ordered)
    return ordered


def write(directory: Path, sizes: dict[str, int] | None = None,
          seed: int = SEED) -> dict[str, Path]:
    """Write one file of candidates per size, and return them by name.

    Existing files are rewritten. The generator is cheap and the seed fixes the
    contents, so there is no cache to invalidate: a corpus file is always the
    one the current seed describes.
    """
    sizes = SIZES if sizes is None else sizes
    directory.mkdir(parents=True, exist_ok=True)
    written = {}
    for name, count in sizes.items():
        path = directory / f"{name}.txt"
        path.write_text("\n".join(paths(count, seed)) + "\n", encoding="utf-8")
        written[name] = path
    return written
