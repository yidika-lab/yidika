# Changelog

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
