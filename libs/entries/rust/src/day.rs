use std::time::SystemTime;

fn civil(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let (era, day_of_era) = (shifted.div_euclid(146_097), shifted.rem_euclid(146_097));
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
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

fn days_since_epoch(time: SystemTime) -> i64 {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => (after.as_secs() / 86_400) as i64,
        Err(before) => {
            let before = before.duration();
            let whole = before.as_secs();
            let reached =
                whole / 86_400 + u64::from(whole % 86_400 > 0 || before.subsec_nanos() > 0);
            -(reached as i64)
        }
    }
}

pub fn utc(time: SystemTime) -> String {
    let (year, month, day) = civil(days_since_epoch(time));
    let sign = if year < 0 { "-" } else { "" };
    format!("{sign}{:04}-{month:02}-{day:02}", year.unsigned_abs())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::utc;

    #[test]
    fn a_day_is_the_utc_date_of_a_time() {
        for (seconds, day) in [
            (0, "1970-01-01"),
            (86_399, "1970-01-01"),
            (86_400, "1970-01-02"),
            (951_782_400, "2000-02-29"),
            (4_102_444_800, "2100-01-01"),
            (253_402_300_799, "9999-12-31"),
        ] {
            assert_eq!(
                utc(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)),
                day,
                "{seconds}"
            );
        }
    }

    #[test]
    fn a_time_before_1970_is_the_day_it_falls_in() {
        let before =
            |seconds: u64, nanos: u32| utc(SystemTime::UNIX_EPOCH - Duration::new(seconds, nanos));
        for (seconds, nanos, day) in [
            (0, 1, "1969-12-31"),
            (1, 0, "1969-12-31"),
            (86_400, 0, "1969-12-31"),
            (86_400, 1, "1969-12-30"),
            (86_401, 0, "1969-12-30"),
            (58_060_800, 0, "1968-02-29"),
            (58_060_801, 0, "1968-02-28"),
            (2_203_891_200, 0, "1900-03-01"),
            (2_203_891_201, 0, "1900-02-28"),
            (62_135_596_800, 0, "0001-01-01"),
        ] {
            assert_eq!(before(seconds, nanos), day, "{seconds} {nanos}");
        }
    }

    #[test]
    fn a_year_outside_0000_to_9999_is_written_with_all_its_digits() {
        assert_eq!(
            utc(SystemTime::UNIX_EPOCH + Duration::from_secs(253_402_300_800)),
            "10000-01-01"
        );
        assert_eq!(
            utc(SystemTime::UNIX_EPOCH - Duration::from_secs(62_167_219_200)),
            "0000-01-01"
        );
        assert_eq!(
            utc(SystemTime::UNIX_EPOCH - Duration::from_secs(62_167_219_201)),
            "-0001-12-31"
        );
    }
}
