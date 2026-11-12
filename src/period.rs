//! The periods which energy is shown in, e.g. half hours or days, and where each starts and ends, which for days, weeks
//! and months depends on the timezone.

use bruce_graphql::types::{Date, DateTime};
use time_tz::Tz;

/// How long the periods of the graph and table are.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Period {
    TenSeconds,
    Minute,
    FiveMinutes,
    TenMinutes,
    HalfHour,
    Day,
    Week,
    Month,
}

impl Period {
    pub const ALL: [Period; 8] = [
        Period::TenSeconds,
        Period::Minute,
        Period::FiveMinutes,
        Period::TenMinutes,
        Period::HalfHour,
        Period::Day,
        Period::Week,
        Period::Month,
    ];

    /// The length of a period shorter than a day, in seconds.
    pub fn seconds(&self) -> Option<i64> {
        match self {
            Period::TenSeconds => Some(10),
            Period::Minute => Some(60),
            Period::FiveMinutes => Some(300),
            Period::TenMinutes => Some(600),
            Period::HalfHour => Some(1800),
            _ => None,
        }
    }

    /// The next shorter period, to drill down to, None for the shortest.
    pub fn shorter(&self) -> Option<Period> {
        match self {
            Period::TenSeconds => None,
            Period::Minute => Some(Period::TenSeconds),
            Period::FiveMinutes => Some(Period::Minute),
            Period::TenMinutes => Some(Period::FiveMinutes),
            Period::HalfHour => Some(Period::TenMinutes),
            Period::Day => Some(Period::HalfHour),
            Period::Week => Some(Period::Day),
            Period::Month => Some(Period::Week),
        }
    }

    /// Whether the period is a day or longer.
    pub fn is_calendar(&self) -> bool {
        self.seconds().is_none()
    }
}

impl std::fmt::Display for Period {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Period::TenSeconds => write!(f, "10s"),
            Period::Minute => write!(f, "1min"),
            Period::FiveMinutes => write!(f, "5min"),
            Period::TenMinutes => write!(f, "10min"),
            Period::HalfHour => write!(f, "30min"),
            Period::Day => write!(f, "Day"),
            Period::Week => write!(f, "Week"),
            Period::Month => write!(f, "Month"),
        }
    }
}

/// The start and end of the day, week (from Monday) or month containing date, in the timezone, as unix timestamps.
pub fn calendar_bounds(date: Date, period: Period, tz: &'static Tz) -> (i64, i64) {
    let (start, end) = match period {
        Period::Week => {
            let monday = *date - time::Duration::days(date.weekday().number_days_from_monday() as i64);
            (monday, monday + time::Duration::days(7))
        },
        Period::Month => {
            let first = date.replace_day(1).unwrap_or(*date);
            let next = if first.month() == time::Month::December {
                first.replace_year(first.year() + 1).and_then(|date| date.replace_month(time::Month::January))
            }
            else {
                first.replace_month(first.month().next())
            }.unwrap_or(first);
            (first, next)
        },
        _ => (*date, *date + time::Duration::days(1)),
    };
    (Date::from(start).at_midnight(tz).unix_timestamp(), Date::from(end).at_midnight(tz).unix_timestamp())
}

/// The start and end of the period containing time, all as unix timestamps, in the timezone for days, weeks and months.
pub fn period_bounds(time: i64, period: Period, tz: &'static Tz) -> (i64, i64) {
    match period.seconds() {
        Some(seconds) => {
            let start = time - time.rem_euclid(seconds);
            (start, start + seconds)
        },
        None => match DateTime::from_unix_timestamp(time) {
            Ok(time) => calendar_bounds(time.to_timezone(tz).to_date(), period, tz),
            Err(_) => (time, time),
        },
    }
}
