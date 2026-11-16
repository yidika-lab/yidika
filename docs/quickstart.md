# Quickstart

## Your first program

Create `hello.yk`:

```yidika
fn main() {
    println("Hello, Yidika!");
}
```

```output
Hello, Yidika!
```

Run it:

```sh
yidi hello.yk
```

The first run compiles the program to a native binary and caches it; the next runs start the cached binary. `yidi hello.yk --watch` runs it again every time you save.

## A bit more

A program is a set of functions; `main` is where it starts. Statements end with `;`, `if` and `while` take their condition in parentheses, and variables are created by assigning to them.

```yidika
fn fib(n: int) -> int {
    if (n < 2) { return n; }
    return fib(n - 1) + fib(n - 2);
}

fn main() {
    line = "";
    for (i in 0..9) {
        line = line + str(fib(i)) + " ";
    }
    println(line + "...");
    println("fib(20) =", fib(20));
}
```

```output
0 1 1 2 3 5 8 13 21 34 ...
fib(20) = 6765
```

`println` prints its arguments separated by spaces and ends the line; `str()` turns any value into a string. `0..9` includes both ends.

## A project

```sh
yidi init hello-app
cd hello-app
yidi run main.yk
```

`yidi init` writes a `project.toml` (name, version, dependencies), a `main.yk` and a `.gitignore`. Add packages with `yidi add`; see [Packages and the registry](packages.md).

## Next

- [Variables and types](variables-and-types.md) to learn the language;
- [Command line](cli.md) for everything `yidi` can do;
- [Known limits](limits.md) before you build something large.
