#!/usr/bin/env python3
"""Turn docs/ into GitHub wiki pages: python3 wiki.py <docs-dir> <out-dir> <repo-url>."""
import os
import re
import sys

FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
CODE_SPAN = re.compile(r"(`+)(?:(?!\1).)+?\1", re.S)
LINK = re.compile(r'\]\(([^)\s]+)((?:\s+"[^"]*")?)\)')
UNSUPPORTED = re.compile(r"^ {0,3}\[[^\]]+\]:\s|\]\(<", re.M)
SCHEME = re.compile(r"^(?:[a-zA-Z][a-zA-Z0-9+.-]*:|//|#)")


def die(message):
    sys.exit(f"wiki.py: {message}")


def split_fences(text):
    """Yield (chunk, is_code) pairs so fenced blocks are never rewritten."""
    chunk, fence, in_code = [], None, False
    for line in text.splitlines(keepends=True):
        m = FENCE.match(line)
        if in_code:
            chunk.append(line)
            if m and m.group(1)[0] == fence[0] and len(m.group(1)) >= len(fence):
                yield "".join(chunk), True
                chunk, in_code = [], False
        elif m:
            if chunk:
                yield "".join(chunk), False
            chunk, fence, in_code = [line], m.group(1), True
        else:
            chunk.append(line)
    if chunk:
        yield "".join(chunk), in_code


def is_image(chunk, end):
    """Whether the link text closed at chunk[end] opens with `![`."""
    depth = 0
    for i in range(end, -1, -1):
        depth += {"]": 1, "[": -1}.get(chunk[i], 0)
        if depth == 0:
            return i > 0 and chunk[i - 1] == "!"
    return False


def title_of(text, path):
    for chunk, is_code in split_fences(text):
        if is_code:
            continue
        for line in chunk.splitlines():
            if line.startswith("# "):
                return line[2:].strip()
    die(f"{path} has no H1 title")


def main():
    if len(sys.argv) != 4:
        die("usage: wiki.py <docs-dir> <out-dir> <repo-url>")
    docs, out, repo = os.path.realpath(sys.argv[1]), sys.argv[2], sys.argv[3].rstrip("/")
    root = os.path.dirname(docs)

    sources = sorted(
        os.path.join(d, f)
        for d, _, files in os.walk(docs)
        for f in files
        if f.endswith(".md")
    )
    names = {}
    for path in sources:
        text = open(path, encoding="utf-8").read()
        if path == os.path.join(docs, "README.md"):
            name = "Home"
        else:
            title = title_of(text, path)
            if not re.fullmatch(r"[A-Za-z0-9 _-]+", title):
                die(f"{path}: title {title!r} has a character the wiki link cannot carry")
            name = title.replace(" ", "-")
        if name.lower() in (n.lower() for n in names.values()):
            die(f"{path}: two pages are named {name}, ignoring case")
        names[path] = name

    def convert(target, page, image):
        if SCHEME.match(target):
            return target
        rel, _, anchor = target.partition("#")
        resolved = os.path.normpath(os.path.join(os.path.dirname(page), rel))
        if not os.path.exists(resolved):
            die(f"{page}: link to {target} has no target")
        if os.path.commonpath([resolved, root]) != root:
            die(f"{page}: link to {target} leaves the repository")
        suffix = "#" + anchor if anchor else ""
        if resolved in names:
            return names[resolved] + suffix
        kind = "raw" if image else "tree" if os.path.isdir(resolved) else "blob"
        return f"{repo}/{kind}/main/{os.path.relpath(resolved, root)}{suffix}"

    def rewrite(chunk, page):
        spans = []

        def mask(m):
            spans.append(m.group(0))
            return f"\0{len(spans) - 1}\0"

        chunk = CODE_SPAN.sub(mask, chunk)
        if UNSUPPORTED.search(chunk):
            die(f"{page}: reference-style and angle-bracket links are not supported")
        chunk = LINK.sub(
            lambda m: f"]({convert(m.group(1), page, is_image(chunk, m.start()))}{m.group(2)})",
            chunk,
        )
        return re.sub(r"\0(\d+)\0", lambda m: spans[int(m.group(1))], chunk)

    os.makedirs(out, exist_ok=True)
    pages = {}
    for path, name in names.items():
        text = open(path, encoding="utf-8").read()
        pages[name] = "".join(
            chunk if is_code else rewrite(chunk, path)
            for chunk, is_code in split_fences(text)
        )
        with open(os.path.join(out, name + ".md"), "w", encoding="utf-8") as f:
            f.write(pages[name])

    sidebar = ["[Home](Home)"]
    for chunk, is_code in split_fences(pages["Home"]):
        for line in () if is_code else chunk.splitlines():
            if line.startswith("## "):
                sidebar += ["", f"**{line[3:].strip()}**", ""]
            elif line.startswith("- ["):
                m = re.match(r"- \[(?:[^\]`]|`[^`]*`)+\]\([^)\s]+\)", line)
                if not m:
                    die(f"docs/README.md: cannot make a sidebar entry from {line!r}")
                sidebar.append(m.group(0))
    with open(os.path.join(out, "_Sidebar.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(sidebar) + "\n")
    with open(os.path.join(out, "_Footer.md"), "w", encoding="utf-8") as f:
        f.write(
            f"This wiki is generated from [docs/]({repo}/tree/main/docs) on main. "
            "Edit the docs there, through a pull request.\n"
        )


main()
