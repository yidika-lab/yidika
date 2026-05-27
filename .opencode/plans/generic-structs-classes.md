# Generic Structs & Classes — Implementation Plan

## Goal
Support `struct Pair<T, U> { first: T; second: U; }` and `class Box<T> { value: T; fn get(self) -> T { ... } }` with type-checked instantiations.

## 1. Type Checker — `src/semantic/typeck.rs`

### 1a. New structs (after `GenericFnDef`, line ~237)

```rust
struct GenericStructDef { fields: Vec<Param>, generics: Vec<String> }
struct GenericClassDef { fields: Vec<Param>, methods: Vec<ItemKind>, generics: Vec<String> }
```

### 1b. `mangle_struct_name(name, &[TypeExpr]) -> String`
Safe LLVM name: `Pair` + `int` + `str` → `Pair-int-str`, nested: `List-int` etc.
Replace non-alphanumeric chars with nothing.

### 1c. Add fields to `TypeChecker` struct
```rust
generic_structs: HashMap<String, GenericStructDef>,
generic_classs: HashMap<String, GenericClassDef>,
struct_fields: HashMap<String, Vec<(String, TypeExpr)>>, // concrete field types per mangled name
```
Remove `#[allow(dead_code)]` from `GenericFnDef.body`.

### 1d. `check_item` for `ItemKind::Struct`
- If `generics` empty: store field types resolved in `struct_fields[name]`, register name in `self.types` (current behavior)
- If `generics` non-empty: store in `generic_structs[name]`, DO NOT register in `self.types`

### 1e. `check_item` for `ItemKind::Class`
- Currently unhandled (falls through to `_ => Ok(())`)
- Same logic: if generics empty → register class, if non-empty → store in `generic_classs`

### 1f. `resolve_type` — handle `TypeExpr::Generic`
When `Pair<int, str>` is used as a type annotation:
1. Look up `"Pair"` in `generic_structs` (or `generic_classs`)
2. Create mangled name: `mangle_struct_name("Pair", &[Int(0), Str])` → `"Pair-int-str"`
3. Substitute type params in field types → concrete field types
4. Store in `struct_fields["Pair-int-str"]`
5. Register in `self.types["Pair-int-str"] = Named("Pair-int-str")`
6. Return `Named("Pair-int-str")`

```rust
TypeExpr::Generic(name, args) => {
    if let Some(gdef) = self.generic_structs.get(name) {
        let mangled = mangle_struct_name(name, args);
        if !self.struct_fields.contains_key(&mangled) {
            let mut type_args = HashMap::new();
            for (gp, arg) in gdef.generics.iter().zip(args.iter()) {
                type_args.insert(gp.clone(), arg.clone());
            }
            let concrete_fields: Vec<(String, TypeExpr)> = gdef.fields.iter().map(|p| {
                (p.name.clone(), self.resolve_type(&substitute_type(&p.type_expr.value, &type_args)))
            }).collect();
            self.struct_fields.insert(mangled.clone(), concrete_fields);
            self.types.insert(mangled.clone(), TypeExpr::Named(mangled.clone()));
        }
        TypeExpr::Named(mangled)
    } else if let Some(gdef) = self.generic_classs.get(name) {
        // Same pattern for classes
        // ...
    } else {
        other.clone()
    }
}
```

### 1g. `Expr::StructLit` handler in `check_expr` (replace catch-all `_ => Infer`)
```rust
Expr::StructLit(sname, fields) => {
    // Check concrete (non-generic) struct first
    if let Some(def_fields) = self.struct_fields.get(sname).cloned() {
        for (fname, fexpr) in fields {
            let val_ty = self.check_expr(fexpr)?;
            if let Some((_, fty)) = def_fields.iter().find(|(n, _)| n == fname) {
                if !types_compatible(fty, &val_ty) {
                    return self.fail(...);
                }
            } else {
                return self.fail(format!("Unknown field '{}' in struct '{}'", fname, sname));
            }
        }
        Ok(TypeExpr::Named(sname.clone()))
    }
    // Check generic struct — infer type params from field values
    else if let Some(gdef) = self.generic_structs.get(sname) {
        let mut type_args = HashMap::new();
        for (fname, fexpr) in fields {
            if let Some(p) = gdef.fields.iter().find(|p| p.name == *fname) {
                let val_ty = self.check_expr(fexpr)?;
                self.infer_from(&p.type_expr.value, &val_ty, &gdef.generics, &mut type_args, expr.span)?;
            } else {
                return self.fail(format!("Unknown field '{}' in struct '{}'", fname, sname));
            }
        }
        for gp in &gdef.generics {
            if !type_args.contains_key(gp) {
                return self.fail(format!("Could not infer type parameter '{}' for struct '{}'", gp, sname));
            }
        }
        let concrete_args: Vec<TypeExpr> = gdef.generics.iter().map(|g| type_args.get(g).unwrap().clone()).collect();
        let mangled = mangle_struct_name(sname, &concrete_args);
        if !self.struct_fields.contains_key(&mangled) {
            let field_types: Vec<(String, TypeExpr)> = gdef.fields.iter().map(|p| {
                (p.name.clone(), substitute_type(&p.type_expr.value, &type_args))
            }).collect();
            self.struct_fields.insert(mangled.clone(), field_types);
            self.types.insert(mangled.clone(), TypeExpr::Named(mangled.clone()));
        }
        Ok(TypeExpr::Named(mangled))
    } else {
        Ok(TypeExpr::Infer)
    }
}
```

### 1h. `Expr::Field(obj, field)` — return correct type
Currently the field access falls through to `_ => Ok(TypeExpr::Infer)` too. Need to add a handler that:
1. Dermine `obj` type
2. If `Named("Pair-int-str")` or similar, strip mangled name
3. Look up in `struct_fields` to find the field type
4. Return it

```rust
Expr::Field(obj, field) => {
    let obj_ty = self.check_expr(obj)?;
    match &obj_ty {
        TypeExpr::Named(name) => {
            if let Some(def_fields) = self.struct_fields.get(name) {
                if let Some((_, fty)) = def_fields.iter().find(|(n, _)| n == field) {
                    return Ok(fty.clone());
                }
            }
        }
        _ => {}
    }
    Ok(TypeExpr::Infer)
}
```

### 1i. Generic class methods monomorphization
When a generic class method is called, same pattern as generic fn:
1. Infer class type params from `self` type or type annotation
2. Instantiate method signatures: substitute type params in params + ret_type
3. Type-check method body with concrete param types

## 2. Parser — `src/syntax/parser.rs`
No changes needed. Already parses:
- `struct Pair<T, U> { ... }` → `ItemKind::Struct { generics: ["T", "U"] }`
- `Pair<int, str>` in type context → `TypeExpr::Generic`
- `Pair { first: 42, ... }` → `Expr::StructLit("Pair", ...)`

## 3. Codegen — `src/codegen/llvm.rs`

### 3a. Add `generic_struct_defs: HashMap<String, Vec<Param>>` and `generic_class_defs` to `LlvmCodegen`

### 3b. `compile_module`: for structs with generics, store in `generic_struct_defs` instead of emitting

### 3c. `type_to_llvm`: handle `TypeExpr::Generic` for struct/class names (same mangling + substitution)

### 3d. Struct literal compilation: when name not in `struct_defs`, check `generic_struct_defs`
- Infer type params from field expressions (same logic as type checker)
- Create mangled name
- Emit `%struct.MANGLED = type { ... }`
- Store in `struct_defs` for later lookups

### 3e. Field access: handle mangled names (strip `%struct.` prefix works regardless)

## 4. Tests — `src/lib.rs`

```rust
#[test]
fn generic_struct() {
    check("struct Pair<T, U> { first: T; second: U; }
           fn main() { p: Pair<int, str> = Pair { first: 42, second: \"hi\" }; }").unwrap();
    check("struct Pair<T, U> { first: T; second: U; }
           fn main() { p: Pair<str, bool> = Pair { first: \"a\", second: true }; }").unwrap();
}

#[test]
fn generic_struct_field_access() {
    check("struct Pair<T, U> { first: T; second: U; }
           fn main() { p: Pair<int, str> = Pair { first: 42, second: \"hi\" }; print(p.first); }").unwrap();
}

#[test]
fn generic_struct_infer() {
    check("struct Pair<T, U> { first: T; second: U; }
           fn main() { p = Pair { first: 42, second: \"hi\" }; }").unwrap();
}

#[test]
fn generic_struct_wrong_field_type() {
    assert!(check("struct Pair<T, U> { first: T; second: U; }
                    fn main() { p: Pair<int, str> = Pair { first: \"bad\", second: \"hi\" }; }").is_err());
}

#[test]
fn generic_struct_unknown_field() {
    assert!(check("struct Pair<T, U> { first: T; second: U; }
                    fn main() { p: Pair<int, str> = Pair { first: 42, second: \"hi\", third: 1 }; }").is_err());
}
```

## 5. Order of Implementation
1. Type checker: GenericStructDef + struct_fields + mangle (1a-1d)
2. Type checker: resolve_type for Generic (1f)
3. Type checker: StructLit handler (1g)
4. Type checker: Field handler (1h)
5. Type checker: Generic classes (1e, 1i)
6. Codegen: generic struct def storage + lazy instantiation (3a-3e)
7. Tests (4)
8. All tests pass (123+)
