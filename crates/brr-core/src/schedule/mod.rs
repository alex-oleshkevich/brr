use std::{fmt, str::FromStr};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    clock::{self, LocalTime},
    error::Error,
};

#[cfg(test)]
mod tests;

const FIELD: &str = "backup_schedule";
const DAILY: &str = "daily";
const WEEKLY: &str = "weekly";

// Discriminants follow libc tm_wday, which clock::next_local_time compares against.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Sun,
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
}

const WEEKDAYS: [Weekday; 7] = [
    Weekday::Sun,
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
];

impl Weekday {
    fn name(self) -> &'static str {
        match self {
            Weekday::Sun => "sun",
            Weekday::Mon => "mon",
            Weekday::Tue => "tue",
            Weekday::Wed => "wed",
            Weekday::Thu => "thu",
            Weekday::Fri => "fri",
            Weekday::Sat => "sat",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupSchedule {
    Daily(LocalTime),
    Weekly(Weekday, LocalTime),
}

impl FromStr for BackupSchedule {
    type Err = Error;

    fn from_str(s: &str) -> Result<BackupSchedule, Error> {
        let invalid = || Error::InvalidArgument {
            field: FIELD.to_string(),
            message: "expected daily@HH:MM or weekly@<mon..sun>@HH:MM".to_string(),
        };
        let parts: Vec<&str> = s.split('@').collect();
        let time = |text: &str| text.parse::<LocalTime>().map_err(|_| invalid());

        match parts[..] {
            [DAILY, at] => Ok(BackupSchedule::Daily(time(at)?)),
            [WEEKLY, day, at] => Ok(BackupSchedule::Weekly(
                WEEKDAYS
                    .into_iter()
                    .find(|weekday| weekday.name() == day)
                    .ok_or_else(invalid)?,
                time(at)?,
            )),
            _ => Err(invalid()),
        }
    }
}

impl fmt::Display for BackupSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackupSchedule::Daily(at) => write!(f, "{DAILY}@{at}"),
            BackupSchedule::Weekly(day, at) => write!(f, "{WEEKLY}@{}@{at}", day.name()),
        }
    }
}

impl BackupSchedule {
    pub fn next_after(&self, now_ms: i64) -> i64 {
        match self {
            BackupSchedule::Daily(at) => clock::next_local_time(now_ms, *at, None),
            BackupSchedule::Weekly(day, at) => {
                clock::next_local_time(now_ms, *at, Some(*day as u8))
            }
        }
    }
}
