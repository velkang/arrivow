/// `a × b / c`, rounded down, without overflowing u128 in between. The result must fit in u128.
pub fn mul_div(a: u128, b: u128, c: u128) -> u128 {
    let (high, low) = wide_mul(a, b);
    // Long division of the 256-bit product by c, one bit at a time from the top.
    (0..256).rev().fold((0u128, 0u128), |(quotient, remainder), bit| {
        let next = if bit >= 128 { (high >> (bit - 128)) & 1 } else { (low >> bit) & 1 };
        // remainder < c, so doubling it only overflows when c ≥ 2^127, far beyond any NEAR amount.
        let remainder = (remainder << 1) | next;
        if remainder >= c {
            ((quotient << 1) | 1, remainder - c)
        } else {
            (quotient << 1, remainder)
        }
    })
    .0
}

/// The full 256-bit product of two u128s, as (high, low) halves.
fn wide_mul(a: u128, b: u128) -> (u128, u128) {
    let mask = u64::MAX as u128;
    let (a_high, a_low) = (a >> 64, a & mask);
    let (b_high, b_low) = (b >> 64, b & mask);
    let low_low = a_low * b_low;
    let low_high = a_low * b_high;
    let high_low = a_high * b_low;
    let middle = (low_low >> 64) + (low_high & mask) + (high_low & mask);
    let low = (low_low & mask) | (middle << 64);
    let high = a_high * b_high + (low_high >> 64) + (high_low >> 64) + (middle >> 64);
    (high, low)
}

#[cfg(test)]
mod tests {
    use super::mul_div;

    const NEAR: u128 = 10u128.pow(24);

    #[test]
    fn mul_div_matches_plain_arithmetic_on_small_numbers() {
        assert_eq!(mul_div(7, 9, 4), 15);
        assert_eq!(mul_div(0, 9, 4), 0);
    }

    #[test]
    fn mul_div_handles_products_far_beyond_u128() {
        // 1,000 NEAR × 1,000,000 NEAR overflows u128 by many orders of magnitude.
        assert_eq!(mul_div(1_000 * NEAR, 1_000_000 * NEAR, 3_000 * NEAR), 1_000_000 * NEAR / 3);
        // A product near 2^252 with the largest divisor the long division supports.
        assert_eq!(mul_div(1 << 126, 1 << 126, 1 << 126), 1 << 126);
    }
}
