use crate::trap::Trap;

pub fn c_definitions() -> String {
    let messages: Vec<String> = Trap::ALL.iter().map(|e| format!("\"{}\"", e.message())).collect();
    let codes: String = Trap::ALL.iter().map(|e| format!("#define YK_TRAP_{} {}\n", c_name(*e), e.code())).collect();
    format!("{}#define YK_TRAP_COUNT {}\nstatic const char* const yk_trap_messages[] = {{ {} }};\n", codes, Trap::ALL.len(), messages.join(", "))
}

fn c_name(error: Trap) -> String {
    let mut name = String::new();
    for (i, c) in format!("{:?}", error).chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            name.push('_');
        }
        name.push(c.to_ascii_uppercase());
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_trap_gets_a_c_code_and_message() {
        let c = c_definitions();
        assert!(c.contains("#define YK_TRAP_DIVISION_BY_ZERO 0"));
        assert!(c.contains("#define YK_TRAP_NEGATIVE_EXPONENT 2"));
        assert!(c.contains(&format!("#define YK_TRAP_COUNT {}", Trap::ALL.len())));
        for e in Trap::ALL {
            assert!(c.contains(&format!("\"{}\"", e.message())), "{:?}", e);
        }
    }
}
