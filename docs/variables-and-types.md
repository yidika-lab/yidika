# Variables and types

## Variables

Assigning to a name creates a variable. Its type comes from the value, or from an annotation after `:`. Variables are mutable by default.

```yidika
fn main() {
    count = 10;
    count = count + 5;
    ratio: real = 2.5;
    name: str = "Ada";
    ready: bool = true;
    later: auto = count * 2;
    print(count, ratio, name, ready, later);
}
```

```output
15 2.5 Ada true 30
```

## Constants

`const<T>` or `as const` makes a variable immutable. Assigning to it again is a type error, reported before the program runs.

```yidika
fn main() {
    limit: const<int> = 100;
    answer: int = 42 as const;
    print(limit, answer);
}
```

```output
100 42
```

## Moves

A `str` or a list has one owner. Assigning it to another variable, or passing it to a function you wrote, moves it there, and the old name can no longer be used. `int`, `real`, `bool` and structs are copied instead, and the standard library (`print`, `fs.write`, ...) never takes ownership.

```yidika
fn shout(s: str) -> str {
    return s + "!";
}

fn main() {
    name = "Ada";
    loud = shout(name + "");
    print(name, loud);
    count = 3;
    other = count;
    print(count, other);
}
```

```output
Ada Ada!
3 3
```

`shout(name)` followed by `print(name)` stops before the program runs with `Cannot use variable 'name' after it has been moved`. Pass a new value instead, as `name + ""` does here, or let the function return what you still need. There is no `&` to lend a value and no `clone()` yet ([Known limits](limits.md)).

## Basic types

| Type | Values | Example |
|---|---|---|
| `int` | 64-bit signed integers | `42`, `-7`, `0xff`, `1_000_000` |
| `real` | 64-bit floating point | `2.5`, `1e-3` |
| `bool` | `true`, `false` | |
| `str` | UTF-8 text | `"Ada"` |
| `complex` | complex numbers | `2.0 + 3.0i` |
| `T?` | a `T` or `null` | see [Errors and null](errors-and-null.md) |
| `A \| B` | a value of one of the listed types | see [Enums, unions and match](enums-and-match.md) |

## Arithmetic

Integer division truncates toward zero; `**` is the power operator and is right-associative.

```yidika
fn main() {
    print(7 / 2, 7 % 2, -7 / 2);
    print(7.0 / 2.0, 2 ** 10, 2 ** 3 ** 2);
    print(1 + 2 * 3, (1 + 2) * 3);
    x = 5;
    x += 3;
    x *= 2;
    x++;
    print(x);
}
```

```output
3 1 -3
3.5 1024 512
7 9
17
```

Mixing `int` and `real` gives a `real`:

```yidika
fn main() {
    print(1 + 0.5, 3 * 1.5);
}
```

```output
1.5 4.5
```

## Comparison, logic and bits

```yidika
fn main() {
    x = 7;
    print(x > 3 && x < 10, !(x == 7), x != 7 || false);
    print(6 & 3, 6 | 3, 6 ^ 3, 1 << 4, 256 >> 2, ~0);
}
```

```output
true false false
2 7 5 16 64 -1
```

## Complex numbers

```yidika
fn main() {
    z = 2.0 + 3.0i;
    w = 1.0 - 1.0i;
    print(z + w);
    print(z * w);
}
```

```output
3 + 2i
5 + 1i
```
