use test_case::test_case;

use super::{LocalTime, local_day_start, local_offset_ms, next_local_time, next_local_time_with};
use crate::Error;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;
// 2024-01-15, a Monday outside every DST change.
const SAMPLE_TS: i64 = 1_705_312_800_000;
const FOUR_AM: LocalTime = LocalTime { hour: 4, minute: 0 };

fn local_weekday(ts: i64) -> i64 {
    ((ts + local_offset_ms(ts)).div_euclid(DAY_MS) + 4).rem_euclid(7)
}

#[test]
fn local_time_parses_and_prints() {
    let time: LocalTime = "04:05".parse().unwrap();

    assert_eq!(time, LocalTime { hour: 4, minute: 5 });
    assert_eq!(time.to_string(), "04:05");
    assert_eq!(serde_json::to_string(&time).unwrap(), "\"04:05\"");
    assert_eq!(
        serde_json::from_str::<LocalTime>("\"23:59\"").unwrap().hour,
        23
    );
}

#[test_case("24:00")]
#[test_case("12:60")]
#[test_case("4:00")]
#[test_case("04:0")]
#[test_case("0400")]
#[test_case("+4:00")]
#[test_case("")]
fn local_time_rejects(input: &str) {
    assert!(matches!(
        input.parse::<LocalTime>(),
        Err(Error::InvalidArgument { field, .. }) if field == "time"
    ));
}

#[test]
fn day_start_is_local_midnight() {
    let start = local_day_start(SAMPLE_TS);

    assert_eq!((start + local_offset_ms(start)).rem_euclid(DAY_MS), 0);
    assert!(start <= SAMPLE_TS && SAMPLE_TS - start < DAY_MS);
}

#[test]
fn next_time_after_the_hour_is_tomorrow() {
    let now = local_day_start(SAMPLE_TS) + 5 * HOUR_MS;

    assert_eq!(
        next_local_time(now, FOUR_AM, None),
        local_day_start(now) + DAY_MS + 4 * HOUR_MS
    );
}

#[test]
fn next_time_before_the_hour_is_today() {
    let now = local_day_start(SAMPLE_TS) + 3 * HOUR_MS;

    assert_eq!(
        next_local_time(now, FOUR_AM, None),
        local_day_start(now) + 4 * HOUR_MS
    );
}

#[test]
fn next_time_is_strictly_after_now() {
    let now = local_day_start(SAMPLE_TS) + 4 * HOUR_MS;

    assert_eq!(
        next_local_time(now, FOUR_AM, None),
        local_day_start(now) + DAY_MS + 4 * HOUR_MS
    );
}

#[test_case(0)]
#[test_case(3)]
#[test_case(6)]
fn next_time_lands_on_the_weekday(weekday: u8) {
    let now = local_day_start(SAMPLE_TS) + 5 * HOUR_MS;
    let next = next_local_time(now, FOUR_AM, Some(weekday));

    assert!(next > now && next - now <= 7 * DAY_MS);
    assert_eq!(local_weekday(next), i64::from(weekday));
    assert_eq!(
        (next + local_offset_ms(next)).rem_euclid(DAY_MS),
        4 * HOUR_MS
    );
}

const CET_MS: i64 = HOUR_MS;
const CEST_MS: i64 = 2 * HOUR_MS;
// 2024-03-31 01:00 UTC and 2024-10-27 01:00 UTC: Berlin's 2024 DST transitions.
const BERLIN_SPRING_FORWARD_UTC: i64 = 1_711_846_800_000;
const BERLIN_FALL_BACK_UTC: i64 = 1_729_990_800_000;
// 2024-03-31 03:30 CEST (01:30 UTC), the day Berlin springs forward.
const SPRING_FORWARD_NOW: i64 = 1_711_848_600_000;
// 2024-10-27 03:30 CET (02:30 UTC), the day Berlin falls back.
const FALL_BACK_NOW: i64 = 1_729_996_200_000;
const THREE_THIRTY: LocalTime = LocalTime {
    hour: 3,
    minute: 30,
};

fn berlin_offset(ts: i64) -> i64 {
    if (BERLIN_SPRING_FORWARD_UTC..BERLIN_FALL_BACK_UTC).contains(&ts) {
        CEST_MS
    } else {
        CET_MS
    }
}

#[test_case(SPRING_FORWARD_NOW)]
#[test_case(FALL_BACK_NOW)]
fn daily_time_keeps_local_wall_clock_across_dst(now: i64) {
    let next = next_local_time_with(berlin_offset, now, THREE_THIRTY, None);

    // Both targets are 03:30 local on the next day, which is exactly 24 h later in these zones.
    assert_eq!(next, now + DAY_MS);
    assert_eq!(
        (next + berlin_offset(next)).rem_euclid(DAY_MS),
        3 * HOUR_MS + 30 * 60_000
    );
}
