# AGENTS.md — Yidika (`yidi`)

Rust implementation of the **Yidika** language: lexer → parser → type checker → either a tree-walking **interpreter** or an **LLVM-IR AOT backend**. Single crate `yidi` (lib `yidi` + bin `yidi`, `src/main.rs`).

## The one rule that matters: interpreter/AOT parity

Every language and stdlib feature must work in **both** backends. They are separate implementations of the same semantics, and nothing enforces that except tests.

- **Interpreter** — `src/interpret/` (`env.rs` evaluation driver, `expr.rs` expression eval, `stmt.rs`, `builtins.rs`, `class.rs` `FnDef`/`ClassDef`). The `net` module is served by `src/netlib/` (tokio async server; route handlers are MCJIT-compiled at runtime via `LLVM-C.dll`).
- **AOT** — `src/codegen/llvm.rs`. Emits LLVM IR **as text strings**, then shells out to a generated `.bat` that runs `vcvars64.bat` → `clang -c` → `cl.exe` (compiles the embedded `RUNTIME_C`) → `link.exe`.

`src/compat.rs` (87 `compat_*` tests) is the enforcement: it runs one snippet through both backends and asserts identical stdout via `check_compat`. **Adding a feature without a compat test is the most common way to break this repo.** The harness is `run_interp` / `run_aot` / `check_compat` / `check_compat_approx` (use `_approx` for nondeterministic output like PIDs/timestamps).

## Prerequisites (Windows only for AOT)

| Need | Used by | Failure mode |
|---|---|---|
| LLVM (`clang.exe`, default `C:\Program Files\LLVM\bin`) | AOT IR→obj | `"clang.exe not found"` |
| MSVC Build Tools (`vcvars64.bat`, `cl.exe`, `link.exe`) | AOT runtime compile + link | `"Visual Studio 2019/2022/2025 not found"` |
| `LLVM-C.dll` on `PATH` | interp `net` MCJIT handlers | handler JIT result is discarded (`let _ = compile_*_handler(...)` in `netlib/mod.rs`) — server runs un-JITed; `netlib` tests print `SKIP:` and pass |

`detect_vcvars()` / `detect_clang()` (`llvm.rs`, `llvm_api.rs::find_llvm_lib`) probe fixed paths then `PATH`.

## Commands

```sh
cargo run -- <file>.yk           # interpret (bare path == run)
cargo run -- run <file>.yk       # interpret explicitly
cargo run -- build <file>.yk     # AOT compile -> <file>.exe next to the source
cargo run -- <file>.yk --watch   # re-run on change (poller, 500ms)
cargo run -- info                # detected hardware, printed to stdout

cargo test                       # everything — SLOW, see below
cargo test --lib -- --skip compat  # fast loop; skips every AOT-invoking test
cargo test --lib compat_net_     # single compat test by substring
cargo check --tests             # tests compile; needs no MSVC/LLVM
```

### CI does not reflect reality

`.github/workflows/ci.yml` runs on `ubuntu-latest`: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.

- The AOT backend is **Windows-only** (no `#[cfg]` guard — it hardcodes `cmd.exe`, `vcvars64.bat`, `link.exe`, `.exe`). Every `compat_*` test necessarily fails on the Linux runner. Don't try to make that CI green; verify locally on Windows.
- `cargo fmt --check` currently **fails on `main`**. Running `cargo fmt` reformats the whole tree into a huge diff — don't fold that into a feature change.
- Zero Rust warnings is a hard project rule (`CONTRIBUTING.md` + clippy `-D warnings`). Keep it.

### Test gotchas

- `src/compat.rs` serializes all AOT work behind a global `AOT_LOCK: Mutex<()>`, and each AOT test shells out to `vcvars` + `clang` + `cl.exe` + `link.exe`. So the compat suite runs strictly one at a time and dominates total `cargo test` wall time. **A panicked assertion while holding the lock poisons it and every later AOT test fails with `PoisonError`** — drop the lock guard before any assertion that can panic.
- AOT builds write `yk_rt.c`, `yk_rt.obj`, `<name>.ll`, `<name>.obj` next to the `.yk` source and only clean them up on success. Interrupt a failing build and they linger.

## Layout notes a filename won't tell you

- **`src/codegen/llvm.rs` (~420 KB, ~8 300 lines)** holds two unrelated things. Lines 10–3939 are `RUNTIME_C`: a C/C++ runtime stored in a raw string literal and compiled into every AOT binary (HTTP/1.1 + HPACK/HTTP-2, WebSocket, radix route trie, TLS memory pool, Winsock-IOCP on Windows / io_uring on Linux). Editing it means editing a Rust string: mind the escaping. It `#define WIN32_LEAN_AND_MEAN` before including `winsock2.h` — do not remove that. Everything from line 3941 on is the IR emitter (`LlvmCodegen` + its `HashMap` side-tables) and the Windows toolchain shell-out.
- **`src/codegen/llvm_api.rs`** loads `LLVM-C.dll` at runtime via `libloading`. Used by `src/jit/` and `netlib`. `llvm.rs::compile_to_exe_fast` / `cache_runtime_obj` are `#[allow(dead_code)]` dead paths kept for reference — don't assume they run.
- **`src/netlib/hyper_server.rs`** calls many `pub(crate)` helpers from `netlib/mod.rs` (`parse_request_line`, `execute_handler`, `write_response_async`, …). Editing any of those signatures breaks the server.
- **Server handlers get an isolated interpreter** via `SubInterpreterPool` (`interpret/env.rs`), not a shared `Interpreter`.
- **FFI import libs** are looked up at `<output_dir>/lib/ffi/yk_ffi_<name>.lib` (`codegen/backend.rs`); they are built by `src/module/mod.rs` from `use ... from "c++:path"` / `"rust:path"` imports.
- `src/bin/lex_test.rs` is a scratch binary hardcoded to `examples/basics/main.yk`.
- `install.cmd` is an unrelated **Claude Code** installer. Ignore it.

## Language surface that trips people up

- `T[]`, `list[T]`, `set[T]`, `map[K,V]` **and** `List<T>`, `Set<T>`, `Map<K,V>` all parse (`syntax/parser.rs`). Both forms are live — don't "normalize" old examples.
- Mutable by default; `const` / `as const` is the immutable form. Reassigning a `const` is a type error.
- `if (cond)` / `while (cond)` require parentheses.
- Handler sugar the parser desugars: `app.get("/p") { body }` → an **argumentless** `Expr::Closure` (`parser.rs:1095`). It is only enabled for `get, post, put, delete, patch, ws, serve` — `app.logger(f) { }` will not parse.
- Handler values come in three AST shapes — `Expr::LitStr` (static body), `Expr::Ident` (function ref), `Expr::FnLit` / `Expr::Closure`. All must be handled in `compile_call`, `compile_expr` **and** the interpreter dispatch; missing one shows up only as a compat-test mismatch.
- `unsupported_stds` in `llvm.rs::validate_modules` is `[]` — every std module is claimed AOT-supported. Keep it true.
- Std module surface is listed once in `src/stdlib.rs::SUBMODULES`; the interpreter's implementation is a separate dispatch in `interpret/builtins.rs` / `interpret/expr.rs`.

## Docs that will waste your time

- `docs/dev/ARCHITECTURE.md` describes an MLIR dialect pipeline (YK-Core / YK-Mem / YK-Hardware). **It does not exist.** Real codegen emits LLVM IR text directly from `llvm.rs`.
- `README.md`'s "Project Structure" omits `src/netlib`, `src/jit`, `src/codegen`, `src/hardware`, `src/memory`, `src/package`, `src/runtime`.
- `TASKS.md` is the live French-language roadmap for the `net` module (Phase 1 sockets done; HTTP enrichment, SMTP/FTP/WS/MQTT/TLS/QUIC pending). Read it before starting net work.
- `.opencode/plans/` holds design plans, not current state.
- This file supersedes the old sprint-by-sprint session log; history lives in `git log`.

## Conventions

From `CONTRIBUTING.md`: `cargo fmt` style, **no comments in code** (self-documenting names; `.yk` example files *do* use `//`), small focused functions, conventional-commit messages, one feature per PR.

## Agent autonomy

The user (2026-11) does not want to be asked before running non-critical commands (builds, tests, cargo, reads, local edits). Act and report briefly; reserve confirmation for destructive/irreversible or shared-state actions (force push, deletions outside scratch files, remote services).