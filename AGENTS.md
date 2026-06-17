## Goal
Build a universal `net` module for Yidika exposing every network protocol (TCP, UDP, HTTP, SMTP, FTP, WS, MQTT, TLS, QUIC) with Python-like ergonomics, while keeping per-connection memory < 150 B for 15M+ clients.

## Constraints & Preferences
- 238 tests passent après chaque modification (hors compat tests qui requièrent clang+MSVC)
- Approche pragmatique, pas de refactor total
- Zéro warning Rust

## Progress
### Done
- Phases 1–12 roadmap initiale (inférence type, enum, closures, spawn/await, FFI C++)
- Phases 2–7 compatibilité — Modules `math`, `time`, `sys`, `path`, `fs`/`io`, `base64`, `json.stringify`, `re.match`/`re.replace` en AOT via runtime C/C++
- Phase 4 architecturale — Héritage/Objets/Interfaces/Classes génériques en AOT
- **`fetch()` globale (client HTTP)** : interpréteur via `ureq`, AOT via WinHTTP C API
- **MapLit : identifiants comme clés strings** : `{ key: val }` convertit les `Ident` en `LitStr`
- **ListLit/SetLit/MapLit en AOT** — runtime C `yk_list`, dispatch via `"ptr"` type
- **`print(%yk_result)` avec strings** : `yk_result_str_new` heap-alloue les strings, `yk_print_result_val` détecte string vs int par heuristique. `Error("fail")` s'affiche correctement.
- **List methods `sort/reverse/insert/remove/clear` en AOT** : C runtime + LLVM dispatch
- **`net` Server en AOT** : `Server()` constructeur, `app.get(path, handler)`, `app.serve(addr)` avec :
  - Retrait de `"net"` de `unsupported_stds`
  - Runtime C Winsock (TCP socket, bind, listen, accept, HTTP parsing, dispatch)
  - Handlers littéraux (`app.get("/hello", "Hello!")`) via `generate_static_handler_ir`
  - Handlers fonction (`app.get("/hello", hello_fn)`) via `generate_fn_handler_ir` + `fn_defs`
  - Compat test `compat_net_server_import`
- **Compat tests** : 82 tests interp vs AOT (try/?, list methods, net server, nullable, union, shift, pow, etc.)
- **Closures/arrow functions comme handlers `app.get()`** : `Expr::FnLit`/`Expr::Closure` détectés dans `compile_server_method`, convertis en `FnDef` → `generate_fn_handler_ir` → `handler_irs`
- **Nullable/Safe-call/Elvis en AOT complet** :
  - `type_to_llvm` existant → `%__nullable_{inner} = type { inner, i1 }`
  - `wrap_in_nullable` helper : `LitNull` → `zeroinitializer` + flag=0, non-null → `insertvalue` + flag=1
  - `Decl`/`Assign`/`Return` : wrapping auto des valeurs non-nullables dans le struct nullable
  - `print(%__nullable_*)` : branch `isnull`/`nonnull` → affiche `(null)` ou la valeur
  - `str(%__nullable_*)` : idem avec phi merge
  - SafeCall AOT existant (branch + extractvalue + phi) + fix `compile_field_access`/`guess_field_type` pour noms manglés `_struct_Foo`
  - Elvis AOT existant (branch + extract + phi) + fix phi `%%` double-prefix
  - Type checker : `T` assignable à `T?` via `types_compatible(Nullable(inner), actual)`
- **`net` Server : keep-alive + thread pool** :
  - `yk_server` struct étendu avec `listen_fd` + `running` flag
  - Thread pool : 8 workers `_beginthreadex`, `accept()` thread-safe, `WaitForMultipleObjects`
  - Keep-alive : détection `Connection: keep-alive` (case-insensitive), HTTP/1.1 keep-alive par défaut
  - `yk_handle_client` : loop par connexion pour keep-alive, fermeture sur `Connection: close`
  - `yk_check_keep_alive` helper : recherche case-insensitive dans les headers
- Tests : 269 → 274 → 279 → 281 → 284 → 301 → 303 → 305 → 300 → 307 → 319 → 320 → 320 (stable, 0 failed hors JIT ignorés)
- **Nouvelle syntaxe types** : `T[]` pour list (remplace `list<T>`), `set[T]` (remplace `Set<T>`), `map[K,V]` (remplace `Map<K,V>`) avec crochets
- **Nouvelle syntaxe littéraux** : `map{ key: value }` pour les maps explicites, `set{ 1, 2, 3 }` gardé (inchangé)
- **Parens obligatoires** : `if (cond) { }` et `while (cond) { }` imposent les parenthèses
- **Phase 3a : `.length` propriété** — interpréteur (Expr::Field/SafeCall → `s.len()`, `items.len()`, etc.), type checker (match types → `Int(0)`), AOT codegen (string: `yk_string_len_ptr`, ptr: `yk_list_len`). ✅
- **Phase 3b : `.toString()` méthode** — interpréteur (dispatch via `Value::to_string()` avant arms structurelles), type checker (`"toString"` → `TypeExpr::Str`), AOT codegen (dispatch par type avec `yk_string_from_int/real/bool/complex`, `yk_result_to_string`, `yk_list_to_string`, nullable phi merge). ✅
- **Phase 2 : Syntaxe classe avec constructeur auto** — `class Foo(x: int) { init { } }` avec paramètres constructeur automatiquement champs + appel `init()` sur instantiation. Interpréteur : `Expr::Call(Ident("Foo"), args)` détecté comme constructeur de classe, mapping args→fields, appel `run_init_blocks` (fix: exécution `init_body` propre de la classe). Type checker : `get_class(name)` → validation args contre `constructor` params. AOT codegen : classe identifiée dans `_ =>` de `compile_call`, génération struct LLVM (vtable ptr + champs). `class_defs` enrichi des `constructor` params. `run_init_blocks` corrigé pour exécuter aussi l'`init_body` de la classe courante. ✅
- **`init {}` en AOT** : compilation via `compile_class_init` → génération de `__class_init_<module>_<class>` (alloca self, exécute init_body, ret void). Appelée après création de struct dans `Expr::StructLit` et constructeur call. Compat tests verts interp=AOT. ✅
- **`const<T>` syntaxe type + Const en AOT** : nouveau `TypeExpr::Const(Box<TypeExpr>)` variant dans l'AST. Parseur : `x: const<real>` et `x: const` (sans type). Type checker : `resolve_type` unwraps `Const(inner)` → `inner` ; `Decl` détecte `Const` pour marquer `is_const = true`. AOT : `type_to_llvm` unwraps `Const` ; globaux `ItemKind::Const` émis comme globaux LLVM compile-time (initialiseurs pour `LitInt`/`LitReal`/`LitBool`/`LitStr`, fallback runtime store dans `@main`). Interpréteur : inchangé (ignore type_expr via `..`). 🔧
- **Ranges descendants en AOT** : `5..1` → 5,4,3,2,1 ; `'z'..'a'` → zyx...a. Direction déterminée à runtime via `icmp sle` + `select` pour condition et step. ✅
- **Sprint 2 complet (Memory pool + parsing + route trie)** :
  - **Memory pool TLS** : `yk_pool_init`/`yk_pool_alloc`/`yk_pool_reset` — bloc-based allocator avec TLS (`__declspec(thread)`), blocks 64KB
  - **TLS handler buffer** : `yk_get_handler_buf` remplace `handler_buf[16384]` stack
  - **Single-pass HTTP parsing** : `yk_scan_http` remplace 4 appels `strstr`+`memchr` par un scan unique
  - **Radix tree route trie** : `yk_route_node`/`yk_route_insert`/`yk_route_match_node` remplace boucle O(n)
  - **Pool alloc pour accept ops** : `yk_post_accept` utilise `yk_pool_alloc`
  - **Code mort supprimé** : `yk_route`, `yk_route_match`, `yk_check_keep_alive`, `YK_HANDLER_BUF_SIZE`
- **Sprint 3 — Linux + io_uring** : serveur HTTP cross-platform Windows/Linux
  - **Platform abstraction** : `yk_socket_t`, `YK_INVALID_SOCKET`, `yk_closesocket`, `YK_TLS`, `YK_GET_ERR()`
  - **Linux compat stubs** : OVERLAPPED dummy, WSAStartup/WSACleanup no-op, etc.
  - **yk_process_request extraite** : fonction cross-platform de parsing + route + handler + build response
  - **yk_conn cross-platform** : conditionnel `OVERLAPPED ov` sous `#ifdef _WIN32`
  - **yk_server struct** : conditionnel `HANDLE iocp` (Win) / `struct io_uring ring` (Linux) sous `#ifdef`
  - **yk_server_new** : pool mémoire Winsock uniquement sous `#ifdef _WIN32`
  - **yk_server_serve** : implémentations séparées Winsock IOCP / Linux io_uring
  - **yk_iocp_worker** : Windows IOCP thread pool (inchangé)
  - **yk_io_uring_worker** : Linux io_uring boucle CQE, accept/recv/send dispatch, keep-alive
  - **io_uring init** : `IORING_SETUP_SQPOLL | IORING_SETUP_COOP_TASKRUN`, fallback sans SQPOLL
  - **Accept batching** : 32 SQE préparés d'avance pour io_uring
  - **Tests** : 238 + 82 compat = 320 pass, 0 failed, zéro warning Rust
- **Sprint 4 — Zero copy + dispatch** :
  - Vectored I/O : header dans `c->resp_buf` + body séparé (global constant) → 2-buffer WSASend/writev
  - `yk_conn` enrichi : `body_ptr`, `body_len`, `header_len` pour les envois vectorisés
  - `yk_process_request` : `memcpy` supprimé, body passe directement du handler vers le socket
  - `yk_on_recv`/`yk_on_send` Windows : WSASend avec 2 WSABUF (header + body en un seul appel)
  - `yk_io_uring_worker` Linux : `writev` avec 2 iovec (header + body)
  - Partial sends gérés : calcul d'offset header/body pour reprise d'envoi
  - Tests : 238 + 82 compat = 320 pass, 0 failed, zéro warning Rust
- **Sprint 5 — HTTP/2 baseline** :
  - H2 frame format : 9-byte header, 10 frame types (DATA/HEADERS/PRIORITY/RST_STREAM/SETTINGS/PUSH_PROMISE/PING/GOAWAY/WINDOW_UPDATE/CONTINUATION), flags, error codes
  - HPACK Huffman decoder : 256-entry code table (code + length), bit-by-bit tree walking
  - HPACK static table : 61 entries conformes RFC 7541 (name + value pairs)
  - HPACK decoder complet : integer (prefix 4/6/7 bits), string (Huffman/plain 7-bit length), indexed header field (0x80), literal with incremental indexing (0x40), literal without indexing/never indexed (0x00), with bounds-checked static table access
  - H2 stream : idle/open/half-closed/closed states, up to 128 concurrent, header_data + body storage
  - H2 session : SETTINGS exchange (2-byte ID + 4-byte value), frames dispatch, stream lifecycle
  - `yk_h2_send_response` : HPACK-indexed `:status` (0x88=200, 0x8D=404, etc.)
  - Détection preface H2 (24 octets) dans `yk_process_request`
  - Blocking read loop dans `yk_h2_run` pour keep-alive multi-request
  - Fix C compilation MSVC : forward decl `yk_h2_run`, replace compound literal, add `YK_H2_REFUSED_STREAM`
  - Fix HPACK static table : correct name/value pairs for all 61 entries
  - Fix SETTINGS frame format : 6-byte per entry (2-byte ID + 4-byte value)
  - Fix HPACK "never indexed" mask : `(first & 0xC0)` au lieu de `(first & 0xF0)`
  - Fix padded HEADERS uint32_t underflow
  - Compat test `compat_h2_basic_get` : envoi raw H2 frames (preface + SETTINGS + HEADERS GET) → validation réponse HEADERS `:status 200`
  - Tests : 239 + 82 = 321 pass, 0 failed, zéro warning Rust

- **Sprint 6 — Cache-line packing + H2 multiplexing** (17 June 2026) :
  - **yk_conn field reorder** : 8-byte pointers first, OVERLAPPED (Win), ints packed. Linux: 120B → 112B, Windows: 160B → 148B. Zéro padding perdu.
  - **yk_vmem_pool_destroy + free(s)** : ajoutés dans les deux branches `yk_server_serve` (Windows IOCP, Linux io_uring) pour éliminer la fuite mémoire à l'arrêt.
  - **H2 async re-entry fix** : `yk_process_request` détecte `c->h2` déjà actif avant de chercher la preface H2. Évite le fallback H1 sur les recvs suivants.
  - **H2 WINDOW_UPDATE flow control** : `yk_h2_handle_data` envoie WINDOW_UPDATE (stream + connection) après chaque DATA recue. Permet le contrôle de flux bidirectionnel.
  - **H2 END_STREAM on DATA** : `yk_h2_handle_data` détecte désormais `flags & YK_H2_FLAG_END_STREAM` et appelle `yk_h2_process_stream` pour les requêtes POST/PUT avec body.
  - **Stream struct shrink** : retrait des champs morts `method[64]` + `path[4096]` de `yk_h2_stream` (non utilisés — les headers sont parsés depuis `header_data`). Chaque session passe de ~537 KB à ~23 KB (128 streams).
  - Tests : 321/321 pass, 0 failed.

- **Phase 1 — Sockets bruts** (17 June 2026) :
  - **TcpStream** : `TcpStream(addr)` constructor (connect), `.send(data)`, `.recv(n)`, `.close()`. Interp via `std::net::TcpStream`, AOT via `yk_tcp_connect`/`send`/`recv`/`close` (Winsock + POSIX, cross-platform with `#ifdef`). 3 compat tests verts (echo interp==AOT).
  - **UdpSocket** : `UdpSocket(addr)` constructor (bind), `.send_to(data, addr)`, `.recv_from(n)`, `.close()`. Interp via `std::net::UdpSocket`, AOT via `yk_udp_bind`/`send_to`/`recv_from`.
  - **TcpListener** : `TcpListener(addr)` constructor (bind+listen), `.accept()` → TcpStream, `.close()`. Interp via `std::net::TcpListener`, AOT via `yk_tcp_listen`/`yk_tcp_accept`.
  - **DNS lookup** : `use { lookup } from "net"` → `lookup(host)` retourne l'IP sous forme de string. Interp via `ToSocketAddrs`, AOT via `yk_dns_lookup` (getaddrinfo + inet_ntop).
  - Tests : 324/324 pass (0 failed, 2 ignorés JIT).

- **Sprint 7 — Fusion async + parallel : work-stealing handler pool** (17 June 2026) :
  - **Handler pool activé par défaut** : `yk_ensure_handler_pool(s)` appelé dans `yk_server_serve` (Win + Linux). 4 threads dédiés. Plus de blocage des workers IOCP sur les handlers.
  - **Dangling pointer fix** : `yk_hw_item` utilisait des pointeurs vers des structs `req`/`resp` sur la pile de `yk_process_request`. Remplacés par des types nommés `yk_hw_resp`/`yk_hw_req` copiés par valeur dans la queue — plus de use-after-free.
  - **Async send dans les workers** : les threads du handler pool exécutent le handler (compute pur), construisent la réponse, et postent un **WSASend** (Win) / **io_uring writev** (Linux) — plus aucun `send()`/`recv()` bloquant. Le keep-alive est géré par le worker IOCP/io_uring via `yk_on_send`.
  - **Lock-free MPMC queue** : Treiber stack (CAS) remplace l'anneau borné (mutex + capacité 1024). Enqueue wait-free (CAS only), dequeue lock-free avec fast path CAS + CV wait. Pas de `Sleep(0)` spin, pas de contention mutex.
  - **TLS node cache** : `yk_tls_hw_node_free` amortit les `malloc`/`free` par thread.
  - **Dangling method/path fix** : `yk_hw_item` embarque ses propres buffers `_method[64]`/`_path[4096]` pour que les handlers offloadés lisent des données valides (évite le use-after-free du stack de `yk_process_request` où `c->method`/`c->path` pointaient vers `method_buf`/`path_buf` locaux).
  - Architecture : 8 workers IOCP/io_uring (I/O pur) + 4 threads handler pool (compute + async send).
  - Tests : 321/321 pass, 0 failed.

### In Progress

### Blocked
- (none)

## Current Sessions

### Session 7 (14 June 2026) — Sprint 2 complet

### Session 8 (14 June 2026) — Sprint 3 complet (Linux + io_uring)
- **Platform abstraction** : `yk_socket_t`, `YK_INVALID_SOCKET`, `yk_closesocket`, `YK_TLS`, `YK_GET_ERR()` — définit SOCKET=sur Linux, maps `closesocket`→`close`, TLS via `__declspec(thread)`/`__thread`, errno→WSAGetLastError
- **Windows compat stubs Linux** : OVERLAPPED dummy, WSAStartup/WSACleanup no-op, WSAIoctl stub, MAKEWORD, SD_BOTH, SO_UPDATE_ACCEPT_CONTEXT, WAIT_* defines
- **yk_process_request extraite** : fonction cross-platform qui appelle yk_scan_http → radix tree match → handler dispatch → build response. Retourne 1 (prêt), 0 (need more data), -1 (WS handled)
- **yk_on_recv Windows réécrite** : appelle yk_process_request, puis WSARecv/WSASend selon le retour
- **#ifdef _WIN32** autour du code IOCP (yk_on_send, yk_iocp_worker, yk_server_serve)
- **Linux io_uring implémenté** :
  - `#elif __linux__` avec `<liburing.h>` + `<sys/epoll.h>`
  - `yk_io_uring_worker` : boucle `io_uring_wait_cqe`, dispatch accept/recv/send via `user_data`
  - `yk_server_serve` Linux : socket `SOCK_NONBLOCK`, `io_uring_queue_init_params` avec `IORING_SETUP_SQPOLL | IORING_SETUP_COOP_TASKRUN`, fallback sans SQPOLL, batching 32 accept SQE
  - Keep-alive géré (post recv après send completion)
- **TLS pool** : threads `pthread_t` pour Linux, `_beginthreadex` pour Windows
- **yk_server struct cross-platform** : `yk_socket_t listen_fd` + `#ifdef _WIN32` (iocp/pools) / `#elif __linux__` (io_uring ring)
- **yk_conn struct** : OVERLAPPED ov seulement sous `#ifdef _WIN32`
- **yk_server_new** : allocation pool Winsock seulement sous `#ifdef _WIN32`
- **yk_post_accept** : sous `#ifdef _WIN32` uniquement (utilise AcceptEx)
- **Tests** : 238 passé (0 failed, 2 ignorés JIT) + 82 compat passé (0 failed) = 320 total, zéro warning Rust

### Session 9 (14 June 2026) — Sprint 4 : Zero copy + dispatch
- **Vectored I/O** : header dans `c->resp_buf` + body séparé (global constant LLVM) → 2-buffer WSASend/writev
- **`yk_conn` enrichi** : `body_ptr`, `body_len`, `header_len` pour les envois vectorisés

### Session 10 (14 June 2026) — Sprint 5 : HTTP/2 baseline + bug fixes
- **Sprint 5 complet** : H2 frame handling, HPACK decode/encode, stream/session management, preface detection
- **Fixes C MSVC** : forward declaration `yk_h2_run`, `YK_H2_REFUSED_STREAM` constante, compound literal → tableau local
- **Fix HPACK static table** : 62→61 entrées, paires (name, value) correctes conformes RFC 7541
- **Fix SETTINGS frame format** : 8-byte → 6-byte par entrée (ID 2 + value 4), valeurs correctes (65535, 16384)
- **Fix response HPACK encoding** : switch sur status code → indexed entries (0x88=200, 0x8D=404, etc.)
- **Fix "never indexed" HPACK mask** : `0xF0` → `0xC0` pour couvrir 0x10-0x1F
- **Fix padded HEADERS underflow** : bounds check sur pad length pour éviter wrap uint32_t
- **Blocking read loop** : `yk_h2_run` refactorisé avec boucle `recv` pour keep-alive multi-request
- **Compat test H2** : `compat_h2_basic_get` envoie raw H2 frames préfabriquées, valide HEADERS `:status 200`
- **Tests** : 239 + 82 compat = 321 pass (0 failed), zéro warning Rust
- **AGENTS.md** : Sprint 5 marqué terminé
- **`yk_process_request`** : `memcpy` supprimé, body passe directement du handler vers le socket via `body_ptr`/`body_len`
- **`yk_on_recv`/`yk_on_send` Windows** : WSASend avec 2 WSABUF (header + body en un seul appel système)
- **`yk_io_uring_worker` Linux** : `writev` avec 2 iovec (header + body) au lieu de 1
- **Partial sends** : calcul d'offset header/body pour reprise d'envoi (quand header partiellement envoyé → body inclus dans 2e buffer ; quand header entièrement envoyé → body seul avec offset ajusté)
- **Tests** : 238 passé (0 failed) + 82 compat passé (0 failed) = 320 total, zéro warning Rust
- **Memory pool TLS** : `yk_pool_init`/`yk_pool_alloc`/`yk_pool_reset` — bloc-based allocator avec TLS (`__declspec(thread)`), blocks 64KB, allocation alignée, fallback block sur mesure pour grandes tailles
- **TLS handler buffer** : `yk_get_handler_buf` remplace `handler_buf[16384]` stack — auto-croissance TLS, `yk_tls_handler_buf`/`yk_tls_handler_buf_size`
- **Single-pass HTTP parsing** : `yk_scan_http` remplace les 4 appels `strstr`+`memchr` — scan unique du buffer, extrait method/path/version/keep-alive/WebSocket upgrade en une passe
- **Radix tree route trie** : `yk_route_node` segment-based trie, `yk_route_insert`/`yk_route_match_node` remplacent `yk_route_match` + boucle `O(n)`, support `{param}` et `{param:int}`, préférence exact match sur param
- **Pool alloc pour accept ops** : `yk_post_accept` utilise `yk_pool_alloc` au lieu de `malloc`/`free`
- **Suppression code mort** : `yk_route`, `yk_route_match`, `yk_check_keep_alive`, `YK_HANDLER_BUF_SIZE` supprimés
- **Tests** : 238 pass (0 failed, 2 ignorés JIT), zéro warning Rust
- **TASKS.md** : Sprint 2 marqué terminé
- **POST/PUT/DELETE/PATCH + WS en AOT** : `compile_server_method` étendu, détection `"get"|"post"|"put"|"delete"|"patch"|"ws"|"serve"`
- **POST/PUT/DELETE/PATCH + WS en interp** : `call_net_method` dispatch, `Value::Fn(fndef)` supporté
- **Trailing block** `app.get("/") { body }` : parser désucre en `fn() { body }`, fonction `is_method_call` ajoutée
- **WebSocket runtime C** : SHA-1, base64, upgrade 101, frame loop (text/ping/pong/close), `yk_ws_handle`
- **FnLit handler fix** : FnLit/Closure args sautés dans `compile_call` pour éviter le conflit `ret %yk_string` dans fn `i64`
- **String constants uniques** : `FnIrGen` utilise handler_name comme préfixe
- **Route matching WS** : détection `Upgrade: websocket` dans `yk_on_recv`
- **Forward decl** `yk_ws_handle` pour éviter C3861 (C multi-pass)
- **Tests** : 322 passent (0 failed), zéro warning Rust
- **TASKS.md** créé pour tracker toutes les tâches restantes

### Session 4 (12 June 2026)
- **Union AOT complet** — auto-wrapping `Decl`/`Assign`/paramètres dans `%yk_variant` :
  - `wrap_in_variant` fixé : alloca + store + ptrtoint pour strings (évite LLVM type conflict)
  - `print(%yk_variant)` / `str(%yk_variant)` tag dispatch fixé : lookup `"int(0)"` au lieu de `"int"`
  - `fn_param_union_variants` : nouveau HashMap pour stocker les TypeExpr des paramètres union
  - `compile_call` : wrapping auto des arguments dans `%yk_variant` pour paramètres union
  - `Expr::Variant` : payload string via alloca + store + ptrtoint
  - 4 compat union tests passent (étaient bloqués)
- **Zéro warning Rust** maintenu
- **Test count** : 307 passed, 0 failed

### Session 1 (29 May 2026)
- **Tuple AOT compat tests** : `check_compat` pour `(42, "hello")` avec `t.0`/`t.1`. Fix bugs:
  - `tuple_elem_types` helper + reverse lookup via `tuple_type_names`
  - Separator `||` au lieu de `_` pour éviter conflit avec `%yk_string`
  - `expr_type_str` for `Expr::Field` : détection tuple → retourne le bon type d'élément
  - Codegen `Expr::Field` : `field_ty` utilise le vrai type d'élément au lieu de `"i64"` dur
- **PostDec en lexer** : `--` n'était pas tokenizé (manquait dans le lexer ligne 175). Ajouté `Token::Dec` pour `--`. Test compat vert.
- **PostInc** : test compat vert (fonctionnait déjà)
- **MapLit** : pas de compat test — `print` AOT utilise `yk_list_print` qui affiche les entiers bruts, pas le format `{key: value}`

### Session 3 (12 June 2026)
- **Interface AOT : dispatch complet avec return types** :
  - Ajout de `interface_method_ret_types` HashMap pour stocker les types de retour des méthodes d'interface
  - Dispatch `compile_call` `%iface.*` : utilisation du vrai type de retour au lieu de `"i64"` dur
  - Gestion `void` retour : pas de `%name =` pour les `call void`
- **Object AOT : réparation complète** :
  - Suppression de `"objects not supported in AOT"` du check `unsupported_aot_features`
  - Ajout de `object_method_has_self` HashSet pour distinguer les méthodes avec/sans `self`
  - Dispatch conditionnel : `self_int` passé seulement si la méthode a `self`
  - `compile_object_init` appelée depuis `compile_modules` (était dead code)
- **Class method dispatch** : correction des arguments (self + tous les args au lieu de self seul), ajout du `%` manquant devant `self_int`
- **Interface wrapping dans les appels de fonction** :
  - Ajout de `fn_param_types` HashMap pour connaître les types de paramètres des fonctions
  - À l'appel `describe(Circle{...})`, wrapping auto `%class.Circle` → `%iface.Drawable` (fat pointer)
- **Regex module** : nom `"regex"` ajouté au dispatch (`mod_name == "regex"`) en plus de `"re"`
- **`str(list)` AOT** : ajout d'un bras `"ptr"` dans `str()` dispatch → `yk_list_to_string(ptr)`
- **Void function calls** : les appels de fonction à retour `void` n'ont plus de `%name =`
- **Compat test pass count** : 267 → 300 pass (PoisonError en cascade résolu)
- **AOT_LOCK poisoning fix** : `drop(_lock)` explicite avant les assertions pouvant panic dans les serveurs tests (`net_server_int_param`, `wildcard_route`, `real_request`). Fin du PoisonError en cascade.
- **`req.*` field access dans le codegen régulier** : `compile_expr` `Expr::Field` pour `obj_ty == "i64"` avec champs `method`/`path`/`body` → `inttoptr` + `getelementptr` + `insertvalue` pour construire `%yk_string`. Permet la compilation AOT des fonctions handler régulières (dead code, mais nécessaire pour IR valide).
- **`%yk_variant` dispatch `print`/`str`** : tag-based dispatch pour `int`/`str` variants (int→yk_print_int/yk_string_from_int, str→yk_print_str_ptr/inttoptr). Préparé pour quand l'auto-wrapping union sera implémenté.
- **Union variant name registration** : `type_to_llvm` pour `TypeExpr::Union` enregistre désormais tous les noms de variants via `get_variant_tag`.
- **Zéro warning Rust** : suppression de `wrap_in_iface` (dead code, wrapping fait inline dans `compile_stmt` et `compile_call`).
- **Nouveaux champs LlvmCodegen** : `fn_param_types`, `interface_method_ret_types`, `object_method_has_self`, `class_modules`

### Session 5 (12 June 2026)
- **Tous les opérateurs arithmétiques manquants** : ajout de `%`, `**`, `&`, `|`, `^`, `~`, `<<`, `>>`, `+=`, `-=`, `*=`, `/=`, `%=`, `**=`, `|=`, `&=`, `^=`, `<<=`, `>>=` avec :
  - **Token/Lexer** : 21 nouveaux tokens (Percent, StarStar, Caret, Tilde, Shl, Shr, tous les Eq)
  - **AST** : 7 nouveaux BinOp (Mod, Pow, BitAnd, BitOr, BitXor, Shl, Shr) + UnOp::BitNot
  - **Précédence** : table étendue de 7 à 12 niveaux, `**` right-associatif, unaires à 13
  - **Compound assign** : desugar dans le parseur (`a += b` → `a = a + b`)
  - **À l'interpréteur** : Mod/Pow/Bitwise/Shift dans `eval_binop`, BitNot dans `eval_unop`
  - **En AOT** : `srem`/`frem` (mod), `yk_pow_int`/`yk_pow_real` (puissance C runtime), `and`/`or`/`xor` bitwise, `shl`/`ashr`, `xor -1` (bitnot)
  - **Fix compile_binop Assign** : utilise `find_alloca_for_expr` au lieu du SSA value
  - **Fix `>>` nested generic** : `eat_gt()` split `Shr` → deux `Gt` dans le token stream
- **Test count** : 238 passed, 0 failed (hors compat)

### Blocked
- (none)

## Key Decisions
- `yk_server_new()` retourne `i64` handle, `yk_server_add_route(i64, ptr method, ptr path, i64 fn_ptr)` enregistre les routes, `yk_server_serve(i64, ptr addr)` démarre la boucle TCP
- Handlers AOT : collectés dans `handler_irs: Vec<String>` et émis après les corps de fonction dans `compile_modules`
- `fn_defs: HashMap<String, FnDef>` pré-rempli dans `compile_modules` pour activer la compilation des handlers
- Winsock via `WIN32_LEAN_AND_MEAN` pour éviter le conflit `winsock.h` vs `winsock2.h`
- `yk_server_serve` est un stub si `s->count == 0`
- **Thread pool** : 8 workers `_beginthreadex` (Win) / `pthread_create` (Linux), accept thread-safe, `WaitForMultipleObjects` / `pthread_join`
- **Keep-alive** : détection `Connection: keep-alive` (case-insensitive), HTTP/1.1 keep-alive par défaut, HTTP/1.0 close par défaut
- `yk_server` struct : `yk_socket_t listen_fd` + `volatile int running` + `#ifdef _WIN32` (iocp/pools) / `#elif __linux__` (io_uring ring)
- `#define WIN32_LEAN_AND_MEAN` nécessaire pour éviter le conflit `winsock.h` vs `winsock2.h`

## Next Steps
1. **Sprint 8 — SIMD + tuning** : AVX2 parsing, sysctl, benchmarks

## Critical Context
- `LlvmCodegen` a maintenant `handler_irs: Vec<String>` et `fn_defs: HashMap<String, FnDef>` ; `fn_defs` est peuplé dans `compile_modules` et utilisé dans `compile_server_method`
- `compile_server_method` est appelée depuis `Expr::Field(obj, field)` dans `compile_call`, pour `o_ty == "i64"` et `field ∈ {"get","post","serve"}`
- Handlers : `Expr::LitStr` → `generate_static_handler_ir` ; `Expr::Ident(fn_name)` → lookup `fn_defs` → `generate_fn_handler_ir` ; `Expr::FnLit`/`Expr::Closure` → conversion en `FnDef` → `generate_fn_handler_ir`
- `generate_fn_handler_ir` retourne `Option<String>` — les fonctions sans `return` explicite sont ignorées (return `None`)
- `handler_irs` émis après tous les corps de fonction, avant `string_constants`
- `yk_server_serve` utilise désormais IOCP (Windows) ou io_uring (Linux) ; thread pool de 8 workers partageant un même ring/C-IOCP
- `yk_scan_http` remplace les 4 appels `strstr`+`memchr` par un scan unique du buffer (method/path/version/keep-alive/WS upgrade en une passe)
- `yk_route_insert`/`yk_route_match_node` (radix tree segment-based) remplace `yk_route_match` + boucle O(n)
- `yk_pool_alloc`/`yk_pool_reset` (TLS block allocator) remplace `malloc`/`free` pour accept ops
- `yk_get_handler_buf` (TLS auto-croissance) remplace `handler_buf[16384]` stack
- `yk_server` struct étendue avec `listen_fd: SOCKET` et `running: volatile int`
- `#define WIN32_LEAN_AND_MEAN` nécessaire pour éviter le conflit `winsock.h` (inclus par `windows.h`) vs `winsock2.h`
- `FnDef` importé depuis `crate::interpret::FnDef` (re-exporté par `src/interpret/mod.rs`)
- `unsupported_stds` = `[]` (vide) — tous les modules std supportés en AOT

## Relevant Files
- `src/compat.rs` : module de compat tests (82 tests, tous verts), contient `run_interp`, `run_aot`, `check_compat`, `check_compat_approx`
- `src/codegen/llvm.rs` : RUNTIME_C avec plateforme abstraite (#ifdef _WIN32 / __linux__), serveur HTTP Winsock IOCP + io_uring + thread pool, handlers AOT, compilation batch MSVC/gcc
- `src/codegen/backend.rs` : collecte des `.lib` FFI depuis les imports dans `LlvmBackend::compile_with_paths`
- `src/module/mod.rs` : `compile_cpp_ffi` branch clang++ → `-Xlinker /IMPLIB:`
- `src/cli/mod.rs` : `build_program` → `pub(crate)`
- `src/interpret/env.rs` : `run_main()` retourne `Ok(interp.output)` avec les print capturés
- `src/interpret/expr.rs` : `builtin_funcs` pour `math`/`time`/`net`, `builtin_modules` pour `io`/`sys`/`fs`/etc.
- `src/semantic/typeck.rs` : registre les modules std ; `sys`/`fs` absents de la liste `"io" | "json" | "..."` (tombent dans `_` → `Infer`)
- `src/stdlib.rs` : liste SUBMODULES (fs, sys, json, datetime, path, base64, re, math, time)
