# Structs, classes and interfaces

## Structs

A struct groups named fields. Build one with `Name { field: value }`.

```yidika
struct Point {
    x: int;
    y: int;
}

fn manhattan(p: Point) -> int {
    return p.x + p.y;
}

fn main() {
    p = Point { x: 3, y: 4 };
    print(p.x, p.y, manhattan(p));
}
```

```output
3 4 7
```

## Classes

A class has fields and methods. Methods take `self` (or `&self`) first.

```yidika
class Dog {
    name: str;
    fn speak(self) -> str {
        return self.name + " says woof";
    }
}

fn main() {
    d = Dog { name: "Rex" };
    print(d.speak());
}
```

```output
Rex says woof
```

Constructor parameters can be written after the class name, and an `init` block runs when the object is created:

```yidika
class Counter(start: int) {
    fn get(&self) -> int { return self.start; }
    init { print("counter created"); }
}

fn main() {
    c = Counter(99);
    print(c.get());
}
```

```output
counter created
99
```

## Inheritance

A class marked `open` can be extended with `: Parent`. A child inherits the parent's methods and can override them.

```yidika
open class Animal(name: str) {
    fn speak(self) -> str { return self.name + " makes a sound"; }
}

class Dog(name: str, age: int) : Animal {
    fn speak(self) -> str { return self.name + " barks"; }
}

class Cat(name: str) : Animal {
}

fn main() {
    print(Dog { name: "Rex", age: 3 }.speak());
    print(Cat { name: "Tom" }.speak());
}
```

```output
Rex barks
Tom makes a sound
```

## Generic classes

```yidika
class Box<T> {
    val: T;
    fn get(self) -> T { return self.val; }
}

fn main() {
    b: Box<int> = Box { val: 42 };
    print(b.get());
}
```

```output
42
```

## Interfaces

An interface lists methods; a class `implements` it, and any value of that class can be used where the interface is expected.

```yidika
interface Drawable {
    fn draw(&self) -> str;
}

class Circle implements Drawable {
    r: int;
    fn draw(&self) -> str { return "circle of radius " + str(self.r); }
}

fn render(d: Drawable) -> str {
    return "<" + d.draw() + ">";
}

fn main() {
    print(render(Circle { r: 2 }));
}
```

```output
<circle of radius 2>
```

## Objects

`object` declares a single instance, useful for a group of functions:

```yidika
object Log {
    fn info(msg: str) {
        print("[info] " + msg);
    }
}

fn main() {
    Log.info("started");
}
```

```output
[info] started
```
