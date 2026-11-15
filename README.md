# Yidika (yidi)

A **modern systems programming language** designed for **hardware-aware computing** - blending the safety and expressiveness of Rust with the ergonomics of modern scripting languages.

> **Status**: MVP / Experimental - parsing, type checking, and an interpreter are functional.

---

## Features

- **Mutable by default** - variables are mutable unless declared `const`
- **Strong, static typing** with type inference
- **Algebraic data types** - structs, unions, type aliases, generics
- **Pattern matching** (`match`) with guards
- **Async/await & spawn** (parsed; synchronous runtime for now)
- **Module system** with relative path imports
- **FFI** - import C/C++/Rust symbols via `use from "lang:path"`
- **Built-in collections** - lists, maps, sets (interpreter support in progress)
- **Rich type system** - `int`, `rint`, `real`, `bool`, `str`, `symbol`, `null`, `None`, vectors, matrices

---

## Quick Start

### Install

Prebuilt binaries are published on [GitHub Releases](https://github.com/yidika-lab/yidika/releases) for every tagged version. The install scripts pick the right one for your machine, check its SHA-256 checksum and put `yidi` in `~/.yidi/bin`. Nothing outside your home folder is changed and no admin rights are needed.

| OS | Architectures | Requirements |
|---|---|---|
| Windows | x86_64; ARM64 runs the x86_64 build through Windows 11 emulation | Windows 10 or 11, PowerShell 5.1 or later |
| macOS | Apple Silicon (arm64), Intel (x86_64) | macOS 11 or later |
| Linux | x86_64, aarch64 | glibc 2.35 or later (Ubuntu 22.04, Debian 12, Fedora 36 and newer) |

#### Linux and macOS

```sh
curl -fsSL https://raw.githubusercontent.com/yidika-lab/yidika/main/install.sh | sh
```

#### Windows (PowerShell, no administrator rights needed)

```powershell
irm https://raw.githubusercontent.com/yidika-lab/yidika/main/install.ps1 | iex
```

That is the whole install. The script then:

1. checks the download against its SHA-256 checksum;
2. runs a test program with the installed `yidi` and prints `self-test passed: Hello from Yidika!`;
3. adds `~/.yidi/bin` to your `PATH` for future terminals: in `~/.zshrc`, `~/.bashrc` (plus `~/.bash_profile` on macOS), fish's `conf.d` or `~/.profile` depending on your shell, and in the user `PATH` on Windows;
4. makes `yidi` usable right away. On Windows it works in the same PowerShell window. On Linux and macOS a script piped into `sh` cannot change the shell that started it, so the installer opens a new shell where `yidi` is ready; type `exit` to go back.

Then write your first program (on Windows, create `hello.yk` with any editor and run the second line):

```sh
printf 'fn main() {\n    print("Hello, Yidika!");\n}\n' > hello.yk
yidi hello.yk
```

#### Options, updates and removal

| Task | Linux and macOS | Windows (PowerShell) |
|---|---|---|
| Install a specific version | `curl -fsSL .../install.sh \| YIDI_VERSION=v0.1.0 sh` | `$env:YIDI_VERSION='v0.1.0'; irm .../install.ps1 \| iex` |
| Leave my shell profile alone | `curl -fsSL .../install.sh \| YIDI_NO_MODIFY_PATH=1 sh` | Not needed: only the user `PATH` is changed |
| Do not open a new shell at the end | `curl -fsSL .../install.sh \| YIDI_NO_SHELL=1 sh` | Not applicable |
| Install somewhere else | `YIDI_HOME=/opt/yidi` before `sh` | `$env:YIDI_HOME='D:\tools\yidi'` before `irm` |
| Update | Run the install command again | Run the install command again |
| Uninstall | `rm -rf ~/.yidi`, then delete the `# yidi` line and the line below it in your shell profile | `Remove-Item -Recurse $HOME\.yidi`, then remove the entry from your user `PATH` |

Prefer not to pipe a script into your shell? Download the archive for your platform from the [latest release](https://github.com/yidika-lab/yidika/releases/latest), check it against its `.sha256` file (`sha256sum -c`, `shasum -a 256 -c` or `Get-FileHash`), extract it and put `yidi` (or `yidi.exe` with its `yk_ffi_gui.dll`) in a folder on your `PATH`.

#### What works on each platform

The interpreter (`yidi run --interp`) works on every platform above. Native compilation (`yidi build`, `yidi run`) needs `clang++` on Linux and macOS (`sudo apt install clang`, `xcode-select --install`), and LLVM with the Visual Studio Build Tools on Windows.

`yidi run` compiles the program once, caches the binary in `~/.cache/yidi/run`, and runs it at native speed: `fib(35)` takes 0.03 s, the same as `rustc -O`, against about 14 s in the interpreter. A program the compiler cannot build yet (closures, GUI, FFI), or a machine without a C++ compiler, uses the interpreter instead. `yidi run --interp` forces the interpreter.

### Build from source

Requires [Rust](https://www.rust-lang.org/) stable (via `rustup`) and a C toolchain: `build-essential` on Debian/Ubuntu, Xcode Command Line Tools (`xcode-select --install`) on macOS, Visual Studio Build Tools on Windows.

```sh
cargo install --git https://github.com/yidika-lab/yidika yidi
```

Or from a clone:

```sh
git clone https://github.com/yidika-lab/yidika.git
cd yidika
cargo build --release
```

### Usage

```sh
# Run a .yk file (interpreter - quiet, just output)
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
yidi hello.yk
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
- [MVP Specification](docs/dev/SPEC_MVP.md) - complete language reference
- [Architecture](docs/dev/ARCHITECTURE.md) - compiler pipeline & MLIR/LLVM strategy
- [Type System](docs/dev/TYPE_SYSTEM.md)
- [Memory Model](docs/dev/MEMORY_MODEL.md)
- [Module System](docs/dev/MODULE_SYSTEM.md)
- [FFI / ABI](docs/dev/FFI_ABI.md)
- [Execution Model](docs/dev/EXECUTION_MODEL.md)
- [Syntax Tree](docs/dev/SYNTAX_TREE.md)

Comparisons with other languages:
- [Yidika vs Rust, TypeScript/TSX, Python and Kotlin](docs/COMPARISON.md)
- [Python syntax comparison](docs/comparison-python.md)
- [TypeScript syntax comparison](docs/comparison-ts.md)

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
- [ ] Code generation (MLIR dialect -> LLVM IR)
- [ ] Full async runtime
- [ ] GPU/NPU backend
- [ ] Package manager
- [ ] Language server (LSP)

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

---

## License

MIT - see [LICENSE](LICENSE).

Copyright (c) 2022 Espoir LOEMBA


yidika dans a été designer et crée dans ses première version en 2022 par Espoir LOEMBA

dont la première version était écrit en Clang comme MVP
et écrit totalement en Rust en 2026