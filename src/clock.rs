//! The app's idea of the time now. It is the real time, except for an offline profile, see Profile::offline, whose
//! data ends when it was last fetched: its pages behave as if it were then, e.g. today and the last 7 days, so that
//! they show its latest data however long ago that was.

use std::sync::atomic::{AtomicI64, Ordering};

use bruce_graphql::types::{Date, DateTime};
use time::OffsetDateTime;
use time_tz::{OffsetDateTimeExt, Tz};

/// The time is real.
const REAL: i64 = i64::MIN;
/// The unix timestamp it is, or REAL.
static AS_OF: AtomicI64 = AtomicI64::new(REAL);

/// Makes it the unix timestamp as_of from now on, or the real time again with None. Set when a profile is chosen.
pub fn set_as_of(as_of: Option<i64>) {
    AS_OF.store(as_of.unwrap_or(REAL), Ordering::SeqCst);
}

pub fn now_utc() -> OffsetDateTime {
    match AS_OF.load(Ordering::SeqCst) {
        REAL => OffsetDateTime::now_utc(),
        as_of => OffsetDateTime::from_unix_timestamp(as_of).unwrap_or_else(|_| OffsetDateTime::now_utc()),
    }
}

pub fn now(tz: &Tz) -> DateTime {
    DateTime::from_unix_timestamp(now_utc().unix_timestamp()).map_or_else(|_| DateTime::now(tz), |now| now.to_timezone(tz))
}

pub fn today(tz: &Tz) -> time::Date {
    now_utc().to_timezone(tz).date()
}

/// Today as the GraphQL types' date.
pub fn today_date(tz: &Tz) -> Date {
    Date::from(today(tz))
}
