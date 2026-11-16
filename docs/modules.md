# Modules and imports

## Standard modules

`use { sqrt } from "math";` imports functions from a module; `use { base64 } from "std";` imports the module itself as a name.

```yidika
use { sqrt } from "math";
use { base64 } from "std";

fn main() {
    println(sqrt(16.0));
    println(base64.encode("hello"));
}
```

```output
4
aGVsbG8=
```

The full list is on the [standard library](standard-library.md) page.

## Your own files

Another file of your project is imported by its relative path, without the `.yk`. Only functions marked `export` can be imported.

`text.yk`:

```yidika-file
export fn shout(s: str) -> str {
    return s.toUpper() + "!";
}
```

`main.yk`:

```yidika-file
use { shout } from "./text";

fn main() {
    println(shout("hello"));
}
```

## Packages

Installed packages live in `@yidi/<name>/` and are imported by name, as with Node or Python:

```yidika-file
use { hello } from "greet";        // @yidi/greet/greet.yk, or @yidi/greet/main.yk
use { upper } from "greet/text";   // @yidi/greet/text.yk
```

The search starts in the importing file's folder and walks up to the first `@yidi/greet/`. See [Packages and the registry](packages.md).

## Other languages

`use { f } from "c++:path"` and `use { f } from "rust:path"` import functions from a C++ or Rust library through its C ABI. This currently works on Windows only.
