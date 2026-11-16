# site

Builds the documentation website from `docs/*.md`.

```sh
pip install markdown-it-py==3.0.0
python site/build.py          # writes site/dist
python -m http.server -d site/dist
```

Nothing is copied by hand. The navigation, grouped by need (Learn, Guides, Reference, Understand), comes from `docs/README.md`; highlighted keywords from `src/syntax/lexer.rs`; the version from `Cargo.toml`; the home page sample from `benchmarks/fib/fib.yk`; the four language examples on the home page from the first tested example under a heading of the tour pages (`HOME_TABS` in `build.py`). The build also writes `changelog.html` from `CHANGELOG.md`, `search.json` (pages, headings and inline code names, searched with `/` or Ctrl+K), `llms.txt`, `llms-full.txt`, a Markdown copy of each page, and `404.html`.

## Design

- Tokens: one table at the top of `style.css` (colors for light and dark, type scale, 4 px spacing scale, header height). Ink `#111513` on `#f7f8f6`, the strand green `#0a7a55` as accent, the tisserin yellow `#d99a00` only for signals; code blocks are dark in both themes.
- Type: Schibsted Grotesk for headings, Instrument Sans for text, JetBrains Mono for code with ligatures off, so `->` stays two characters.
- Header: 64 px (56 on phones), 24 px side padding, logo 28 px, links 15 px spaced 24, search field 240x36.
- Background motif: a woven pattern (24 px SVG tile, 7% ink, fading out) behind the home hero only.

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
