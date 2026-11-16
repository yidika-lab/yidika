# Coming from Python

Yidika keeps much of what makes Python pleasant: no variable keyword, `f"..."` strings, `match`, classes with an explicit `self`. What changes is that types are checked before the program runs, blocks use braces, and the native build runs as fast as compiled code. This page maps what you know to what to write.

## At a glance

| Python | Yidika |
|---|---|
| `count = 10` | `count = 10;` |
| `name: str = "Ada"` | `name: str = "Ada";` |
| `LIMIT: Final = 100` | `limit: const<int> = 100;` |
| `int`, `float`, `bool`, `str` | `int`, `real`, `bool`, `str` |
| `f"{name} is {age}"` | `f"{name} is {age}"` |
| `def add(a: int, b: int) -> int:` | `fn add(a: int, b: int) -> int {` |
| `lambda x: x + 1` | `(x) => x + 1` |
| `print(a, b)` | `println(a, b);` |
| `for x in items:` | `for (x: items) {` |
| `for i in range(5):` | `for (i in 5) {` |
| `and`, `or`, `not` | `&&`, `\|\|`, `!` |
| `None`, `Optional[str]` | `null`, `str?` |
| `raise` / `try` / `except` | `Result<T, E>` and `?` |
| `threading`, `multiprocessing` | `spawn` / `await` |
| `import json` | `use { json } from "std";` |
| `pip install pkg` | `yidi add pkg` |
| `if __name__ == "__main__":` | `fn main() { ... }` |

## Braces and semicolons

Indentation does not carry meaning. A block is in braces, a statement ends with `;`, and the condition of `if` and `while` goes in parentheses:

```python
def size(x: int) -> str:
    if x > 100:
        return "large"
    elif x > 10:
        return "medium"
    return "small"
```

```yidika
fn size(x: int) -> str {
    if (x > 100) {
        return "large";
    } else if (x > 10) {
        return "medium";
    }
    return "small";
}

fn main() {
    println(size(500), size(50), size(5));
}
```

```output
large medium small
```

## Types are checked

Python's annotations are hints; Yidika's are rules, checked for the whole program before it starts. Passing a `str` where an `int` is expected stops with the file, line and column, even in code that would only run later. Without an annotation the type comes from the value, as in Python:

```yidika
fn main() {
    count = 10;
    ratio: real = 2.5;
    name = "Ada";
    println(count * 2, ratio * 2.0, f"{name} has {len(name)} letters");
}
```

```output
20 5 Ada has 3 letters
```

## Ranges and division

Two habits to change:

- `a..b` includes both ends, where `range(a, b)` stops before `b`. `for (i in n)` counts from `0` to `n - 1`, like `range(n)`.
- `/` between two `int` values drops the remainder toward zero: `-7 / 2` is `-3`, where Python's `-7 // 2` is `-4`.

```yidika
fn main() {
    total = 0;
    for (i in 1..4) { total += i; }
    println(total, 7 / 2, -7 / 2, 7.0 / 2.0, 2 ** 10);
}
```

```output
10 3 -3 3.5 1024
```

## Classes

Fields are declared with their type. Constructor parameters can follow the class name, and `init` plays the role of the body of `__init__`:

```python
class Counter:
    def __init__(self, start: int):
        self.start = start
        print("counter created")

    def get(self) -> int:
        return self.start
```

```yidika
class Counter(start: int) {
    fn get(self) -> int { return self.start; }
    init { println("counter created"); }
}

fn main() {
    c = Counter(99);
    println(c.get());
}
```

```output
counter created
99
```

## Errors are values

Instead of raising, a function that can fail returns `Result<T, E>`. `?` gives the value inside `Ok`, or returns the `Error` to the caller at once:

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

The type tells the caller that the function can fail; there is no exception that escapes without warning.

## Threads run in parallel

There is no global interpreter lock: two `spawn` calls run on two cores at the same time. See [Parallel work](parallel-work.md).

```yidika
fn work(n: int) -> int {
    total = 0;
    for (i in 1..n) { total += i; }
    return total;
}

fn main() {
    a = spawn work(1000);
    b = spawn work(2000);
    x = await a;
    y = await b;
    println(x + y);
}
```

```output
2501500
```

## Things that will surprise you

- `print` and `println` both end the line for now.
- Assigning a `str` or a list to another variable, or passing it to a function you wrote, moves it: the old name cannot be used afterwards. See [Moves](variables-and-types.md#moves).
- `for (x in list)` gives the indices; `for (x: list)` gives the items.
- There are no list comprehensions, and lists have no `map` or `filter` yet; write the loop.
- `break` and `continue` do not exist yet: return from a function or put the test in the `while` condition ([Known limits](limits.md)).
- Writing into a list item or a field (`l[0] = 9`, `p.x = 5`) does not change the value yet; build a new one.

## Next

- [Variables and types](variables-and-types.md) starts the language tour;
- [Files and JSON](files-and-json.md) covers what you would do with `open` and `json`.
