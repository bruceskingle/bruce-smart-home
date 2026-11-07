//! Octopus's tariffs as far as the app needs to know them: when the cheap times are, how Intelligent Octopus bills the
//! car's charging, for estimating the rate of import the meter has not read yet, see power::combine::Tariff, and the
//! colours of the labels Octopus's rates give the Electricity page.

use time_tz::{OffsetDateTimeExt, Tz};

use crate::power::combine::Tariff;

/// The labels of Octopus's rates on the Electricity page, with their default colours, in the order they are shown, see
/// crate::known_labels: the house's and the car's, as Octopus bills them, the Powerwalls charging from the grid, where it
/// is measured, see power::measured, in purple as in the user's Home Assistant, and the estimates, see OctopusTariff.
/// Readings may have other labels, e.g. older readings have time of use buckets, which come after these.
pub const LABELS: [(&str, &str); 12] = [
    ("Home Low Rate", "#46ee43"),
    ("Home Standard Rate", "#f6671f"),
    ("EV Low Rate", "#271cff"),
    ("EV Standard Rate", "#f72525"),
    ("Battery Low Rate", "#9b51e0"),
    ("Battery Standard Rate", "#d946ef"),
    ("Home Low Rate (estimated)", "#a6f5a4"),
    ("Home Standard Rate (estimated)", "#fab48f"),
    ("EV Low Rate (estimated)", "#8f8aff"),
    ("EV Standard Rate (estimated)", "#fa8f8f"),
    ("Battery Low Rate (estimated)", "#c9a7f2"),
    ("Battery Standard Rate (estimated)", "#f0abfc"),
];

/// The rates of a day and night tariff, as the meter's readings name them, e.g. Home Low Rate is the house's at Low Rate.
const LOW_RATE: &str = "Low Rate";
const STANDARD_RATE: &str = "Standard Rate";

/// The times of a tariff's cheapest rate, in minutes from midnight, each from up to to, which may be past midnight,
/// e.g. 23:30 to 05:30 is (1410, 330).
#[derive(Clone, Debug, PartialEq)]
pub struct CheapTimes(Vec<(i64, i64)>);

impl CheapTimes {
    /// The cheap times from the timeslots of a tariff's time of use scheme, as names and times, e.g. ECO7_NIGHT from
    /// 23:30:00 to 05:30:00. The cheap ones are those named as night, off peak, low or cheap, or if none are, the
    /// shortest. None if there is only one timeslot, so only one rate, or the times can not be read.
    pub fn from_timeslots(timeslots: &[(String, String, String)]) -> Option<Self> {
        let minutes = |time: &str| -> Option<i64> {
            let mut parts = time.split(':');
            Some(parts.next()?.parse::<i64>().ok()? * 60 + parts.next()?.parse::<i64>().ok()?)
        };
        let slots: Vec<(String, i64, i64)> = timeslots.iter()
            .map(|(name, from, to)| Some((name.to_uppercase(), minutes(from)?, minutes(to)?)))
            .collect::<Option<_>>()?;
        if slots.len() < 2 {
            return None;
        }
        let named: Vec<(i64, i64)> = slots.iter()
            .filter(|(name, _, _)| ["NIGHT", "OFF_PEAK", "OFFPEAK", "LOW", "CHEAP"].iter().any(|word| name.contains(word)))
            .map(|(_, from, to)| (*from, *to))
            .collect();
        let length = |(from, to): &(i64, i64)| (to - from).rem_euclid(24 * 60);
        let cheap = if named.is_empty() {
            slots.iter().map(|(_, from, to)| (*from, *to)).min_by_key(length).into_iter().collect()
        }
        else {
            named
        };
        Some(Self(cheap))
    }

    /// Whether the minute from midnight is in the cheap times.
    fn contains(&self, minute: i64) -> bool {
        self.0.iter().any(|(from, to)| if from <= to { (*from..*to).contains(&minute) } else { minute >= *from || minute < *to })
    }
}

/// An Octopus day and night tariff, for estimating the rate of import the meter has not read yet: the low rate in the
/// cheap times, and on an Intelligent Octopus tariff while the car is charging, as outside the cheap times it charges
/// the car only in dispatches, at the low rate, otherwise the standard rate. With the charge cap off, Octopus has billed
/// the car at the standard rate past 9 hours of charging, even in the cheap times, which is not estimated, as the rule
/// is not known.
#[derive(Debug)]
pub struct OctopusTariff {
    pub cheap: CheapTimes,
    /// Whether it is an Intelligent Octopus tariff, on which charging the car outside the cheap times is at the low rate.
    pub intelligent: bool,
    /// The timezone the cheap times are in, the account's billing timezone.
    pub tz: &'static Tz,
}

impl Tariff for OctopusTariff {
    fn estimated_rate(&self, half_hour: i64, car_charging: bool) -> Option<String> {
        let low = self.cheap_time(half_hour)? || (self.intelligent && car_charging);
        Some(String::from(if low { LOW_RATE } else { STANDARD_RATE }))
    }

    fn cheap_time(&self, half_hour: i64) -> Option<bool> {
        let time = time::OffsetDateTime::from_unix_timestamp(half_hour).ok()?.to_timezone(self.tz);
        Some(self.cheap.contains(time.hour() as i64 * 60 + time.minute() as i64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(slots: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
        slots.iter().map(|(name, from, to)| (name.to_string(), from.to_string(), to.to_string())).collect()
    }

    #[test]
    fn the_cheap_times_are_the_night_timeslot_or_else_the_shortest() {
        let go = CheapTimes::from_timeslots(&slots(&[("ECO7_DAY", "05:30:00", "23:30:00"), ("ECO7_NIGHT", "23:30:00", "05:30:00")]));
        assert_eq!(go, Some(CheapTimes(vec![(1410, 330)])));
        let go = go.unwrap();
        assert!(go.contains(23 * 60 + 30) && go.contains(0) && go.contains(5 * 60) && !go.contains(5 * 60 + 30) && !go.contains(12 * 60));
        let unnamed = CheapTimes::from_timeslots(&slots(&[("A", "07:00", "02:00"), ("B", "02:00", "07:00")]));
        assert_eq!(unnamed, Some(CheapTimes(vec![(120, 420)])));
        assert_eq!(CheapTimes::from_timeslots(&slots(&[("ALL", "00:00", "00:00")])), None);
    }

    #[test]
    fn charging_is_low_rate_outside_the_cheap_times_only_on_intelligent_octopus() {
        let cheap = CheapTimes::from_timeslots(&slots(&[("ECO7_DAY", "05:30", "23:30"), ("ECO7_NIGHT", "23:30", "05:30")])).unwrap();
        let tariff = OctopusTariff { cheap, intelligent: true, tz: time_tz::timezones::db::europe::LONDON };
        // 1 November 2026, in GMT, so the times are UTC.
        let day = 1_793_491_200;
        assert_eq!(tariff.estimated_rate(day + 5 * 3600, false).as_deref(), Some(LOW_RATE));
        assert_eq!(tariff.estimated_rate(day + 6 * 3600, false).as_deref(), Some(STANDARD_RATE));
        assert_eq!(tariff.estimated_rate(day + 6 * 3600, true).as_deref(), Some(LOW_RATE));
        assert_eq!((tariff.cheap_time(day), tariff.cheap_time(day + 12 * 3600)), (Some(true), Some(false)));
        let go = OctopusTariff { intelligent: false, ..tariff };
        assert_eq!(go.estimated_rate(day + 6 * 3600, true).as_deref(), Some(STANDARD_RATE));
    }
}
