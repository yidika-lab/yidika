# Changelog

## [Unreleased]

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
