use test_case::test_case;

use super::{BackupSchedule, Weekday};
use crate::{Error, LocalTime, clock};

const DAY_MS: i64 = 86_400_000;
const WEEK_DAYS: i64 = 7;
// 1970-01-01 was a Thursday; Weekday counts Sunday as 0.
const EPOCH_WEEKDAY: i64 = 4;
// 2024-01-15, a Monday outside every DST change.
const SAMPLE_TS: i64 = 1_705_312_800_000;

#[test_case("daily@03:30", BackupSchedule::Daily(LocalTime { hour: 3, minute: 30 }))]
#[test_case("weekly@tue@03:30", BackupSchedule::Weekly(Weekday::Tue, LocalTime { hour: 3, minute: 30 }))]
fn parses_and_displays(text: &str, expected: BackupSchedule) {
    let parsed: BackupSchedule = text.parse().unwrap();

    assert_eq!(parsed, expected);
    assert_eq!(parsed.to_string(), text);
}

#[test_case("weekly@xyz@03:30")]
#[test_case("daily@24:00")]
#[test_case("hourly")]
#[test_case("daily")]
#[test_case("weekly@mon")]
#[test_case("daily@03:30@x")]
fn rejects_malformed(text: &str) {
    assert!(matches!(
        text.parse::<BackupSchedule>(),
        Err(Error::InvalidArgument { field, .. }) if field == "backup_schedule"
    ));
}

#[test]
fn next_after_is_strict() {
    let at = LocalTime {
        hour: 3,
        minute: 30,
    };
    let now = clock::next_local_time(SAMPLE_TS, at, None);

    assert_eq!(BackupSchedule::Daily(at).next_after(now), now + DAY_MS);
}

#[test]
fn weekly_lands_on_its_weekday() {
    let at = LocalTime {
        hour: 3,
        minute: 30,
    };
    let next = BackupSchedule::Weekly(Weekday::Fri, at).next_after(SAMPLE_TS);
    let local_day = (next + clock::local_offset_ms(next)).div_euclid(DAY_MS);

    assert_eq!(
        (local_day + EPOCH_WEEKDAY).rem_euclid(WEEK_DAYS),
        Weekday::Fri as i64
    );
    assert!(next > SAMPLE_TS);
}
