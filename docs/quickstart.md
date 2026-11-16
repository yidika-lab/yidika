# Quickstart

## Your first program

Create `hello.yk`:

```yidika
print("Hello, Yidika!");
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

A program runs from top to bottom, as a script does. Statements end with `;`, `if` and `while` take their condition in parentheses, and variables are created by assigning to them. Functions can be declared anywhere in the file, before or after the code that calls them.

```yidika
line = "";
for (i in 0..9) {
    line = line + str(fib(i)) + " ";
}
print(line + "...");
print("fib(20) =", fib(20));

fn fib(n: int) -> int {
    if (n < 2) { return n; }
    return fib(n - 1) + fib(n - 2);
}
```

```output
0 1 1 2 3 5 8 13 21 34 ...
fib(20) = 6765
```

`print` prints its arguments separated by spaces and ends the line; `str()` turns any value into a string. `0..9` includes both ends.

## Variables outside functions

A variable created outside any function belongs to the whole file: every function can read it and assign to it. A `fn main()`, if you write one, runs after the code outside functions.

```yidika
visits = 0;

fn visit(page: str) {
    visits = visits + 1;
    print(f"{visits}: {page}");
}

fn main() {
    visit("home");
    visit("docs");
    print("total", visits);
}
```

```output
1: home
2: docs
total 2
```

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
