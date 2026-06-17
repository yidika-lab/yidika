# Yidika — Réseau : `net` module complet (tous protocoles)

## Objectif final
Faire de `use { ... } from "net"` la bibliothèque réseau universelle du développeur Yidika : **TCP, UDP, DNS, HTTP, SMTP, FTP, WS, MQTT, TLS, QUIC** — tout en quelques lignes, style Python, en AOT natif.

## Légende
- ✅ Terminé
- 🔧 En cours
- ⏳ Planifié
- ❌ Bloqué

---

## Phase 1 — Sockets bruts + DNS
*Briques de base pour tout protocole applicatif.*

**API visée :**
```yidika
use { TcpStream, TcpListener, UdpSocket, dns, Address } from "net";

// TCP client
conn = TcpStream.connect("192.168.1.1:80");
conn.send("GET / HTTP/1.1\r\n\r\n");
data = conn.recv(4096);
conn.close();

// TCP server
sv = TcpListener.bind("0.0.0.0:8080");
loop { c = sv.accept(); c.send("HTTP/1.1 200 OK\r\n\r\nOK"); c.close(); }

// UDP
udp = UdpSocket.bind("0.0.0.0:0");
udp.send_to("hello", "192.168.1.1:9999");
(data, addr) = udp.recv_from(4096);

// DNS
ip = dns.lookup("example.com");  // → "93.184.216.34"
```

| # | Tâche | Runtime C | Interp (Rust std) | AOT |
|---|-------|-----------|-------------------|-----|
| 1.1 | **`TcpStream.connect(addr)`** | `socket`+`connect`+`send`+`recv`+`closesocket` | `std::net::TcpStream` | `yk_tcp_connect()` |
| 1.2 | **`TcpListener.bind(addr)` + `.accept()`** | `socket`+`bind`+`listen`+`accept` | `std::net::TcpListener` | `yk_tcp_listen()` + `yk_tcp_accept()` |
| 1.3 | **`UdpSocket`** (bind, send_to, recv_from) | `SOCK_DGRAM` + `sendto`/`recvfrom` | `std::net::UdpSocket` | `yk_udp_*()` |
| 1.4 | **`dns.lookup(host)` → IP string** | `getaddrinfo` | `std::net::lookup_host` | `yk_dns_lookup()` |
| 1.5 | **`Address` type** (parse IP:port) | struct `{ u8[16], u16 }` | Rust parse + fmt | LLVM struct |

**Blocage** : Rien. On a déjà `socket()`, `connect()`, `send()`, `recv()` dans le C runtime. Juste à wrapper.

---

## Phase 2 — HTTP Client + Server enrichi

**API visée :**
```yidika
use { http } from "net";

// Client
resp = http.get("https://api.example.com/users");
print(resp.status, resp.body);

resp = http.post("https://api.example.com/users", '{"name":"Alice"}',
                 {"Content-Type": "application/json"});

// Server enrichi
app = http.serve("0.0.0.0:3000");
app.get("/", fn(req) => ({ status: 200, body: "Hello" }));
app.get("/json", fn(req) => ({ status: 200, headers: {"X-Custom": "v"}, body: '{ "ok": true }' }));
```

| # | Tâche | Dépend de |
|---|-------|-----------|
| 2.1 | **`http.get(url)` / `.post()` / `.put()` / `.delete()`** | fetch existant + ergonomie |
| 2.2 | **`http.Response`** — `.status`, `.body`, `.headers` | — |
| 2.3 | **Headers personnalisés réponse Server** | — |
| 2.4 | **`http.serve(addr)`** = alias Server() | existant |
| 2.5 | **Middleware : `app.use(fn)`** | — |

---

## Phase 3 — Protocoles applicatifs courants

**SMTP, FTP, WebSocket client, MQTT.**

| # | Tâche | API |
|---|-------|-----|
| 3.1 | **`smtp`** — login, send | `smtp.connect(host, user, pass).send(from, to, subj, body)` |
| 3.2 | **`ws`** — WebSocket client | `ws.connect(url).on("message", fn).send(data)` |
| 3.3 | **`ftp`** — get, put, ls | `ftp.connect(host, user, pass).get("/file.txt")` |
| 3.4 | **`mqtt`** — publish, subscribe | `mqtt.connect(broker).publish("topic", payload)` |
| 3.5 | **`sse`** — Server-Sent Events client | `sse.connect(url).on("event", fn)` |

---

## Phase 4 — TLS / Chiffrement

| # | Tâche | API |
|---|-------|-----|
| 4.1 | **`tls.wrap(tcp, cert?, key?)** | Wrapper TLS sur TcpStream |
| 4.2 | **HTTPS serveur** | `http.serve(":443", cert: "cert.pem", key: "key.pem")` |
| 4.3 | **HTTPS client** | `http.get("https://...")` transparent |

---

## Phase 5 — Protocoles avancés

| # | Tâche | Notes |
|---|-------|-------|
| 5.1 | **`quic`** | QUIC via msquic / quinn |
| 5.2 | **`dns.Server`** | Serveur DNS autoritaire |
| 5.3 | **`dhcp`** | DHCP client |
| 5.4 | **`ping`** | ICMP echo |

---

## Architecture d'implémentation

```
use { X } from "net"
         │
         ▼
    Interpreter: call_net("X", args) → Rust std::net
    AOT:         compile_call "X"    → C runtime yk_X()

C runtime (déjà présent dans llvm.rs):
  ─ socket(), connect(), send(), recv(), closesocket()
  ─ socket(AF_INET, SOCK_DGRAM), sendto(), recvfrom()
  ─ getaddrinfo()
  ─ (Tout est déjà #ifdef _WIN32 / __linux__)
```

---

## Déjà fait (réutilisable)

| Fonction C | Usage |
|------------|-------|
| `socket()`, `bind()`, `listen()`, `accept()` | TCP raw (pour TcpListener) |
| `connect()` | TCP client (pour TcpStream) |
| `send()`, `recv()`, `closesocket()` | I/O socket |
| `yk_server_new/serve/add_route()` | HTTP server existant |
| `yk_fetch()` | HTTP client via WinHTTP |
| `yk_ws_*()` | WebSocket serveur |
| `yk_h2_*()` | HTTP/2 transparent |
| `yk_log()` | Debug logger |
