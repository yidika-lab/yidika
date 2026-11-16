# Control flow

## if

The condition goes in parentheses and the body in braces.

```yidika
fn size(x: int) -> str {
    if (x > 100) {
        return "large";
    } else if (x > 10) {
        return "medium";
    } else {
        return "small";
    }
}

fn main() {
    println(size(500), size(50), size(5));
}
```

```output
large medium small
```

`if` is also an expression when each branch holds one value:

```yidika
fn main() {
    x = 5;
    label = if (x > 3) { "big" } else { "small" };
    println(label);
}
```

```output
big
```

## while

```yidika
fn main() {
    n = 1;
    while (n < 100) {
        n = n * 3;
    }
    println(n);
}
```

```output
243
```

## for

`a..b` counts from `a` to `b`, both included, and counts down when `a > b`. `for (i in n)` counts from `0` to `n - 1`.

```yidika
fn main() {
    total = 0;
    for (i in 1..4) { total += i; }
    println(total);

    down = "";
    for (i in 3..1) { down = down + str(i); }
    println(down);

    count = 0;
    for (i in 5) { count += 1; }
    println(count);
}
```

```output
10
321
5
```

To walk the items of a list, write `for (x: list)`:

```yidika
fn main() {
    sum = 0;
    for (x: [10, 20, 30]) { sum += x; }
    println(sum);
}
```

```output
60
```

## loop

`loop` repeats its body until a `return` leaves the function.

```yidika
fn first_square_over(limit: int) -> int {
    n = 1;
    loop {
        if (n * n > limit) { return n * n; }
        n++;
    }
}

fn main() {
    println(first_square_over(50));
}
```

```output
64
```

There is no `break` or `continue` yet; put the loop in a function and `return`, or use a `while` condition. See [Known limits](limits.md).
