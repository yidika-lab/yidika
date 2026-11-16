# Parallel work

Yidika runs `spawn` on real operating system threads, so two calls can use two processor cores at the same time. This guide splits a computation in two, runs both halves at once and joins the results.

## One thread

Counting the primes up to 100 000 takes one loop:

```yidika
fn is_prime(n: int) -> bool {
    if (n < 2) { return false; }
    d = 2;
    while (d * d <= n) {
        if (n % d == 0) { return false; }
        d += 1;
    }
    return true;
}

fn count_primes(lo: int, hi: int) -> int {
    found = 0;
    for (n in lo..hi) {
        if (is_prime(n)) { found += 1; }
    }
    return found;
}

fn main() {
    println(count_primes(1, 100000), "primes up to 100000");
}
```

```output
9592 primes up to 100000
```

## Two threads

`spawn f(args)` starts the call on another thread and gives back a task at once. `await task` waits for that thread and gives its return value.

```yidika
fn is_prime(n: int) -> bool {
    if (n < 2) { return false; }
    d = 2;
    while (d * d <= n) {
        if (n % d == 0) { return false; }
        d += 1;
    }
    return true;
}

fn count_primes(lo: int, hi: int) -> int {
    found = 0;
    for (n in lo..hi) {
        if (is_prime(n)) { found += 1; }
    }
    return found;
}

fn main() {
    low = spawn count_primes(1, 50000);
    high = spawn count_primes(50001, 100000);
    a = await low;
    b = await high;
    println(a, "+", b, "=", a + b);
}
```

```output
5133 + 4459 = 9592
```

Both halves run while `main` waits. The ranges do not overlap and each thread only reads its own arguments, so the threads never need to share anything.

> [!NOTE]
> Write each `await` on its own line: `await a + await b` currently parses as `await (a + await b)`. In native code a spawned function must return an `int` for now ([Known limits](limits.md)).

## Try it

Split the same count into four tasks of 25 000 numbers each and print the total.

<details>
<summary>Show a solution</summary>

```yidika-file
fn main() {
    t1 = spawn count_primes(1, 25000);
    t2 = spawn count_primes(25001, 50000);
    t3 = spawn count_primes(50001, 75000);
    t4 = spawn count_primes(75001, 100000);
    a = await t1;
    b = await t2;
    c = await t3;
    d = await t4;
    println(a + b + c + d);
}
```

Add `is_prime` and `count_primes` from above to the file.

</details>

## Next

- [Concurrency](concurrency.md) in the language tour;
- [Interpreter and native code](engines.md) to see why the native build of this program is much faster.
