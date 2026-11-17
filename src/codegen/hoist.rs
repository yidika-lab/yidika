pub fn allocas_to_entry(ir: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    let mut allocas: Vec<&str> = Vec::new();
    let mut in_fn = false;
    let mut last_alloca: Option<&str> = None;
    for line in ir.lines() {
        if !in_fn {
            out.push(line);
            in_fn = line.starts_with("define ") && line.trim_end().ends_with('{');
            continue;
        }
        if line == "}" {
            emit_function(&mut out, &allocas, &body);
            out.push(line);
            allocas.clear();
            body.clear();
            in_fn = false;
        } else if is_static_alloca(line) {
            allocas.push(line);
            last_alloca = alloca_name(line);
            continue;
        } else if last_alloca.is_some_and(|name| is_zero_store_to(line, name)) {
            allocas.push(line);
        } else {
            body.push(line);
        }
        last_alloca = None;
    }
    let mut text = out.join("\n");
    if ir.ends_with('\n') {
        text.push('\n');
    }
    text
}

fn emit_function<'a>(out: &mut Vec<&'a str>, allocas: &[&'a str], body: &[&'a str]) {
    let entry_label = body.iter().position(|l| !l.trim().is_empty()).filter(|&i| is_label(body[i]));
    let split = entry_label.map(|i| i + 1).unwrap_or(0);
    out.extend_from_slice(&body[..split]);
    out.extend_from_slice(allocas);
    out.extend_from_slice(&body[split..]);
}

fn is_label(line: &str) -> bool {
    let t = line.trim();
    t.ends_with(':') && !t.contains(' ')
}

fn alloca_name(line: &str) -> Option<&str> {
    line.trim().split_once(" = alloca ").map(|(name, _)| name)
}

fn is_zero_store_to(line: &str, name: &str) -> bool {
    let t = line.trim();
    t.starts_with("store ") && t.contains(" zeroinitializer, ptr ") && t.ends_with(&format!("ptr {}", name))
}

fn is_static_alloca(line: &str) -> bool {
    let t = line.trim();
    match t.split_once(" = alloca ") {
        Some((name, rest)) => name.starts_with('%') && !name.contains(' ') && !rest.contains(", i64 %") && !rest.contains(", i32 %"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alloca_inside_a_loop_moves_to_the_entry_block() {
        let ir = "define i32 @main() {\nentry:\n  br label %loop\nloop:\n  %s = alloca %yk_string, align 8\n  br label %loop\n}\n";
        let hoisted = allocas_to_entry(ir);
        assert_eq!(hoisted, "define i32 @main() {\nentry:\n  %s = alloca %yk_string, align 8\n  br label %loop\nloop:\n  br label %loop\n}\n");
    }

    #[test]
    fn a_function_without_an_entry_label_gets_its_allocas_first() {
        let ir = "define void @f() {\n  call void @g()\n  %x = alloca i64, align 8\n  ret void\n}";
        assert_eq!(allocas_to_entry(ir), "define void @f() {\n  %x = alloca i64, align 8\n  call void @g()\n  ret void\n}");
    }

    #[test]
    fn the_zero_store_right_after_an_alloca_moves_with_it() {
        let ir = "define void @f() {\nentry:\n  br label %l\nl:\n  %s = alloca %yk_string, align 8\n  store %yk_string zeroinitializer, ptr %s\n  store %yk_string zeroinitializer, ptr %s\n  br label %l\n}";
        assert_eq!(allocas_to_entry(ir), "define void @f() {\nentry:\n  %s = alloca %yk_string, align 8\n  store %yk_string zeroinitializer, ptr %s\n  br label %l\nl:\n  store %yk_string zeroinitializer, ptr %s\n  br label %l\n}");
    }

    #[test]
    fn a_dynamically_sized_alloca_stays_where_it_is() {
        let ir = "define void @f(i64 %n) {\nentry:\n  call void @g()\n  %buf = alloca i8, i64 %n, align 8\n  ret void\n}";
        assert_eq!(allocas_to_entry(ir), ir);
    }

    #[test]
    fn code_outside_functions_is_unchanged() {
        let ir = "%yk_string = type { ptr, i64, i64 }\ndeclare void @g()\n@s = private constant [2 x i8] c\"a\\00\"";
        assert_eq!(allocas_to_entry(ir), ir);
    }
}
