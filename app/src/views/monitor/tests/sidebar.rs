use std::time::Duration;

use super::{format_elapsed, parse_sampling_interval};

#[test]
fn elapsed_time_uses_unbounded_hours_and_two_digit_minutes_and_seconds() {
    assert_eq!(format_elapsed(Duration::ZERO), "00:00:00");
    assert_eq!(format_elapsed(Duration::from_secs(3_661)), "01:01:01");
    assert_eq!(format_elapsed(Duration::from_secs(360_005)), "100:00:05");
}

#[test]
fn sampling_interval_accepts_only_positive_whole_seconds_in_millisecond_range() {
    assert_eq!(parse_sampling_interval("2"), Some(Duration::from_secs(2)));
    for invalid in ["", "0", "-1", "1.5", "18446744073709552"] {
        assert_eq!(parse_sampling_interval(invalid), None, "{invalid}");
    }
}
