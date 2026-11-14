use std::{fmt, str::FromStr, sync::Once};

use nix::libc;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::error::Error;

#[cfg(test)]
mod tests;

const MS_PER_MINUTE: i64 = 60_000;
pub const MS_PER_DAY: i64 = 86_400_000;
const WEEKDAYS: i64 = 7;
// 1970-01-01 was a Thursday; tm_wday counts Sunday as 0.
const EPOCH_WEEKDAY: i64 = 4;
const TIME_FIELD: &str = "time";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcDateTime {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    pub millisecond: i64,
}

impl UtcDateTime {
    // Hinnant's days-to-civil-date algorithm.
    pub fn from_unix_ms(unix_ms: i64) -> UtcDateTime {
        let days = unix_ms.div_euclid(MS_PER_DAY) + 719_468;
        let ms_of_day = unix_ms.rem_euclid(MS_PER_DAY);
        let era = days.div_euclid(146_097);
        let day_of_era = days.rem_euclid(146_097);
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let month = if shifted_month < 10 {
            shifted_month + 3
        } else {
            shifted_month - 9
        };

        UtcDateTime {
            year: year_of_era + era * 400 + i64::from(month <= 2),
            month,
            day: day_of_year - (153 * shifted_month + 2) / 5 + 1,
            hour: ms_of_day / 3_600_000,
            minute: ms_of_day % 3_600_000 / MS_PER_MINUTE,
            second: ms_of_day % MS_PER_MINUTE / 1000,
            millisecond: ms_of_day % 1000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub hour: u8,
    pub minute: u8,
}

impl fmt::Display for LocalTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

impl FromStr for LocalTime {
    type Err = Error;

    fn from_str(s: &str) -> Result<LocalTime, Error> {
        let invalid = || Error::InvalidArgument {
            field: TIME_FIELD.to_string(),
            message: "expected `HH:MM` on a 24-hour clock, for example `04:00`".to_string(),
        };
        let (hour, minute) = s.split_once(':').ok_or_else(invalid)?;
        let digits = |part: &str| part.len() == 2 && part.bytes().all(|b| b.is_ascii_digit());

        if !digits(hour) || !digits(minute) {
            return Err(invalid());
        }

        let (hour, minute) = (
            hour.parse().map_err(|_| invalid())?,
            minute.parse().map_err(|_| invalid())?,
        );

        if hour > 23 || minute > 59 {
            return Err(invalid());
        }

        Ok(LocalTime { hour, minute })
    }
}

impl Serialize for LocalTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for LocalTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<LocalTime, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

impl JsonSchema for LocalTime {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LocalTime".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": "^([01][0-9]|2[0-3]):[0-5][0-9]$" })
    }
}

// The libc crate does not bind tzset for this target.
unsafe extern "C" {
    fn tzset();
}

pub fn local_offset_ms(ts_ms: i64) -> i64 {
    static TZSET: Once = Once::new();

    let seconds = ts_ms.div_euclid(1000) as libc::time_t;
    // SAFETY: libc FFI. `tm` is plain data, so all-zero is a valid value; `localtime_r` is the
    // thread-safe variant and writes only into `tm`; `tzset` runs once so TZ is read before use.
    let offset = unsafe {
        TZSET.call_once(|| tzset());
        let mut tm: libc::tm = std::mem::zeroed();

        if libc::localtime_r(&seconds, &mut tm).is_null() {
            return 0;
        }

        tm.tm_gmtoff
    };

    offset as i64 * 1000
}

pub fn local_day_start(now_ms: i64) -> i64 {
    local_day_start_with(local_offset_ms, now_ms)
}

pub fn next_local_time(now_ms: i64, at: LocalTime, weekday: Option<u8>) -> i64 {
    next_local_time_with(local_offset_ms, now_ms, at, weekday)
}

// The offset lookup is a parameter so DST math can be tested without touching the TZ environment.
fn local_day_start_with(offset_at: impl Fn(i64) -> i64, now_ms: i64) -> i64 {
    let offset = offset_at(now_ms);
    let start = (now_ms + offset).div_euclid(MS_PER_DAY) * MS_PER_DAY - offset;

    // On a DST day the offset at midnight differs from the offset now.
    start + offset - offset_at(start)
}

fn next_local_time_with(
    offset_at: impl Fn(i64) -> i64,
    now_ms: i64,
    at: LocalTime,
    weekday: Option<u8>,
) -> i64 {
    let first_day = local_day_start_with(&offset_at, now_ms);
    let offset_first = offset_at(first_day);
    let time_of_day = (i64::from(at.hour) * 60 + i64::from(at.minute)) * MS_PER_MINUTE;

    (0..=WEEKDAYS)
        .find_map(|day| {
            let base = first_day + day * MS_PER_DAY + time_of_day;
            // The offset can differ at the target (DST), so shift by the difference from local midnight.
            let candidate = base + offset_first - offset_at(base);
            let local_day = (candidate + offset_at(candidate)).div_euclid(MS_PER_DAY);
            let candidate_weekday = (local_day + EPOCH_WEEKDAY).rem_euclid(WEEKDAYS);

            (candidate > now_ms && weekday.is_none_or(|w| i64::from(w) == candidate_weekday))
                .then_some(candidate)
        })
        .unwrap_or(now_ms + WEEKDAYS * MS_PER_DAY)
}
