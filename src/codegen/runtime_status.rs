use crate::http_status::{MAX_STATUS, MIN_STATUS, REASONS, UNKNOWN_REASON};

pub fn c_definitions() -> String {
    format!("{}\n{}\n", c_reason_function(), c_valid_status_function())
}

fn c_reason_function() -> String {
    let cases: String = REASONS
        .iter()
        .map(|(code, phrase)| format!("    case {}: return \"{}\";\n", code, phrase))
        .collect();
    format!(
        "static const char* yk_status_reason(int code) {{\n    switch (code) {{\n{}    default: return \"{}\";\n    }}\n}}\n",
        cases, UNKNOWN_REASON
    )
}

fn c_valid_status_function() -> String {
    format!(
        "static int yk_valid_status(int code) {{\n    return (code >= {} && code <= {}) ? code : 500;\n}}\n",
        MIN_STATUS, MAX_STATUS
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_becomes_a_c_case() {
        let c = c_definitions();
        for (code, phrase) in REASONS {
            assert!(c.contains(&format!("case {}: return \"{}\";", code, phrase)), "{code}");
        }
        assert!(c.contains(&format!("default: return \"{}\";", UNKNOWN_REASON)));
    }

    #[test]
    fn invalid_codes_map_to_500_in_c() {
        let c = c_definitions();
        assert!(c.contains("code >= 100 && code <= 599"));
        assert!(c.contains(": 500;"));
    }

    #[test]
    fn reasons_need_no_c_escaping() {
        for (_, phrase) in REASONS {
            assert!(!phrase.contains('"') && !phrase.contains('\\'), "{phrase}");
        }
    }
}
