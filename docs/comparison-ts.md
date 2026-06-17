# TypeScript vs Yidika — Syntax Comparison

## Variables

```ts
// TypeScript
let x: number = 5;
const y = "hello";
var z = true;
```

```yidika
// Yidika
x: int = 5
y: const<str> = "hello"
z: bool = true
```

> Yidika n'a pas de `let`/`var` — la présence du `:` distingue déclaration d'assignation. `const` est un modificateur de type, pas un mot-clé de déclaration.

## Types primitifs

| Concept | TypeScript | Yidika |
|---------|-----------|--------|
| Entier | `number` | `int` (ou `int8`, `int16`, `int32`, `int64`) |
| Entier rangeé | — | `rint`, `rint32` |
| Flottant | `number` | `real` (ou `real16`, `real32`, `real64`) |
| Booléen | `boolean` | `bool` |
| Chaîne | `string` | `str` |
| Symbole | — | `symbol` |
| Complexe | — | `complex` |

## Types composés

```ts
// TypeScript
let list: number[] = [1, 2, 3];
let pair: [number, string] = [42, "hello"];
let dict: Record<string, number> = { a: 1 };
```

```yidika
// Yidika
list: list<int> = [1, 2, 3]
pair: (int, str) = (42, "hello")
dict: map<str, int> = { a: 1 }
set: set<int> = set { 1, 2, 3 }
```

> Yidika utilise `list<T>` au lieu de `T[]`. `set { ... }` nécessite le préfixe `set` pour distinguer des maps/blocs.

## Nullable / Optionnel

```ts
// TypeScript
let x: number | null = null;
let y = x ?? 42;
let z = obj?.field;
```

```yidika
// Yidika
x: int? = null
y: int = x ?: 42
z: int? = obj?.field
```

| Concept | TS | Yidika |
|---------|-----|--------|
| Type nullable | `T \| null` | `T?` |
| Elvis / null-coalescing | `??` | `?:` |
| Safe-call | `?.` | `?.` |

## Fonctions

```ts
// TypeScript
function add(x: number, y: number): number {
    return x + y;
}
const square = (x: number): number => x * x;
```

```yidika
// Yidika
fn add(x: int, y: int) -> int {
    return x + y
}
square = fn(x: int) -> int { x * x }
square = (x: int) => x * x
```

> Yidika utilise `fn` au lieu de `function`. Les closures s'écrivent `(x) => expr` (pas de `|x|` pipe syntax).

## Conditionnelles (if)

```ts
// TypeScript
if (x > 5) {
    console.log("yes");
} else if (x > 0) {
    console.log("maybe");
} else {
    console.log("no");
}

// Expression ternaire
const result = x > 0 ? "pos" : "neg";
```

```yidika
// Yidika
if (x > 5) {
    print("yes")
} else if (x > 0) {
    print("maybe")
} else {
    print("no")
}

// Expression if
result = if (x > 0) "pos" else "neg"
// Ou ternaire
result = x > 0 ? "pos" : "neg"
```

> Les parenthèses autour de la condition sont obligatoires dans les deux langages. Yidika propose `if` expression ET ternaire.

## Boucles

```ts
// TypeScript
for (let i = 0; i < 3; i++) { console.log(i); }
for (const x of items) { console.log(x); }
while (i < 3) { console.log(i); i++; }
```

```yidika
// Yidika
for (i: int = 0; i < 3; i = i + 1) { print(i) }
for (x: items) { print(x) }           // for-of
for (x in items) { print(x) }         // for-in
while (i < 3) { print(i); i = i + 1 }
loop { print("infinite") }
infiny { print("infinite aussi") }
```

> Yidika supporte les trois formes de `for` (C-style, for-of, for-in). Pas de `do..while`, mais `loop`/`infiny` pour l'infini.

## Classes

```ts
// TypeScript
class Point {
    constructor(public x: number, public y: number) {}
    area(): number { return this.x * this.y; }
}
const p = new Point(10, 20);
```

```yidika
// Yidika
class Point(x: int, y: int) {
    fn area(&self) -> int { return self.x * self.y }
    init { }
}
p = Point { x: 10, y: 20 }
p = Point(10, 20)
```

> En Yidika, les paramètres du constructeur principal deviennent automatiquement des champs. Pas de `new` — appel direct ou littéral structuré.

## Héritage

```ts
// TypeScript
interface Drawable { draw(): void; }
class Circle extends Shape implements Drawable {
    draw() { console.log("circle"); }
}
```

```yidika
// Yidika
interface Drawable {
    fn draw(&self) -> str
}
class Circle : Shape use Drawable {
    override fn draw(&self) -> str { return "circle" }
}
```

> Yidika utilise `:` pour l'héritage et `use` pour les interfaces (au lieu de `extends`/`implements`).

## Match / Pattern matching

```ts
// TypeScript (switch)
switch (x) {
    case 1: return "one";
    case 2: return "two";
    default: return "other";
}
```

```yidika
// Yidika (match expression)
result = match x {
    1 => "one",
    2 => "two",
    _ => "other",
}
```

> Le `match` de Yidika est une expression (retourne une valeur, pas de `break` nécessaire). Patterns riches : literaux, variants, listes, guards.

## Unions / Enums

```ts
// TypeScript
type Value = number | string;
enum Color { Red, Green, Blue }
```

```yidika
// Yidika
x: int | str = 42     // union type

union Value {
    int_val: int
    str_val: str
}

enum Color {
    Red
    Green
    Blue
}

c = Color::Red
```

## Gestion d'erreurs

```ts
// TypeScript
try {
    const result = riskyOperation();
} catch (err) {
    console.error(err);
}
```

```yidika
// Yidika
result = try {
    risky_operation()
} catch (err) {
    print(err)
}
```

> Avec l'opérateur `?` pour propagation :
```ts
// TS
function foo(): number { const x = risky(); if (x instanceof Error) throw x; return x; }
```

```yidika
// Yidika — try operator
fn foo() -> int {
    x: int = risky()?  // propage Error automatiquement
    return x
}
```

## Async / Concurrent

```ts
// TypeScript
async function fetch(): Promise<string> {
    return await fetchUrl();
}
```

```yidika
// Yidika
async fn fetch() -> str {
    return await fetch_url()
}

// spawn explicite
task = spawn slow_function()
result = await task
```

> Yidika sépare `spawn` (lancer) de `await` (attendre) — deux opérateurs distincts.

## Strings

```ts
// TypeScript
let s = "hello";
let t = `value is ${x}`;
let raw = String.raw`no \n escape`;
```

```yidika
// Yidika
s = "hello"
t = f"value is {x}"
raw = `no \n escape`   // backtick = raw string
```

## Tuples

```ts
// TypeScript
let t: [number, string] = [42, "hello"];
t[0]  // 42
t[1]  // "hello"
```

```yidika
// Yidika
t: (int, str) = (42, "hello")
t.0   // accès par .index
t.1
```

> Yidika accède aux tuples avec `.0`, `.1` (comme Rust), pas `[0]`, `[1]`.

## Listes (Arrays)

```ts
// TypeScript
let items = [1, 2, 3];
items.push(4);
items.sort();
items.reverse();
```

```yidika
// Yidika
items: list<int> = [1, 2, 3]
items.push(4)
items.sort()
items.reverse()
items.remove(1)
items.clear()
items.len()    // ou items.length
```

## Modules / Imports

```ts
// TypeScript
import { sqrt, sin } from "math";
import json from "./util";
```

```yidika
// Yidika
use { sqrt, sin } from "math"
use json from "./util"
use PI as const from "math"
use { sqrt as square_root } from "math"
```

> Yidika utilise `use ... from` (pas `import`). Les imports peuvent être constants avec `as const`. FFI C/C++ via préfixe `c:` ou `cpp:`.

## Génériques

```ts
// TypeScript
function identity<T>(x: T): T { return x; }
class Box<T> { val: T; }
```

```yidika
// Yidika
fn identity<T>(x: T) -> T { return x }
struct Box<T> { val: T }
```

> Même syntaxe avec `<>`. Fonctions, structs, classes, interfaces supportent les génériques.

## Opérateurs

| Opération | TypeScript | Yidika |
|-----------|-----------|--------|
| Addition | `+` | `+` |
| Soustraction | `-` | `-` |
| Multiplication | `*` | `*` |
| Division | `/` | `/` |
| Modulo | `%` | `%` |
| Puissance | `**` | `**` |
| ET bitwise | `&` | `&` |
| OU bitwise | `\|` | `\|` |
| XOR bitwise | `^` | `^` |
| NON bitwise | `~` | `~` |
| Décalage gauche | `<<` | `<<` |
| Décalage droit | `>>` | `>>` |
| ET logique | `&&` | `&&` |
| OU logique | `\|\|` | `\|\|` |
| Incrément | `++` | `++` |
| Décrément | `--` | `--` |
| Compound | `+= -= *= /=` | `+= -= *= /= %= **= \|= &= ^= <<= >>=` |

Tous les opérateurs sont identiques entre TS et Yidika.

## Différences clés

| Feature | TypeScript | Yidika |
|---------|-----------|--------|
| Déclaration | `let`, `const`, `var` | `:` (colon) |
| Fonction | `function`, `=>` | `fn`, `=>` |
| Héritage | `extends` | `:` |
| Interface | `implements` | `use` |
| Nullable | `T \| null` | `T?` |
| Elvis | `??` | `?:` |
| Tuples | `t[0]` | `t.0` |
| Raw string | `String.raw\`...\`` | `` `...` `` |
| F-string | `` `...${x}...` `` | `f"...{x}..."` |
| Async | `async/await` | `async/await` + `spawn` |
| Match | `switch` (stmt) | `match` (expr) |
| Import | `import ... from` | `use ... from` |
| Semicolons | Requis | Optionnels |
| Commentaires | `//` `/* */` | `//` `/* */` |
