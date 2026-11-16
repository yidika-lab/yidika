# Collections

## Lists

`[a, b, c]` makes a list; `list[T]` or `T[]` is its type.

```yidika
fn main() {
    l = [3, 1, 2];
    l.push(4);
    println(l, l.len(), len(l), l[0]);
    l.sort();
    println(l);
    l.reverse();
    println(l);
    l.insert(0, 9);
    l.remove(1);
    println(l);
    last = l.pop();
    println(last, l);
}
```

```output
[3, 1, 2, 4] 4 4 3
[1, 2, 3, 4]
[4, 3, 2, 1]
[9, 3, 2, 1]
1 [9, 3, 2]
```

Lists hold any type, and keep it:

```yidika
fn first(xs: list[str]) -> str {
    return xs[0];
}

fn main() {
    names = ["Ada", "Grace"];
    println(names, first(names));
    rs = [1.5, 2.5];
    println(rs[1] * 2.0);
}
```

```output
[Ada, Grace] Ada
5
```

Reading past the end stops the program with `Index 5 out of bounds for list of length 2`.

## Sets

A set keeps one copy of each value.

```yidika
fn main() {
    s: set[int] = {3, 1, 3, 2};
    println(s, s.len());
}
```

```output
set{3, 1, 2} 3
```

## Maps

```yidika
fn main() {
    ages = {"Ada": 36, "Grace": 85};
    println(ages["Ada"], ages.len());
    println(ages.keys());
}
```

```output
36 2
[Ada, Grace]
```

## Tuples

```yidika
fn main() {
    t = (42, "answer");
    println(t.0, t.1);
}
```

```output
42 answer
```

Writing into an existing item (`l[0] = 9`, `m["k"] = v`) is not supported yet; see [Known limits](limits.md).
