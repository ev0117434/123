use anyhow::{bail, Result};

/// Parse decimal price string to i64 with scale 1e8
/// Uses decimal arithmetic to avoid float errors
///
/// Examples:
/// - "100.5" -> 10050000000
/// - "0.00001234" -> 1234
/// - "12345.6789" -> 1234567890000
///
/// Round half-up: if the digit after the 8th decimal is >= 5, round up
#[inline(always)]
pub fn parse_price_i64_1e8(s: &str) -> Result<i64> {
    let s = s.trim();

    if s.is_empty() {
        bail!("Empty price string");
    }

    // Find decimal point
    let parts: Vec<&str> = s.split('.').collect();

    if parts.len() > 2 {
        bail!("Invalid price format: multiple decimal points");
    }

    let integer_part = parts[0];
    let decimal_part = if parts.len() == 2 { parts[1] } else { "" };

    // Parse integer part
    let mut result: i64 = 0;

    if !integer_part.is_empty() {
        for ch in integer_part.bytes() {
            if !ch.is_ascii_digit() {
                bail!("Invalid character in integer part: {}", ch as char);
            }
            let digit = (ch - b'0') as i64;
            result = result.checked_mul(10)
                .ok_or_else(|| anyhow::anyhow!("Integer overflow"))?;
            result = result.checked_add(digit)
                .ok_or_else(|| anyhow::anyhow!("Integer overflow"))?;
        }
    }

    // Scale integer part by 1e8
    result = result.checked_mul(100_000_000)
        .ok_or_else(|| anyhow::anyhow!("Overflow scaling integer part"))?;

    // Process decimal part (up to 8 digits + 1 for rounding)
    if !decimal_part.is_empty() {
        let mut decimal_value: i64 = 0;
        let mut scale: i64 = 10_000_000; // Start with 1e7 (for first decimal digit)
        let mut round_digit: Option<u8> = None;

        for (i, ch) in decimal_part.bytes().enumerate() {
            if !ch.is_ascii_digit() {
                bail!("Invalid character in decimal part: {}", ch as char);
            }

            let digit = ch - b'0';

            if i < 8 {
                // First 8 decimal digits - add to value
                decimal_value += (digit as i64) * scale;
                scale /= 10;
            } else if i == 8 {
                // 9th digit - used for rounding
                round_digit = Some(digit);
                break; // We only need the 9th digit for rounding
            }
        }

        result = result.checked_add(decimal_value)
            .ok_or_else(|| anyhow::anyhow!("Overflow adding decimal part"))?;

        // Round half-up: if 9th digit >= 5, add 1
        if let Some(d) = round_digit {
            if d >= 5 {
                result = result.checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("Overflow during rounding"))?;
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_price_integer() {
        assert_eq!(parse_price_i64_1e8("100").unwrap(), 10_000_000_000);
        assert_eq!(parse_price_i64_1e8("1").unwrap(), 100_000_000);
        assert_eq!(parse_price_i64_1e8("0").unwrap(), 0);
    }

    #[test]
    fn test_parse_price_decimal() {
        assert_eq!(parse_price_i64_1e8("100.5").unwrap(), 10_050_000_000);
        assert_eq!(parse_price_i64_1e8("0.00001234").unwrap(), 1_234);
        assert_eq!(parse_price_i64_1e8("12345.6789").unwrap(), 1_234_567_890_000);
    }

    #[test]
    fn test_parse_price_rounding() {
        // Round down (4 < 5)
        assert_eq!(parse_price_i64_1e8("0.000000004").unwrap(), 0);
        assert_eq!(parse_price_i64_1e8("0.123456784").unwrap(), 12_345_678);

        // Round up (5 >= 5)
        assert_eq!(parse_price_i64_1e8("0.000000005").unwrap(), 1);
        assert_eq!(parse_price_i64_1e8("0.123456785").unwrap(), 12_345_679);

        // Round up (9 >= 5)
        assert_eq!(parse_price_i64_1e8("0.000000009").unwrap(), 1);
        assert_eq!(parse_price_i64_1e8("0.123456789").unwrap(), 12_345_679);
    }

    #[test]
    fn test_parse_price_edge_cases() {
        // No decimal point
        assert_eq!(parse_price_i64_1e8("42").unwrap(), 4_200_000_000);

        // Trailing zeros
        assert_eq!(parse_price_i64_1e8("100.00000000").unwrap(), 10_000_000_000);

        // Leading zeros (in decimal)
        assert_eq!(parse_price_i64_1e8("0.00000001").unwrap(), 1);
        assert_eq!(parse_price_i64_1e8("0.1").unwrap(), 10_000_000);
    }

    #[test]
    fn test_parse_price_real_crypto() {
        // Bitcoin-like price
        assert_eq!(parse_price_i64_1e8("43567.89").unwrap(), 4_356_789_000_000);

        // Ethereum-like price
        assert_eq!(parse_price_i64_1e8("2345.67").unwrap(), 234_567_000_000);

        // Low price altcoin
        assert_eq!(parse_price_i64_1e8("0.00012345").unwrap(), 12_345);
    }

    #[test]
    fn test_parse_price_errors() {
        assert!(parse_price_i64_1e8("").is_err());
        assert!(parse_price_i64_1e8("abc").is_err());
        assert!(parse_price_i64_1e8("12.34.56").is_err());
    }

    #[test]
    fn test_parse_price_large_numbers() {
        // Should handle large prices
        assert_eq!(parse_price_i64_1e8("999999.99999999").unwrap(), 99_999_999_999_999);
    }
}
