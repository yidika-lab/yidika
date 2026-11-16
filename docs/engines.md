# Interpreter and native code

`yidi` has two engines for the same language.

| | Interpreter | Native code |
|---|---|---|
| How | walks the program's syntax tree | emits LLVM IR, compiled with `clang` and linked |
| Start | immediate | a compile on the first run, then a cached binary |
| Speed | `fib(35)` in about 14 s | `fib(35)` in 0.03 s, the same as `rustc -O` |
| Needs | nothing | a C++ compiler (see [Installation](installation.md)) |
| Covers | the whole language | most of it, growing (see [Known limits](limits.md)) |

## What yidi run does

1. Parse and type-check every file of the program. An error stops here, whatever the engine.
2. Look for a cached binary built from exactly these sources (`~/.cache/yidi/run` on Linux).
3. If there is none, compile one. When the native backend does not support something the program uses, it refuses with a message such as `json.parse is not supported by the native backend yet`, instead of building a binary that would print something else.
4. Run the binary, or, if step 3 failed or no C++ compiler is installed, run the program in the interpreter.

`yidi run --interp` skips steps 2 to 4. `yidi build` stops at step 3 and reports the refusal.

## The same output, checked

Both engines are separate implementations of one set of rules. What keeps them together is the test suite: each language feature has a test that runs one program in both engines and compares the output byte for byte, on Linux, macOS and Windows. Every example in this documentation is such a test too.

When the two disagree, the interpreter is the reference and the difference is a bug. [Report it](https://github.com/yidika-lab/yidika/issues/new) with the program and both outputs.
