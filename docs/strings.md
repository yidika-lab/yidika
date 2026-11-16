# Strings

A `str` is UTF-8 text. `+` joins two strings; `str()` turns any value into one.

```yidika
fn main() {
    name = "Ada";
    n = 3;
    print("hi " + name + ", you have " + str(n) + " messages");
    print(str(2.5) + " " + str(true));
}
```

```output
hi Ada, you have 3 messages
2.5 true
```

## Interpolation

An `f"..."` string evaluates the expressions between braces.

```yidika
fn main() {
    name = "Ada";
    print(f"{name} has {len(name)} letters, {len(name) * 2} doubled");
}
```

```output
Ada has 3 letters, 6 doubled
```

## Methods

```yidika
fn main() {
    s = "Hello World";
    print(s.len(), len(s), s[0]);
    print(s.toUpper(), s.toLower());
    print(s.contains("World"), s.startsWith("He"), s.endsWith("ld"));
    print(s.replace("World", "Yidika"));
    print(s.split(" "));
    print("  padded  ".trim() + "|");
    print("ab" == "ab", "ab" != "cd");
}
```

```output
11 11 H
HELLO WORLD hello world
true true true
Hello Yidika
[Hello, World]
padded|
true true
```

String methods currently build natively only in part, so a program that uses them runs in the interpreter (`yidi run` switches by itself). See [Known limits](limits.md).
