# AGENTS.md - Yidika (`yidi`)

Rust implementation of the **Yidika** language: lexer -> parser -> type checker -> either a tree-walking **interpreter** or an **LLVM-IR AOT backend**. Single crate `yidi` (lib `yidi` + bin `yidi`, `src/main.rs`).

## The one rule that matters: interpreter/AOT parity

Every language and stdlib feature must work in **both** backends. They are separate implementations of the same semantics, and nothing enforces that except tests.

- **Interpreter** - `src/interpret/` (`env.rs` evaluation driver, `expr.rs` expression eval, `stmt.rs`, `builtins.rs`, `class.rs` `FnDef`/`ClassDef`). The `net` module is served by `src/netlib/` (tokio async server; route handlers are MCJIT-compiled at runtime via `LLVM-C.dll`).
- **AOT** - `src/codegen/llvm.rs`. Emits LLVM IR **as text strings**, then shells out to a generated `.bat` that runs `vcvars64.bat` -> `clang -c` -> `cl.exe` (compiles the embedded runtime, `src/codegen/runtime/yk_rt.c`) -> `link.exe`.

`src/compat.rs` (87 `compat_*` tests) is the enforcement: it runs one snippet through both backends and asserts identical stdout via `check_compat`. **Adding a feature without a compat test is the most common way to break this repo.** The harness is `run_interp` / `run_aot` / `check_compat` / `check_compat_approx` (use `_approx` for nondeterministic output like PIDs/timestamps).

## Design rules (mandatory for every change)

- **Cross-platform source, one target per build.** Every language feature, std module, runtime piece and the GUI engine must be designed to work on Windows, Linux and macOS (Android/iOS later). Each build produces a binary for one chosen target, like `cargo build --target`. OS differences live in the runtime/engine behind `#[cfg]` / `#ifdef`, never in user `.yk` code. Do not add single-OS code without the abstraction that lets the other targets plug in. Existing Windows-only parts (the WinHTTP client, `gui/src/win.rs`) are debt to remove, not a pattern to copy.
- **TDD.** Write the failing test first, then the minimal code that passes it, then refactor. Unit tests sit next to the code (`#[cfg(test)] mod tests`); any language or std feature also gets a `compat_*` test.
- **SOLID**, applied to Rust: single responsibility per function/type/module; extend through traits and new modules instead of growing `match` blocks in existing ones; depend on traits at module boundaries, not on concrete backends.
- **KISS.** The simplest design that satisfies the tests. No speculative abstraction or configuration.
- **One function = one task. One type (struct/class/enum/trait) = one intent. One folder = one group of related intents.** A function that needs "and" to be described must be split.
- **No duplicated logic.** Anything written twice is extracted into one reusable function, type or table. Lists and tables must have a single source of truth (e.g. the std surface is `src/stdlib.rs::SUBMODULES`; read it, never copy it). The interpreter and AOT backend implementing the same semantics twice is the one accepted exception; share everything they can share (tables, parsing, validation, the `gui/` engine).
- **Secure by design, no known CVE.** Validate and bound every external input (network, files, FFI, env): size limits, path canonicalisation, header/CRLF sanitisation, no `from_utf8_unchecked` on untrusted bytes. Keep `unsafe` minimal and confined to FFI/ABI edges. New or upgraded dependencies must pass `cargo audit` (RustSec) with no open advisory; the embedded C runtime uses bounded buffers only.
- **One spelling per construct.** A language feature has one way to be written. Do not add a keyword alias, a second literal form or a second builtin name for something that already exists; `src/syntax/one_spelling.rs` pins the spellings that were removed. A second form needs a real use the first cannot cover, and the PR must say which.
- **Research before building.** Before designing a feature, check the state of the art: relevant specs (RFCs, CSS, Unicode), how mature projects solve it, and existing maintained crates, so the repo doesn't reinvent them. State what was checked and why the chosen approach wins in the PR description.

## Writing and design style: no AI tells

Applies to everything an agent produces: code, identifiers, comments in `.yk` examples, docs, commit messages, PR and issue bodies, UI strings, GUI examples, and replies to the user. The goal is text and interfaces that read as written and designed by a careful human, not the statistically average output of a model.

**Characters. Never use:**

| Forbidden | Use instead |
|---|---|
| Em dash `—` (U+2014), en dash `–` (U+2013) | A comma, a colon, parentheses, a new sentence, or `-` for ranges (`10-20`) |
| Curly quotes `“ ” ‘ ’` | Straight quotes `" '` |
| Ellipsis character `…` | Three dots `...`, or rephrase |
| Arrows `→ ← ⇒ ↔` in prose | Words ("then", "becomes"), or `->` inside code |
| Decorative symbols `• ✓ ✔ ✗ ★ ✨` and emoji (`✅ ❌ 🚀 💡 🔥 👉`) in headings, lists, commits, logs, UI | Plain words, Markdown `-` lists, real icons in UI |
| Invisible characters: non-breaking space (U+00A0, U+202F), zero-width space/joiner (U+200B, U+200D), BOM | Normal spaces; nothing |
| `×` for dimensions or multiplication in prose | `x` (`1920x1080`) |

Before finishing, search your output: `grep -nP '[\x{2013}\x{2014}\x{2018}\x{2019}\x{201C}\x{201D}\x{2026}\x{2192}\x{00A0}\x{200B}]' <files>` must print nothing for the files you touched (the table above is the only exception).

**Phrasing. Avoid:**
- Filler words that signal generated text: "delve", "seamless(ly)", "robust", "leverage", "elevate", "unlock", "empower", "cutting-edge", "game-changer", "tapestry", "testament", "it's worth noting", "in today's fast-paced world".
- Reflexive triplets ("fast, simple, and powerful") and the "not just X, but Y" construction.
- Opening or closing filler ("Great question", "I hope this helps", "In conclusion", "Let me know if you have any questions").
- Bold on every other phrase, a heading over every paragraph, bullet lists for things that read better as a sentence.
- Vague claims without a number or a reference. Say what was measured or checked.

**Design (GUI examples, docs pages, any UI). Avoid:**
- Purple/indigo/violet gradients (`#6366f1`, `#8b5cf6`, Tailwind `indigo-500` defaults), gradient text, neon glow, glassmorphism.
- Emoji or Unicode glyphs used as icons, including `☰` / `✕` / `⋮` for a mobile menu, close or overflow button. Use one consistent SVG icon set, an accessible `<button>` with a label (`aria-label="Open menu"`), and a visible focus state.
- The template layout: centered hero with one call-to-action, then three identical rounded cards with an icon each.
- The same large radius, drop shadow and pill badge on every element; Inter everywhere because it is the default.
- Mobile menus as a generic full-height slide-in drawer with a blurred backdrop by default. Start from the platform convention (bottom tab bar for 3 to 5 top-level screens, native navigation stack otherwise) and justify anything else.

**No AI attribution on GitHub.** Commits, PRs, issues, comments, releases and changelogs never credit an AI: no `Co-Authored-By:` trailer naming an AI or bot, no "Generated with ..." line, no AI or bot account as author or committer. Work is committed under the human maintainer's git identity, and `gh` actions run as that maintainer.

Instead: start from a stated reference (a brand, a platform's guidelines such as Apple HIG or Material, or a product the user names), choose the palette, type scale and spacing scale on purpose, and keep them in one shared token table (see "No duplicated logic").

## Prerequisites for AOT

| Need | Used by | Failure mode |
|---|---|---|
| Linux / macOS: `clang++` on `PATH` (`apt install clang`, `xcode-select --install`) | AOT IR + runtime compile and link (`codegen/posix_toolchain.rs`) | `"clang++ not found on PATH"` |
| LLVM (`clang.exe`, default `C:\Program Files\LLVM\bin`) | AOT IR -> obj | `"clang.exe not found"` |
| MSVC Build Tools (`vcvars64.bat`, `cl.exe`, `link.exe`) | AOT runtime compile + link | `"Visual Studio 2019/2022/2025 not found"` |
| `LLVM-C.dll` on `PATH` | interp `net` MCJIT handlers | handler JIT result is discarded (`let _ = compile_*_handler(...)` in `netlib/mod.rs`) - server runs un-JITed; `netlib` tests print `SKIP:` and pass |

`detect_vcvars()` / `detect_clang()` (`llvm.rs`, `llvm_api.rs::find_llvm_lib`) probe fixed paths then `PATH` on Windows. On Linux and macOS the runtime is compiled once per runtime version and compiler, then cached in `$XDG_CACHE_HOME/yidi` (`~/.cache/yidi`, `~/Library/Caches/yidi` on macOS): the first `yidi build` takes about 8 s, later ones about 0.2 s.

## Commands

```sh
cargo run -- <file>.yk           # run (bare path == run)
cargo run -- run <file>.yk       # compile once into ~/.cache/yidi/run, run the binary; interpreter if the AOT cannot build it
cargo run -- run <file>.yk --interp  # interpreter only
cargo run -- build <file>.yk     # AOT compile -> <file>.exe next to the source
cargo run -- <file>.yk --watch   # OS file events (notify); restarts the program on every .yk change
cargo run -- info                # detected hardware, printed to stdout

cargo test                       # everything - SLOW, see below
cargo test --lib -- --skip compat  # fast loop; skips every AOT-invoking test
cargo test --lib compat_net_     # single compat test by substring
cargo check --tests             # tests compile; needs no MSVC/LLVM
```

### CI and releases

`.github/workflows/ci.yml` runs: clippy `-D warnings` (blocking for the `gui` crate; non-blocking for the workspace until the 116-lint backlog in issue #14 is cleared), `cargo test --workspace --lib --bins -- --skip compat` on Linux, macOS and Windows, the `compat` suite on a Windows runner (its LLVM + Visual Studio install is enough for the AOT toolchain), and `cargo audit`.

- `compat_*` runs on Windows (MSVC toolchain) and on Linux and macOS (`clang++`). On Linux it takes about 15 s once the runtime is cached, against about 5 min on the Windows runner. Tests that need WinHTTP or the GUI engine are ignored outside Windows (#65).
- `cargo fmt --check` **fails on `main`**, so the `format` job is `continue-on-error`. Running `cargo fmt` reformats the whole tree into a huge diff; do it in its own commit, never inside a feature change (issue #2).
- Zero Rust warnings is a hard project rule (`CONTRIBUTING.md` + clippy `-D warnings`). Keep it.
- Releases follow `RELEASING.md` (semver, Keep a Changelog, tag from `main` only); the release workflow refuses a tag that breaks those rules. Every user-visible PR adds a line under `## [Unreleased]` in `CHANGELOG.md`.
- `.github/workflows/release.yml`: pushing a tag `vX.Y.Z` equal to the `Cargo.toml` version builds and publishes `yidi-<target>.tar.gz|zip` + `.sha256` for Linux, macOS and Windows. `install.sh` / `install.ps1` download from `releases/latest/download/`, so asset names must stay version-less.

### Test gotchas

- `src/compat.rs` serializes all AOT work behind a global `AOT_LOCK: Mutex<()>`, and each AOT test shells out to `vcvars` + `clang` + `cl.exe` + `link.exe`. So the compat suite runs strictly one at a time and dominates total `cargo test` wall time. Always take it through `aot_lock()`, which recovers from poisoning; still drop the guard before assertions so a failure does not leave a server process or build directory behind.
- AOT builds write `yk_rt.c`, `yk_rt.obj`, `<name>.ll`, `<name>.obj` next to the `.yk` source and only clean them up on success. Interrupt a failing build and they linger.

## Layout notes a filename won't tell you

- **`src/codegen/runtime/yk_rt.c` (~150 KB, ~3 900 lines)** is the C/C++ runtime compiled into every AOT binary (HTTP/1.1 + HPACK/HTTP-2, WebSocket, radix route trie, TLS memory pool; Winsock-IOCP server on Windows, a blocking worker-pool server on POSIX, and an io_uring server behind `-DYK_USE_URING` that does not compile yet, #66). `llvm.rs` embeds it with `include_str!` and prepends the status table from `runtime_status.rs`. It `#define WIN32_LEAN_AND_MEAN` before including `winsock2.h` - do not remove that. OS calls go through the small platform layer at the top of the file (`yk_thread_spawn`, `yk_mutex_lock`, `yk_sleep_ms`, `yk_path_kind`, `yk_send_all`, ...). Add to it instead of calling Win32 or POSIX directly.
- **`src/codegen/llvm.rs` (~280 KB)** is the IR emitter (`LlvmCodegen` + its `HashMap` side-tables) and the Windows toolchain shell-out.
- **`rt/` (crate `yk_runtime`)** is the shared Rust runtime: one implementation that the interpreter calls as a Rust library and that native binaries link as a static library (`libyk_runtime.a`, `yk_runtime.lib` on Windows). Put a std feature here when the C runtime would need a second implementation of it (JSON, HTTP client, TUI). `codegen/runtime_lib.rs` finds the library beside `yidi` or in the cargo profile directory and adds the system libraries each OS needs; build it with `cargo build -p yk_runtime` before running tests. Windows builds use the static CRT (`.cargo/config.toml`) because native programs link `libcmt`.
- **`src/codegen/llvm_api.rs`** loads `LLVM-C.dll` at runtime via `libloading`. Used by `src/jit/` and `netlib`. `llvm.rs::compile_to_exe_fast` / `cache_runtime_obj` are `#[allow(dead_code)]` dead paths kept for reference - don't assume they run.
- **`src/netlib/hyper_server.rs`** calls many `pub(crate)` helpers from `netlib/mod.rs` (`parse_request_line`, `execute_handler`, `write_response_async`, ...). Editing any of those signatures breaks the server.
- **Server handlers get an isolated interpreter** via `SubInterpreterPool` (`interpret/env.rs`), not a shared `Interpreter`.
- **FFI import libs** are looked up at `<output_dir>/lib/ffi/yk_ffi_<name>.lib` (`codegen/backend.rs`); they are built by `src/module/mod.rs` from `use ... from "c++:path"` / `"rust:path"` imports.
- `src/bin/lex_test.rs` is a scratch binary hardcoded to `examples/basics/main.yk`.
- `install.cmd` is an unrelated **Claude Code** installer. Ignore it.

## Language surface that trips people up

- Collection types are written as in TypeScript: `T[]`, `Set<T>`, `Map<K, V>`. `list[T]`, `[T]`, `List<T>`, `set[T]` and `map[K, V]` are parse errors that name the right form.
- Mutable by default; `const x = v;` or `const x: T = v;` is the immutable form, in a function or at the top of a file. Reassigning a `const` is a type error.
- `if (cond)` / `while (cond)` require parentheses.
- Handler sugar the parser desugars: `app.get("/p") { body }` -> an **argumentless** `Expr::Closure` (`parser.rs:1095`). It is only enabled for `get, post, put, delete, patch, ws, serve` - `app.logger(f) { }` will not parse.
- Handler values come in three AST shapes - `Expr::LitStr` (static body), `Expr::Ident` (function ref), `Expr::FnLit` / `Expr::Closure`. All must be handled in `compile_call`, `compile_expr` **and** the interpreter dispatch; missing one shows up only as a compat-test mismatch.
- `unsupported_stds` in `llvm.rs::validate_modules` is `[]` - every std module is claimed AOT-supported. Keep it true.
- Std module surface is listed once in `src/stdlib.rs::SUBMODULES`; the interpreter's implementation is a separate dispatch in `interpret/builtins.rs` / `interpret/expr.rs`.

## Docs that will waste your time

- `docs/dev/ARCHITECTURE.md` describes an MLIR dialect pipeline (YK-Core / YK-Mem / YK-Hardware). **It does not exist.** Real codegen emits LLVM IR text directly from `llvm.rs`.
- `README.md`'s "Project Structure" omits `src/netlib`, `src/jit`, `src/codegen`, `src/hardware`, `src/memory`, `src/package`, `src/runtime`.
- `TASKS.md` is the live French-language roadmap for the `net` module (Phase 1 sockets done; HTTP enrichment, SMTP/FTP/WS/MQTT/TLS/QUIC pending). Read it before starting net work.
- `.opencode/plans/` holds design plans, not current state.
- This file supersedes the old sprint-by-sprint session log; history lives in `git log`.

## Conventions

From `CONTRIBUTING.md`: `cargo fmt` style, **no comments in code** (self-documenting names; `.yk` example files *do* use `//`), small focused functions, conventional-commit messages, one feature per PR. The **Design rules** section above takes precedence.

## Agent autonomy

The user (2026-11) does not want to be asked before running non-critical commands (builds, tests, cargo, reads, local edits). Act and report briefly; reserve confirmation for destructive/irreversible or shared-state actions (force push, deletions outside scratch files, remote services).