# Yidika (yidi)

A **modern systems programming language** designed for **hardware-aware computing** — blending the safety and expressiveness of Rust with the ergonomics of modern scripting languages.

> **Status**: MVP / Experimental — parsing, type checking, and an interpreter are functional.

---

## Features

- **Mutable by default** — variables are mutable unless declared `const`
- **Strong, static typing** with type inference
- **Algebraic data types** — structs, unions, type aliases, generics
- **Pattern matching** (`match`) with guards
- **Async/await & spawn** (parsed; synchronous runtime for now)
- **Module system** with relative path imports
- **FFI** — import C/C++/Rust symbols via `use from "lang:path"`
- **Built-in collections** — lists, maps, sets (interpreter support in progress)
- **Rich type system** — `int`, `rint`, `real`, `bool`, `str`, `symbol`, `null`, `None`, vectors, matrices

---

## Quick Start

### Prerequisites

- [Rust](https://www.rust-lang.org/) 1.70+ (install via `rustup`)

### Install

```sh
git clone https://github.com/espik-dev/yidika.git
cd yidika
cargo build --release
```

### Usage

```sh
# Run a .yk file (interpreter — quiet, just output)
yidi test.yk

# Run with file watching
yidi test.yk --watch

# Build for production
yidi build test.yk

# Package management
yidi add <package>
yidi install
```

### Hello World

Create `hello.yk`:
```rust
fn main() {
    print("Hello, Yidika!");
}
```

Run it:
```sh
cargo run -- hello.yk
```

### More examples

Variables and mutability:
```rust
fn main() {
    x: int = 10;      // mutable by default
    x = 6;            // OK

    name: const = "Alice";  // immutable
    // name = "Bob";        // ERROR
}
```

Functions and control flow:
```rust
fn factorial(n: int) -> int {
    if (n <= 1) { return 1; }
    return n * factorial(n - 1);
}

fn main() {
    result: int = factorial(5);
    print(result);  // 120

    for (i in 0..5) {
        print(i);
    }
}
```

Structs:
```rust
struct Person {
    name: str;
    age: int;
}

fn main() {
    p: auto = Person { name: "Alice", age: 30 };
    print(p.name);
}
```

---

## Project Structure

```
src/
├── cli/           # CLI argument parsing & command execution
├── diagnostics/   # Error types & source-span tracking
├── interpret/     # Tree-walking interpreter
├── module/        # Module loader (resolve imports, detect cycles)
├── semantic/      # Type checker & environment
│   ├── env.rs     # Symbol table (types & function signatures)
│   └── typeck.rs  # Type inference & checking
└── syntax/        # Lexer, parser, AST, tokens
    ├── ast.rs     # AST node definitions
    ├── lexer.rs   # Tokenizer
    ├── parser.rs  # Recursive-descent Pratt parser
    └── token.rs   # Token enum
```

---

## Language Specification

Detailed specs are in [`docs/dev/`](docs/dev/):
- [MVP Specification](docs/dev/SPEC_MVP.md) — complete language reference
- [Architecture](docs/dev/ARCHITECTURE.md) — compiler pipeline & MLIR/LLVM strategy
- [Type System](docs/dev/TYPE_SYSTEM.md)
- [Memory Model](docs/dev/MEMORY_MODEL.md)
- [Module System](docs/dev/MODULE_SYSTEM.md)
- [FFI / ABI](docs/dev/FFI_ABI.md)
- [Execution Model](docs/dev/EXECUTION_MODEL.md)
- [Syntax Tree](docs/dev/SYNTAX_TREE.md)

---

## Testing

```sh
cargo test                    # Run all tests
cargo test --lib              # Unit tests only
cargo check --tests           # Verify tests compile (no linker needed)
cargo clippy                  # Lint checks
cargo run -- test.yk          # Run a .yk file
cargo run -- build test.yk    # Build a .yk file
cargo run -- test.yk --watch  # Watch mode
```

---

## Roadmap

- [x] Lexer & Pratt parser
- [x] AST with span tracking
- [x] Type checker (env + type inference)
- [x] Module loader (import resolution, cycle detection)
- [x] Tree-walking interpreter
- [ ] Code generation (MLIR dialect → LLVM IR)
- [ ] Full async runtime
- [ ] GPU/NPU backend
- [ ] Package manager
- [ ] Language server (LSP)

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

---

## License

MIT — see [LICENSE](LICENSE).

Copyright (c) 2022 Espoir LOEMBA


yidika dans a été designer et crée dans ses première version en 2022 par Espoir LOEMBA

dont la première version était écrit en Clang comme MVP
et écrit totalement en Rust en 2026