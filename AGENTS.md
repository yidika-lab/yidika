## Goal
Corriger et compléter le langage Yidika (compilateur + interpréteur Rust) en priorisant le codegen LLVM AOT.

## Constraints & Preferences
- 269 tests passent après chaque modification (hors compat tests qui requièrent clang+MSVC)
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
- **Compat tests** : 36 tests interp vs AOT (try/?, list methods, net server)
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
  - Thread pool : 4 workers `_beginthreadex`, `accept()` thread-safe, `WaitForMultipleObjects`
  - Keep-alive : détection `Connection: keep-alive` (case-insensitive), HTTP/1.1 keep-alive par défaut
  - `yk_handle_client` : loop par connexion pour keep-alive, fermeture sur `Connection: close`
  - `yk_check_keep_alive` helper : recherche case-insensitive dans les headers
- Tests : 269 → 274

### In Progress
- (none)

### Blocked
- (none)

## Key Decisions
- `yk_server_new()` retourne `i64` handle, `yk_server_add_route(i64, ptr method, ptr path, i64 fn_ptr)` enregistre les routes, `yk_server_serve(i64, ptr addr)` démarre la boucle TCP
- Handlers AOT : collectés dans `handler_irs: Vec<String>` et émis après les corps de fonction dans `compile_modules`
- `fn_defs: HashMap<String, FnDef>` pré-rempli dans `compile_modules` pour activer la compilation des handlers
- Winsock via `WIN32_LEAN_AND_MEAN` pour éviter le conflit `winsock.h` vs `winsock2.h`
- Le serveur TCP est synchrone (bloquant), sans keep-alive ni thread pool (v1 simple)
- `yk_server_serve` est un stub si `s->count == 0`
- **Thread pool** : 4 workers `_beginthreadex`, `accept()` thread-safe sur Windows, `WaitForMultipleObjects` pour attendre
- **Keep-alive** : détection `Connection: keep-alive` (case-insensitive), HTTP/1.1 keep-alive par défaut, HTTP/1.0 close par défaut. Loop par connexion via `yk_handle_client`
- `yk_server` struct étendue : ajout de `listen_fd: SOCKET` et `running: volatile int` pour le shutdown
- `net` Server AOT : `yk_server_new()` retourne `i64` handle, `yk_server_add_route(i64, ptr method, ptr path, i64 fn_ptr)` enregistre les routes, `yk_server_serve(i64, ptr addr)` démarre la boucle TCP
- Handlers AOT : collectés dans `handler_irs: Vec<String>` et émis après les corps de fonction dans `compile_modules`
- `fn_defs: HashMap<String, FnDef>` pré-rempli dans `compile_modules` pour activer la compilation des handlers
- Winsock via `WIN32_LEAN_AND_MEAN` pour éviter le conflit `winsock.h` vs `winsock2.h`
- Le serveur TCP est synchrone (bloquant), sans keep-alive ni thread pool (v1 simple)
- `yk_server_serve` est un stub si `s->count == 0`

## Next Steps
1. **Route matching avancé** (wildcards `:param`, type `application/json` auto)
2. **Compat tests pour `net` Server** avec test HTTP réel (thread séparé)
3. **Objets/Interfaces AOT** restants

## Critical Context
- `LlvmCodegen` a maintenant `handler_irs: Vec<String>` et `fn_defs: HashMap<String, FnDef>` ; `fn_defs` est peuplé dans `compile_modules` et utilisé dans `compile_server_method`
- `compile_server_method` est appelée depuis `Expr::Field(obj, field)` dans `compile_call`, pour `o_ty == "i64"` et `field ∈ {"get","post","serve"}`
- Handlers : `Expr::LitStr` → `generate_static_handler_ir` ; `Expr::Ident(fn_name)` → lookup `fn_defs` → `generate_fn_handler_ir` ; `Expr::FnLit`/`Expr::Closure` → conversion en `FnDef` → `generate_fn_handler_ir`
- `generate_fn_handler_ir` retourne `Option<String>` — les fonctions sans `return` explicite sont ignorées (return `None`)
- `handler_irs` émis après tous les corps de fonction, avant `string_constants`
- `yk_server_serve` utilise désormais un thread pool de 4 workers (`_beginthreadex`) qui font `accept()` en parallèle ; chaque worker gère le keep-alive via `yk_handle_client` ; boucle par connexion jusqu'à `Connection: close`
- `yk_check_keep_alive` cherche `\r\nConnection: keep-alive` (case-insensitive) ; HTTP/1.1 keep-alive par défaut, HTTP/1.0 close par défaut
- `yk_server` struct étendue avec `listen_fd: SOCKET` et `running: volatile int`
- `#define WIN32_LEAN_AND_MEAN` nécessaire pour éviter le conflit `winsock.h` (inclus par `windows.h`) vs `winsock2.h`
- `FnDef` importé depuis `crate::interpret::FnDef` (re-exporté par `src/interpret/mod.rs`)
- `unsupported_stds` = `[]` (vide) — tous les modules std supportés en AOT

## Relevant Files
- `src/compat.rs` : module de compat tests (36 tests, tous verts), contient `run_interp`, `run_aot`, `check_compat`, `check_compat_approx`
- `src/codegen/llvm.rs` : `ffi_modules`/`ffi_decls` dans `LlvmCodegen`, handler FFI dans `compile_call`, `compile_to_exe_with_extra_libs`, batch file `\r\n` + nom unique, `extractvalue` retourne type réel ; `TypeExpr::Generic` géré dans `type_to_llvm` ; `handler_irs`/`fn_defs` pour net server ; `compile_server_method` ; `yk_server_serve` TCP Winsock thread pool + keep-alive
- `src/codegen/backend.rs` : collecte des `.lib` FFI depuis les imports dans `LlvmBackend::compile_with_paths`
- `src/module/mod.rs` : `compile_cpp_ffi` branch clang++ → `-Xlinker /IMPLIB:`
- `src/cli/mod.rs` : `build_program` → `pub(crate)`
- `src/interpret/env.rs` : `run_main()` retourne `Ok(interp.output)` avec les print capturés
- `src/interpret/expr.rs` : `builtin_funcs` pour `math`/`time`/`net`, `builtin_modules` pour `io`/`sys`/`fs`/etc.
- `src/semantic/typeck.rs` : registre les modules std ; `sys`/`fs` absents de la liste `"io" | "json" | "..."` (tombent dans `_` → `Infer`)
- `src/stdlib.rs` : liste SUBMODULES (fs, sys, json, datetime, path, base64, re, math, time)
