# Functions and closures

## Functions

Parameters are typed; the return type follows `->`. A function without `->` returns nothing.

```yidika
fn add(a: int, b: int) -> int {
    return a + b;
}

fn greet(name: str) {
    print("hello " + name);
}

fn main() {
    print(add(2, 3));
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
    print(first("left", "right"));
    print(first(1, 2));
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
    print(apply(twice, 21));
    print(apply((x) => x + 1, 41));
    print(apply(fn(y: int) -> int { return y * y; }, 7));
}
```

```output
42
42
49
```

## Lambda parameters

A lambda parameter takes a type the same way a function parameter does. Without one, its type comes from how the lambda is used.

```yidika
fn main() {
    inc = (n: int) => n + 1;
    greet = (who: str, times: int) => who + "!" + str(times);
    show = (x) => print(x);
    print(inc(41));
    print(greet("hi", 2));
    show("done");
}
```

```output
42
hi!2
done
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
    print(next());

    add5 = adder(5);
    print(add5(10));
}
```

```output
3
15
```
