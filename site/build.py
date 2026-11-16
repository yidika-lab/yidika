import html
import json
import re
import shutil
import sys
from pathlib import Path

from markdown_it import MarkdownIt

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
SITE = ROOT / "site"
REPO = "https://github.com/yidika-lab/yidika"
REGISTRY = "https://yidika-lab.github.io/registry/"
YIDIKA_FENCES = {"yidika", "yidika-file"}


def read_version():
    match = re.search(r'^version\s*=\s*"([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M)
    return match.group(1)


def read_release_date(version):
    match = re.search(r"^## \[" + re.escape(version) + r"\] - (\S+)", (ROOT / "CHANGELOG.md").read_text(), re.M)
    return match.group(1) if match else ""


def read_keywords():
    lexer = (ROOT / "src" / "syntax" / "lexer.rs").read_text()
    return set(re.findall(r'"([a-z]+)" => Token::', lexer))


def read_nav():
    groups = []
    for line in (DOCS / "README.md").read_text().splitlines():
        if line.startswith("## "):
            groups.append((line[3:].strip(), []))
        item = re.match(r"- \[(.+?)\]\((.+?\.md)\)", line)
        if item and groups:
            groups[-1][1].append((item.group(1), item.group(2)))
    return groups


TOKEN = re.compile(
    r'(?P<comment>//[^\n]*)'
    r'|(?P<string>f?"(?:\\.|[^"\\])*"|`[^`]*`|\'(?:\\.|[^\'\\])\')'
    r'|(?P<number>\b(?:0x[0-9a-fA-F_]+|\d[\d_]*(?:\.\d+)?(?:e[+-]?\d+)?i?)\b)'
    r'|(?P<word>[A-Za-z_][A-Za-z0-9_]*)'
)


def highlight_yidika(code, keywords):
    out = []
    pos = 0
    for m in TOKEN.finditer(code):
        out.append(html.escape(code[pos:m.start()]))
        text = html.escape(m.group(0))
        kind = m.lastgroup
        if kind == "word":
            word = m.group(0)
            after = code[m.end():m.end() + 1]
            if word in keywords:
                kind = "kw"
            elif word[0].isupper():
                kind = "type"
            elif after == "(":
                kind = "call"
            else:
                kind = None
        out.append(f'<span class="tk-{kind}">{text}</span>' if kind else text)
        pos = m.end()
    out.append(html.escape(code[pos:]))
    return "".join(out)


def slugify(text):
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")


def rewrite_link(href):
    if re.match(r"^[a-z]+:", href) or href.startswith("#"):
        return href
    path, _, anchor = href.partition("#")
    suffix = "#" + anchor if anchor else ""
    if path.endswith(".md") and "/" not in path:
        return path[:-3] + ".html" + suffix
    if path.startswith("assets/"):
        return path + suffix
    return f"{REPO}/blob/main/docs/{path}{suffix}"


def fence_html(info, code, keywords):
    lang = info.split()[0] if info else ""
    if lang == "output":
        return f'<figure class="output"><figcaption>Output</figcaption><pre>{html.escape(code)}</pre></figure>'
    body = highlight_yidika(code, keywords) if lang in YIDIKA_FENCES else html.escape(code)
    label = {"yidika": "yidika", "yidika-file": "yidika", "sh": "shell", "powershell": "powershell", "toml": "toml"}.get(lang, lang)
    return (
        f'<figure class="code"><figcaption><span>{label}</span>'
        f'<button type="button" class="copy" aria-label="Copy code">Copy</button></figcaption>'
        f'<pre><code>{body}</code></pre></figure>'
    )


def make_renderer(keywords, toc):
    md = MarkdownIt("commonmark", {"html": True}).enable("table")

    def fence(self, tokens, idx, options, env):
        token = tokens[idx]
        return fence_html(token.info.strip(), token.content, keywords)

    def heading_open(self, tokens, idx, options, env):
        token = tokens[idx]
        text = tokens[idx + 1].content
        slug = slugify(text)
        if token.tag == "h2":
            toc.append((text, slug))
        return f'<{token.tag} id="{slug}">'

    def link_open(self, tokens, idx, options, env):
        token = tokens[idx]
        token.attrSet("href", rewrite_link(token.attrGet("href")))
        return self.renderToken(tokens, idx, options, env)

    def image(self, tokens, idx, options, env):
        token = tokens[idx]
        token.attrSet("src", rewrite_link(token.attrGet("src")))
        return self.renderToken(tokens, idx, options, env)

    md.add_render_rule("fence", fence)
    md.add_render_rule("heading_open", heading_open)
    md.add_render_rule("link_open", link_open)
    md.add_render_rule("image", image)
    return md


def nav_html(nav, current):
    parts = []
    for group, items in nav:
        links = "".join(
            f'<li><a href="{f[:-3]}.html"{" aria-current=\"page\"" if f == current else ""}>{html.escape(t)}</a></li>'
            for t, f in items
        )
        parts.append(f'<h2>{html.escape(group)}</h2><ul>{links}</ul>')
    return "".join(parts)


def toc_html(toc):
    if not toc:
        return ""
    items = "".join(f'<li><a href="#{slug}">{html.escape(text)}</a></li>' for text, slug in toc)
    return f'<nav class="toc" aria-label="On this page"><h2>On this page</h2><ul>{items}</ul></nav>'


def pager_html(order, current):
    names = [f for _, f in order]
    if current not in names:
        return ""
    i = names.index(current)
    prev_link = f'<a class="prev" href="{names[i - 1][:-3]}.html"><small>Previous</small>{html.escape(order[i - 1][0])}</a>' if i > 0 else "<span></span>"
    next_link = f'<a class="next" href="{names[i + 1][:-3]}.html"><small>Next</small>{html.escape(order[i + 1][0])}</a>' if i + 1 < len(names) else "<span></span>"
    return f'<nav class="pager" aria-label="Pages">{prev_link}{next_link}</nav>'


def page_title(markdown):
    match = re.search(r"^# (.+)$", markdown, re.M)
    return match.group(1).strip() if match else "Yidika"


def fill(template, values):
    return re.sub(r"\{\{(\w+)\}\}", lambda m: values[m.group(1)], template)


def build_doc(name, layout, nav, order, keywords, meta, out):
    markdown = (DOCS / name).read_text()
    toc = []
    body = make_renderer(keywords, toc).render(markdown)
    stem = name[:-3]
    actions = (
        f'<div class="page-actions">'
        f'<button type="button" class="copy-page" data-src="{stem}.md">Copy page</button>'
        f'<a href="{stem}.md">View as Markdown</a>'
        f'<a href="{REPO}/edit/main/docs/{name}">Edit on GitHub</a></div>'
    )
    values = dict(meta, title=html.escape(page_title(markdown)), nav=nav_html(nav, name), toc=toc_html(toc),
                  content=actions + body + pager_html(order, name), body_class="doc")
    (out / f"{stem}.html").write_text(fill(layout, values))
    shutil.copy(DOCS / name, out / name)


def docmap_html(nav):
    columns = []
    for group, items in nav:
        links = "".join(f'<li><a href="{f[:-3]}.html">{html.escape(t)}</a></li>' for t, f in items)
        columns.append(f'<div><h3>{html.escape(group)}</h3><ul>{links}</ul></div>')
    return "".join(columns)


def build_home(layout, nav, keywords, meta, out):
    home = (SITE / "home.html").read_text()
    sample = (ROOT / "benchmarks" / "fib" / "fib.yk").read_text()
    content = fill(home, dict(meta, sample=highlight_yidika(sample, keywords), docmap=docmap_html(nav)))
    values = dict(meta, title="Yidika", nav=nav_html(nav, ""), toc="", content=content, body_class="home")
    (out / "index.html").write_text(fill(layout, values))


def build_llms(nav, meta, out):
    lines = [f"# Yidika {meta['version']}", "", "> A statically typed language with an interpreter and an LLVM native backend that print the same output.", ""]
    full = []
    for group, items in nav:
        lines.append(f"## {group}")
        for title, name in items:
            lines.append(f"- [{title}]({meta['base']}{name})")
            full.append((DOCS / name).read_text())
        lines.append("")
    (out / "llms.txt").write_text("\n".join(lines))
    (out / "llms-full.txt").write_text("\n\n".join(full))


def build(out):
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    keywords = read_keywords()
    nav = read_nav()
    order = [item for _, items in nav for item in items]
    version = read_version()
    meta = {"version": version, "released": read_release_date(version), "repo": REPO, "registry": REGISTRY,
            "base": "https://yidika-lab.github.io/yidika/"}
    layout = (SITE / "layout.html").read_text()
    for _, name in order:
        build_doc(name, layout, nav, order, keywords, meta, out)
    build_home(layout, nav, keywords, meta, out)
    build_llms(nav, meta, out)
    shutil.copytree(DOCS / "assets", out / "assets")
    shutil.copy(SITE / "style.css", out / "style.css")
    shutil.copy(SITE / "site.js", out / "site.js")
    (out / ".nojekyll").write_text("")


if __name__ == "__main__":
    build(Path(sys.argv[1]) if len(sys.argv) > 1 else SITE / "dist")
