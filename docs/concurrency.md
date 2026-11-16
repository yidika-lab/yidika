# Concurrency

`spawn` starts a function call on another thread and gives back a task; `await` waits for it and returns its value.

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
    println(x, y, x + y);
}
```

```output
500500 2001000 2501500
```

Both calls run at the same time. In native code a spawned function must return an `int` for now; other return types make `yidi run` use the interpreter. Write `x = await a;` on its own line: `await a + await b` currently parses as `await (a + await b)`.
