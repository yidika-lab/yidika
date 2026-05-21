# Changelog

## [0.1.0] — 2022

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
