# Coming from JavaScript and TypeScript

Yidika reads much like TypeScript with types checked for real: braces, `(x) => x + 1`, classes, `?.`. The differences are where JavaScript is loose: numbers, `null`, exceptions and `async`. This page maps what you know to what to write.

## At a glance

| JavaScript / TypeScript | Yidika |
|---|---|
| `let count = 10` | `count = 10;` |
| `const limit = 100` | `limit: const<int> = 100;` |
| `let name: string = "Ada"` | `name: str = "Ada";` |
| `number` | `int` for whole numbers, `real` for decimals |
| `` `${name} is ${age}` `` | `f"{name} is {age}"` |
| `function add(a: number, b: number): number` | `fn add(a: int, b: int) -> int` |
| `(x) => x + 1` | `(x) => x + 1` |
| `console.log(a, b)` | `print(a, b);` |
| `for (const x of items)` | `for (x: items)` |
| `switch` | `match` |
| `{ x: 1, y: 2 }` | a `struct`: `Point { x: 1, y: 2 }` |
| `string \| null`, `??` | `str?`, `?:` |
| `throw` / `try` / `catch` | `Result<T, E>` and `?` |
| `async` / `await`, `Promise.all` | `spawn` / `await` on threads |
| `import { f } from "./util"` | `use { f } from "./util";` |
| `npm install pkg` | `yidi add pkg` |

## Numbers are typed

JavaScript has one `number`. Yidika has `int` (64-bit) and `real` (64-bit floating point), and integer division drops the remainder:

```yidika
fn main() {
    print(7 / 2, 7.0 / 2.0, 7 % 2);
    print(0.1 + 0.2 == 0.3);
    print(9007199254740993 - 1);
}
```

```output
3 3.5 1
false
9007199254740992
```

The last line is exact: an `int` holds every whole number up to about 9.2 x 10^18, where a JavaScript number starts rounding after 2^53.

## Objects become structs

An object literal with a known shape is a `struct`. Its fields are checked when the program is compiled, so a typo is an error before anything runs.

```ts
interface Point { x: number; y: number }
const p: Point = { x: 3, y: 4 };
console.log(p.x + p.y);
```

```yidika
struct Point {
    x: int;
    y: int;
}

fn main() {
    p = Point { x: 3, y: 4 };
    print(p.x + p.y);
}
```

```output
7
```

Classes look familiar; methods name `self` (the `this` of JavaScript) as their first parameter:

```yidika
class Counter(start: int) {
    fn next(self) -> int { return self.start + 1; }
}

fn main() {
    c = Counter(41);
    print(c.next());
}
```

```output
42
```

## null is opt-in

A variable can be `null` only when its type says so, with `?`. `?.` and `?:` (the `??` of JavaScript) work as you expect:

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
}
```

```output
Ada
nobody
```

There is no `undefined`, and reading a field of a value that is not marked `?` can never meet `null`.

## Errors are values

Instead of `throw`, a function that can fail returns `Result<T, E>`. `?` gives the value inside `Ok`, or returns the `Error` to the caller at once, like a `throw` that the type system can see:

```ts
function parsePort(s: string): number {
  if (s === "") throw new Error("empty port");
  return 8080;
}
```

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

## async becomes threads

JavaScript runs one thread and `await` waits for I/O. In Yidika, `spawn` starts a call on another operating system thread, and `await` waits for its result. Two `spawn` calls use two cores at once, for computation as well as I/O.

```ts
const [a, b] = await Promise.all([work(1000), work(2000)]);
```

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
    print(x, y);
}
```

```output
500500 2001000
```

## Things that will surprise you

- Every statement ends with `;`, and `if` and `while` need parentheses: `if (x > 3) { ... }`.
- `for (x in list)` gives the indices; `for (x: list)` gives the items.
- `1..4` includes both ends: `for (i in 1..4)` runs four times.
- `==` compares values, strings included; there is no `===`.
- Assigning a `str` or a list to another variable, or passing it to a function you wrote, moves it: the old name cannot be used afterwards. See [Moves](variables-and-types.md#moves).
- `break` and `continue` do not exist yet: return from a function or put the test in the `while` condition ([Known limits](limits.md)).
- Writing into a list item or a field (`l[0] = 9`, `p.x = 5`) does not change the value yet; build a new one.

## Next

- [Variables and types](variables-and-types.md) starts the language tour;
- [Build a web server](web-server.md) is the Yidika version of a small Express app.
