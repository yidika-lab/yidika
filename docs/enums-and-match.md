# Enums, unions and match

## match

`match` compares a value against patterns, top to bottom, and yields the value of the first arm that fits. `_` matches anything.

```yidika
fn name(n: int) -> str {
    return match n {
        0 => "zero",
        1 => "one",
        _ => "many",
    };
}

fn main() {
    println(name(0), name(1), name(7));
    lang = "yk";
    println(match lang { "rs" => "Rust", "yk" => "Yidika", _ => "?" });
}
```

```output
zero one many
Yidika
```

## Enums

An enum lists variants; a variant can carry values. Create one with `Enum::Variant` and tell the variants apart with `match`.

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
    println(kind(Shape::Circle(2)), kind(Shape::Square(3)), kind(Shape::Empty));
}
```

```output
round square nothing
```

Binding the carried value in a pattern (`Circle(r) => r * r`) does not work in the interpreter yet; see [Known limits](limits.md).

## Union types

`A | B` holds a value of either type.

```yidika
fn main() {
    x: int | str = 42;
    println(x);
    x = "forty-two";
    println(x);
}
```

```output
42
forty-two
```
