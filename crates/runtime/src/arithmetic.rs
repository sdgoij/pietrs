//! Integer arithmetic with the specification's rounding and sign rules.

use crate::Value;

/// Floored division (Python `//`).
///
/// Returns `None` for a zero divisor and for the one overflow case,
/// `MIN / -1`; the command is ignored in both.
pub fn floored_div(a: Value, b: Value) -> Option<Value> {
    if b == 0 || (a == Value::MIN && b == -1) {
        return None;
    }
    let quotient = a / b;
    let remainder = a % b;
    if remainder != 0 && ((remainder < 0) != (b < 0)) {
        Some(quotient - 1)
    } else {
        Some(quotient)
    }
}

/// Floored modulo (Python `%`): the result has the sign of the divisor.
pub fn floored_mod(a: Value, b: Value) -> Option<Value> {
    if b == 0 {
        return None;
    }
    if a == Value::MIN && b == -1 {
        return Some(0);
    }
    let remainder = a % b;
    if remainder != 0 && ((remainder < 0) != (b < 0)) {
        Some(remainder + b)
    } else {
        Some(remainder)
    }
}

/// `1` if the value is zero, else `0`.
pub const fn not(value: Value) -> Value {
    if value == 0 { 1 } else { 0 }
}

/// `1` if `a > b`, else `0`.
pub const fn greater(a: Value, b: Value) -> Value {
    if a > b { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn floored_division_matches_python() {
        assert_eq!(floored_div(7, 3), Some(2));
        assert_eq!(floored_div(-7, 3), Some(-3));
        assert_eq!(floored_div(7, -3), Some(-3));
        assert_eq!(floored_div(-7, -3), Some(2));
        assert_eq!(floored_div(1, 0), None);
        assert_eq!(floored_div(Value::MIN, -1), None);
    }

    #[test]
    fn floored_modulo_matches_python() {
        assert_eq!(floored_mod(5, 3), Some(2));
        assert_eq!(floored_mod(2, 3), Some(2));
        assert_eq!(floored_mod(-1, 3), Some(2));
        assert_eq!(floored_mod(-4, 3), Some(2));
        assert_eq!(floored_mod(7, -3), Some(-2));
        assert_eq!(floored_mod(-7, -3), Some(-1));
        assert_eq!(floored_mod(1, 0), None);
        assert_eq!(floored_mod(Value::MIN, -1), Some(0));
    }

    #[test]
    fn not_and_greater_are_booleans() {
        assert_eq!(not(0), 1);
        assert_eq!(not(1), 0);
        assert_eq!(not(-1), 0);
        assert_eq!(greater(2, 1), 1);
        assert_eq!(greater(1, 1), 0);
        assert_eq!(greater(1, 2), 0);
    }

    proptest! {
        #[test]
        fn division_and_modulo_reconstruct_the_dividend(a in any::<i64>(), b in any::<i64>()) {
            prop_assume!(b != 0);
            prop_assume!(!(a == Value::MIN && b == -1));
            let quotient = i128::from(floored_div(a, b).unwrap());
            let remainder = i128::from(floored_mod(a, b).unwrap());
            prop_assert_eq!(quotient * i128::from(b) + remainder, i128::from(a));
            prop_assert!(remainder == 0 || (remainder > 0) == (b > 0));
            prop_assert!(remainder.unsigned_abs() < u128::from(b.unsigned_abs()));
        }
    }
}
