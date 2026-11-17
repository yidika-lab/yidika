import hashlib
import html
import json
import platform
import re
import subprocess
import urllib.request
import shutil
import sys
from pathlib import Path

from markdown_it import MarkdownIt

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
SITE = ROOT / "site"
REPO = "https://github.com/yidika-lab/yidika"
REGISTRY = "https://yidika-lab.github.io/registry/"
BASE = "https://yidika-lab.github.io/yidika/"
ANIME = ("anime-4.5.0.esm.min.js", "a19015a1a92d52025a2fb6703b6d67eadd1cc2aeaf880770e96e04cf6aa07be1")
ICONS = SITE / "icons"
TAILWIND_VERSION = "4.3.4"
TAILWIND_SHA256 = {
    "linux-x64": "149a8ece5b855587a9c10de6da9dcbcfad0e47c4ed5dca219f721b3eae39d817",
    "linux-arm64": "f6d7baae1d350ed3a2defe98d0b5cc58e37400c0173b2e38d42f690ccc4bbfe1",
    "macos-arm64": "a516a11b439bfbb5ecd4a2d13b75e6cdcc284dc710aed8731e31d2e5c03895a4",
    "macos-x64": "7ec771593a1fb8bbd204f70429361b0158f7989e4136bc68d04513bf543a15ba",
    "windows-x64.exe": "ad84612696f3be102fa005d4827f0df84c7b4570d31c06865ce5fde0311478af",
}
TAILWIND_CACHE = SITE / ".cache"
LANGUAGE_GROUP = "Coming from"
WIDE_HEADER = "max-w-[1440px]"
NARROW_HEADER = "max-w-[1200px]"
GROUP_ICONS = {"Learn": "graduation-cap", "Build": "hammer", "Coming from": "arrow-right-left", "Tools": "wrench",
               "Reference": "library", "Understand": "lightbulb"}
CALLOUTS = {"NOTE": ("info", "Note"), "TIP": ("lightbulb", "Tip"), "WARNING": ("triangle-alert", "Warning")}
YIDIKA_FENCES = {"yidika", "yidika-file"}
FENCE_LABELS = {"yidika": "yidika", "yidika-file": "yidika", "sh": "shell", "powershell": "powershell", "toml": "toml",
                "text": "terminal", "js": "javascript", "ts": "typescript", "python": "python", "go": "go", "rust": "rust"}
TOUR_TABS = [
    ("Classes", None, "structs-and-classes.md", "Inheritance", "Classes in the language tour"),
    ("Match", None, "enums-and-match.md", "Enums", "Match in the language tour"),
    ("Errors", None, "errors-and-null.md", "The ? operator", "Errors in the language tour"),
    ("Threads", None, "concurrency.md", None, "Threads in the language tour"),
]
BUILD_TABS = [
    ("Web server", "server", "web-server.md", "Three kinds of handlers", "Build a web server"),
    ("Files and JSON", "file-json", "files-and-json.md", "Read a JSON file", "Files and JSON"),
    ("Parallel work", "cpu", "parallel-work.md", "Two threads", "Parallel work"),
]


def read_version():
    return re.search(r'^version\s*=\s*"([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M).group(1)


def read_release_date(version):
    match = re.search(r"^## \[" + re.escape(version) + r"\] - (\S+)", (ROOT / "CHANGELOG.md").read_text(), re.M)
    return match.group(1) if match else ""


def read_keywords():
    return set(re.findall(r'"([a-z]+)" => Token::', (ROOT / "src" / "syntax" / "lexer.rs").read_text()))


def read_nav():
    groups = []
    for line in (DOCS / "README.md").read_text().splitlines():
        if line.startswith("## "):
            groups.append({"title": line[3:].strip(), "items": []})
            continue
        item = re.match(r"(\s*)- (?:\[(.+?)\]\((.+?\.md)\)|(.+))$", line)
        if item and groups:
            if item.group(4):
                groups[-1]["items"].append({"label": item.group(4).strip()})
            else:
                groups[-1]["items"].append({"title": item.group(2), "file": item.group(3), "nested": bool(item.group(1))})
    return groups


def pages_in(nav):
    return [(item["title"], item["file"], group["title"]) for group in nav for item in group["items"] if "file" in item]


TOKEN = re.compile(
    r'(?P<comment>//[^\n]*)'
    r'|(?P<string>f?"(?:\\.|[^"\\\n])*"|`[^`\n]*`|\'(?:\\.|[^\'\\\n])\')'
    r'|(?P<number>\b(?:0x[0-9a-fA-F_]+|\d[\d_]*(?:\.\d+)?(?:e[+-]?\d+)?i?)\b)'
    r'|(?P<word>[A-Za-z_][A-Za-z0-9_]*)'
    r'|(?P<punct>[^\sA-Za-z0-9_"\'`]+)'
)


def token_kind(match, code, keywords):
    kind = match.lastgroup
    if kind != "word":
        return kind
    word = match.group(0)
    if word in keywords:
        return "kw"
    if word[0].isupper():
        return "type"
    if code[match.end():match.end() + 1] == "(":
        return "call"
    return "word"


def highlight_yidika(code, keywords):
    out = []
    pos = 0
    for m in TOKEN.finditer(code):
        out.append(html.escape(code[pos:m.start()]))
        out.append(f'<span class="t tk-{token_kind(m, code, keywords)}">{html.escape(m.group(0))}</span>')
        pos = m.end()
    out.append(html.escape(code[pos:]))
    return "".join(out)


def plain_heading(text):
    return re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text).replace("`", "")


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


def icon(name, size=16):
    path = ICONS / f"{name}.svg"
    if not path.exists():
        raise SystemExit(f"site/icons/{name}.svg is missing: copy it from lucide-static (see site/README.md)")
    svg = re.sub(r"<!--.*?-->", "", path.read_text(), flags=re.S)
    svg = re.sub(r'\s*class="[^"]*"', "", svg)
    svg = re.sub(r'width="24"\s*height="24"', f'width="{size}" height="{size}"', svg)
    svg = svg.replace("<svg", f'<svg class="icon" aria-hidden="true" focusable="false"', 1)
    return re.sub(r">\s+<", "><", re.sub(r"\s+", " ", svg)).strip()


def copy_button(label):
    return (f'<button type="button" class="copy" aria-label="{label}">'
            f'{icon("copy", 14)}{icon("check", 14)}<span class="copy-label">Copy</span></button>')


def code_figure(label, body, lang=""):
    return (
        f'<figure class="code" data-lang="{html.escape(lang)}"><figcaption><span>{html.escape(label)}</span>'
        f'{copy_button("Copy code")}</figcaption>'
        f'<pre><code>{body}</code></pre></figure>'
    )


def output_figure(text):
    return f'<figure class="output"><figcaption><span>Output</span></figcaption><pre>{html.escape(text)}</pre></figure>'


def fence_html(info, code, keywords):
    lang = info.split()[0] if info else ""
    if lang == "output":
        return output_figure(code)
    body = highlight_yidika(code, keywords) if lang in YIDIKA_FENCES else html.escape(code)
    return code_figure(FENCE_LABELS.get(lang, lang or "text"), body, "yidika" if lang in YIDIKA_FENCES else lang)


def make_renderer(keywords, toc):
    md = MarkdownIt("commonmark", {"html": True}).enable("table")

    def fence(self, tokens, idx, options, env):
        return fence_html(tokens[idx].info.strip(), tokens[idx].content, keywords)

    def heading_open(self, tokens, idx, options, env):
        token = tokens[idx]
        text = plain_heading(tokens[idx + 1].content)
        slug = slugify(text)
        if token.tag in ("h2", "h3"):
            toc.append((token.tag, text, slug))
        if token.tag in ("h2", "h3"):
            return f'<{token.tag} id="{slug}"><a class="anchor" href="#{slug}" aria-label="Link to this section">{icon("hash", 16)}</a>'
        return f'<{token.tag} id="{slug}">'

    def link_open(self, tokens, idx, options, env):
        tokens[idx].attrSet("href", rewrite_link(tokens[idx].attrGet("href")))
        return self.renderToken(tokens, idx, options, env)

    def image(self, tokens, idx, options, env):
        tokens[idx].attrSet("src", rewrite_link(tokens[idx].attrGet("src")))
        return self.renderToken(tokens, idx, options, env)

    def table_open(self, tokens, idx, options, env):
        return '<div class="table-wrap"><table>'

    def table_close(self, tokens, idx, options, env):
        return "</table></div>"

    md.add_render_rule("fence", fence)
    md.add_render_rule("heading_open", heading_open)
    md.add_render_rule("link_open", link_open)
    md.add_render_rule("image", image)
    md.add_render_rule("table_open", table_open)
    md.add_render_rule("table_close", table_close)
    return md


def nav_item_html(item, current):
    if "label" in item:
        return f'<li class="sub">{html.escape(item["label"])}</li>'
    current_attr = ' aria-current="page"' if item["file"] == current else ""
    nested = ' class="nested"' if item["nested"] else ""
    return f'<li{nested}><a href="{item["file"][:-3]}.html"{current_attr}>{html.escape(item["title"])}</a></li>'


def nav_html(nav, current):
    return "".join(
        f'<h2>{group_icon(group["title"], 16)}{html.escape(group["title"])}</h2><ul>{"".join(nav_item_html(item, current) for item in group["items"])}</ul>'
        for group in nav
    )


def group_icon(title, size):
    return icon(GROUP_ICONS[title], size) if title in GROUP_ICONS else ""


def toc_html(toc):
    items = "".join(f'<li class="{tag}"><a href="#{slug}">{html.escape(text)}</a></li>' for tag, text, slug in toc)
    if not items:
        return ""
    return f'<nav class="toc" aria-label="On this page"><h2>On this page</h2><ul>{items}</ul></nav>'


def pager_html(order, current):
    names = [name for _, name, _ in order]
    if current not in names:
        return ""
    i = names.index(current)
    prev_link = f'<a class="prev" href="{names[i - 1][:-3]}.html"><small>{icon("arrow-left", 14)}Previous</small>{html.escape(order[i - 1][0])}</a>' if i > 0 else "<span></span>"
    next_link = f'<a class="next" href="{names[i + 1][:-3]}.html"><small>Next{icon("arrow-right", 14)}</small>{html.escape(order[i + 1][0])}</a>' if i + 1 < len(names) else "<span></span>"
    return f'<nav class="pager" aria-label="Pages">{prev_link}{next_link}</nav>'


def page_title(markdown):
    match = re.search(r"^# (.+)$", markdown, re.M)
    return match.group(1).strip() if match else "Yidika"


def fill(template, values):
    with_icons = re.sub(r"\{\{icon:([a-z-]+)(?::(\d+))?\}\}", lambda m: icon(m.group(1), int(m.group(2) or 16)), template)
    return re.sub(r"\{\{(\w+)\}\}", lambda m: values[m.group(1)], with_icons)


def page_actions(stem, edit_path):
    return (
        '<div class="page-actions">'
        f'<button type="button" class="copy-page" data-src="{stem}.md">{icon("copy", 14)}{icon("check", 14)}<span class="copy-label">Copy page</span></button>'
        f'<a href="{stem}.md">{icon("file-text", 14)}View as Markdown</a>'
        f'<a href="{REPO}/edit/main/{edit_path}">{icon("pencil", 14)}Edit on GitHub</a></div>'
    )


def breadcrumb(trail):
    parts = '<span aria-hidden="true">/</span>'.join(f"<span>{html.escape(part)}</span>" for part in trail)
    return f'<nav class="crumbs" aria-label="Breadcrumb">{parts}</nav>'


def callout_html(match):
    icon_name, title = CALLOUTS[match.group(1)]
    return (f'<aside class="callout callout-{match.group(1).lower()}">'
            f'<p class="callout-title">{icon(icon_name)}{title}</p><p>{match.group(2)}</aside>')


def with_callouts(rendered):
    return re.sub(r"<blockquote>\s*<p>\[!(NOTE|TIP|WARNING)\]\s*(.*?)</blockquote>", callout_html, rendered, flags=re.S)


FIGURE = r'<figure class="code" data-lang="{lang}">.*?</figure>'
OUTPUT = r'(?:\s*<figure class="output">.*?</figure>)?'
SIDE_BY_SIDE = re.compile(f"({FIGURE.format(lang='(?!yidika)[a-z]+')})\\s*({FIGURE.format(lang='yidika')}{OUTPUT})", re.S)


def with_compare(rendered):
    return SIDE_BY_SIDE.sub(r'<div class="compare"><div>\1</div><div>\2</div></div>', rendered)


def render_markdown(markdown, keywords):
    toc = []
    return with_compare(with_callouts(make_renderer(keywords, toc).render(markdown))), toc


def write_page(out, name, layout, values):
    (out / name).write_text(fill(layout, values))


def template(name):
    return (SITE / name).read_text()


def doc_values(meta, nav, current, title, toc, content):
    values = dict(meta, page_title=f"{html.escape(title)} | Yidika", nav=nav_html(nav, current), toc=toc_html(toc),
                  content=content, body_class="doc", header_width=WIDE_HEADER)
    values["main"] = fill(template("doc.html"), values)
    return values


def build_doc(name, group, layout, nav, order, keywords, meta, out):
    markdown = (DOCS / name).read_text()
    body, toc = render_markdown(markdown, keywords)
    stem = name[:-3]
    title = page_title(markdown)
    head = '<div class="doc-head">' + breadcrumb(["Docs", group]) + page_actions(stem, f"docs/{name}") + "</div>"
    write_page(out, f"{stem}.html", layout, doc_values(meta, nav, name, title, toc, head + body + pager_html(order, name)))
    shutil.copy(DOCS / name, out / name)
    return search_entries(markdown, title, f"{stem}.html", group)


def build_changelog(layout, nav, keywords, meta, out):
    markdown = (ROOT / "CHANGELOG.md").read_text()
    body, toc = render_markdown(markdown, keywords)
    head = '<div class="doc-head">' + breadcrumb(["Project"]) + page_actions("changelog", "CHANGELOG.md") + "</div>"
    toc = [entry for entry in toc if entry[0] == "h2"]
    write_page(out, "changelog.html", layout, doc_values(meta, nav, "", "Changelog", toc, head + body))
    shutil.copy(ROOT / "CHANGELOG.md", out / "changelog.md")
    return search_entries(markdown, "Changelog", "changelog.html", "Project")


def search_entries(markdown, title, url, group):
    entries = [{"t": title, "s": group, "u": url}]
    anchor = ""
    heading = title
    seen = set()
    for line in markdown.splitlines():
        head = re.match(r"^##+ (.+)$", line)
        if head:
            heading = plain_heading(head.group(1).strip())
            anchor = "#" + slugify(heading)
            entries.append({"t": heading, "s": title, "u": url + anchor})
            continue
        for word in re.findall(r"`([A-Za-z_][\w.]*)(?:\(\))?`", line):
            if (word, anchor) not in seen and len(word) > 2:
                seen.add((word, anchor))
                entries.append({"t": word, "s": title if heading == title else f"{title}, {heading}", "u": url + anchor, "c": 1})
    return entries


def example_blocks(name, heading):
    markdown = (DOCS / name).read_text()
    if heading:
        markdown = markdown[markdown.index(f"## {heading}"):]
    fences = re.findall(r"^```([\w-]*)\n(.*?)^```", markdown, re.S | re.M)
    return fences[0], fences[1]


def tab_html(group, i, tab, keywords):
    label, icon_name, name, heading, link_text = tab
    (code_lang, code), (result_lang, result) = example_blocks(name, heading)
    tab_id = f"{group}-{slugify(label)}"
    focus = "" if i == 0 else ' tabindex="-1"'
    tab_icon = icon(icon_name) if icon_name else ""
    button = (f'<button type="button" role="tab" id="tab-{tab_id}" aria-controls="{tab_id}" '
              f'aria-selected="{"true" if i == 0 else "false"}"{focus}>{tab_icon}{html.escape(label)}</button>')
    anchor = "#" + slugify(heading) if heading else ""
    link = f'<a class="more" href="{name[:-3]}.html{anchor}">{html.escape(link_text)}{icon("arrow-right", 14)}</a>'
    panel = (f'<div class="tabpanel" role="tabpanel" id="{tab_id}" aria-labelledby="tab-{tab_id}"{"" if i == 0 else " hidden"}>'
             f'{fence_html(code_lang, code, keywords)}{fence_html(result_lang, result, keywords)}{link}</div>')
    return button, panel


def tabs_html(group, tabs, label, keywords):
    pairs = [tab_html(group, i, tab, keywords) for i, tab in enumerate(tabs)]
    buttons = "".join(button for button, _ in pairs)
    panels = "".join(panel for _, panel in pairs)
    return f'<div class="tabs" role="tablist" aria-label="{label}">{buttons}</div>{panels}'


def docmap_html(nav):
    columns = []
    for group in nav:
        links = "".join(nav_item_html(dict(item, nested=False), "") if "file" in item else nav_item_html(item, "")
                        for item in group["items"])
        columns.append(f'<div><h3>{group_icon(group["title"], 18)}{html.escape(group["title"])}</h3><ul>{links}</ul></div>')
    return "".join(columns)


def build_home(layout, nav, keywords, meta, out):
    sample = (ROOT / "benchmarks" / "fib" / "fib.yk").read_text()
    values = dict(meta, sample=highlight_yidika(sample, keywords), docmap=docmap_html(nav),
                  tabs=tabs_html("tour", TOUR_TABS, "Language examples", keywords),
                  build=tabs_html("build", BUILD_TABS, "Programs you can build", keywords),
                  languages=language_cards(nav))
    write_page(out, "index.html", layout, dict(meta, page_title="Yidika, a typed language that runs now or compiles to native code",
                                                 main=fill(template("home.html"), values), body_class="home",
                                                 header_width=NARROW_HEADER))


def language_pages(nav):
    group = next(group for group in nav if group["title"] == LANGUAGE_GROUP)
    return [(item["title"], item["file"]) for item in group["items"] if "file" in item]


def first_paragraph(markdown):
    return next(block for block in markdown.split("\n\n")[1:] if block.strip() and not block.startswith("#")).strip()


def inline_markdown(text):
    return MarkdownIt("commonmark").renderInline(text)


def language_cards(nav):
    cards = []
    for title, name in language_pages(nav):
        intro = first_paragraph((DOCS / name).read_text()).split(". ")[0] + "."
        cards.append(
            f'<a class="group flex flex-col gap-3 rounded-lg border border-rule bg-bg p-6 text-ink no-underline hover:border-accent" href="{name[:-3]}.html">'
            f'<span class="flex items-center justify-between text-sm font-semibold text-muted">Coming from'
            f'<span class="text-faint group-hover:text-accent-ink">{icon("arrow-right", 18)}</span></span>'
            f'<b class="text-2xl font-bold tracking-tight">{html.escape(title)}</b>'
            f'<span class="text-[0.9375rem] leading-relaxed text-muted">{inline_markdown(intro)}</span></a>')
    return "".join(cards)


def language_switch(nav, current):
    links = []
    for title, name in language_pages(nav):
        state = ' aria-current="page"' if name == current else ""
        links.append(f'<a class="rounded-full border border-rule-strong bg-bg px-3.5 py-1.5 text-sm font-medium text-ink no-underline hover:border-ink '
                     f'aria-[current=page]:border-ink aria-[current=page]:bg-ink aria-[current=page]:text-bg" href="{name[:-3]}.html"{state}>{html.escape(title)}</a>')
    return "".join(links)


def without_title_and_intro(markdown):
    intro = first_paragraph(markdown)
    return re.sub(r"^# .+\n", "", markdown, count=1, flags=re.M).replace(intro, "", 1)


def build_learn_page(name, layout, nav, keywords, meta, out):
    markdown = (DOCS / name).read_text()
    body, toc = render_markdown(without_title_and_intro(markdown), keywords)
    title = page_title(markdown)
    values = dict(meta, title=html.escape(title), intro=inline_markdown(first_paragraph(markdown)), content=body,
                  toc=toc_html(toc), languages=language_switch(nav, name))
    write_page(out, f"{name[:-3]}.html", layout, dict(meta, page_title=f"{html.escape(title)} | Learn Yidika",
                                                   main=fill(template("learn-page.html"), values), body_class="learn",
                                                   header_width=NARROW_HEADER))
    shutil.copy(DOCS / name, out / name)
    return search_entries(markdown, title, f"{name[:-3]}.html", "Learn")


def build_learn(layout, nav, meta, out):
    values = dict(meta, languages=language_cards(nav))
    write_page(out, "learn.html", layout, dict(meta, page_title="Learn Yidika", main=fill(template("learn.html"), values),
                                             body_class="learn", header_width=NARROW_HEADER))
    return [{"t": "Learn Yidika", "s": "Learn", "u": "learn.html"}]


def build_not_found(layout, nav, meta, out):
    content = ('<div class="py-16"><h1>This page does not exist</h1>'
               '<p>It may have moved when the documentation was reorganised. Search with <kbd>/</kbd>, '
               'or start from the <a href="quickstart.html">Quickstart</a>.</p></div>')
    values = doc_values(meta, nav, "", "Not found", [], content)
    values["head_extra"] = '<base href="/yidika/">'
    write_page(out, "404.html", layout, values)


def build_llms(nav, meta, out):
    lines = [f"# Yidika {meta['version']}", "", "> A statically typed language with an interpreter and an LLVM native backend that print the same output.", ""]
    full = []
    for group in nav:
        lines.append(f"## {group['title']}")
        for item in group["items"]:
            if "file" in item:
                lines.append(f"- [{item['title']}]({BASE}{item['file']})")
                full.append((DOCS / item["file"]).read_text())
        lines.append("")
    (out / "llms.txt").write_text("\n".join(lines))
    (out / "llms-full.txt").write_text("\n\n".join(full))


def copy_vendor(out):
    name, expected = ANIME
    data = (SITE / "vendor" / name).read_bytes()
    actual = hashlib.sha256(data).hexdigest()
    if actual != expected:
        raise SystemExit(f"site/vendor/{name}: SHA-256 is {actual}, expected {expected}")
    (out / "vendor").mkdir()
    (out / "vendor" / name).write_bytes(data)


def tailwind_platform():
    system = {"Linux": "linux", "Darwin": "macos", "Windows": "windows"}[platform.system()]
    arch = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x64"
    return f"{system}-{arch}" + (".exe" if system == "windows" else "")


def tailwind_binary():
    name = tailwind_platform()
    expected = TAILWIND_SHA256[name]
    path = TAILWIND_CACHE / f"tailwindcss-{TAILWIND_VERSION}-{name}"
    if not path.exists() or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
        url = f"https://github.com/tailwindlabs/tailwindcss/releases/download/v{TAILWIND_VERSION}/tailwindcss-{name}"
        data = urllib.request.urlopen(url, timeout=60).read()
        actual = hashlib.sha256(data).hexdigest()
        if actual != expected:
            raise SystemExit(f"{url}: SHA-256 is {actual}, expected {expected}")
        TAILWIND_CACHE.mkdir(exist_ok=True)
        path.write_bytes(data)
        path.chmod(0o755)
    return path


def build_css(out):
    css = out.resolve() / "style.css"
    subprocess.run([str(tailwind_binary()), "--input", str(SITE / "tailwind.css"), "--output", str(css),
                    "--minify"], check=True, cwd=SITE, capture_output=True)
    if not css.is_file():
        sys.exit(f"Tailwind did not write {css}")


def copy_static(out):
    shutil.copytree(DOCS / "assets", out / "assets")
    shutil.copy(SITE / "site.js", out / "site.js")
    build_css(out)
    copy_vendor(out)
    (out / ".nojekyll").write_text("")


def build(out):
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    keywords = read_keywords()
    nav = read_nav()
    order = pages_in(nav)
    version = read_version()
    meta = {"version": version, "released": read_release_date(version), "repo": REPO, "registry": REGISTRY, "base": BASE, "head_extra": ""}
    layout = (SITE / "layout.html").read_text()
    search = []
    learn = {name for _, name in language_pages(nav)}
    for _, name, group in order:
        if name in learn:
            search.extend(build_learn_page(name, layout, nav, keywords, meta, out))
        else:
            search.extend(build_doc(name, group, layout, nav, order, keywords, meta, out))
    search.extend(build_learn(layout, nav, meta, out))
    search.extend(build_changelog(layout, nav, keywords, meta, out))
    build_home(layout, nav, keywords, meta, out)
    build_not_found(layout, nav, meta, out)
    build_llms(nav, meta, out)
    (out / "search.json").write_text(json.dumps(search, separators=(",", ":")))
    copy_static(out)


if __name__ == "__main__":
    build((Path(sys.argv[1]) if len(sys.argv) > 1 else SITE / "dist").resolve())
