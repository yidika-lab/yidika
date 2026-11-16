# Coming from Go

Go and Yidika share goals: a static type system that stays out of the way, one tool for building and packages, and native binaries. Yidika adds classes with inheritance, enums with `match`, and `Result` with `?` in place of `if err != nil`. This page maps what you know to what to write.

## At a glance

| Go | Yidika |
|---|---|
| `count := 10` | `count = 10;` |
| `var name string = "Ada"` | `name: str = "Ada";` |
| `const limit = 100` | `limit: const<int> = 100;` |
| `int`, `float64`, `bool`, `string` | `int`, `real`, `bool`, `str` |
| `fmt.Sprintf("%s is %d", name, age)` | `f"{name} is {age}"` |
| `fmt.Println(a, b)` | `println(a, b);` |
| `func add(a, b int) int` | `fn add(a: int, b: int) -> int` |
| `if x > 3 {` | `if (x > 3) {` |
| `for i := 0; i < n; i++` | `for (i in n)` |
| `for _, x := range items` | `for (x: items)` |
| `switch` | `match` |
| `[]int`, `map[string]int` | `int[]`, `map[str, int]` |
| `(value, error)` and `if err != nil` | `Result<T, E>` and `?` |
| `nil` | `null`, on `T?` types only |
| `go f()` with a channel or `WaitGroup` | `spawn f()` and `await` |
| `type Shape interface` (implicit) | `interface Shape` and `implements` |
| `go mod init`, `go get` | `yidi init`, `yidi add` |

## Loops

There is no three-part `for`. `for (i in n)` counts from `0` to `n - 1`, `a..b` counts from `a` to `b` with both ends included, and `for (x: items)` walks a list. `while` is a separate keyword:

```yidika
fn main() {
    total = 0;
    for (i in 5) { total += i; }
    sum = 0;
    for (x: [10, 20, 30]) { sum += x; }
    n = 1;
    while (n < 100) { n = n * 3; }
    println(total, sum, n);
}
```

```output
10 60 243
```

## Errors without if err != nil

A function that can fail returns `Result<T, E>`. `?` passes the error to the caller, which is the whole of `if err != nil { return err }`:

```go
func nextPort(s string) (int, error) {
    p, err := parsePort(s)
    if err != nil {
        return 0, err
    }
    return p + 1, nil
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
    println(next_port("8080"));
    println(next_port(""));
}
```

```output
Ok(8081)
Error(empty port)
```

## Interfaces are declared

A Go type satisfies an interface by having the methods. A Yidika class names the interfaces it implements, and a missing method stops the program before it runs: `Class 'Circle' does not implement method 'draw' required by interface 'Drawable'`.

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

## Goroutines and await

`spawn` starts a call on an operating system thread and gives back a task; `await` waits for it and gives its return value, so there is no channel to create for a single result:

```go
results := make(chan int, 2)
go func() { results <- work(1000) }()
go func() { results <- work(2000) }()
x, y := <-results, <-results
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
    println(x + y);
}
```

```output
2501500
```

## Things that will surprise you

- Every statement ends with `;`.
- Assigning a `str` or a list to another variable, or passing it to a function you wrote, moves it: the old name cannot be used afterwards. See [Moves](variables-and-types.md#moves).
- Names are not exported by their first letter: a function is visible to other files when it is marked `export`.
- `nil` is not the zero value of every type: only a `T?` can be `null`.
- There is no `break` or `continue` yet; return from a function or put the test in the `while` condition ([Known limits](limits.md)).
- Writing into a list item or a field (`l[0] = 9`, `p.x = 5`) does not change the value yet; build a new one.

## Next

- [Control flow](control-flow.md) in the language tour;
- [Build a web server](web-server.md), the Yidika version of a small `net/http` service.
