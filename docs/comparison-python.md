# Python vs Yidika — Syntax Comparison

## Variables

```python
# Python
x: int = 5
y = "hello"
z: Final = True
```

```yidika
# Yidika
x: int = 5
y = "hello"
z: const<bool> = true
```

> Yidika sépare déclaration (`x: int = 5`) et assignation (`x = 10`). Pas de mot-clé `let`/`var` — le `:` fait la différence.

## Types primitifs

| Concept | Python | Yidika |
|---------|--------|--------|
| Entier | `int` | `int` (ou `int8`, `int16`, `int32`, `int64`) |
| Entier rangeé | — | `rint`, `rint32` |
| Flottant | `float` | `real` (ou `real16`, `real32`, `real64`) |
| Booléen | `bool` | `bool` |
| Chaîne | `str` | `str` |
| Complexe | `complex` | `complex` |
| Symbole | — | `symbol` |
| Aucun | `None` | `null` / `None` |

## Types composés

```python
# Python
items: list[int] = [1, 2, 3]
pair: tuple[int, str] = (42, "hello")
d: dict[str, int] = {"a": 1}
s: set[int] = {1, 2, 3}
```

```yidika
# Yidika
items: list<int> = [1, 2, 3]
pair: (int, str) = (42, "hello")
d: map<str, int> = { a: 1 }
s: set<int> = set { 1, 2, 3 }
```

> Yidika utilise `map<K,V>` plutôt que `dict`. Les sets nécessitent le préfixe `set` devant `{ }`.

## Nullable / Optionnel

```python
# Python
x: int | None = None
y = x if x is not None else 42
z = obj.field if obj is not None else None
```

```yidika
# Yidika
x: int? = null
y = x ?: 42
z = obj?.field
```

| Concept | Python | Yidika |
|---------|--------|--------|
| Type nullable | `T \| None` | `T?` |
| Elvis / coalescing | `x if x is not None else 42` | `x ?: 42` |
| Safe-call | `obj.field if obj else None` | `obj?.field` |

## Fonctions

```python
# Python
def add(x: int, y: int) -> int:
    return x + y

square = lambda x: x * x
```

```yidika
# Yidika
fn add(x: int, y: int) -> int {
    return x + y
}

square = fn(x: int) -> int { x * x }
square = (x: int) => x * x
```

> Python utilise `def` et `lambda`, Yidika utilise `fn` et `=>`. Les blocs Yidika sont `{ }`, pas l'indentation.

## Conditionnelles (if)

```python
# Python
if x > 5:
    print("yes")
elif x > 0:
    print("maybe")
else:
    print("no")

# Expression ternaire
result = "pos" if x > 0 else "neg"
```

```yidika
# Yidika
if (x > 5) {
    print("yes")
} else if (x > 0) {
    print("maybe")
} else {
    print("no")
}

# Expression if
result = if (x > 0) "pos" else "neg"
# Ou ternaire
result = x > 0 ? "pos" : "neg"
```

> Python utilise l'indentation et `elif`. Yidika utilise `{ }`, `else if`, et `? :` comme ternaire. Les parenthèses autour de la condition sont obligatoires en Yidika.

## Boucles

```python
# Python
for i in range(3):
    print(i)
for x in items:
    print(x)
while i < 3:
    print(i)
    i += 1
```

```yidika
# Yidika
for (i: int = 0; i < 3; i = i + 1) { print(i) }
for (x: items) { print(x) }        # for-of
for (x in items) { print(x) }      # for-in
while (i < 3) { print(i); i = i + 1 }
loop { print("infini") }
```

> Yidika supporte le C-style `for (init; cond; inc)`, pas de `for i in range()` — les ranges sont `0..3`. L'indentation Python est remplacée par `{ }`.

## Classes

```python
# Python
@dataclass
class Point:
    x: int
    y: int
    def area(self) -> int:
        return self.x * self.y

p = Point(10, 20)
```

```yidika
# Yidika
class Point(x: int, y: int) {
    fn area(&self) -> int { return self.x * self.y }
    init { }
}

p = Point { x: 10, y: 20 }
p = Point(10, 20)
```

> En Yidika, les paramètres du constructeur principal deviennent automatiquement des champs — similaire à `@dataclass` mais natif. Self s'écrit `&self` pour la référence.

## Héritage

```python
# Python
class Animal:
    def speak(self): ...

class Dog(Animal):
    def speak(self):
        return "woof"
```

```yidika
# Yidika
interface Drawable {
    fn draw(&self) -> str
}

class Dog : Animal use Drawable {
    override fn draw(&self) -> str { return "woof" }
}
```

> Python: parenthèses pour l'héritage. Yidika: `:` pour la classe parente, `use` pour les interfaces.

## Match / Pattern matching

```python
# Python (3.10+)
match x:
    case 1: return "one"
    case 2: return "two"
    case _: return "other"
```

```yidika
# Yidika
result = match x {
    1 => "one",
    2 => "two",
    _ => "other",
}
```

> Python: `case pattern:`, Yidika: `pattern => expr`. Le match Yidika est toujours une expression (retourne une valeur). Les deux supportent guards (`if condition` après le pattern).

## Unions / Enums

```python
# Python
from typing import Union
x: Union[int, str] = 42

from enum import Enum
class Color(Enum):
    RED = 1
    GREEN = 2
```

```yidika
# Yidika
x: int | str = 42       # union type directe

enum Color {
    Red
    Green
    Blue
}
```

> Yidika n'a pas besoin d'importer `Enum` — c'est natif.

## Gestion d'erreurs

```python
# Python
try:
    result = risky_operation()
except Exception as err:
    print(err)
```

```yidika
# Yidika
result = try {
    risky_operation()
} catch (err) {
    print(err)
}
```

> Avec l'opérateur `?` pour propagation :
```python
# Python
def foo() -> int:
    x = risky()
    if isinstance(x, Exception):
        raise x
    return x
```

```yidika
# Yidika — propagation automatique
fn foo() -> int {
    x: int = risky()?  // propage Error vers l'appelant
    return x
}
```

## Async / Concurrent

```python
# Python
async def fetch() -> str:
    return await fetch_url()

import asyncio
task = asyncio.create_task(slow())
result = await task
```

```yidika
# Yidika
async fn fetch() -> str {
    return await fetch_url()
}

task = spawn slow_function()
result = await task
```

> Yidika a `spawn` natif (pas besoin d'`asyncio`). Même sémantique async/await.

## Strings

```python
# Python
s = "hello"
t = f"value is {x}"
r = r"no \n escape"   # raw string
```

```yidika
# Yidika
s = "hello"
t = f"value is {x}"
raw = `no \n escape`  # raw string
```

> Python utilise `r"..."` pour raw, Yidika utilise `` `...` ``. Les f-strings utilisent la même syntaxe `f"...{expr}..."`.

## Tuples

```python
# Python
t = (42, "hello")
t[0]  # 42
t[1]  # "hello"
```

```yidika
# Yidika
t: (int, str) = (42, "hello")
t.0   # accès par .index
t.1
```

> Python: `t[0]`. Yidika: `t.0` (comme Rust).

## Listes (Arrays)

```python
# Python
items = [1, 2, 3]
items.append(4)
items.sort()
items.reverse()
items.pop(1)
items.clear()
len(items)
```

```yidika
# Yidika
items: list<int> = [1, 2, 3]
items.push(4)
items.sort()
items.reverse()
items.remove(1)
items.clear()
items.len()       # ou items.length
```

> Noms différents: `push` vs `append`, `remove` vs `pop`. `.length`/`.len()` vs `len()`.

## Modules / Imports

```python
# Python
from math import sqrt, sin
import json
from math import pi as PI
```

```yidika
# Yidika
use { sqrt, sin } from "math"
use json from "json"
use PI as const from "math"
use { sqrt as square_root } from "math"
```

> Yidika inverse l'ordre: `use ... from` au lieu de `from ... import`. Les imports constants utilisent `as const`.

## Génériques

```python
# Python (via typing)
from typing import TypeVar, Generic
T = TypeVar('T')
def identity(x: T) -> T: return x
class Box(Generic[T]):
    def __init__(self, val: T): ...
```

```yidika
# Yidika (natif)
fn identity<T>(x: T) -> T { return x }
struct Box<T> { val: T }
```

> En Yidika les génériques sont natifs et intégrés dans la syntaxe — pas besoin de `TypeVar` ou `Generic`.

## Comparaison des blocs et de l'indentation

```python
# Python — indentation significative
def foo(x):
    if x > 0:
        for i in range(x):
            print(i)
    else:
        print("negatif")
```

```yidika
# Yidika — accolades, pas d'indentation significative
fn foo(x: int) {
    if (x > 0) {
        for (i: int = 0; i < x; i = i + 1) {
            print(i)
        }
    } else {
        print("negatif")
    }
}
```

> La différence la plus fondamentale : Python utilise l'indentation comme syntaxe, Yidika utilise `{ }` comme C/JS.

## Opérateurs

| Opération | Python | Yidika |
|-----------|--------|--------|
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
| ET logique | `and` | `&&` |
| OU logique | `or` | `\|\|` |
| NON logique | `not` | `!` |
| Incrément | `+= 1` | `++` |
| Décrément | `-= 1` | `--` |
| Compound | `+= -= *=` | `+= -= *= /= %= **= \|= &= ^= <<= >>=` |

> Python utilise `and`/`or`/`not` (mots), Yidika utilise `&&`/`||`/`!` (symboles C-style). Python n'a pas de `++`/`--`.

## Différences clés

| Feature | Python | Yidika |
|---------|--------|--------|
| Blocs | Indentation | `{ }` |
| Déclaration | `x: int = 5` (type hint) | `x: int = 5` (obligatoire) |
| Fonction | `def` | `fn` |
| Lambda | `lambda x: expr` | `(x) => expr` |
| Héritage | `class Dog(Animal)` | `class Dog : Animal` |
| Interface | `Protocol` / ABC | `interface` + `use` |
| Nullable | `T \| None` | `T?` |
| Elvis | `x if x else 42` | `x ?: 42` |
| Safe-call | `getattr(x, 'f', None)` | `x?.f` |
| Tuples | `t[0]` | `t.0` |
| Raw string | `r"..."` | `` `...` `` |
| F-string | `f"...{x}..."` | `f"...{x}..."` |
| Async | `async/await` + `asyncio` | `async/await` + `spawn` natif |
| Match | `match/case` (stmt) | `match { => }` (expr) |
| Opérateurs logiques | `and` `or` `not` | `&&` `\|\|` `!` |
| Import | `from ... import` | `use ... from` |
| Semicolons | Non | Optionnels |
| Commentaires | `#` | `//` `/* */` |
| Boucle infinie | `while True` | `loop` / `infiny` |
| Ternary | `x if cond else y` | `cond ? x : y` ou `if (cond) x else y` |

## Correspondances de concepts

| Concept Python | Équivalent Yidika |
|----------------|-------------------|
| `None` | `null` |
| `len(x)` | `x.len()` ou `x.length` |
| `str(x)` | `str(x)` |
| `print(x)` | `print(x)` |
| `range(5)` | `0..4` (ou `0..5` pour inclusif) |
| `isinstance(x, T)` | `match` / `is` |
| `@dataclass` | `class(x: int)` (constructeur auto) |
| `TypeVar` | Génériques natifs `<>` |
| `set {1, 2}` | `set { 1, 2 }` |
| `dict(a=1)` | `{ a: 1 }` |
