# Changelog

## [Unreleased]

### Changed

- `benchmarks/gate.py` reports the size of both binaries and, on Linux, the libraries a Yidika program links.
- A native program starts its stdout flusher thread at its first `print`, not at startup. A program that prints nothing no longer creates a thread: an empty program starts in 314 us instead of 359 us (C: 300 us, Rust: 458 us; median of 5 runs of 3000 `posix_spawn` calls on one core).

- Integers are formatted with a two-digits-at-a-time table instead of `snprintf`, and `+` and `str(int)` write their result into a stack slot instead of a heap header that was freed right after. 2 million `print(i)` take 0.04 s (C `printf`: 0.11 s, Rust `println!`: 0.38 s), and building 3 million short strings takes 0.10 s (Rust: 0.10 s, C with `snprintf`: 0.18 s).

- Native functions start on a 64-byte boundary (`-mllvm -align-all-functions=6`). With `ld.lld`, which `yidi` prefers when installed, a hot recursive function could land across a cache line: `fib(40)` took 0.31 s against 0.28 s for the same C code linked by the default linker. It now takes 0.27-0.28 s (Rust: 0.31 s). Other benchmarks are unchanged, and a binary grows by about 32 bytes.

- When a native program's output goes to a file or a pipe, `print` fills a 64 KB buffer instead of making one system call per line, and a background thread flushes it every 50 ms so output still shows up while the program runs. Printing 2 million lines to a file takes 0.12 s with 3 637 `write` calls, against 0.51 s and 2 million calls before (C: 0.11 s, Rust `println!`: 0.38 s). Output to a terminal is unchanged, and servers flush after every request.

- Top-level variables that no function reads are now ordinary locals of the program instead of globals, so LLVM keeps them in registers and vectorizes the loops that use them. A 500 million iteration `while` loop at the top level drops from 0.83 s to 0.28 s, the same time as the equivalent C (`clang -O3`) and Rust (`rustc -O`) programs.

- Native programs no longer load the C++ standard library and drop the runtime code they do not call. `print("hi")` now builds to a 10 KB binary that starts with 1.6 MB of memory, the same as C, down from 230 KB and 4.1 MB.

- Status messages (`Added`, `Installed`, `Removed`, `Created`, `Published`, `Upgraded`) go to stderr, so stdout carries only a program's output, `--version` and `info`. Errors start with `error:`.
- `yidi upgrade` messages start with their verb: `Checked yidi 0.4.0: already the latest version`, `Found yidi 0.5.0`, `Upgraded yidi from 0.4.0 to 0.5.0`.

### Fixed

- `l: int[] = []` and `m: Map<str, int> = {}` type-check: an empty literal takes the element types of the declaration. Collections of a different element type (`l: int[] = ["a"]`) are still rejected.
- Native programs stop with an error instead of printing a wrong or random value: division and `%` by zero (`division by zero`), `i64::MIN / -1` (`division overflow`), `**` with a negative exponent, `pop()` on an empty list, and a stack overflow (`stack overflow: the recursion probably never ends`, through a guard page like Rust). Division was undefined behaviour in the generated IR. The interpreter gives the same messages, from one table (`trap.rs`) that also generates the C runtime's.
- Integer `+`, `-`, `*`, `**` and unary `-` wrap around in both backends, like Rust built with `--release`; shifts use their amount modulo 64. The interpreter crashed with an internal Rust panic in debug builds and wrapped silently in release builds; the native code used `nsw`, which let LLVM assume overflow never happens. Checking every operation was measured 3x slower on a vectorized loop, so it was not chosen.
- `**` on integers takes logarithmic time: `yk_pow_int` multiplied `b` times, so a large exponent hung the program, and it overflowed signed integers in C.
- `len()` of a nullable value (`str?`) is rejected by the type checker. Natively it returned `0`.
- `l[-1]` is out of bounds in the interpreter too. It returned the last item there and stopped natively. The error for a string index gives its length in characters, not bytes.
- An `if` expression or `?:` whose branch creates its own blocks (a division, a nested `if`) produced invalid IR: the `phi` named the first block of the branch instead of the last.

- Lambda parameters can have a type: `(n: int) => n + 1`, `(who: str, times: int) => ...`. The `:` made the parser read `(n: int)` as a vector literal. All lambda forms now go through one parser function.
- `input()` works in native programs; it was an undefined symbol. Both backends read the line with the same function in `yk_runtime`, which caps a line at 1 MiB and replaces invalid UTF-8.
- `name = input(); "hi " + name` type-checks: `input` was typed as returning nothing.
- `typeof`, `int`, `real` and `bool` called as functions are rejected by the type checker. No backend implemented them, so they failed only when the program ran. Global builtins and their return types now live in one table, `semantic::builtins`, read by the type checker and the native code generator.

- A top-level list that a function also uses (`kept = []` then `kept.push(x)` inside `fn add`) builds natively. Its global was declared `i64` because list and set literals had no native type, so `kept.push` was compiled as a call to a function value (#125).
- A closure whose body is a `print(...)` call, such as `f = () => print(a)`, builds natively. It was typed as returning `i64` and emitted `ret void 0` (#126).

- Programs that run with `yidi file.yk` now also build with `yidi build`, in cases that used to fail with an LLVM error or that built but behaved differently:
  - a std module imported under another name (`use {p} from "path"`) is recognised natively;
  - a function with no written return type returns the type of its `return` value instead of `void`;
  - `"text" + obj.field` concatenates when the field is a string, in structs and classes;
  - `app.logger(fn)` works in native servers and prints the same lines as the interpreter, including 404s;
  - statements before the `return` of a route handler (`print`, assignments) run natively instead of being dropped; a handler the native server cannot compile yet is now reported at build time instead of silently losing code;
  - `path.join` accepts more than two parts;
  - native servers print the same `Server listening on: host:port` line as the interpreter.

  A new test builds every example in `examples/` natively. The ones that still cannot be built are listed with their reason in `src/compat.rs`.

- `regex.match` and `regex.replace` give the same result in native programs as in `yidi run --interp`: both now use the Rust `regex` crate from `yk_runtime`, so `\w` and `\p{Lu}` follow Unicode, and a pattern runs in linear time instead of risking a stack overflow on long inputs.

### Added

- Quality gates in CI: the compat suite runs with AddressSanitizer and UndefinedBehaviorSanitizer (`YIDI_SANITIZE=address,undefined`), and `benchmarks/gate.py` builds each program of `benchmarks/gate/` in Yidika and in C, then fails if Yidika is more than 1.15x slower, uses more than 1.3x the memory, or prints something different. It runs on Linux and macOS.

- Native programs free their strings. A string built by `+`, `str()` or a function is released when its variable gets a new value, when the function returns, or as soon as a temporary is no longer needed, the way Rust drops a `String`. A loop that builds 3 million short strings now peaks at 4 MB instead of 379 MB, and runs in 0.42 s instead of 0.79 s. When the compiler cannot prove a string has a single owner (it went into a list, a struct or a closure), it keeps it alive, so the rule never frees memory still in use.

- `YIDI_KEEP_IR=1` keeps the generated `.ll` file next to the program, and `YIDI_SANITIZE=address` builds it with AddressSanitizer. `YK_PEAK_MEMORY=1` makes a native program print its peak memory to stderr.

- A shared Rust runtime library, `yk_runtime`, linked into every native program and shipped beside `yidi` in each release archive. Features that need the same code in both engines (JSON, the HTTP client for `fetch`) will live there, so they behave the same on Linux, macOS and Windows. A program that does not use it keeps its size.

- `yidi` shows its progress the way uv does: a bar with the current step for `yidi build`, a line while `yidi run` compiles a program that is not cached yet, a byte count for downloads in `yidi add`, `yidi sync`, `yidi update` and `yidi upgrade`, then one colored line such as `Built fib.yk in 1.21s`. Colors and bars only appear in a terminal; piped output is plain text.

## [0.4.0] - 2026-11-17

### Removed

Each construct now has one spelling. Programs using a removed form get a parse or type error; write the form on the right.

- `println(...)`: use `print(...)`, which already ended the line.
- `infiny { }`: use `loop { }`.
- `None`: use `null`. `None` printed `none` in the interpreter and `0` in native code, so the two engines disagreed.
- `class C use I`: use `class C implements I`. `implements` was lexed as `use`, so `implements {x} from "m"` parsed as an import.
- C-style `for (i = 0; i < n; i += 1)`: use `for (i in 0..n-1)`. Its initialisation was parsed and thrown away, so the loop never ran it.
- `f'...'`: use `f"..."` or `` f`...` ``.
- `tuple{a, b}`, `map{...}` and `Map{...}`: use `(a, b)` and `{k: v}`.
- The types `int 8` (with a space), `{K, V}`, `list<T>` and `set<T>`: use `int8`, `map[K, V]`, `list[T]` and `set[T]`.
- `mut` and `ref` as keywords; they meant nothing (`ref` was another way to write `&`).
- `a ? b : c`, which could never be reached because `?` after a value is the error operator: use `if (a) b else c`.
- Native code no longer accepts `string`, `integer`, `float`, `double` or `boolean` as type names; the type checker already refused them.
- Constants have one form, as in JavaScript: `const limit = 100;` or `const limit: int = 100;`, in a function or at the top of a file. `limit: const<int> = 100`, `limit: const = 100` and `100 as const` are removed.
- Collection types are written as in TypeScript: `int[]`, `Set<int>`, `Map<str, int>`. `list[int]`, `[int]`, `List<int>`, `set[int]` and `map[str, int]` are removed; the parse error names the form to write.

### Fixed

- A `match` run by the interpreter no longer writes `DEBUG match_pattern: ...` lines to stderr.

## [0.3.0] - 2026-11-16

### Added

- Code outside functions (#113): a file can hold statements next to its functions, as a script does. They run first, in order, then `fn main()` if the file has one. A variable created there is visible to every function, which can read it and assign to it, in both engines.
- A function can call a function declared further down the file.
- `RELEASING.md`: the rules for version numbers, the changelog and tags, checked by the release workflow.

### Fixed

- `const NAME: T = value;` at the top of a file is visible inside functions; the type checker lost it and native code did not define it.
- Native servers (#109):
  - `app.serve(3000)` with a port number no longer crashes the binary;
  - an unknown route answers `404 Not Found` with a body, as in the interpreter;
  - `GET` and `POST` on the same path keep their own handler in both engines.
- Native binaries no longer print a wrong value where the interpreter is right (#96):
  - a method inherited from a parent class returns its real value, and a child class keeps the parent's fields first in memory;
  - a set literal drops duplicates, and sets print as `set{...}`;
  - a generic function is compiled once per argument type, so `id("s")` returns `s`;
  - `await` returns the value computed by `spawn`;
  - lists of `str`, `real` and `bool` print and index correctly, and strings stored in a list stay valid after the function that built it returns;
  - an index out of bounds stops the program with the same error as the interpreter instead of returning 0.
- `if (c) { "a" } else { "b" }` used as an expression yields the branch value in both engines (it was `none` and `0`).
- `spawn` with arguments no longer overflows the interpreter's stack.
- Native `match` tests every arm in order. Before, a value that failed the first pattern went straight into the second arm's body, so `match n { 0 => "zero", 1 => "one", _ => "many" }` returned `"one"` for 9.
- Native `match` on an enum parameter compares variant tags, and a bare variant pattern such as `Empty` no longer matches everything.
- Native `match` on a `str` builds (the IR used `memcmp` without declaring it).
- Native `base64.encode` pads by the real input length (`"a"` gave `YQA=`, `"abc"` gave `YWJ=`).
- The interpreter no longer crashes on recursion: it stopped with `stack overflow, aborting` after about 600 nested calls in a release build and 17 in a debug build. Its stack now grows on demand, and a program that goes past 10 000 nested calls gets an error that points at the call.

### Security

- Native `base64.decode` read past the end of an input whose length is not a multiple of 4. It now checks every character and padding position, and stops with `base64.decode: invalid input`, as the interpreter does.

### Changed

- A std function that the native backend does not implement (for example `json.parse`) now fails the build with "`json.parse` is not supported by the native backend yet", so `yidi run` falls back to the interpreter instead of running a binary that returns 0.

## [0.2.0] - 2026-11-15

### Added

- `yidi run` compiles to native code by default and caches the binary; `--interp` forces the interpreter. `yidi build` works on Linux and macOS as well as Windows.
- Closures: lambdas capture the variables of their scope in both engines, can be stored, passed, returned and called where they are written.
- Packages:
  - `yidi init [name]` creates a project, and asks for the name when none is given (`.` is the current folder).
  - `yidi add`, `yidi remove`, `yidi sync` and `yidi update`, with semver ranges, dependencies of dependencies, and a `yidi.lock` that records the commit and SHA-256 of every package.
  - Packages live in `@yidi/<name>/` and are imported by name: `use {hello} from "greet";`.
  - One download cache shared by all projects, hard-linked into each project; a second install needs no network.
  - A registry, <https://yidika-lab.github.io/registry/>: `yidi add greet` by short name, `yidi publish` to release a package, `YIDI_REGISTRY` for a mirror or a private index.
- `yidi upgrade` installs the latest release over the running binary after checking its SHA-256; `--check` only reports.
- `--watch` reacts to file-system events and restarts the program in about 0.2 s.
- One-command installers for Linux, macOS and Windows.

### Changed

- Maps and structs keep insertion order.
- `print(a, b)` prints on one line; chars print as characters; reals print their shortest exact form in both engines.
- The interpreter resolves variables 1.7 to 2 times faster and uses lexical scoping.

### Fixed

- `yidi add` could delete the project directory when a package name was `..` (#89).
- A `return` inside a lambda changed the return type of the enclosing function.
- Integer `==` and `!=`, ranges and several print forms produced wrong or invalid native code.
- The native HTTP server dropped requests over 64 bytes and now sends RFC 9110 status lines.

## [0.1.0] - 2022

Initial MVP release.

### Added

- Lexer with full token set
- Recursive-descent Pratt parser
- AST with span tracking
- Type checker (environment, type inference, const checking)
- Module loader (relative imports, cycle detection, `.yk` extension auto-resolution)
- Tree-walking interpreter
- Built-in functions: `print`, `println`, `len`, `str`, `input`
- List operations: `push`, `pop` (method-style)
- CLI: `check`, `run`, `build` commands
- 48+ unit tests across all modules
- MIT License
