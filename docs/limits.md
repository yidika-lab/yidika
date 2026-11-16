# Known limits

Yidika is young. This page lists what does not work yet, so you can plan around it. Each item has a tracking issue; the documentation examples avoid all of them.

## Language ([#100](https://github.com/yidika-lab/yidika/issues/100))

| Missing | Do this instead |
|---|---|
| `break`, `continue` | Put the loop in a function and `return`, or move the test into the `while` condition |
| `1 \| 2 => ...` in `match` | One arm per value |
| A guard using its binding: `x if x < 0 => ...` | An `if` before the `match` |
| `Ok(n)` / `Error(e)` as `match` patterns | `?` inside a function that returns `Result` |
| Binding an enum payload: `Circle(r) => r * r` | `Circle(_)` and keep the value elsewhere |
| `data class` | A `class` with constructor parameters |
| `10 as real` | `10.0`, or arithmetic with a real: `n * 1.0` |
| `int8`, `int64`, `real32` with literals | `int` and `real` |

## Ownership

Passing a `str` or a list to your own function, or assigning it to another variable, moves it ([Moves](variables-and-types.md#moves)). There is no `&` to lend it and no `clone()` yet: pass `s + ""` for a string, or return the value from the function.

## Do not rely on these

They run without an error but do not do what they say ([#100](https://github.com/yidika-lab/yidika/issues/100)):

- Writing into an item or a field: `l[0] = 9`, `m["k"] = 3` and `p.x = 5` leave the value unchanged. Build a new list or struct instead.
- `for (x in list)` gives the indices; use `for (x: list)` for the items.
- `await a + await b`: write `x = await a;` and `y = await b;` on separate lines.

## Library

- Strings: no `indexOf`, no `for` over the characters.
- Lists and sets: no `contains`, `map`, `filter` yet ([#55](https://github.com/yidika-lab/yidika/issues/55)).
- `print` and `println` both end the line.

## Native code ([#96](https://github.com/yidika-lab/yidika/issues/96))

These work, but `yidi run` runs them in the interpreter because the native backend cannot build them yet:

- string methods (`s.len()`, `s.toUpper()`, `s.split()`, ...);
- `for (x: list)`, maps, tuples;
- classes whose methods write `self` fields;
- `json.parse`, the `path` and `datetime` modules;
- `spawn` of a function that does not return `int`.

In native code a route handler is one `return` of a string or a `Response`, built from literals, `+`, `if` and `req.method`, `req.path`, `req.body`. Handlers that take the route parameters by name (`fn greet(name: str)` on `/hello/{name}`) and `req.body` itself only work in the interpreter for now.

## Platforms

- Importing C++ or Rust libraries (`use ... from "c++:..."`) and the `gui` module work on Windows only.
- Native compilation needs a C++ compiler on the machine; prebuilt toolchains are not bundled.
