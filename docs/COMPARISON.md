# Yidika vs Rust, TypeScript/TSX, Python et Kotlin

Comparaison au **14 novembre 2026**, à partir du code de ce dépôt (`yidi`), pas de la documentation marketing.

## Méthode et limites

- **Comparaison de surface et d'outillage**, pas de benchmark. Aucun chiffre de performance n'est inventé : les toolchains Node/Deno, Kotlin/JVM et un compilateur C/MSVC complet ne sont pas disponibles dans cet environnement, et le backend AOT de Yidika est Windows-only.
- Les affirmations sur Rust, TypeScript, Python et Kotlin décrivent leurs implémentations de référence actuelles (rustc/LLVM, tsc/Node, CPython 3.13+, Kotlin/JVM).
- Les affirmations sur Yidika sont vérifiées dans le code : `src/interpret/` (interpréteur), `src/codegen/llvm.rs` (AOT, Windows), `src/netlib/` (HTTP), `gui/` (GUI), `src/stdlib.rs` (surface std).
- Les comparaisons syntaxiques détaillées existent déjà : [`comparison-python.md`](./comparison-python.md) et [`comparison-ts.md`](./comparison-ts.md). Ce document est la vue d'ensemble multi-langages.

## Résumé en une table

| Critère | Yidika | Rust | TypeScript / TSX | Python | Kotlin |
|---|---|---|---|---|---|
| Modèle | Impératif, mutable par défaut | Impératif, immuable par défaut | Impératif, mutable (`let`) | Impératif dynamique | Impératif + fonctionnel |
| Typage | Statique, fort, inférence | Statique, fort, inférence | Statique, structurel, effacement à l'exécution | Dynamique, duck typing | Statique, nominal, inférence |
| Gestion mémoire | Interpréteur : GC de valeurs ; AOT : runtime C | Ownership + borrow checker | Ramasse-miettes (V8) | Comptage de références + GC cyclique | GC JVM/ART |
| Compilation | Interpréteur **et** AOT LLVM (Windows) | AOT natif | Transpilation vers JS | Bytecode CPython | Bytecode JVM, AOT Android (ART), Kotlin/Native |
| Concurrence | `spawn`/`await` parsés, runtime synchrone | Threads, async, rayon | Event loop, Web Workers | `asyncio`, threads, `multiprocessing` | Coroutines, threads |
| GUI | Module `gui` (Win32, Windows uniquement) | Crates externes (`egui`, `iced`…) | React/TSX vers DOM | Tkinter, Qt, etc. | Jetpack Compose, JavaFX |
| HTTP intégré | Oui (`net`), interpréteur + handlers JIT | Crates (`axum`, `actix`) | Frameworks npm (`express`, `next`) | Frameworks (`fastapi`, `django`) | Ktor, Spring Boot |
| FFI | `use ... from "c++:..."` / `"rust:..."` | `extern "C"`, `bindgen`, `cbindgen` | N-API / WASM | `ctypes`, `cffi`, extensions C | JNI, `cinterop` |
| Écosystème | Quasi nul (dépôt unique) | Très large (crates.io) | Immense (npm) | Immense (PyPI) | Large (Maven Central) |
| Maturité | MVP expérimental | Stable depuis 2015 | Stable depuis 2012 | Stable depuis 1991 | Stable depuis 2016 |
| Courbe d'apprentissage | Faible (syntaxe type Python) | Élevée (borrow checker) | Moyenne | Très faible | Moyenne |

## Les cinq langages en un paragraphe

**Rust** - langage système sans GC, dont la sûreté mémoire est prouvée à la compilation. Excellent pour l'infrastructure, les performances prévisibles et l'embarqué. Prix à payer : temps de compilation, complexité des lifetimes et de l'async, verbosité des conversions.

**TypeScript / TSX** - JavaScript typé, transpilé. TSX ajoute le JSX pour décrire des interfaces. C'est le choix par défaut du web front-end et d'une grande partie du back-end Node. Types effacés à l'exécution : la sûreté s'arrête aux frontières du typage statique.

**Python** - dynamique, lisible, omniprésent en data/IA, scripting et prototypage. La productivité est inégalée, mais les performances brutes et la détection d'erreurs à la compilation ne sont pas au rendez-vous ; les parties chaudes finissent en C/C++/Rust.

**Kotlin** - langage JVM moderne, concis, avec null-sécurité dans le type système, coroutines et interopérabilité Java totale. Standard du développement Android via Compose. Démarrage JVM plus lourd, écosystème backend solide (Ktor, Spring).

**Yidika** - projet expérimental qui vise le créneau « syntaxe proche d'un langage de script, exécution native ». Il combine un interpréteur (portable) et un backend AOT LLVM (Windows), avec HTTP et GUI intégrés. Aujourd'hui : idées intéressantes, écosystème inexistant, outillage de production absent.

## Le même programme, cinq fois

### 1. Bonjour + boucle

```rust
// Rust
fn main() {
    for i in 0..3 {
        println!("Hello #{i}");
    }
}
```

```tsx
// TypeScript / TSX (ici TS pur)
for (let i = 0; i < 3; i++) {
  console.log(`Hello #${i}`);
}
```

```python
# Python
for i in range(3):
    print(f"Hello #{i}")
```

```kotlin
// Kotlin
fun main() {
    for (i in 0 until 3) {
        println("Hello #$i")
    }
}
```

```yidika
// Yidika
fn main() {
    for (i in 0..3) {
        print("Hello #" + str(i));
    }
}
```

### 2. Structure de données + pattern matching

```rust
// Rust
enum Shape { Circle(f64), Rect(f64, f64) }

fn area(s: &Shape) -> f64 {
    match s {
        Shape::Circle(r) => 3.14159 * r * r,
        Shape::Rect(w, h) => w * h,
    }
}
```

```tsx
// TypeScript
type Shape =
  | { kind: "circle"; r: number }
  | { kind: "rect"; w: number; h: number };

function area(s: Shape): number {
  switch (s.kind) {
    case "circle": return Math.PI * s.r ** 2;
    case "rect": return s.w * s.h;
  }
}
```

```python
# Python
from dataclasses import dataclass

@dataclass
class Circle: r: float
@dataclass
class Rect: w: float; h: float

def area(s):
    match s:
        case Circle(r): return 3.14159 * r * r
        case Rect(w, h): return w * h
```

```kotlin
// Kotlin
sealed interface Shape
data class Circle(val r: Double) : Shape
data class Rect(val w: Double, val h: Double) : Shape

fun area(s: Shape): Double = when (s) {
    is Circle -> Math.PI * s.r * s.r
    is Rect -> s.w * s.h
}
```

```yidika
// Yidika
interface Shape {
    fn area(&self) -> real;
}

class Circle implements Shape {
    r: real;
    fn area(&self) -> real { return 3.14159 * self.r * self.r; }
}

class Rect implements Shape {
    w: real;
    h: real;
    fn area(&self) -> real { return self.w * self.h; }
}
```

### 3. Serveur HTTP JSON

```rust
// Rust (axum)
#[tokio::main]
async fn main() {
    let app = axum::Router::new().route("/ping", axum::routing::get(|| async { "pong" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

```tsx
// TypeScript (express)
import express from "express";
const app = express();
app.get("/ping", (_req, res) => res.json({ ok: true }));
app.listen(8080);
```

```python
# Python (FastAPI)
from fastapi import FastAPI
app = FastAPI()

@app.get("/ping")
def ping():
    return {"ok": True}
```

```kotlin
// Kotlin (Ktor)
fun main() {
    embeddedServer(Netty, port = 8080) {
        routing { get("/ping") { call.respond(mapOf("ok" to true)) } }
    }.start(wait = true)
}
```

```yidika
// Yidika
use { Server } from "net";

fn main() {
    app: Server = Server();
    app.get("/ping") {
        "pong"
    }
    app.serve(8080);
}
```

### 4. Interface graphique

```rust
// Rust (egui, très simplifié)
eframe::run_native("T", Default::default(), Box::new(|_| Ok(Box::new(App::default()))))?;
```

```tsx
// TSX
export function App() {
  return <button onClick={() => alert("click")}>OK</button>;
}
```

```python
# Python (tkinter)
root = tk.Tk()
tk.Button(root, text="OK", command=lambda: print("click")).pack()
root.mainloop()
```

```kotlin
// Kotlin (Compose)
@Composable fun App() {
    Button(onClick = { println("click") }) { Text("OK") }
}
```

```yidika
// Yidika
use { gui } from "std";

fn on_click(ev: int) {
    print("click");
}

fn main() {
    w = gui.window("T", 200, 100);
    root = gui.div(w);
    b = gui.button(root, "OK");
    gui.on(b, "click", on_click);
    gui.run(w);
}
```

## Matrice de fonctionnalités

| Fonctionnalité | Yidika | Rust | TSX | Python | Kotlin |
|---|---|---|---|---|---|
| Typage statique | Oui | Oui | Oui (effacé) | Non | Oui |
| Inférence de type | Oui | Oui | Oui | - | Oui |
| Génériques | Oui (`list<T>`, `map<K,V>`, classes) | Oui | Oui | Typing seulement | Oui |
| ADT / unions | Oui (`union` de types, interfaces) | Oui (`enum`) | Oui (unions de types) | Non (classes) | Oui (`sealed`) |
| Null-sécurité | `T?` + `null`/`None` | `Option<T>` | `strictNullChecks` | Non | Oui (`T?`) |
| Immutabilité | `const` / `as const` | Défaut | `const` | Conventions | `val` |
| Surcharge d'opérateurs | Non | Oui (traits) | Non | Oui (`__add__`) | Oui |
| Métaprogrammation | Non | Macros | Décorateurs, types conditionnels | Métaclasses | Annotations, KSP |
| Async/await | Syntaxe, exécution synchrone | Oui (tokio…) | Oui (promesses) | Oui (`asyncio`) | Oui (coroutines) |
| Threads natifs | `spawn` (runtime virtuel) | Oui | Workers | Oui | Oui |
| GC | Interpréteur | Non | Oui | Oui | Oui |
| Compilation native | Oui (Windows) | Oui | Non (via bundlers) | Non (via Nuitka/PyInstaller) | Kotlin/Native |
| Interpréteur | Oui | Non | Non (Node exécute du JS) | Oui | Non (JVM) |
| HTTP stdlib | Oui | Non (crates) | Non (npm) | Non (stdlib bas niveau) | Non (Ktor) |
| GUI stdlib | Oui (Win32) | Non | Non | Non | Non |
| FFI directe | Oui (C++/Rust) | Oui | N-API | Oui | Oui |

## Parité interpréteur / AOT : le point faible structurel

C'est la contrainte la plus importante à connaître avant de contribuer.

- Chaque fonctionnalité doit être implémentée **deux fois** : dans `src/interpret/` et dans `src/codegen/llvm.rs`.
- Rien ne le vérifie à la compilation. Seule la suite `src/compat.rs` (89 tests `compat_*`) exécute le même programme avec les deux backends et compare la sortie.
- Ces tests ne tournent que sur Windows avec LLVM + MSVC : sur Linux, `cargo test --lib -- --skip compat` laisse 89 tests de côté.
- L'AOT émet de l'IR LLVM **sous forme de texte**, puis shell-out vers `clang.exe`, `cl.exe` et `link.exe`. Le runtime C/C++ (HTTP, WebSocket, TLS, ordonnanceur) est un littéral de chaîne de ~4 000 lignes dans `llvm.rs`.
- Conséquence pratique : une fonctionnalité « qui marche » dans `yidi mon_fichier.yk` peut échouer silencieusement ou produire un autre résultat avec `yidi build mon_fichier.yk`.

À comparer :

- **Rust** n'a qu'un backend : pas de divergence possible.
- **TypeScript** a plusieurs implémentations (tsc, esbuild, swc) mais un seul modèle d'exécution (JS), validé par une suite de conformité.
- **Python** a CPython, PyPy, GraalPy, mais une spécification de langage et une suite de tests de conformité.
- **Kotlin** cible JVM, JS, Native et WASM, avec une suite de tests commune `kotlin-test` et des tests de conformité par cible.

## Performances : ce qu'on peut dire sans mesurer

Aucun benchmark n'a été exécuté ici. Voici ce qui est raisonnable d'attendre, et pourquoi :

| | Yidika interpréteur | Yidika AOT | Rust | TSX/Node | Python | Kotlin/JVM |
|---|---|---|---|---|---|---|
| Démarrage | Faible (binaire Rust) | Faible | Faible | Moyen (V8) | Faible | Moyen (JVM) |
| Débit numérique | Lent (arbre syntaxique) | Proche du C | Très rapide | Rapide (JIT) | Lent | Rapide (JIT) |
| Latence HTTP | Moyenne | Bonne | Excellente | Bonne | Moyenne | Bonne |
| Empreinte mémoire | Faible | Faible | Très faible | Moyenne | Moyenne | Élevée |

Le backend AOT n'applique **aucune optimisation LLVM** au-delà de ce que `clang -O?` reçoit dans le script généré, et l'absence de passe d'optimisation au niveau IR laisse beaucoup de performance sur la table. L'interpréteur alloue des `Value` et des `Frame` par appel : il est adapté au prototypage, pas aux charges soutenues.

Pour obtenir des chiffres reproductibles, il faudrait au minimum :

```sh
# Yidika (interpréteur)
cargo run --release -- bench/loop.yk

# Yidika (AOT, Windows + MSVC + LLVM)
cargo run --release -- build bench/loop.yk && ./bench/loop.exe

# Rust
rustc -O bench/loop.rs -o bench/loop_rs && ./bench/loop_rs

# Python
python3 bench/loop.py

# Kotlin (JVM)
kotlinc bench/Loop.kt -include-runtime -d loop.jar && java -jar loop.jar

# TypeScript
npx tsx bench/loop.ts
```

## Quand choisir quoi

| Besoin | Choix raisonnable |
|---|---|
| Système, embarqué, zéro GC, sûreté mémoire | **Rust** |
| Web front-end, UI réactive, SSR | **TypeScript / TSX** |
| Data, IA, scripting, automatisation | **Python** |
| Android, back-end JVM, code concis et null-safe | **Kotlin** |
| Prototype rapide avec HTTP/GUI intégrés et sortie native | **Yidika**, en acceptant les limites ci-dessous |
| Apprendre la compilation (lexer, parser, types, LLVM) | **Yidika** : base de code compacte et lisible |

## Limites assumées de Yidika (état du dépôt)

1. **Backend AOT Windows uniquement** - `cmd.exe`, `vcvars64.bat`, `cl.exe`, `link.exe`, `.exe` sont codés en dur, sans `#[cfg]`.
2. **GUI Windows uniquement** - `gui/src/win.rs` est derrière `#[cfg(windows)]` ; ailleurs, `gui.run` retombe sur une pompe sans fenêtre.
3. **Handlers `net` JIT** - sans `LLVM-C.dll` sur le `PATH`, les handlers compilés à la volée sont ignorés (le serveur continue en mode interprété).
4. **Pas de registre de paquets** - `yidi add`/`install` ne s'appuient sur aucun index public.
5. **Pas de garanties mémoire formelles** - l'interpréteur s'appuie sur des `Rc`/clones et des pointeurs bruts pour la GUI et le JIT ; l'AOT s'appuie sur le runtime C.
6. **Écosystème** - pas de bibliothèques tierces, pas de framework web, pas d'ORM, pas d'outils de profilage.
7. **Outillage** - pas de LSP, pas de formateur stable, pas de débogueur intégré.
8. **CI** - la CI Linux ne peut pas valider l'AOT ; `cargo fmt --check` et `cargo clippy -D warnings` échouent déjà sur `main` avec les versions récentes des outils.

## Conclusion

Yidika n'est pas un concurrent de Rust, TypeScript, Python ou Kotlin : c'est un **laboratoire de langage**. Sa proposition - syntaxe accessible, HTTP et GUI dans la stdlib, compilation native - est cohérente, et la base de code (un seul crate Rust, ~8 300 lignes pour le codegen) est étonnamment compacte.

En l'état :

- pour **produire**, utilisez Rust, TypeScript, Python ou Kotlin ;
- pour **expérimenter** un langage et son compilateur, Yidika est un excellent terrain de jeu, à condition de traiter la parité interpréteur/AOT comme une exigence de premier ordre et de valider sur Windows.
