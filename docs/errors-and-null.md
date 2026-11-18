# Errors and null

## Result

A function that can fail returns `Result<T, E>`: `Ok(value)` on success, `Error(reason)` on failure.

```yidika
fn parse_port(s: str) -> Result<int, str> {
    if (s == "") { return Error("empty port"); }
    return Ok(8080);
}

fn main() {
    print(parse_port("8080"));
    print(parse_port(""));
}
```

```output
Ok(8080)
Error(empty port)
```

## The ? operator

`?` after a `Result` gives the value inside an `Ok`, or returns the `Error` from the current function at once.

```yidika
fn parse_port(s: str) -> Result<int, str> {
    if (s == "") { return Error("empty port"); }
    return Ok(8080);
}

fn next_port(s: str) -> Result<int, str> {
    p: int = parse_port(s)?;
    return Ok(p + 1);
}

fn main() {
    print(next_port("8080"));
    print(next_port(""));
}
```

```output
Ok(8081)
Error(empty port)
```

## Nullable types

`T?` is a `T` or `null`. `?:` gives a fallback for `null`, and `?.` reads a field only when the value is not `null`.

```yidika
struct User {
    name: str;
}

fn find(id: int) -> User? {
    if (id == 1) { return User { name: "Ada" }; }
    return null;
}

fn main() {
    print(find(1)?.name ?: "nobody");
    print(find(2)?.name ?: "nobody");
    n: int? = null;
    print(n ?: -1);
}
```

```output
Ada
nobody
-1
```

`len()` of a value that may be `null` does not compile: `s: str? = null; len(s)` stops with `len() of a value that may be null: give a default with ?: first`. Write `len(s ?: "")`.

## Runtime errors

Some mistakes can only be seen while the program runs. Yidika then stops the program with exit code 1 and one line on stderr, the same in `yidi run` and in a native build. It never continues with a wrong or random value.

| Cause | Message |
|---|---|
| `a / 0`, `a % 0` | `division by zero` |
| `i64::MIN / -1`, `i64::MIN % -1` | `division overflow: i64::MIN / -1` |
| `2 ** n` with `n < 0` | `negative exponent in integer power` |
| `l[i]` with `i < 0` or `i >= len(l)` | `Index i out of bounds for list of length n` |
| `l.pop()` on an empty list | `pop from empty list` |
| Recursion that never ends | `stack overflow: the recursion probably never ends` |

`yidi run` reports the line too. A native program detects a stack overflow through a guard page, like Rust; a recursion that the optimizer turns into a loop, such as `return f(n + 1) + 1;`, runs forever instead, as it does in C and Rust.

