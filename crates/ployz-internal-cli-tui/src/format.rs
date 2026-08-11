/// Formats signed nanoseconds as whole milliseconds using Go-compatible
/// floating-point rounding, with half milliseconds rounded away from zero.
pub fn format_rtt(duration_nanoseconds: i64) -> String {
    let millis = ((duration_nanoseconds as f64) / 1_000_000.0).round() as i64;
    format!("{millis}ms")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_oracle_rounding_cases() {
        let cases = [
            (0, "0ms"),
            (39_200_000, "39ms"),
            (39_500_000, "40ms"),
            (39_600_000, "40ms"),
            (-39_200_000, "-39ms"),
            (-39_500_000, "-40ms"),
            (-39_600_000, "-40ms"),
            (140_000_000, "140ms"),
            (1_000_000_000, "1000ms"),
            (1_500_000_000, "1500ms"),
        ];

        for (input, expected) in cases {
            assert_eq!(format_rtt(input), expected);
        }
    }

    #[test]
    fn matches_go_float_rounding_at_duration_extremes() {
        assert_eq!(format_rtt(i64::MAX), "9223372036855ms");
        assert_eq!(format_rtt(i64::MIN), "-9223372036855ms");
    }
}
