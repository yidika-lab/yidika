# site

Builds the documentation website from `docs/*.md`.

```sh
pip install markdown-it-py==3.0.0
python site/build.py          # writes site/dist
python -m http.server -d site/dist
```

CSS comes from Tailwind CSS v4.3.4. `build.py` downloads the standalone CLI for the current platform into `site/.cache` (gitignored), checks its SHA-256 against the table in `build.py` (copied from the release's `sha256sums.txt`), and stops if it differs. It then writes `dist/style.css`, minified, with only the classes used in `layout.html`, `home.html`, `doc.html`, `learn.html`, `learn-page.html`, `build.py` and `site.js`.

The pages listed under "Coming from" in `docs/README.md` are built with `learn-page.html` instead of the docs layout: no sidebar, a language switcher, and a code block in another language followed by a Yidika block is shown side by side. `learn.html` is the hub the Learn button opens.

Nothing is copied by hand. The navigation, grouped by need (Learn, Guides, Reference, Understand), comes from `docs/README.md`; highlighted keywords from `src/syntax/lexer.rs`; the version from `Cargo.toml`; the home page sample from `benchmarks/fib/fib.yk`; the four language examples on the home page from the first tested example under a heading of the tour pages (`HOME_TABS` in `build.py`). The build also writes `changelog.html` from `CHANGELOG.md`, `search.json` (pages, headings and inline code names, searched with `/` or Ctrl+K), `llms.txt`, `llms-full.txt`, a Markdown copy of each page, and `404.html`.

## Design

- Tokens: one table at the top of `tailwind.css`. Colors are CSS variables on `:root`, redefined for dark mode, and exposed to Tailwind through `@theme inline`, so a utility such as `bg-surface` or `text-muted` follows the theme without `dark:` variants. The `@layer components` block styles the HTML that `build.py` generates from Markdown (code blocks, tables, callouts, sidebar, side-by-side comparisons). Ink `#0b0d0c` on white `#ffffff`, the strand green `#0a7a55` as accent, the tisserin yellow `#d99a00` only for signals; code blocks are dark in both themes.
- Type: Geist for headings and text, Geist Mono for code with ligatures off, so `->` stays two characters. One family keeps the page calm; weight and size carry the hierarchy.
- Header: 64 px (56 on phones), 24 px side padding, logo 28 px, links 15 px spaced 24, search field 240x36.
- Background: pure white (#ffffff) in light mode, near black (#0b0d0c) in dark mode. No pattern: one faint green light (`--glow`, 10% accent) sits behind the hero code.

## Header and theme

The header is 72 px tall (60 on phones) with 40 px side padding (16 on phones) and 32 px between groups; on the home page it lines up with the 1200 px content. The theme button cycles through system, light and dark. The choice is stored in `localStorage` under `yidika-theme`, and a small inline script in `layout.html` applies it before the first paint, so a dark page never flashes white.

## Callouts and exercises

A blockquote that starts with `[!NOTE]`, `[!TIP]` or `[!WARNING]` (the GitHub alert syntax, so the Markdown reads the same on GitHub) becomes a callout with a Lucide icon. A guide ends with a "Try it" exercise whose solution sits in `<details>`.

## Icons

Icons are [Lucide](https://lucide.dev) 1.66.0 (ISC, `site/icons/LICENSE`), copied as SVG files into `site/icons/` from `lucide-static@1.66.0/icons/`. Templates write `{{icon:name}}` or `{{icon:name:size}}`, Python code calls `icon(name, size)`, and the build inlines the SVG with `aria-hidden="true"`, so icons follow `currentColor` and need no request. A missing file stops the build. To add one, copy `https://cdn.jsdelivr.net/npm/lucide-static@1.66.0/icons/<name>.svg` into `site/icons/`. The GitHub mark is the one exception: Lucide ships no brand icons, and a brand is shown with its own mark (Octicons `mark-github`).

## Motion

`site.js` loads Anime.js 4.5.0 from `site/vendor/`; `build.py` refuses to build if its SHA-256 differs from the one recorded there, which matches the file jsDelivr serves for `animejs@4.5.0/dist/bundles/anime.esm.min.js`. Anime.js is imported on its own, so copy buttons, tabs and search work even if it fails to load.

| Motion | Where | When |
|---|---|---|
| Code tokens fall into place | home hero | once on load |
| Strand drawn from the beak | home hero logo | once on load |
| Bars grow to scale, numbers count | speed section | once, when visible |
| Wires drawn through both engines | engines section | once, when visible |
| Rise and fade, 60 ms stagger | blocks marked `data-reveal` | once, when visible |

Nothing loops or follows the pointer, documentation pages only move the "On this page" marker, and with `prefers-reduced-motion` nothing moves at all. Every element is readable before any script runs.
