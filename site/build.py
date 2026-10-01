#!/usr/bin/env python3
"""Builds the Ferrite site into site/dist/ — standard library only.

    python3 site/build.py            # build
    python3 site/build.py --serve    # build, then serve on http://localhost:8000

Content lives in site/src/ as plain HTML fragments:

  src/layout.html          the page shell ({{TITLE}}, {{DESC}}, {{BODY}}, ...)
  src/index.html           the landing page body
  src/docs/<slug>.html     one documentation article each, with a front-matter
                           comment (title / order / group / summary)

Every link between pages is relative ({{ROOT}} is "" or "../"), so the site
works unchanged from a project sub-path such as /Ferrite-Browser/ on GitHub
Pages, or straight from the filesystem. After building, every internal link
and #anchor is checked; a broken one fails the build.
"""
from __future__ import annotations

import html
import re
import shutil
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SRC = HERE / "src"
STATIC = HERE / "static"
DIST = HERE / "dist"

REPO = "https://github.com/rayanjainn/Ferrite-Browser"
SITE_NAME = "Ferrite"

# Inline icons: 24x24, stroke-based, tinted by currentColor.
ICONS = {
    "shield": '<path d="M12 3l7 3v6c0 4.5-3 7.6-7 9-4-1.4-7-4.5-7-9V6l7-3z"/><path d="M9 12l2 2 4-4"/>',
    "eye": '<path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>',
    "flask": '<path d="M9 3h6"/><path d="M10 3v6L4.5 19a1.5 1.5 0 0 0 1.3 2.2h12.4a1.5 1.5 0 0 0 1.3-2.2L14 9V3"/><path d="M7.5 15h9"/>',
    "scale": '<path d="M12 3v18"/><path d="M6 21h12"/><path d="M5 7h14"/><path d="M5 7l-3 7a3 3 0 0 0 6 0L5 7z"/><path d="M19 7l-3 7a3 3 0 0 0 6 0l-3-7z"/>',
    "hand": '<circle cx="12" cy="12" r="9"/><path d="M8 12.5l2.7 2.7L16 9.5"/>',
    "chain": '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>',
    "key": '<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9"/><path d="M16 7l3 3"/>',
    "download": '<path d="M12 3v12"/><path d="M7 11l5 5 5-5"/><path d="M4 20h16"/>',
    "laptop": '<rect x="4" y="5" width="16" height="11" rx="2"/><path d="M2 20h20"/>',
    "windows": '<path d="M3 5.5l8-1.1v7H3z"/><path d="M12.5 4.2L21 3v8.4h-8.5z"/><path d="M3 12.6h8v7l-8-1.1z"/><path d="M12.5 12.6H21V21l-8.5-1.2z"/>',
    "terminal": '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9l3 3-3 3"/><path d="M13 15h4"/>',
    "github": '<path d="M9 19c-4.3 1.4-4.3-2.5-6-3m12 5v-3.5c0-1 .1-1.4-.5-2 2.8-.3 5.5-1.4 5.5-6a4.6 4.6 0 0 0-1.3-3.2 4.2 4.2 0 0 0-.1-3.2s-1.1-.3-3.5 1.3a12.3 12.3 0 0 0-6.2 0C6.5 2.8 5.4 3.1 5.4 3.1a4.2 4.2 0 0 0-.1 3.2A4.6 4.6 0 0 0 4 9.5c0 4.6 2.7 5.7 5.5 6-.6.6-.6 1.2-.5 2V21"/>',
    "moon": '<path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"/>',
    "sun": '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',
    "menu": '<path d="M4 7h16M4 12h16M4 17h16"/>',
    "check": '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
    "arrow": '<path d="M5 12h14"/><path d="M13 6l6 6-6 6"/>',
    "book": '<path d="M4 5.5A2.5 2.5 0 0 1 6.5 3H20v16H6.5A2.5 2.5 0 0 0 4 21.5z"/><path d="M4 5.5v16"/>',
    "lock": '<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>',
    "layers": '<path d="M12 3l9 5-9 5-9-5 9-5z"/><path d="M3 13l9 5 9-5"/>',
    "gauge": '<path d="M4 18a9 9 0 1 1 16 0"/><path d="M12 14l4-5"/>',
    "list": '<path d="M9 6h11M9 12h11M9 18h11"/><circle cx="4.5" cy="6" r="1"/><circle cx="4.5" cy="12" r="1"/><circle cx="4.5" cy="18" r="1"/>',
    "block": '<circle cx="12" cy="12" r="9"/><path d="M5.6 5.6l12.8 12.8"/>',
}


def icon(name: str, cls: str = "") -> str:
    body = ICONS[name]
    c = f' class="{cls}"' if cls else ""
    return (
        f'<svg{c} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" '
        f'stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{body}</svg>'
    )


def render_icons(text: str) -> str:
    return re.sub(r"\{\{icon:([a-z]+)(?::([\w-]+))?\}\}", lambda m: icon(m.group(1), m.group(2) or ""), text)


# ── Documentation set ────────────────────────────────────────────────────────

FRONT = re.compile(r"\A\s*<!--(.*?)-->\s*", re.S)


def parse_front(text: str) -> tuple[dict, str]:
    m = FRONT.match(text)
    if not m:
        raise SystemExit("a docs fragment is missing its front-matter comment")
    meta = {}
    for line in m.group(1).strip().splitlines():
        key, _, value = line.partition(":")
        meta[key.strip()] = value.strip()
    return meta, text[m.end():]


def slugify(s: str) -> str:
    s = re.sub(r"<[^>]+>", "", s)
    s = html.unescape(s).lower()
    s = re.sub(r"[^a-z0-9]+", "-", s).strip("-")
    return s or "section"


def add_heading_ids(body: str) -> tuple[str, list[tuple[int, str, str]]]:
    """Gives every h2/h3 an id and an anchor link; returns the outline."""
    outline: list[tuple[int, str, str]] = []
    seen: set[str] = set()

    def sub(m: re.Match) -> str:
        level, attrs, inner = int(m.group(1)), m.group(2), m.group(3)
        idm = re.search(r'id="([^"]+)"', attrs)
        hid = idm.group(1) if idm else slugify(inner)
        base, n = hid, 2
        while hid in seen:
            hid, n = f"{base}-{n}", n + 1
        seen.add(hid)
        outline.append((level, hid, re.sub(r"<[^>]+>", "", inner)))
        attrs = attrs if idm else f'{attrs} id="{hid}"'
        return f'<h{level}{attrs}>{inner}<a class="anchor" href="#{hid}" aria-label="Link to this section">#</a></h{level}>'

    body = re.sub(r"<h([23])([^>]*)>(.*?)</h\1>", sub, body, flags=re.S)
    return body, outline


def load_docs() -> list[dict]:
    docs = []
    for path in sorted((SRC / "docs").glob("*.html")):
        meta, body = parse_front(path.read_text(encoding="utf-8"))
        docs.append(
            {
                "slug": path.stem,
                "title": meta["title"],
                "order": int(meta.get("order", 99)),
                "group": meta.get("group", "Docs"),
                "summary": meta.get("summary", ""),
                "body": body,
            }
        )
    docs.sort(key=lambda d: d["order"])
    return docs


def doc_href(slug: str, root: str) -> str:
    # The first page is the docs landing page.
    return f"{root}docs/" if slug == "index" else f"{root}docs/{slug}.html"


def sidebar(docs: list[dict], current: str, root: str) -> str:
    out = ['<nav class="side" aria-label="Documentation">']
    group = None
    for d in docs:
        if d["group"] != group:
            if group is not None:
                out.append("</div>")
            out.append(f'<div class="grp"><h5>{html.escape(d["group"])}</h5>')
            group = d["group"]
        cur = ' aria-current="page"' if d["slug"] == current else ""
        out.append(f'<a href="{doc_href(d["slug"], root)}"{cur}>{html.escape(d["title"])}</a>')
    out.append("</div></nav>")
    return "\n".join(out)


def toc(outline: list[tuple[int, str, str]]) -> str:
    if len(outline) < 2:
        return '<aside class="toc"></aside>'
    items = "".join(
        f'<a class="l{lvl}" href="#{hid}">{html.escape(text)}</a>' for lvl, hid, text in outline
    )
    return f'<aside class="toc" aria-label="On this page"><h5>On this page</h5>{items}</aside>'


def pager(docs: list[dict], i: int, root: str) -> str:
    prev_d = docs[i - 1] if i > 0 else None
    next_d = docs[i + 1] if i + 1 < len(docs) else None
    parts = []
    if prev_d:
        parts.append(f'<a class="prev" href="{doc_href(prev_d["slug"], root)}"><small>Previous</small>{html.escape(prev_d["title"])}</a>')
    if next_d:
        parts.append(f'<a class="next" href="{doc_href(next_d["slug"], root)}"><small>Next</small>{html.escape(next_d["title"])}</a>')
    return f'<div class="pager">{"".join(parts)}</div>' if parts else ""


# ── Page assembly ────────────────────────────────────────────────────────────


def sub_tokens(text: str, root: str) -> str:
    text = text.replace("{{ROOT}}", root).replace("{{REPO}}", REPO)
    return render_icons(text)


def page(layout: str, *, title: str, desc: str, body: str, root: str, nav: str, cls: str = "") -> str:
    full_title = SITE_NAME if title == SITE_NAME else f"{title} · {SITE_NAME}"
    out = layout
    for key, value in {
        "TITLE": html.escape(full_title),
        "DESC": html.escape(desc, quote=True),
        "BODY": body,
        "BODYCLASS": cls,
        "NAV_HOME": ' aria-current="page"' if nav == "home" else "",
        "NAV_DOCS": ' aria-current="page"' if nav == "docs" else "",
    }.items():
        out = out.replace("{{" + key + "}}", value)
    return sub_tokens(out, root)


def build() -> None:
    if DIST.exists():
        shutil.rmtree(DIST)
    DIST.mkdir(parents=True)
    shutil.copytree(STATIC, DIST / "static")
    (DIST / ".nojekyll").write_text("")

    layout = (SRC / "layout.html").read_text(encoding="utf-8")
    docs = load_docs()

    # Landing page.
    index = (SRC / "index.html").read_text(encoding="utf-8")
    meta, body = parse_front(index)
    (DIST / "index.html").write_text(
        page(layout, title=SITE_NAME, desc=meta["description"], body=body, root="", nav="home"),
        encoding="utf-8",
    )

    # Documentation.
    for i, d in enumerate(docs):
        root = "../"
        body, outline = add_heading_ids(d["body"])
        article = (
            f'<article><div class="crumbs"><a href="{root}">Home</a> / '
            f'<a href="{doc_href("index", root)}">Docs</a> / {html.escape(d["group"])}</div>'
            f'<h1>{html.escape(d["title"])}</h1>{body}{pager(docs, i, root)}</article>'
        )
        shell = f'<div class="wrap docs">{sidebar(docs, d["slug"], root)}{article}{toc(outline)}</div>'
        out = page(
            layout,
            title=d["title"],
            desc=d["summary"] or f'{d["title"]} — Ferrite documentation',
            body=shell,
            root=root,
            nav="docs",
        )
        target = DIST / "docs" / ("index.html" if d["slug"] == "index" else f'{d["slug"]}.html')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(out, encoding="utf-8")

    print(f"built {1 + len(docs)} pages into {DIST.relative_to(HERE.parent)}")


# ── Link check ───────────────────────────────────────────────────────────────


def check_links() -> int:
    pages = {p: p.read_text(encoding="utf-8") for p in DIST.rglob("*.html")}
    ids = {p: set(re.findall(r'\bid="([^"]+)"', t)) for p, t in pages.items()}
    bad = 0
    for p, text in pages.items():
        for href in re.findall(r'(?:href|src)="([^"]+)"', text):
            if re.match(r"(https?:|mailto:|data:|javascript:|//)", href):
                continue
            path, _, frag = href.partition("#")
            if path == "":
                target = p
            else:
                target = (p.parent / path).resolve()
                if target.is_dir():
                    target = target / "index.html"
            if not target.exists():
                print(f"BROKEN {p.relative_to(DIST)}: {href}")
                bad += 1
            elif frag and target.suffix == ".html" and frag not in ids.get(target, set()):
                print(f"MISSING ANCHOR {p.relative_to(DIST)}: {href}")
                bad += 1
    return bad


def main() -> None:
    build()
    bad = check_links()
    if bad:
        raise SystemExit(f"{bad} broken link(s)")
    print("links ok")
    if "--serve" in sys.argv:
        import functools
        import http.server

        handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(DIST))
        print("serving http://localhost:8000")
        http.server.ThreadingHTTPServer(("127.0.0.1", 8000), handler).serve_forever()


if __name__ == "__main__":
    main()
