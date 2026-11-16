# site

Builds the documentation website from `docs/*.md`.

```sh
pip install markdown-it-py==3.0.0
python site/build.py          # writes site/dist
python -m http.server -d site/dist
```

The navigation comes from `docs/README.md`, the keywords to highlight from `src/syntax/lexer.rs`, the version from `Cargo.toml`, and the home page sample from `benchmarks/fib/fib.yk`, so none of them is copied here. `.github/workflows/site.yml` publishes `site/dist` to GitHub Pages on every push to `main`.

Colors, type and spacing are the tokens at the top of `style.css`. The logo is the tisserin (`docs/assets/yidika-logo.svg`, and the full scene in `tisserin.svg`). The palette starts from it: ink `#1d1b18` on paper `#faf8f4`, one green accent `#0b6e4f` (the strand it weaves), and its yellow kept for the bird only. Type is IBM Plex Sans and Plex Mono.
