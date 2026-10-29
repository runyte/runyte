# SPDX-License-Identifier: MPL-2.0
"""Check the ordered guide chapters, repository links, and legacy anchors."""
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]


def prose(source):
    """Ignore fenced and inline examples when checking documentation links."""
    fence = None
    for line in source.splitlines():
        marker = re.match(r"^ {0,3}(`{3,}|~{3,})", line)
        if marker:
            run = marker[1]
            if fence is None:
                fence = run
            elif run[0] == fence[0] and len(run) >= len(fence):
                fence = None
            continue
        if fence is None and not line.startswith("    "):
            yield re.sub(r"(`+).*?\1", "", line)


def anchors(source):
    result = set()
    for line in prose(source):
        result.update(re.findall(r'<a id="([^"]+)"', line))
    # Backticks are removed, but their heading text contributes to the anchor.
    used = {}
    for line in source.splitlines():
        if not re.match(r"^#{1,6} ", line):
            continue
        title = line.split(" ", 1)[1].lower()
        slug = re.sub(r"[^\w\- ]", "", title).replace(" ", "-")
        number = used.get(slug, 0)
        used[slug] = number + 1
        result.add(slug + (f"-{number}" if number else ""))
    return result


def check(root=ROOT):
    guide = root / "docs/user-guide"
    index = (guide / "README.md").read_text(encoding="utf-8")
    chapters = re.findall(r"^- \[.*?\]\(([^)]+)\)$", index, re.MULTILINE)
    assert chapters and len(chapters) == len(set(chapters)), "empty or repeated chapter list"
    assert set(chapters) == {p.name for p in guide.glob("*.md")} - {"README.md"}, "unlisted chapter"
    documents = [guide / "README.md", root / "docs/user-guide.md"]
    for position, chapter in enumerate(chapters):
        path = guide / chapter
        source = path.read_text(encoding="utf-8")
        assert source.startswith("## "), f"missing chapter heading: {path}"
        _body, navigation = source.split("<!-- navigation -->")
        assert "[Contents](README.md)" in navigation, path
        if position:
            assert f"]({chapters[position - 1]})" in navigation, path
        if position + 1 < len(chapters):
            assert f"]({chapters[position + 1]})" in navigation, path
        documents.append(path)
    for path in documents:
        for line in prose(path.read_text(encoding="utf-8")):
            for destination in re.findall(r"\]\(([^\s)]+)\)", line):
                url = urlsplit(destination)
                if url.scheme or url.netloc:
                    continue
                target = (path.parent / unquote(url.path)).resolve() if url.path else path
                assert target.exists(), f"{path}: missing {destination}"
                if url.fragment and target.suffix == ".md":
                    assert unquote(url.fragment) in anchors(target.read_text(encoding="utf-8")), f"{path}: missing anchor {destination}"
    print(f"Checked {len(chapters)} chapters, navigation, local links and compatibility anchors")


if __name__ == "__main__":
    check(Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT)
