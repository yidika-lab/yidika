# site

Builds the documentation website from `docs/*.md`.

```sh
pip install markdown-it-py==3.0.0
python site/build.py          # writes site/dist
python -m http.server -d site/dist
```

The navigation comes from `docs/README.md`, the keywords to highlight from `src/syntax/lexer.rs`, the version from `Cargo.toml`, and the home page sample from `benchmarks/fib/fib.yk`, so none of them is copied here. `.github/workflows/site.yml` publishes `site/dist` to GitHub Pages on every push to `main`.

Colors, type and spacing are the tokens at the top of `style.css`. The logo is the head of a tisserin, in one color plus the green strand it weaves (`docs/assets/yidika-logo.svg`). The palette starts from it: ink `#1d1b18` on paper `#faf8f4`, one green accent `#0b6e4f`. Type is IBM Plex Sans and Plex Mono.
