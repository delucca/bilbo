use std::time::SystemTime;

fn civil(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let (era, day_of_era) = (shifted / 146_097, shifted % 146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * march_month + 2) / 5 + 1;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    };
    (year_of_era + era * 400 + u64::from(month <= 2), month, day)
}

/// The UTC day of a time in milliseconds since 1970; an earlier time is 1970-01-01.
pub fn day(millis: i64) -> String {
    let seconds = u64::try_from(millis).map_or(0, |millis| millis / 1000);
    let (year, month, day) = civil(seconds / 86_400);
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn utc(time: SystemTime) -> String {
    let seconds = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let rest = seconds % 86_400;
    let (year, month, day) = civil(seconds / 86_400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rest / 3600,
        rest % 3600 / 60
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::{day, utc};

    #[test]
    fn a_time_prints_as_a_utc_date_and_minute() {
        let at = |seconds: u64| utc(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
        assert_eq!(at(0), "1970-01-01 00:00 UTC");
        assert_eq!(at(59), "1970-01-01 00:00 UTC");
        assert_eq!(at(1_794_069_552), "2026-11-07 16:39 UTC");
        assert_eq!(at(1_709_164_800), "2024-02-29 00:00 UTC");
        assert_eq!(at(951_868_799), "2000-02-29 23:59 UTC");
        assert_eq!(at(4_107_542_400), "2100-03-01 00:00 UTC");
        assert_eq!(at(1_798_761_599), "2026-12-31 23:59 UTC");
        assert_eq!(
            utc(SystemTime::UNIX_EPOCH - Duration::from_secs(1)),
            "1970-01-01 00:00 UTC"
        );
    }

    #[test]
    fn a_day_is_utc_and_never_before_1970() {
        assert_eq!(day(0), "1970-01-01");
        assert_eq!(day(86_399_999), "1970-01-01");
        assert_eq!(day(86_400_000), "1970-01-02");
        assert_eq!(day(1_794_069_552_000), "2026-11-07");
        assert_eq!(day(1_709_164_800_000), "2024-02-29");
        assert_eq!(day(-1), "1970-01-01");
        assert_eq!(day(i64::MIN), "1970-01-01");
    }
}
