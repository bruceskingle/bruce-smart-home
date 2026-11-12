//! Smart meter telemetry: a reading every 10 seconds from the meter, made available by a consumer access device such as
//! an Octopus Home Mini.
//!
//! The server only has the last 30 days, so older data exists only in the store. Each reading is kept as the server
//! gave it, in the profile's store, see store.rs and data_set/records.rs, by its time, with a `#fetched/<date>` record
//! of when each day was last fetched, and whether all of its missing hours were. Readings are only ever added, never
//! removed, so a reload which gets less than before cannot lose data, and a day which is too old to fetch again is
//! never fetched.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use bruce_graphql::types::DateTime;
use bruce_graphql::{GraphQLCaller, GraphQLQueryBuilder, ProgressMonitor};
use time_tz::Tz;

use crate::CacheConfig;
use crate::power::grid::{GridPoint, GridSample};
use crate::components::export::{ExportTable, ExportValue};
use crate::data_set::{CancelToken, call_unless_cancelled};
use super::rate_limit::FieldRateLimit;
use super::graphql::TelemetryGrouping;
use super::graphql::meter::meter_telemetry;

/// Which readings are missing is worked out an hour, about 360 readings, at a time.
const HOUR_SECONDS: i64 = 3600;
/// The most which is fetched in one request, as consecutive missing hours. The server returns 13 hours of 10 second
/// readings, but nothing for 18 hours and an error for 24, and only allows 125 requests an hour, so a day is fetched in
/// two requests, or three on the day the clocks go back.
const MAX_REQUEST_SECONDS: i64 = 12 * HOUR_SECONDS;
/// The rate limited field which provides the readings, as the server's rateLimitInfo names it.
const RATE_LIMITED_FIELD: &str = "Query.smartMeterTelemetry";
/// Each request also asks for this long either side of its hours, so that the last reading before the hour and the first
/// after it are cached with it, and a day's file has the last reading before the day and the first after it. The one
/// before gives the energy of the first reading, see energy_since, and the one after shows that the hour or day has all
/// its readings. They cannot be counted, as the meter reads about every 10.3 seconds, so about 1 in 35 of the 10 second
/// slots has no reading.
const BOUNDARY_SECONDS: i64 = 60;
const DAY_SECONDS: i64 = 24 * 3600;
/// How far back the server has data, 30 days, less an hour so that a request near the limit is not refused.
const SERVER_HISTORY_SECONDS: i64 = 30 * DAY_SECONDS - 3600;
/// A day is complete, and not fetched again unless reloaded, when it has the first reading after it, or if there is
/// none, e.g. as the Home Mini was offline at midnight, once it has been fetched this long after it ended.
const COMPLETE_AFTER_SECONDS: i64 = DAY_SECONDS;
/// A day which is not complete, e.g. today, is fetched again when its cache is older than this.
const REFRESH_SECONDS: i64 = 5 * 60;

/// A reading of the meter, the values are as the server gives them, see GridPoint for their units.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct TelemetryReading {
    /// When it was read, a unix timestamp.
    pub read_at: i64,
    /// The power at the time, in W.
    pub demand: Option<f64>,
    /// The energy imported since the reading before, in Wh.
    pub consumption_delta: Option<f64>,
    /// The cost of that energy, in pence, without and with tax.
    pub cost_delta: Option<f64>,
    pub cost_delta_with_tax: Option<f64>,
    /// The meter's import and export registers, in Wh.
    pub consumption: Option<f64>,
    pub export: Option<f64>,
}

fn number(value: Option<super::decimal::Decimal>) -> Option<f64> {
    value.and_then(|value| value.to_string().parse().ok())
}

impl GridSample for TelemetryReading {
    fn read_at(&self) -> i64 {
        self.read_at
    }

    fn demand(&self) -> Option<f64> {
        self.demand
    }

    fn consumption_delta(&self) -> Option<f64> {
        self.consumption_delta
    }

    fn consumption(&self) -> Option<f64> {
        self.consumption
    }

    fn export(&self) -> Option<f64> {
        self.export
    }
}

impl TelemetryReading {
    fn from(item: meter_telemetry::SmartMeterTelemetryType) -> Option<Self> {
        Some(TelemetryReading {
            read_at: item.read_at_?.unix_timestamp(),
            demand: number(item.demand_),
            consumption_delta: number(item.consumption_delta_),
            cost_delta: number(item.cost_delta_),
            cost_delta_with_tax: number(item.cost_delta_with_tax_),
            consumption: number(item.consumption_),
            export: number(item.export_),
        })
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|now| now.as_secs() as i64).unwrap_or_default()
}

/// The sort key of a day's #fetched record.
fn fetched_key(day: &bruce_graphql::types::Date) -> String {
    format!("#fetched/{day}")
}

/// When a day was last fetched, as a unix timestamp, 0 if not all its missing hours were, so that it is fetched again,
/// as a file dated the epoch was, None if it never has been.
fn modified(records: &crate::data_set::Records, day: &bruce_graphql::types::Date) -> anyhow::Result<Option<i64>> {
    let key = fetched_key(day);
    let stale = records.get(&key)?.map(|fetched| fetched.get("stale").and_then(|stale| stale.as_bool()) == Some(true));
    Ok(match stale {
        None => None,
        Some(true) => Some(0),
        Some(false) => records.fetched_at(&key)?.map(|at| at.unix_timestamp()),
    })
}

/// The readings from start up to end, as unix timestamps, made from those kept, which cannot be read are left out.
fn load(records: &crate::data_set::Records, start: i64, end: i64) -> anyhow::Result<BTreeMap<i64, TelemetryReading>> {
    let key = |time: i64| bruce_db::Micros(time * 1_000_000).to_text();
    Ok(records.range(key(start).as_str()..key(end).as_str())?.into_iter()
        .filter_map(|(_, item)| TelemetryReading::from(serde_json::from_value(item).ok()?))
        .map(|reading| (reading.read_at, reading))
        .collect())
}

/// The hours of a day which the server has, as their start times, and those of them which are missing readings.
struct DayHours {
    fetchable: Vec<i64>,
    missing: Vec<i64>,
    /// Whether the day does not need fetching, as it is complete, or was fetched recently.
    up_to_date: bool,
}

fn day_hours(readings: &BTreeMap<i64, TelemetryReading>, day_start: i64, day_end: i64, modified: Option<i64>, now: i64) -> DayHours {
    let fetch_from = day_start.max(now - SERVER_HISTORY_SECONDS);
    let fetch_to = day_end.min(now);
    let fetchable: Vec<i64> = if fetch_from < fetch_to {
        let first_hour = fetch_from - (fetch_from - day_start).rem_euclid(HOUR_SECONDS);
        (0..).map(|index| first_hour + index * HOUR_SECONDS).take_while(|hour| *hour < fetch_to).collect()
    }
    else {
        Vec::new()
    };
    // An hour has its readings if it has some, and there is one just after it, see BOUNDARY_SECONDS. The one after on its
    // own is not enough, as it is also the next hour's first reading.
    let has_reading_after = |time: i64| readings.range(time..time + BOUNDARY_SECONDS).next().is_some();
    let missing = fetchable.iter()
        .copied()
        .filter(|hour| readings.range(*hour..hour + HOUR_SECONDS).next().is_none() || !has_reading_after(hour + HOUR_SECONDS))
        .collect();
    // A file dated the epoch is from a fetch which did not get all the missing hours.
    let complete = has_reading_after(day_end) || modified.is_some_and(|modified| modified >= day_end + COMPLETE_AFTER_SECONDS);
    let recent = modified.is_some_and(|modified| modified > 0 && now - modified < REFRESH_SECONDS);
    DayHours { fetchable, missing, up_to_date: complete || recent }
}

/// The requests for hours, which are in order, as their start and end: a request for each run of consecutive hours, of
/// at most MAX_REQUEST_SECONDS.
fn requests(hours: &[i64]) -> Vec<(i64, i64)> {
    let mut requests: Vec<(i64, i64)> = Vec::new();
    for hour in hours {
        match requests.last_mut() {
            Some((start, end)) if *end == *hour && *end - *start < MAX_REQUEST_SECONDS => *end = hour + HOUR_SECONDS,
            _ => requests.push((*hour, hour + HOUR_SECONDS)),
        }
    }
    requests
}

/// Whether the hour starting at hour overlaps the period from start to end, as unix timestamps.
fn hour_overlaps(hour: i64, start: i64, end: i64) -> bool {
    hour < end && hour + HOUR_SECONDS > start
}

/// The readings of the meter device_id, its ESME or GSME ID, from start up to end, in time order, with the last one
/// before start if it is cached, fetched from the server where the cache does not have them and the server still does,
/// see the module comment. The days which
/// reload covers are fetched again whatever is cached. If cancel is set the hours not yet fetched are left out.
pub async fn fetch(account_number: &str, device_id: &str, start: &DateTime, end: &DateTime, reload: Option<(DateTime, DateTime)>,
    tz: &'static Tz, config: &CacheConfig, request_manager: &impl GraphQLCaller, progress: &(impl ProgressMonitor + ?Sized),
    cancel: &CancelToken) -> anyhow::Result<Vec<TelemetryReading>>
{
    let hash_key = format!("{account_number}#{device_id}#Telemetry");
    let date_format = time::format_description::parse("[day] [month repr:short] [year]").unwrap();
    let hour_format = time::format_description::parse("[hour]:[minute]").unwrap();
    let (start, end) = (start.unix_timestamp(), end.unix_timestamp());
    let mut result = Vec::new();
    // The last reading before start, for the energy of the first reading, see energy_since.
    let mut before = None;

    // Each day's cached readings and the requests for its missing hours, all worked out first so that the progress can
    // show the number of requests for the whole range.
    struct DayFetch {
        day_start: i64,
        day_end: i64,
        day: bruce_graphql::types::Date,
        readings: BTreeMap<i64, TelemetryReading>,
        /// The hours to fetch, and the day's missing hours, whether they are fetched or not.
        hours: Vec<i64>,
        missing: Vec<i64>,
        requests: Vec<(i64, i64)>,
    }
    let now = now();
    let records = crate::data_set::Records::open(config, &hash_key)?;
    let mut days = Vec::new();
    let mut day = DateTime::from_unix_timestamp(start)?.to_timezone(tz).to_date();
    loop {
        let (day_start, day_end) = (day.at_midnight(tz).unix_timestamp(), day.next_day().at_midnight(tz).unix_timestamp());
        if day_start >= end {
            break;
        }
        // With the readings just before and after it, see BOUNDARY_SECONDS.
        let readings = load(&records, day_start - BOUNDARY_SECONDS, day_end + BOUNDARY_SECONDS)?;
        let (mut hours, mut missing) = (Vec::new(), Vec::new());
        if day_end > now - SERVER_HISTORY_SECONDS {
            let day = day_hours(&readings, day_start, day_end, modified(&records, &day)?, now);
            // The missing hours in the range shown, unless the day is up to date, and every hour in the reload period.
            hours = day.fetchable.iter()
                .copied()
                .filter(|hour| (!day.up_to_date && day.missing.contains(hour) && hour_overlaps(*hour, start, end))
                    || reload.is_some_and(|(reload_start, reload_end)| hour_overlaps(*hour, reload_start.unix_timestamp(), reload_end.unix_timestamp())))
                .collect();
            hours.dedup();
            missing = day.missing;
        }
        // An offline profile only has what is cached.
        let requests = if config.offline { Vec::new() } else { requests(&hours) };
        days.push(DayFetch { day_start, day_end, day: day.clone(), readings, hours, missing, requests });
        day = day.next_day();
    }

    let force = reload.is_some();
    let total_requests: usize = days.iter().map(|day| day.requests.len()).sum();
    let mut request_number = 0;
    for DayFetch { day_start, day_end, day, mut readings, hours, missing, requests } in days {
        let mut fetched = 0;
        let mut error = None;
        for (request_start, request_end) in &requests {
            if cancel.is_cancelled() {
                break;
            }
            request_number += 1;
            let (from, to) = (DateTime::from_unix_timestamp(*request_start)?.to_timezone(tz), DateTime::from_unix_timestamp(*request_end)?.to_timezone(tz));
            let message = format!(
                "{} {} {}–{}, request {request_number} of {total_requests}",
                if force { "Reloading" } else { "Fetching" },
                from.format(&date_format).unwrap_or_default(),
                from.format(&hour_format).unwrap_or_default(),
                to.format(&hour_format).unwrap_or_default(),
            );
            progress.progress(message.clone());
            // With the readings just before and after the hours, see BOUNDARY_SECONDS.
            let query = meter_telemetry::Query::builder()
                .with_device_id(device_id.to_string())
                .with_start(DateTime::from_unix_timestamp(request_start - BOUNDARY_SECONDS)?)
                .with_end(DateTime::from_unix_timestamp(request_end + BOUNDARY_SECONDS)?)
                .with_grouping(TelemetryGrouping::TenSeconds)
                .build()?;
            // Retried after a delay if it fails with e.g. a network error, or if there have been too many requests,
            // once the rate limit ends, with the waits shown as progress.
            let rate_limit = FieldRateLimit { request_manager, field: RATE_LIMITED_FIELD };
            match call_unless_cancelled(request_manager, &query, cancel, progress, &message, &rate_limit).await {
                Ok(None) => break,
                Ok(Some(response)) => {
                    // Readings are added, never removed, see the module comment, each kept as the server gave it.
                    for item in response.smart_meter_telemetry_ {
                        let kept = serde_json::to_value(&item)?;
                        if let Some(reading) = TelemetryReading::from(item) {
                            if readings.get(&reading.read_at) != Some(&reading) {
                                records.put(&bruce_db::Micros(reading.read_at * 1_000_000).to_text(), kept)?;
                            }
                            readings.insert(reading.read_at, reading);
                        }
                    }
                    fetched += 1;
                },
                Err(request_error) => {
                    error = Some(request_error);
                    break;
                },
            }
        }
        if !hours.is_empty() {
            // What was fetched is kept even if the rest was not, but the day is only dated as fetched if all its
            // missing hours were, not just those in the range shown, so that the rest are fetched next time.
            let all_fetched = fetched == requests.len() && missing.iter().all(|hour| hours.contains(hour));
            records.put(&fetched_key(&day), if all_fetched { serde_json::json!({}) } else { serde_json::json!({ "stale": true }) })?;
        }
        if let Some(error) = error {
            return Err(error);
        }

        if before.is_none() {
            before = readings.range(..start).next_back().map(|(_, reading)| reading.clone());
        }
        // Only the day's own readings, the ones before and after it are also in the files of the days either side.
        let (from, to) = (start.max(day_start), end.min(day_end));
        result.extend(readings.into_values().filter(|reading| reading.read_at >= from && reading.read_at < to));
        if cancel.is_cancelled() {
            break;
        }
    }
    if let Some(before) = before {
        result.insert(0, before);
    }
    Ok(result)
}


/// The readings as a table, for export.
pub fn readings_export(readings: &[TelemetryReading], tz: &'static Tz) -> ExportTable {
    let value = |value: Option<f64>| value.map(ExportValue::number).unwrap_or(ExportValue::Empty);
    ExportTable {
        columns: ["Read At", "Demand (W)", "Consumption Delta (Wh)", "Cost Delta (p)", "Cost Delta With Tax (p)", "Consumption (Wh)", "Export (Wh)"]
            .iter().map(|column| column.to_string()).collect(),
        rows: readings.iter()
            .map(|reading| vec![
                ExportValue::unix_time(reading.read_at, tz),
                value(reading.demand),
                value(reading.consumption_delta),
                value(reading.cost_delta),
                value(reading.cost_delta_with_tax),
                value(reading.consumption),
                value(reading.export),
            ])
            .collect(),
    }
}

/// The periods as a table, for export.
pub fn points_export(points: &[GridPoint], tz: &'static Tz) -> ExportTable {
    ExportTable {
        columns: ["Start", "End", "Energy (kWh)", "Export (kWh)", "Average Demand (kW)", "Readings"].iter().map(|column| column.to_string()).collect(),
        rows: points.iter()
            .map(|point| vec![
                ExportValue::unix_time(point.start, tz),
                ExportValue::unix_time(point.end, tz),
                ExportValue::number(point.energy / 1000.0),
                ExportValue::number(point.export / 1000.0),
                point.demand.map(|demand| ExportValue::number(demand / 1000.0)).unwrap_or(ExportValue::Empty),
                ExportValue::number(point.readings as f64),
            ])
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{COMPLETE_AFTER_SECONDS, DAY_SECONDS, HOUR_SECONDS, SERVER_HISTORY_SECONDS, TelemetryReading, day_hours, requests};
    use crate::period::Period;
    use crate::power::grid::aggregate;

    fn reading(read_at: i64, demand: f64, consumption_delta: f64) -> TelemetryReading {
        TelemetryReading {
            read_at,
            demand: Some(demand),
            consumption_delta: Some(consumption_delta),
            cost_delta: None,
            cost_delta_with_tax: Some(consumption_delta * 0.03),
            consumption: None,
            export: None,
        }
    }

    /// A day, ending ten days before now.
    const NOW: i64 = 1_800_000_000;
    const DAY_START: i64 = NOW - 11 * DAY_SECONDS;
    const DAY_END: i64 = DAY_START + DAY_SECONDS;

    fn full_hour(hour: i64, readings: &mut BTreeMap<i64, TelemetryReading>) {
        for index in 0..HOUR_SECONDS / 10 {
            let read_at = hour + index * 10;
            readings.insert(read_at, reading(read_at, 1.0, 1.0));
        }
    }

    #[test]
    fn an_hour_is_missing_unless_it_has_readings_and_one_just_after_it_and_complete_or_recent_days_are_up_to_date() {
        let mut readings = BTreeMap::new();
        for hour in 0..24 {
            if hour != 5 {
                full_hour(DAY_START + hour * HOUR_SECONDS, &mut readings);
            }
        }
        // Readings missing from the middle of an hour do not matter, as long as there is one just after it.
        readings.remove(&(DAY_START + 7 * HOUR_SECONDS + 600));
        // Hour 4 has no reading just after it, as hour 5 is missing, and hour 23 none as there is none after the day.
        let day = day_hours(&readings, DAY_START, DAY_END, None, NOW);
        assert_eq!(day.fetchable.len(), 24);
        assert_eq!(day.missing, [DAY_START + 4 * HOUR_SECONDS, DAY_START + 5 * HOUR_SECONDS, DAY_START + 23 * HOUR_SECONDS]);
        // Never fetched, fetched the day after, or a fetch which did not finish: not up to date.
        for modified in [None, Some(DAY_END), Some(0)] {
            assert!(!day_hours(&readings, DAY_START, DAY_END, modified, NOW).up_to_date);
        }
        // Fetched a minute ago, or with no reading after it but fetched a day after it: up to date.
        assert!(day_hours(&readings, DAY_START, DAY_END, Some(NOW - 60), NOW).up_to_date);
        assert!(day_hours(&readings, DAY_START, DAY_END, Some(DAY_END + COMPLETE_AFTER_SECONDS), NOW).up_to_date);
        // With the first reading after the day it is complete, whenever it was fetched.
        readings.insert(DAY_END + 5, reading(DAY_END + 5, 1.0, 1.0));
        assert!(day_hours(&readings, DAY_START, DAY_END, Some(DAY_END), NOW).up_to_date);
    }

    #[test]
    fn hours_the_server_no_longer_has_or_does_not_have_yet_are_not_fetchable() {
        let readings = BTreeMap::new();
        // A day which is all too old.
        let old_start = NOW - 40 * DAY_SECONDS;
        assert!(day_hours(&readings, old_start, old_start + DAY_SECONDS, None, NOW).fetchable.is_empty());
        // A day which is partly too old starts at the first hour the server has.
        let limit = NOW - SERVER_HISTORY_SECONDS;
        let partly_start = limit - 3 * HOUR_SECONDS;
        assert_eq!(day_hours(&readings, partly_start, partly_start + DAY_SECONDS, None, NOW).fetchable.first(), Some(&limit));
        // Today is fetchable up to now, including the hour in progress.
        let today_start = NOW - 5 * HOUR_SECONDS - 600;
        assert_eq!(day_hours(&readings, today_start, today_start + DAY_SECONDS, None, NOW).fetchable.len(), 6);
    }

    #[test]
    fn readings_are_totalled_in_each_period_including_empty_ones() {
        let london = time_tz::timezones::db::europe::LONDON;
        let start = 1_790_000_000 - 1_790_000_000 % 60;
        let readings: Vec<TelemetryReading> = (0..12).map(|index| reading(start + index * 10, 1000.0 + index as f64, 2.0)).collect();
        let minutes = aggregate(&readings, start, start + 180, Period::Minute, london);
        assert_eq!(minutes.len(), 3);
        assert_eq!((minutes[0].readings, minutes[0].energy), (6, 12.0));
        assert_eq!(minutes[0].demand, Some(1002.5));
        assert_eq!(minutes[1].demand, Some(1008.5));
        assert_eq!((minutes[2].readings, minutes[2].demand), (0, None));
        let days = aggregate(&readings, start, start + 180, Period::Day, london);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].energy, 24.0);
    }

    #[test]
    fn energy_is_the_change_in_the_register_so_missing_readings_do_not_lose_any() {
        let london = time_tz::timezones::db::europe::LONDON;
        let start = 1_790_000_000 - 1_790_000_000 % 60;
        let with_register = |read_at: i64, consumption: f64, delta: f64| TelemetryReading { consumption: Some(consumption), ..reading(read_at, 1000.0, delta) };
        // Two readings missing after the second, whose delta only covers 10 seconds of the 30 since it.
        let readings = vec![
            with_register(start, 1000.0, 3.0),
            with_register(start + 10, 1003.0, 3.0),
            with_register(start + 40, 1012.0, 3.0),
            with_register(start + 50, 1015.0, 3.0),
        ];
        let minute = aggregate(&readings, start, start + 60, Period::Minute, london);
        // The first reading has no register reading before it, so uses its delta, then the register's 15 Wh.
        assert_eq!(minute[0].energy, 3.0 + 15.0);
        // At 10 seconds each reading lasts from the one before, so the one after the missing ones is 30 seconds long.
        let periods = aggregate(&readings, start, start + 60, Period::TenSeconds, london);
        let energies: Vec<f64> = periods.iter().map(|period| period.energy).collect();
        assert_eq!(energies, [3.0, 3.0, 9.0, 3.0]);
        let lengths: Vec<i64> = periods.iter().map(|period| period.end - period.start).collect();
        assert_eq!(lengths, [10, 10, 30, 10]);

        // From the second reading, the first is only used for the register reading before it, and is not counted.
        let from_second = aggregate(&readings, start + 10, start + 60, Period::Day, london);
        assert_eq!((from_second[0].energy, from_second[0].readings), (15.0, 3));
    }

    #[test]
    fn consecutive_hours_are_fetched_together_up_to_12_hours() {
        let hour = |index: i64| DAY_START + index * HOUR_SECONDS;
        // A 25 hour day, with 3 hours missing in the middle of it from another fetch.
        let hours: Vec<i64> = (0..25).filter(|index| !(5..8).contains(index)).map(hour).collect();
        assert_eq!(requests(&hours), [(hour(0), hour(5)), (hour(8), hour(20)), (hour(20), hour(25))]);
        assert_eq!(requests(&[]), []);
    }

    #[test]
    fn export_is_the_change_in_the_export_register() {
        let london = time_tz::timezones::db::europe::LONDON;
        let start = 1_790_000_000 - 1_790_000_000 % 60;
        let exporting = |read_at: i64, export: f64| TelemetryReading { export: Some(export), ..reading(read_at, -2000.0, 0.0) };
        let readings = vec![exporting(start - 10, 500.0), exporting(start, 505.0), exporting(start + 40, 527.0)];
        let minute = aggregate(&readings, start, start + 60, Period::Minute, london);
        assert_eq!((minute[0].export, minute[0].demand), (27.0, Some(-2000.0)));
        let periods = aggregate(&readings, start, start + 60, Period::TenSeconds, london);
        let exports: Vec<f64> = periods.iter().map(|period| period.export).collect();
        assert_eq!(exports, [5.0, 22.0]);
    }
}
