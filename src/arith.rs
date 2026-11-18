use crate::trap::Trap;

pub fn div(a: i64, b: i64) -> Result<i64, Trap> {
    if b == 0 {
        return Err(Trap::DivisionByZero);
    }
    a.checked_div(b).ok_or(Trap::DivisionOverflow)
}

pub fn rem(a: i64, b: i64) -> Result<i64, Trap> {
    if b == 0 {
        return Err(Trap::DivisionByZero);
    }
    a.checked_rem(b).ok_or(Trap::DivisionOverflow)
}

pub fn pow(base: i64, exponent: i64) -> Result<i64, Trap> {
    if exponent < 0 {
        return Err(Trap::NegativeExponent);
    }
    let (mut result, mut factor, mut remaining) = (1i64, base, exponent as u64);
    while remaining > 0 {
        if remaining & 1 == 1 {
            result = result.wrapping_mul(factor);
        }
        factor = factor.wrapping_mul(factor);
        remaining >>= 1;
    }
    Ok(result)
}

pub fn shl(a: i64, amount: i64) -> i64 {
    a.wrapping_shl(amount as u32)
}

pub fn shr(a: i64, amount: i64) -> i64 {
    a.wrapping_shr(amount as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn division_by_zero_is_an_error() {
        assert_eq!(div(1, 0), Err(Trap::DivisionByZero));
        assert_eq!(rem(1, 0), Err(Trap::DivisionByZero));
    }

    #[test]
    fn min_divided_by_minus_one_is_an_error() {
        assert_eq!(div(i64::MIN, -1), Err(Trap::DivisionOverflow));
        assert_eq!(rem(i64::MIN, -1), Err(Trap::DivisionOverflow));
    }

    #[test]
    fn division_truncates_towards_zero() {
        assert_eq!(div(-7, 2), Ok(-3));
        assert_eq!(rem(-7, 2), Ok(-1));
    }

    #[test]
    fn power_wraps_and_rejects_negative_exponents() {
        assert_eq!(pow(3, 4), Ok(81));
        assert_eq!(pow(2, 63), Ok(i64::MIN));
        assert_eq!(pow(3, 41), Ok(3i64.wrapping_pow(41)));
        assert_eq!(pow(0, 0), Ok(1));
        assert_eq!(pow(2, -1), Err(Trap::NegativeExponent));
    }

    #[test]
    fn a_huge_exponent_takes_logarithmic_time() {
        assert_eq!(pow(1, i64::MAX), Ok(1));
        assert_eq!(pow(-1, i64::MAX), Ok(-1));
    }

    #[test]
    fn shift_amounts_are_masked_to_63() {
        assert_eq!(shl(1, 70), 64);
        assert_eq!(shl(1, 64), 1);
        assert_eq!(shl(1, -1), i64::MIN);
        assert_eq!(shr(-8, 1), -4);
        assert_eq!(shl(1, (1 << 32) + 1), 2);
    }

}
