# Command line

`yidi` with no argument prints this list.

## Running and building

| Command | What it does |
|---|---|
| `yidi app.yk` | Same as `yidi run app.yk` |
| `yidi run app.yk` | Compile to native code, cache the binary, run it. Falls back to the interpreter when the program cannot be compiled yet or no C++ compiler is installed |
| `yidi run app.yk --interp` | Run in the interpreter only |
| `yidi run app.yk --watch` | Run again every time `app.yk` or a file it imports is saved |
| `yidi build app.yk` | Write a native executable next to the source (`app`, or `app.exe` on Windows) |
| `yidi build app.yk --info` | Same, and print the steps and the detected hardware |
| `yidi info` | Print the detected CPU, memory and accelerators |

Before running, `yidi` parses and type-checks the whole program; an error stops it with the file, the line and the column:

```text
error[E0201]: Variable 'count' not found

  in: app.yk
  at: line 3, column 13
```

## Projects and packages

| Command | What it does |
|---|---|
| `yidi init [name]` | Create a project; asks for the name when none is given (`.` is the current folder) |
| `yidi add <pkg>` | Add a dependency from the registry; `pkg@^1.2` picks a range |
| `yidi add github:user/repo` | Add a dependency straight from a git repository (also `https://...`) |
| `yidi remove <pkg>` | Remove it, with what only it needed |
| `yidi sync` | Install exactly what `yidi.lock` records |
| `yidi update [pkg]` | Move to the newest versions the ranges allow |
| `yidi publish [--dry-run]` | Tag the package and add the release to the registry |
| `yidi registry check <entry.toml>` | Verify a registry entry (used by the registry's CI) |

Details on [Packages and the registry](packages.md).

## yidi itself

| Command | What it does |
|---|---|
| `yidi --version` | Print the version |
| `yidi upgrade` | Install the latest release after checking its SHA-256 |
| `yidi upgrade --check` | Only say whether a newer release exists |

## Environment

| Variable | Effect |
|---|---|
| `YIDI_REGISTRY` | Another registry: a URL or a local folder |
| `YIDI_HOME` | Where the install script puts `yidi` (default `~/.yidi`) |
