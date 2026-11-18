#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trap {
    DivisionByZero,
    DivisionOverflow,
    NegativeExponent,
    PopFromEmptyList,
    StackOverflow,
}

impl Trap {
    pub const ALL: [Trap; 5] = [Trap::DivisionByZero, Trap::DivisionOverflow, Trap::NegativeExponent, Trap::PopFromEmptyList, Trap::StackOverflow];

    pub fn message(self) -> &'static str {
        match self {
            Trap::DivisionByZero => "division by zero",
            Trap::DivisionOverflow => "division overflow: i64::MIN / -1",
            Trap::NegativeExponent => "negative exponent in integer power",
            Trap::PopFromEmptyList => "pop from empty list",
            Trap::StackOverflow => "stack overflow: the recursion probably never ends",
        }
    }

    pub fn code(self) -> i64 {
        Trap::ALL.iter().position(|e| *e == self).unwrap_or(0) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_trap_has_a_distinct_code_and_a_plain_message() {
        for (i, t) in Trap::ALL.iter().enumerate() {
            assert_eq!(t.code(), i as i64);
            assert!(!t.message().contains('"') && !t.message().contains('\\'));
        }
    }
}
