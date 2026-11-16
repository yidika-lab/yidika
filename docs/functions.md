# Functions and closures

## Functions

Parameters are typed; the return type follows `->`. A function without `->` returns nothing.

```yidika
fn add(a: int, b: int) -> int {
    return a + b;
}

fn greet(name: str) {
    println("hello " + name);
}

fn main() {
    println(add(2, 3));
    greet("Ada");
}
```

```output
5
hello Ada
```

## Generic functions

```yidika
fn first<T>(a: T, b: T) -> T {
    return a;
}

fn main() {
    println(first("left", "right"));
    println(first(1, 2));
}
```

```output
left
1
```

## Functions are values

A function type is written `fn(int) -> int`. Named functions and lambdas can be stored, passed and returned.

```yidika
fn twice(x: int) -> int { return x * 2; }

fn apply(f: fn(int) -> int, v: int) -> int {
    return f(v);
}

fn main() {
    println(apply(twice, 21));
    println(apply((x) => x + 1, 41));
    println(apply(fn(y: int) -> int { return y * y; }, 7));
}
```

```output
42
42
49
```

## Closures

A lambda keeps access to the variables of the scope where it was written, even after that function has returned. Changes are shared.

```yidika
fn counter() -> fn() -> int {
    c: int = 0;
    return () => {
        c = c + 1;
        return c;
    };
}

fn adder(n: int) -> fn(int) -> int {
    return (x) => x + n;
}

fn main() {
    next = counter();
    next();
    next();
    println(next());

    add5 = adder(5);
    println(add5(10));
}
```

```output
3
15
```
