# Errors and null

## Result

A function that can fail returns `Result<T, E>`: `Ok(value)` on success, `Error(reason)` on failure.

```yidika
fn parse_port(s: str) -> Result<int, str> {
    if (s == "") { return Error("empty port"); }
    return Ok(8080);
}

fn main() {
    println(parse_port("8080"));
    println(parse_port(""));
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
    println(next_port("8080"));
    println(next_port(""));
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
    println(find(1)?.name ?: "nobody");
    println(find(2)?.name ?: "nobody");
    n: int? = null;
    println(n ?: -1);
}
```

```output
Ada
nobody
-1
```
