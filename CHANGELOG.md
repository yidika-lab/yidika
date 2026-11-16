# Changelog

## [Unreleased]

### Fixed

- Native binaries no longer print a wrong value where the interpreter is right (#96):
  - a method inherited from a parent class returns its real value, and a child class keeps the parent's fields first in memory;
  - a set literal drops duplicates, and sets print as `set{...}`;
  - a generic function is compiled once per argument type, so `id("s")` returns `s`;
  - `await` returns the value computed by `spawn`;
  - lists of `str`, `real` and `bool` print and index correctly, and strings stored in a list stay valid after the function that built it returns;
  - an index out of bounds stops the program with the same error as the interpreter instead of returning 0.
- `if (c) { "a" } else { "b" }` used as an expression yields the branch value in both engines (it was `none` and `0`).
- `spawn` with arguments no longer overflows the interpreter's stack.

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
