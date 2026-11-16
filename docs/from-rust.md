# Coming from Rust

Much of Yidika will look familiar: `fn`, `->`, structs, enums, `match`, `Result` and `?`, generics, and a native build through LLVM. The difference is in how much the language asks of you. Variables are mutable by default, there are no lifetimes or borrows to write, and the same program also runs in an interpreter that starts at once.

## At a glance

| Rust | Yidika |
|---|---|
| `let mut count = 10;` | `count = 10;` |
| `let limit = 100;` | `limit: const<int> = 100;` |
| `i64`, `f64`, `bool`, `String` | `int`, `real`, `bool`, `str` |
| `format!("{name} is {age}")` | `f"{name} is {age}"` |
| `println!("{} {}", a, b)` | `println(a, b);` |
| `\|x\| x + 1` | `(x) => x + 1` |
| `for x in &items` | `for (x: items)` |
| `for i in 0..5` | `for (i in 0..4)` or `for (i in 5)` |
| `Option<T>`, `None`, `unwrap_or` | `T?`, `null`, `?:` |
| `Result<T, E>`, `Ok`, `Err` | `Result<T, E>`, `Ok`, `Error` |
| `impl Point { fn ... }` | methods inside a `class` |
| `trait` + `impl Trait for T` | `interface` + `class T implements I` |
| `Vec<T>`, `HashMap<K, V>` | `T[]` or `list[T]`, `map[K, V]` |
| `std::thread::spawn` + `join` | `spawn` + `await` |
| `cargo new`, `cargo add`, `cargo run` | `yidi init`, `yidi add`, `yidi run` |

## Ranges include their end

`a..b` in Yidika is `a..=b` in Rust: `for (i in 1..4)` runs four times. `for (i in n)` counts from `0` to `n - 1`, like `0..n`. A range with `a > b` counts down instead of being empty.

```yidika
fn main() {
    up = "";
    for (i in 1..4) { up = up + str(i); }
    down = "";
    for (i in 3..1) { down = down + str(i); }
    println(up, down);
}
```

```output
1234 321
```

## Result and ?

`Result` and `?` work as in Rust; the failure variant is `Error` instead of `Err`:

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

Instead of `Option<T>`, a type that can be absent is written `T?`. `?.` reads a field only when the value is present, and `?:` gives a fallback, like `unwrap_or`.

## Traits become interfaces

An interface lists methods, and a class says which interfaces it implements. A value of the class can be passed where the interface is expected, like a `&dyn Trait`:

```yidika
interface Drawable {
    fn draw(&self) -> str;
}

class Circle implements Drawable {
    r: int;
    fn draw(&self) -> str { return "circle of radius " + str(self.r); }
}

fn render(d: Drawable) -> str {
    return "<" + d.draw() + ">";
}

fn main() {
    println(render(Circle { r: 2 }));
}
```

```output
<circle of radius 2>
```

Methods take `self` or `&self`; both are accepted. Classes can also inherit from a class marked `open`, which Rust does not have.

## Enums and match

Enums carry values, and `match` tells the variants apart. Patterns cannot bind the carried value yet (`Circle(r) => ...`); match on `Circle(_)` for now ([Known limits](limits.md)).

```yidika
enum Shape {
    Circle(r: int);
    Square(side: int);
    Empty;
}

fn kind(s: Shape) -> str {
    return match s {
        Circle(_) => "round",
        Square(_) => "square",
        Empty => "nothing",
    };
}

fn main() {
    println(kind(Shape::Circle(2)), kind(Shape::Empty));
}
```

```output
round nothing
```

## Threads

`spawn f(args)` is `thread::spawn(move || f(args))`, and `await t` is `t.join().unwrap()`:

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
    println(x, y);
}
```

```output
500500 2001000
```

## Things that will surprise you

- Variables are mutable unless declared `const<T>` or `as const`.
- A `str` or a list moves on assignment and when passed to your own function, as a `String` or `Vec` would. There is no `&` to borrow it and no `clone()` yet: pass `s + ""` or return the value ([Moves](variables-and-types.md#moves)). Structs, numbers and booleans are copied.
- `match` arms are separated by `,` and a `match` used as a value ends with `;`.
- There is no `break` or `continue` yet, and a `match` arm cannot use `1 | 2` or a guard on its binding ([Known limits](limits.md)).
- Writing into a list item or a field (`l[0] = 9`, `p.x = 5`) does not change the value yet; build a new one.
- `yidi run` compiles to native code when it can and runs the interpreter otherwise; `yidi run app.yk --interp` forces the interpreter. See [Interpreter and native code](engines.md).

## Next

- [Structs, classes and interfaces](structs-and-classes.md) in the language tour;
- [Interpreter and native code](engines.md) for how both engines are kept in step.
