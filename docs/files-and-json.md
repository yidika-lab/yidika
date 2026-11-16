# Files and JSON

This guide reads and writes files, builds paths that work on every system, and reads a JSON settings file into a struct. Every example here is run by the test suite, and the files they create are removed at the end.

## Write, append, read

`fs.write` creates or replaces a file, `fs.append` adds to its end and `fs.read` gives the whole content as a `str`.

```yidika
use { fs, sys } from "std";

fn main() {
    file = "todo-" + str(sys.pid()) + ".txt";
    fs.write(file, "write the docs\n");
    fs.append(file, "ship 0.3\n");
    lines = fs.read(file).trim().split("\n");
    println(len(lines), "tasks");
    for (line: lines) {
        println("-", line);
    }
    fs.remove(file);
    println(fs.exists(file));
}
```

```output
2 tasks
- write the docs
- ship 0.3
false
```

`sys.pid()` puts the process id in the name, so two runs at the same time never share a file. `fs.exists`, `fs.is_file` and `fs.is_dir` check what is there before you read it, and `fs.list(dir)` gives the names in a folder.

## Paths

`path.join` uses the separator of the system the program runs on. The other functions take a path apart:

```yidika
use { path } from "std";

fn main() {
    report = path.join("reports", "2026-11.csv");
    println(path.basename(report));
    println(path.extension(report));
    println(path.basename(path.dirname(report)));
}
```

```output
2026-11.csv
csv
reports
```

## Read a JSON file

`json.parse` turns JSON text into a value whose fields you read with a dot. Copy the fields you need into a struct, so the rest of the program works with checked types.

```yidika
use { fs, json, sys } from "std";

struct Settings {
    name: str;
    port: int;
}

fn parse_settings(text: str) -> Settings {
    data = json.parse(text);
    return Settings { name: data.name, port: data.port };
}

fn main() {
    file = "settings-" + str(sys.pid()) + ".json";
    fs.write(file, "{\"name\": \"shop\", \"port\": 8080, \"debug\": true}");
    s = parse_settings(fs.read(file));
    println(s.name, "listens on", s.port);
    println(json.stringify(s.name), json.stringify(s.port + 1));
    fs.remove(file);
}
```

```output
shop listens on 8080
"shop" 8081
```

`parse_settings` takes the text rather than the file name: passing `file` to a function of your own would move it, and `fs.remove(file)` could not use it afterwards (see [Moves](variables-and-types.md#moves)). `json.stringify` writes a value as JSON: strings get quotes and escapes, numbers and booleans stay as they are.

> [!NOTE]
> `json.parse`, `path` and string methods such as `split` and `trim` build natively only in part, so `yidi run` runs these programs in the interpreter for now ([Known limits](limits.md)). The output is the same.

## Try it

Write a program that counts the lines of its own source file. `sys.args()` is not reliable yet, so put the file name in the program.

<details>
<summary>Show a solution</summary>

```yidika-file
use { fs } from "std";

fn main() {
    lines = fs.read("count.yk").split("\n");
    println(len(lines), "lines");
}
```

</details>

## Next

- [Build a web server](web-server.md) that serves this data;
- [Standard library](standard-library.md) for every function of `fs`, `path` and `json`.
