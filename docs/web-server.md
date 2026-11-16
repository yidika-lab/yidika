# Build a web server

In this guide you write a small HTTP service: a home page, a route that reads the path, a JSON health check and an endpoint that answers `201 Created`. The same file runs in the interpreter and builds into a native binary.

You need `yidi` installed (see [Installation](installation.md)) and `curl` to try the routes.

## The smallest server

Create `server.yk`:

```yidika-file
use { Server } from "net";

fn main() {
    app = Server();
    app.get("/", "Hello from Yidika");
    app.serve(3000);
}
```

Start it, then call it from a second terminal:

```sh
yidi server.yk
```

```text
$ curl localhost:3000
Hello from Yidika
```

`Server()` creates the server, `app.get(path, handler)` adds a route for `GET` requests, and `app.serve(3000)` listens on every interface on port 3000. `app.serve("127.0.0.1:3000")` listens on one address only. `serve` does not return: stop the server with Ctrl+C.

## Three kinds of handlers

A route answers with a string, a function or a block:

```yidika-file
use { Server } from "net";

fn echo(req: auto) -> str {
    return "You asked for " + req.path;
}

fn main() {
    app = Server();
    app.get("/", "Hello from Yidika");
    app.get("/echo/{word}", echo);
    app.get("/health") { "{\"ok\": true}" }
    app.serve(3000);
}
```

```text
$ curl localhost:3000/echo/hi
You asked for /echo/hi
$ curl localhost:3000/health
{"ok": true}
$ curl -i localhost:3000/missing
HTTP/1.1 404 Not Found
...
Not Found
```

- A **string** is sent as it is, for pages that never change.
- A **function** gets the request as `req` and returns the body. `req.method`, `req.path` and `req.body` are the parts you can read.
- A **block** after the call, `app.get("/health") { ... }`, is a handler without parameters. Its value is the response, so it holds an expression, not a `return`.

`{word}` in a path matches one segment, so `/echo/hi` and `/echo/there` both reach `echo`. A route that matches nothing gets `404 Not Found`.

## Status codes

Return a `Response` to choose the status. `app.post`, `app.put`, `app.patch` and `app.delete` add routes for the other methods.

```yidika-file
use { Server } from "net";

fn show(req: auto) -> str {
    return "note " + req.path;
}

fn create(req: auto) -> Response {
    return Response { status: 201, body: "created " + req.path };
}

fn main() {
    app = Server();
    app.get("/notes/{id}", show);
    app.post("/notes/{id}", create);
    app.serve(3000);
}
```

```text
$ curl localhost:3000/notes/7
note /notes/7
$ curl -i -X POST localhost:3000/notes/7
HTTP/1.1 201 Created
...
created /notes/7
```

One path can have a handler per method; a method with no handler on that path gets `404`.

## Ship it

`yidi build` writes a native executable next to the source. It needs no runtime and starts in milliseconds:

```sh
yidi build server.yk
./server
```

> [!NOTE]
> In native code a handler is one `return` of a string or a `Response`, made of literals, `+`, `if`, `req.method`, `req.path` and `req.body`. Handlers that take route parameters by name (`fn greet(name: str)` on `/hello/{name}`) and reading `req.body` work in the interpreter only for now. See [Known limits](limits.md).

## Try it

Add a `GET /version` route that answers `0.2.0`, and a `DELETE /notes/{id}` route that answers `204` with no body.

<details>
<summary>Show a solution</summary>

```yidika-file
use { Server } from "net";

fn remove(req: auto) -> Response {
    return Response { status: 204 };
}

fn main() {
    app = Server();
    app.get("/version", "0.2.0");
    app.delete("/notes/{id}", remove);
    app.serve(3000);
}
```

</details>

## Next

- [Files and JSON](files-and-json.md) to store what the server receives;
- [Standard library](standard-library.md#net) for the HTTP client and raw TCP sockets.
