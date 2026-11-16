# Standard library

`use { math } from "std";` imports a module as a name, used as `math.sqrt(...)`. `use { sqrt } from "math";` imports functions from it, used as `sqrt(...)`. `len`, `str`, `print` and `fetch` need no import.

| Module | Functions |
|---|---|
| `math` | `sqrt`, `pow`, `abs`, `floor`, `ceil`, `round`, `min`, `max`, `sin`, `cos`, `rand` |
| `json` | `stringify`, `parse` |
| `regex` | `match`, `find`, `replace`, `split` |
| `base64` | `encode`, `decode` |
| `path` | `join`, `dirname`, `basename`, `extension`, `is_absolute` |
| `fs` | `read`, `write`, `append`, `remove`, `exists`, `list`, `is_dir`, `is_file` |
| `sys` | `env`, `args`, `exit`, `cwd`, `pid`, `platform`, `sleep` |
| `time`, `datetime` | `now`, `utc`, `timestamp`, `format`, `parse`, `year`, `month`, `day`, `hour`, `minute`, `second` (`time` adds `sleep`) |
| `net` | `Server`, `HTTP`, `TcpStream` |
| `gui` | `window`, `div`, `text`, `button`, `style`, `on`, `run`, ... (Windows only for now) |

Some functions exist only in the interpreter so far; a program that uses one runs in the interpreter (see [Interpreter and native code](engines.md)).

## math

```yidika
use { math } from "std";

fn main() {
    print(math.sqrt(2.0));
    print(math.pow(2.0, 8.0), math.abs(-3.5));
    print(math.floor(2.7), math.ceil(2.1), math.round(2.5));
    print(math.max(3.0, 7.0), math.min(3.0, 7.0));
}
```

```output
1.4142135623730951
256 3.5
2 3 3
7 3
```

## json

```yidika
use { json } from "std";

fn main() {
    print(json.stringify(42));
    print(json.stringify("say \"hi\""));
    print(json.stringify(true));
}
```

```output
42
"say \"hi\""
true
```

## regex

```yidika
use { regex } from "std";

fn main() {
    print(regex.match("\\d+", "order 123"));
    print(regex.match("\\d+", "no digits"));
    print(regex.replace("\\d", "a1b2c3", "#"));
}
```

```output
true
false
a#b#c#
```

## base64 and path

```yidika
use { base64, path } from "std";

fn main() {
    print(base64.encode("hello"));
    print(base64.decode("aGVsbG8="));
    print(path.basename("docs/notes.txt"), path.extension("notes.txt"));
    print(path.is_absolute("notes.txt"));
}
```

```output
aGVsbG8=
hello
notes.txt txt
false
```

`path.is_absolute` follows the platform: `/tmp` is absolute on Linux and macOS, not on Windows, where an absolute path starts with a drive such as `C:\`.

## net

An HTTP server. A route handler is a string, a function or a block whose value is the response:

```yidika-file
use { Server } from "net";

fn hello(req: auto) -> str {
    return "Hello from " + req.path;
}

fn main() {
    app: Server = Server();
    app.get("/", "Static text");
    app.get("/hello", hello);
    app.get("/ping") { "pong" }
    app.serve(3000);
}
```

An HTTP client and a raw TCP connection:

```yidika-file
use { HTTP, TcpStream } from "net";

fn main() {
    http = HTTP();
    status = http.get("https://example.com");
    print(status, http.body);

    sock = TcpStream("127.0.0.1:7000");
    sock.send("hello");
    print(sock.recv(1024));
    sock.close();
}
```
