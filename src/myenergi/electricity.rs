//! The zappis' minute records, as myenergi keeps them and the module caches them, see fetch.rs, for Home's Electricity
//! page: two channels of each zappi, see power::topology, what it charged the car with, and its grid CT clamp, which
//! the site's topology places, e.g. the grid clamp at the grid connection.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::power::measured::{Flow, MeasurementSource, Measurements};
use crate::power::topology::{Channel, Use};

use super::fetch;

/// What a period's electricity is said to be from.
const NAME: &str = "myenergi";

const JOULES_PER_KWH: f64 = 3_600_000.0;

/// A zappi's record of a minute, in J.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Minute {
    /// To the car, h1d to h3d and h1b to h3b, from surplus and from the grid, on every phase.
    charging: f64,
    /// From the grid and to it, through its grid CT clamp, imp and exp.
    imported: f64,
    exported: f64,
}

impl Minute {
    /// The record, and the unix timestamp of the minute it starts, from its own date and time, as a day's records may
    /// include the next day's first minute, see fetch::DaySummary::from_day. Fields which are zero are left out.
    fn from_record(record: &serde_json::Value) -> Option<(i64, Minute)> {
        let number = |name: &str| record.get(name).and_then(|value| value.as_f64()).unwrap_or_default();
        let part = |name: &str| record.get(name).and_then(|value| value.as_i64()).unwrap_or_default();
        let date = time::Date::from_calendar_date(
            part("yr") as i32,
            time::Month::try_from(part("mon") as u8).ok()?,
            part("dom") as u8,
        )
        .ok()?;
        let start = date.midnight().assume_utc().unix_timestamp() + part("hr") * 3600 + part("min") * 60;
        let charging = ["h1d", "h2d", "h3d", "h1b", "h2b", "h3b"]
            .iter()
            .map(|name| number(name))
            .sum();
        Some((
            start,
            Minute {
                charging,
                imported: number("imp"),
                exported: number("exp"),
            },
        ))
    }
}

/// The channel of a zappi's charging, e.g. myenergi:Z12345678, and of its grid clamp.
fn charging_channel(serial: &str) -> String {
    format!("myenergi:Z{serial}")
}

fn grid_channel(serial: &str) -> String {
    format!("myenergi:Z{serial}:grid")
}

/// The channels of the zappis in the latest status: each one's charging, and its grid clamp if it has one.
pub fn channels(store: &bruce_db::Store) -> Vec<Channel> {
    fetch::zappis(store)
        .into_iter()
        .flat_map(|(serial, grid)| {
            let mut channels = vec![Channel {
                id: charging_channel(&serial),
                description: format!("What zappi {serial} charged the car with"),
                use_: Use::car(),
            }];
            if grid {
                channels.push(Channel {
                    id: grid_channel(&serial),
                    description: format!("Zappi {serial}'s grid CT clamp"),
                    use_: Use::Grid,
                });
            }
            channels
        })
        .collect()
}

/// The zappis' records, each by the minute it starts, as unix timestamps.
pub struct ZappiMeasurements {
    zappis: Vec<(String, BTreeMap<i64, Minute>)>,
}

/// The longest gap in a zappi's records which is filled in, in minutes, see filled.
const MAX_FILLED: i64 = 2;

/// The record of the minute starting at minute, or if there is none, as myenergi's records have a minute missing here
/// and there, about 70 a day, the average of the minutes either side of a gap of up to MAX_FILLED minutes, as a
/// record is the energy of its minute, which is lost, and the car charges steadily. None for a longer gap.
fn filled(minutes: &BTreeMap<i64, Minute>, minute: i64) -> Option<Minute> {
    if let Some(record) = minutes.get(&minute) {
        return Some(*record);
    }
    let (before, previous) = minutes.range(..minute).next_back()?;
    let (after, next) = minutes.range(minute + 1..).next()?;
    if (after - before) / 60 - 1 > MAX_FILLED {
        return None;
    }
    Some(Minute {
        charging: (previous.charging + next.charging) / 2.0,
        imported: (previous.imported + next.imported) / 2.0,
        exported: (previous.exported + next.exported) / 2.0,
    })
}

impl ZappiMeasurements {
    /// The records of the minutes which overlap start to end, with how much of each does, None if any is missing,
    /// see filled.
    fn minutes(&self, serial: &str, start: i64, end: i64) -> Option<Vec<(f64, Minute)>> {
        let (_, minutes) = self.zappis.iter().find(|(zappi, _)| zappi == serial)?;
        (start.div_euclid(60) * 60..end)
            .step_by(60)
            .map(|minute| {
                let overlap = (end.min(minute + 60) - start.max(minute)) as f64 / 60.0;
                Some((overlap, filled(minutes, minute)?))
            })
            .collect()
    }

    /// The zappi of a channel, and whether it is its grid clamp's, None if it is not a zappi's.
    fn channel<'a>(channel: &'a str) -> Option<(&'a str, bool)> {
        let rest = channel.strip_prefix("myenergi:Z")?;
        Some(match rest.strip_suffix(":grid") {
            Some(serial) => (serial, true),
            None => (rest, false),
        })
    }
}

impl Measurements for ZappiMeasurements {
    fn name(&self) -> String {
        String::from(NAME)
    }

    fn channels(&self) -> Vec<String> {
        self.zappis
            .iter()
            .flat_map(|(serial, _)| [charging_channel(serial), grid_channel(serial)])
            .collect()
    }

    /// Whether every zappi has a record of each minute of the half hour, or a short gap's filled in, see filled.
    fn complete(&self, half_hour: i64) -> bool {
        self.zappis
            .iter()
            .all(|(serial, _)| self.minutes(serial, half_hour, half_hour + 1800).is_some())
    }

    fn energy(&self, channel: &str, start: i64, end: i64) -> Option<Flow> {
        let (serial, grid) = Self::channel(channel)?;
        let minutes = self.minutes(serial, start, end)?;
        let sum = |field: fn(&Minute) -> f64| {
            minutes
                .iter()
                .map(|(overlap, minute)| overlap * field(minute))
                .sum::<f64>()
                / JOULES_PER_KWH
        };
        Some(if grid {
            Flow {
                import: sum(|minute| minute.imported),
                export: sum(|minute| minute.exported),
            }
        } else {
            Flow {
                import: sum(|minute| minute.charging),
                export: 0.0,
            }
        })
    }

    /// The end of the latest minute every zappi has a record of.
    fn latest(&self) -> Option<i64> {
        self.zappis
            .iter()
            .map(|(_, minutes)| minutes.keys().next_back().map(|minute| minute + 60))
            .min()
            .flatten()
    }

    /// The average power of the minutes, as a record is a minute's energy.
    fn power(&self, channel: &str, start: i64, end: i64) -> Option<f64> {
        let flow = self.energy(channel, start, end)?;
        let seconds = (end - start).max(1) as f64;
        Some((flow.import - flow.export) * JOULES_PER_KWH / seconds)
    }
}

/// The zappis' records, read from the day files, which are large, off the page's thread.
pub struct ZappiMeasurementSource {
    pub store: Arc<bruce_db::Store>,
}

impl MeasurementSource for ZappiMeasurementSource {
    fn name(&self) -> &str {
        "myenergi"
    }

    fn load(&self, start: i64, end: i64) -> Pin<Box<dyn Future<Output = anyhow::Result<Arc<dyn Measurements>>>>> {
        let store = self.store.clone();
        Box::pin(async move {
            let zappis = tokio::task::spawn_blocking(move || load(&store, start, end)).await?;
            let measurements: Arc<dyn Measurements> = Arc::new(ZappiMeasurements { zappis });
            anyhow::Ok(measurements)
        })
    }
}

/// Each zappi's records from start to end, from the UTC days which have them.
fn load(store: &bruce_db::Store, start: i64, end: i64) -> Vec<(String, BTreeMap<i64, Minute>)> {
    let date = |time: i64| {
        time::OffsetDateTime::from_unix_timestamp(time)
            .map(|time| time.date())
            .ok()
    };
    let (Some(first), Some(last)) = (date(start), date(end)) else {
        return Vec::new();
    };
    fetch::zappis(store)
        .into_iter()
        .map(|(serial, _)| {
            let mut minutes = BTreeMap::new();
            let mut day = first;
            while day <= last {
                minutes.extend(
                    fetch::zappi_day(store, &serial, day)
                        .iter()
                        .filter_map(Minute::from_record)
                        .filter(|(minute, _)| *minute + 60 > start && *minute < end),
                );
                let Some(next) = day.next_day() else { break };
                day = next;
            }
            (serial, minutes)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-10-28 00:00 UTC.
    const DAY: i64 = 1_793_145_600;

    fn measurements() -> ZappiMeasurements {
        // Charging at 7.2 kW, importing 7.5 kW, every minute of the first hour of the day.
        let minutes = (0..60)
            .map(|minute| {
                (
                    DAY + minute * 60,
                    Minute {
                        charging: 7200.0 * 60.0,
                        imported: 7500.0 * 60.0,
                        exported: 0.0,
                    },
                )
            })
            .collect();
        ZappiMeasurements {
            zappis: vec![(String::from("12345678"), minutes)],
        }
    }

    #[test]
    fn a_record_is_of_the_minute_its_own_date_and_time_say() {
        let record =
            serde_json::json!({"yr": 2026, "mon": 10, "dom": 28, "hr": 1, "min": 2, "h1d": 60, "h2b": 30, "imp": 120});
        let (start, minute) = Minute::from_record(&record).unwrap();
        assert_eq!(start, DAY + 3600 + 120);
        assert_eq!(
            minute,
            Minute {
                charging: 90.0,
                imported: 120.0,
                exported: 0.0
            }
        );
    }

    #[test]
    fn its_channels_are_the_cars_charging_and_the_grid_clamp() {
        let measurements = measurements();
        let charging = measurements.energy("myenergi:Z12345678", DAY, DAY + 1800).unwrap();
        assert!((charging.import - 3.6).abs() < 1e-9, "{charging:?}");
        let grid = measurements.energy("myenergi:Z12345678:grid", DAY, DAY + 1800).unwrap();
        assert!((grid.import - 3.75).abs() < 1e-9, "{grid:?}");
        // Part of a minute is that part of its record.
        assert!((measurements.power("myenergi:Z12345678", DAY + 10, DAY + 20).unwrap() - 7200.0).abs() < 1e-9);
        assert_eq!(measurements.energy("tesla:site", DAY, DAY + 1800), None);
        assert_eq!(measurements.energy("myenergi:Z87654321", DAY, DAY + 1800), None);
    }

    #[test]
    fn a_gap_of_two_minutes_is_filled_in_and_a_longer_one_is_not() {
        let mut measurements = measurements();
        assert!(measurements.complete(DAY) && measurements.complete(DAY + 1800));
        // The minute after the gap at a different power, so that the filled minutes are the average.
        measurements.zappis[0].1.insert(
            DAY + 720,
            Minute {
                charging: 3600.0 * 60.0,
                imported: 0.0,
                exported: 0.0,
            },
        );
        measurements.zappis[0].1.remove(&(DAY + 600));
        measurements.zappis[0].1.remove(&(DAY + 660));
        assert!(measurements.complete(DAY));
        let charging = measurements.energy("myenergi:Z12345678", DAY + 600, DAY + 780).unwrap();
        // Two minutes at 5.4 kW, the average of 7.2 kW and 3.6 kW, then one at 3.6 kW.
        assert!(
            (charging.import - (2.0 * 5400.0 + 3600.0) / 60_000.0).abs() < 1e-9,
            "{charging:?}"
        );
        measurements.zappis[0].1.remove(&(DAY + 720));
        assert!(!measurements.complete(DAY));
        assert!(measurements.energy("myenergi:Z12345678", DAY, DAY + 1800).is_none());
        assert!(!measurements.complete(DAY + 3600), "no records at all");
    }
}
