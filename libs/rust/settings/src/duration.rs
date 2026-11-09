use std::time::Duration;

use crate::key::{Value, quote};

pub const SECOND: Duration = Duration::from_secs(1);
pub const MINUTE: Duration = Duration::from_secs(60);
pub const HOUR: Duration = Duration::from_secs(60 * 60);
pub const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// `90s`, `60m`, `1h`, one optional space; a bare number is refused.
pub(crate) fn parse(text: &str) -> Option<Duration> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let number: u64 = text[..digits].parse().ok()?;
    let rest = &text[digits..];
    let unit = match rest.strip_prefix(' ').unwrap_or(rest) {
        "s" => SECOND,
        "m" => MINUTE,
        "h" => HOUR,
        _ => return None,
    };
    number.checked_mul(unit.as_secs()).map(Duration::from_secs)
}

pub fn read(value: &Value<'_>) -> Option<Duration> {
    value.text().and_then(parse)
}

/// Whole hours from two up, else minutes, else seconds.
pub fn show(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let text = if seconds >= 2 * HOUR.as_secs() && seconds.is_multiple_of(HOUR.as_secs()) {
        format!("{}h", seconds / HOUR.as_secs())
    } else if seconds > 0 && seconds.is_multiple_of(MINUTE.as_secs()) {
        format!("{}m", seconds / MINUTE.as_secs())
    } else {
        format!("{seconds}s")
    };
    quote(&text)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{DAY, HOUR, MINUTE, parse, show};

    #[test]
    fn a_duration_is_whole_seconds_minutes_or_hours() {
        for (text, duration) in [
            ("90s", Duration::from_secs(90)),
            ("60m", HOUR),
            ("60 m", HOUR),
            ("1h", HOUR),
            ("24h", DAY),
            ("1 m", MINUTE),
            ("0s", Duration::ZERO),
            ("007m", 7 * MINUTE),
        ] {
            assert_eq!(parse(text), Some(duration), "{text:?}");
        }
    }

    #[test]
    fn a_bare_number_another_unit_or_a_stray_space_is_refused() {
        for text in [
            "60",
            "",
            "m",
            "60M",
            "1H",
            "60min",
            "1d",
            "1.5h",
            "-1m",
            "+1m",
            " 1m",
            "1m ",
            "1  m",
            "1\tm",
            "1ms",
            "1h30m",
            "99999999999999999999s",
            "18446744073709551615h",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_duration_shows_in_hours_from_two_then_minutes_then_seconds() {
        assert_eq!(show(HOUR), "\"60m\"");
        assert_eq!(show(2 * HOUR), "\"2h\"");
        assert_eq!(show(DAY), "\"24h\"");
        assert_eq!(show(MINUTE), "\"1m\"");
        assert_eq!(show(Duration::from_secs(90)), "\"90s\"");
        assert_eq!(show(Duration::from_secs(150 * 60)), "\"150m\"");
        assert_eq!(show(Duration::ZERO), "\"0s\"");
        for duration in [MINUTE, HOUR, 2 * HOUR, DAY, Duration::from_secs(61)] {
            let text = show(duration);
            assert_eq!(parse(text.trim_matches('"')), Some(duration), "{text}");
        }
    }
}
