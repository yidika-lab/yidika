# Standard library

`use { math } from "std";` imports a module as a name, used as `math.sqrt(...)`. `use { sqrt } from "math";` imports functions from it, used as `sqrt(...)`. `len`, `str`, `print`, `println` and `fetch` need no import.

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
    println(math.sqrt(2.0));
    println(math.pow(2.0, 8.0), math.abs(-3.5));
    println(math.floor(2.7), math.ceil(2.1), math.round(2.5));
    println(math.max(3.0, 7.0), math.min(3.0, 7.0));
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
    println(json.stringify(42));
    println(json.stringify("say \"hi\""));
    println(json.stringify(true));
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
    println(regex.match("\\d+", "order 123"));
    println(regex.match("\\d+", "no digits"));
    println(regex.replace("\\d", "a1b2c3", "#"));
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
    println(base64.encode("hello"));
    println(base64.decode("aGVsbG8="));
    println(path.is_absolute("/tmp"), path.is_absolute("notes.txt"));
}
```

```output
aGVsbG8=
hello
true false
```

## net

An HTTP server. A route handler is a string, a function or a block:

```yidika-file
use { Server } from "net";

fn hello(req: auto) -> str {
    return "Hello from " + req.path;
}

fn main() {
    app: Server = Server();
    app.get("/", "Static text");
    app.get("/hello", hello);
    app.get("/ping") { return "pong"; }
    app.serve(3000);
}
```

An HTTP client and a raw TCP connection:

```yidika-file
use { HTTP, TcpStream } from "net";

fn main() {
    http = HTTP();
    status = http.get("https://example.com");
    println(status, http.body);

    sock = TcpStream("127.0.0.1:7000");
    sock.send("hello");
    println(sock.recv(1024));
    sock.close();
}
```
