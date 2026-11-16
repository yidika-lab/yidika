<p align="center">
  <img src="docs/assets/tisserin.svg" width="260" alt="The tisserin, Yidika's mascot, placing the last strand of its nest">
</p>

<h1 align="center">Yidika</h1>

<p align="center">
  A statically typed language you can run straight away or compile to native code.<br>
  <a href="https://yidika-lab.github.io/yidika/">Documentation</a> |
  <a href="docs/quickstart.md">Quickstart</a> |
  <a href="https://yidika-lab.github.io/registry/">Packages</a> |
  <a href="CHANGELOG.md">Changelog</a>
</p>

*Yidika* means "to design, to arrange, to put back in order" in Kikongo. The logo is the *tisserin*, the weaver bird that builds its nest one strand at a time, in order.

```yidika-file
fn fib(n: int) -> int {
    if (n < 2) { return n; }
    return fib(n - 1) + fib(n - 2);
}

fn main() {
    println("fib(35) =", fib(35));
}
```

```sh
$ yidi fib.yk
fib(35) = 9227465
```

`yidi` type-checks the program, compiles it to native code through LLVM, caches the binary and runs it. On a Ryzen 5 4500U, `fib(35)` takes 0.034 s, against 0.036 s for the same function built with `rustc -O` ([reproduce](benchmarks/fib/run.sh)). When something cannot be compiled yet, the same program runs in the interpreter instead, with the same output. Both engines are checked against each other by the test suite on Linux, macOS and Windows.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/yidika-lab/yidika/main/install.sh | sh
```

On Windows, in PowerShell:

```powershell
irm https://raw.githubusercontent.com/yidika-lab/yidika/main/install.ps1 | iex
```

The script checks the SHA-256 of the release, installs `yidi` in `~/.yidi/bin` and adds it to your `PATH`; no administrator rights. Native compilation also needs a C++ compiler (`clang`, the Xcode Command Line Tools, or LLVM with the Visual Studio Build Tools). [More options](docs/installation.md).

## What is in the language

- Static types with inference: `int`, `real`, `bool`, `str`, `complex`, nullable `T?`, unions `A | B`, generics.
- Structs, classes with inheritance, interfaces, objects, enums and `match`.
- Lambdas and closures that capture their scope: `(x) => x + n`.
- `Result<T, E>` with `Ok`, `Error` and the `?` operator; `?.` and `?:` for `null`.
- Lists, sets, maps, tuples; `spawn` and `await` on real threads.
- A standard library for math, JSON, regex, base64, paths, files, time and HTTP (server, client, TCP).

```yidika-file
use { Server } from "net";

fn main() {
    app: Server = Server();
    app.get("/hello") { return "Hello from Yidika"; }
    app.serve(3000);
}
```

Yidika is young: some constructs are missing (`break`, for one) and some only run in the interpreter so far. The [known limits](docs/limits.md) page lists them all.

## Packages

```sh
yidi init my-app
yidi add greet               # from the registry
yidi add github:user/repo    # or any git repository
yidi sync                    # install exactly what yidi.lock records
yidi publish                 # tag a release and add it to the registry
```

Packages live in `@yidi/<name>/` and are imported by name: `use { hello } from "greet";`. Every download is checked against the SHA-256 recorded in `yidi.lock`, and one cache is shared by all projects, so a second install takes about a millisecond. [Packages and the registry](docs/packages.md).

## Documentation

The [documentation](https://yidika-lab.github.io/yidika/) is also in [`docs/`](docs/README.md). Every example in it is run by the test suite in both engines, so it cannot drift from what the language does.

## Building from source

```sh
git clone https://github.com/yidika-lab/yidika.git
cd yidika
cargo build --release
cargo test --lib -- --skip compat   # fast; `cargo test` also runs the native parity tests
```

[CONTRIBUTING.md](CONTRIBUTING.md) explains the rules, the first of which is that a feature is done only when it gives the same output in both engines.

## License

MIT, see [LICENSE](LICENSE). Yidika was designed by Espoir LOEMBA in 2022, first prototyped in C, and rewritten in Rust in 2026.
