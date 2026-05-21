# Yidika — Specification MVP

## 1. Syntaxe générale

### Déclarations de variables
```rust
name: Type = value;     // mutable (par défaut)
const name: Type = value; // immutable
```

### Fonctions
```rust
fn name(arg1: Type1, arg2: Type2) -> ReturnType {
    body
}
```

### Blocs = expressions
```rust
let x = if (cond) { 1 } else { 2 };
```

---

## 2. Types

### Primitifs
| Syntaxe | Description |
|---------|-------------|
| `int` | Entier non-signé, précision arbitraire (bigint) |
| `int8`, `int16`, `int32`, `int64` | Entier non-signé taille fixe |
| `rint` | Entier signé, précision arbitraire |
| `rint8`, `rint16`, `rint32`, `rint64` | Entier signé taille fixe |
| `real` | Nombre réel natif (float64) |
| `real16`, `real32`, `real64` | Flottants taille fixe |
| `complex` | Nombre complexe |
| `bool` | `true` ou `false` |
| `str` | Chaîne de caractères immuable (UTF-8) |
| `symbol` | Symbole léger (`@foo`, `"bar"`) |
| `null` | Zéro mémoire (valeur binaire 0) |
| `None` | Absence de valeur |

### Vecteurs
```julia
v: vector = (x:y);     // vecteur 2D
v3: vector = (x:y:z);  // vecteur 3D
```

### Matrices
```julia
m: matrix = ([x:y], [x:y]);
```

### Listes
```ts
list: [Type];
list: [Type] = [val1, val2];
list: Type[] = [val1, val2];
```

### Sets
```ts
set: {Type} = {val1, val2};
```

### Maps
```ts
map: {KeyType: ValueType} = {key1: val1};
```

### Structs
```rust
struct Person {
    name: str;
    age: int8;
}
```

### Classes (heap + héritage)
```kotlin
class Animal {
    fn init(name: str) {
        this.name = name;
    }
}

class Dog: Animal {
    fn init(name: str) {
        super.init(name);
    }
}
```

### Interfaces (contrat sans état)
```kotlin
interface Speaker {
    fn speak() -> str;
}

class Person: Speaker {
    fn speak() -> str {
        return "Hello";
    }
}
```

### Unions
```c
union Value {
    i: int;
    f: real32;
}
```

### Type alias
```ts
type User = int | str;
type Status = "active" | "inactive";
```

### Génériques
```rust
fn identity<T>(x: T) -> T {
    return x;
}

struct Box<T> {
    value: T;
}
```

---

## 3. Ownership et mémoire

### Règles
- **Passage de paramètres** : implicite par référence (style Zig). `fn foo(x: BigStruct)` ne copie pas.
- **Assignation `b = a`** : `move` par défaut (style Rust). `a` devient invalide.
- **Copy trait** : les types primitifs (`int`, `bool`, `real`, etc.) sont copiés, pas déplacés.
- **Mutabilité** : les paramètres sont mutables par défaut. `const` pour verrouiller.

### Régions mémoire
- **Stack** : variables locales, structs de petite taille
- **Arena** (heap optimisé) : listes, maps, sets (allocation contiguë)
- **Device** (GPU/NPU) : mémoire dédiée, sync explicite avec `device.sync(data)`
- **Static** (`const`) : données immuables dans le segment `.data`

---

## 4. Contrôle de flux

### Conditionnel
```kotlin
if (bool) { }
if (bool) { } else { }
if (bool) { } else if (bool) { } else { }
```

### Boucles
```kotlin
for (i in 0..10) { }     // range inclus
for (el in iterable) { } // itération

while (cond) { }
loop { }                 // boucle infinie
```

### Ranges
- `0..10` : de 0 à 10 inclus
- `10..0` : décroissant
- `"a".."z"` : alphabétique

---

## 5. Modules

```ts
use x from "path";
use y as alias from "path";
use {x, y} from "path";

export fn name() {}
export { name1, name2 };
```

---

## 6. Gestion d'erreurs

Modèle `Result<T, E>` (style Rust) :
```rust
fn div(a: int, b: int) -> Result<int, str> {
    if (b == 0) { return Error("division by zero"); }
    return Ok(a / b);
}

let result = div(10, 0)?;  // propagation avec ?
```

---

## 7. Async / Multithreading

```rust
async fn fetch(url: str) -> str {
    let data = await http.get(url);
    return data;
}

let task = spawn fetch("https://example.com");
let result = await task;
```

---

## 8. FFI

```ts
use {gpu_process} from "c++:./libs/engine.hpp";
use {socket_raw} from "c:./include/net.h";
use {engine_core} from "rust:./src/lib.rs";
```

---

## 9. Attributs / Décorateurs

```python
@use(...)       // comme décorateur Python
@align(64)      // alignement mémoire
```

---

## 10. UI Templating (embedded)

```html
<Component name="World">
    <Text>Hello, {name}</Text>
</Component>
```

---

## 11. Destructuration

```ts
let {x, y} = {x: 10, y: 20};
let [a, b] = [1, 2];
```

---

## 12. Opérateurs (priorité haute → basse)

| Priorité | Opérateurs |
|----------|------------|
| 1 | `(...)` groupement |
| 2 | `.` membre, `[]` index, `()` appel |
| 3 | `!` `++` `--` (unaires) |
| 4 | `*` `/` |
| 5 | `+` `-` |
| 6 | `==` `!=` `<` `>` `<=` `>=` |
| 7 | `=` `:=` assignation |
