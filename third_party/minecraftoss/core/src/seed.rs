//! Java world-option and protocol seed semantics.
//! Reference: Minecraft 26.3 `WorldOptions.parseSeed`, remapped common JAR
//! SHA-256 52f29c477afb4eabf0e6a726bfc89911ed06b74d19b5515dbfa39777788c9a25.

/// Java Edition's UI seed parsing: whitespace is trimmed, empty means random,
/// signed decimal input is parsed as `long`, and all other input uses UTF-16
/// `String.hashCode` sign-extended to `long`.
pub fn parse_world_seed(input: &str) -> Option<i64> {
    // Java String.trim removes UTF-16 code units <= U+0020, unlike Rust's
    // Unicode whitespace trim (which would also remove NBSP, em space, etc.).
    let trimmed = input.trim_matches(|c: char| c <= '\u{20}');
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(seed) = trimmed.parse::<i64>() {
        return Some(seed);
    }
    Some(i64::from(java_string_hash(trimmed)))
}

pub fn java_string_hash(input: &str) -> i32 {
    input.encode_utf16().fold(0_i32, |hash, unit| {
        hash.wrapping_mul(31).wrapping_add(i32::from(unit))
    })
}

/// The measurement protocol requires a canonical signed 64-bit decimal string.
pub fn parse_protocol_seed(input: &str) -> Result<i64, String> {
    let value: i64 = input
        .parse()
        .map_err(|_| "seed must be a signed 64-bit decimal string")?;
    if value.to_string() != input {
        return Err("seed must use canonical decimal notation".into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_options_seed_matches_java_utf16_and_overflow() {
        assert_eq!(parse_world_seed(" "), None);
        assert_eq!(parse_world_seed(" -123456789 "), Some(-123456789));
        assert_eq!(parse_world_seed("abc"), Some(96354));
        assert_eq!(parse_world_seed("\u{1f600}"), Some(1772899));
        assert_eq!(parse_world_seed("9223372036854775808"), Some(-1773151197));
        assert_ne!(parse_world_seed("\u{00a0}1\u{00a0}"), Some(1));
    }

    #[test]
    fn protocol_seed_is_canonical() {
        assert_eq!(parse_protocol_seed("-9223372036854775808"), Ok(i64::MIN));
        assert_eq!(parse_protocol_seed("0"), Ok(0));
        assert!(parse_protocol_seed("-0").is_err());
        assert!(parse_protocol_seed("+1").is_err());
        assert!(parse_protocol_seed("01").is_err());
    }
}
