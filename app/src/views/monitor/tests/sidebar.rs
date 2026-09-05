use std::time::Duration;

use super::format_elapsed;

#[test]
fn elapsed_time_uses_unbounded_hours_and_two_digit_minutes_and_seconds() {
    assert_eq!(format_elapsed(Duration::ZERO), "00:00:00");
    assert_eq!(format_elapsed(Duration::from_secs(3_661)), "01:01:01");
    assert_eq!(format_elapsed(Duration::from_secs(360_005)), "100:00:05");
}
