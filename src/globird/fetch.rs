//! Fetching GloBird data into the cache, and what the cached days hold.
//!
//! The usage and costs are cached a file for each NEM day and NMI, e.g. 2026/November/1#usage-<nmi>, holding the rows
//! of that day as GloBird sent them. GloBird only keeps a few weeks of readings, 35 days when first asked, so they
//! are kept for ever, as captured data is, and the last few days are fetched again, as the meter's readings arrive a
//! day or so late and may be corrected. A NEM day is the day in Australia's National Electricity Market time, UTC+10
//! all year, whatever the clocks in Sydney say, and GloBird's tariff periods are in NEM time too.

use std::collections::BTreeMap;

use anyhow::anyhow;
use serde::Deserialize;
use time::{Date, Duration, OffsetDateTime, UtcOffset};

use crate::CacheConfig;

use super::client::GlobirdClient;

/// The tariff's charge types, with their default colours, in the order they are shown, see crate::known_labels: those
/// of the low and standard rates of other suppliers.
pub const LABELS: [(&str, &str); 3] = [("Offpeak Usage", "#46ee43"), ("Shoulder Usage", "#f2c94c"), ("Peak Usage", "#f6671f")];

/// How many days back a fetch asks for, more than GloBird keeps, so that the first fetch gets all it has.
pub const FETCH_DAYS: i64 = 40;
/// How many of the latest days are fetched again, even if cached, as their readings may still arrive or change.
pub const REFETCH_DAYS: i64 = 7;

const CURRENT_USER_KEY: &str = "currentuser";
const METERS_KEY: &str = "readmeters-";
const USAGE_KEY: &str = "usage-";
const COST_KEY: &str = "cost-";

/// The offset of NEM time from UTC.
pub fn nem_offset() -> UtcOffset {
    UtcOffset::from_hms(10, 0, 0).unwrap()
}

/// The NEM day it is now.
pub fn nem_today() -> Date {
    crate::clock::now_utc().to_offset(nem_offset()).date()
}

/// The unix time of the start of a NEM day.
pub fn nem_day_start(date: Date) -> i64 {
    date.midnight().assume_offset(nem_offset()).unix_timestamp()
}

/// A date as GloBird gives one, e.g. 2026/11/01 or 2026-11-01T00:00:00.
fn parse_date(text: &str) -> Option<Date> {
    let format = time::macros::format_description!("[year]-[month]-[day]");
    Date::parse(&text.get(..10)?.replace('/', "-"), &format).ok()
}

/// A power service of an account, whose usage is fetched.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerService {
    pub account_service_id: i64,
    /// The NMI, Australia's identifier of a meter point.
    pub nmi: String,
}

/// The open power services in a currentuser reply's data. Gas services, and closed ones, have no usage to fetch.
pub fn power_services(user: &serde_json::Value) -> Vec<PowerService> {
    let mut services = Vec::new();
    for account in user.get("accounts").and_then(|accounts| accounts.as_array()).into_iter().flatten() {
        for service in account.get("services").and_then(|services| services.as_array()).into_iter().flatten() {
            let power = service.get("serviceType").and_then(|kind| kind.as_str()).is_some_and(|kind| kind.eq_ignore_ascii_case("power"));
            let closed = service.get("closedDate").is_some_and(|date| !date.is_null());
            let id = service.get("accountServiceId").and_then(|id| id.as_i64());
            let nmi = service.get("siteIdentifier").and_then(|nmi| nmi.as_str());
            if let (true, false, Some(account_service_id), Some(nmi)) = (power, closed, id, nmi) {
                services.push(PowerService { account_service_id, nmi: nmi.to_string() });
            }
        }
    }
    services
}

/// The serial number of the energised smart meter in a readmeters reply's data, whose readings are fetched.
pub fn smart_meter_serial(meters: &serde_json::Value) -> Option<String> {
    meters.as_array()?.iter()
        .filter(|meter| meter.get("meterReadType").and_then(|kind| kind.as_str()).is_some_and(|kind| kind.eq_ignore_ascii_case("smart")))
        .filter(|meter| meter.get("serialStatus").and_then(|status| status.as_str()).is_some_and(|status| status.to_lowercase().starts_with("energi")))
        .find_map(|meter| Some(meter.get("serialNumber")?.as_str()?.to_string()))
}

/// A time of day of a tariff period, in NEM time.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimePeriod {
    /// e.g. 15:00:00
    pub start_time: String,
    /// e.g. 21:00:00, or 1.00:00:00 for the next midnight.
    pub end_time: String,
}

/// Minutes from midnight of a period's start or end, e.g. 900 for 15:00:00, 1440 for 1.00:00:00.
fn minutes(text: &str) -> Option<u32> {
    let (days, clock) = text.rsplit_once('.').map_or((0, text), |(days, clock)| (days.parse().unwrap_or(0), clock));
    let mut parts = clock.split(':');
    let hours: u32 = parts.next()?.parse().ok()?;
    let mins: u32 = parts.next()?.parse().ok()?;
    Some(days * 1440 + hours * 60 + mins)
}

impl TimePeriod {
    /// Whether the minute of the day, from midnight, is in the period, which may go past midnight, e.g. 21:00 to 01:00.
    fn contains(&self, minute: u32) -> bool {
        match (minutes(&self.start_time), minutes(&self.end_time)) {
            (Some(start), Some(end)) if start < end => start <= minute && minute < end,
            (Some(start), Some(end)) => minute >= start || minute < end,
            _ => false,
        }
    }
}

/// A row of the usage: a day's readings of one of the meter's registers, with one of its charge types, e.g. Peak Usage,
/// and the times of day of that charge type. The whole day's readings are on each of a register's rows, and usage is
/// the sum of those in the row's periods.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    /// The NEM day, e.g. 2026/11/01.
    pub read_date: String,
    /// The register, E1 for import, B1 for export.
    pub suffix: Option<String>,
    /// e.g. Peak Usage, Offpeak Usage, Standard Solar Feed in.
    pub charge_type: Option<String>,
    /// kWh in the row's periods.
    pub usage: Option<f64>,
    /// The day's 5 minute readings in kWh, from midnight NEM time, 288 of them.
    #[serde(default)]
    pub usage_array: Vec<Option<f64>>,
    #[serde(default)]
    pub time_periods: Vec<TimePeriod>,
    /// How the readings were got, A for actual readings.
    pub min_quality_method: Option<String>,
}

impl UsageRow {
    pub fn date(&self) -> Option<Date> {
        parse_date(&self.read_date)
    }

    pub fn is_import(&self) -> bool {
        self.suffix.as_deref().is_some_and(|suffix| suffix.starts_with('E'))
    }

    fn contains_slot(&self, slot: usize) -> bool {
        self.time_periods.iter().any(|period| period.contains(slot as u32 * 5))
    }
}

/// A row of the costs: a day's cost of one category.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostRow {
    /// e.g. USAGE, SUPPLY, SOLAR, ZEROHERO Credit, Super Export top up.
    pub charge_category: Option<String>,
    /// The NEM day, e.g. 2026/11/01.
    pub date: String,
    /// In AUD, negative for a credit.
    pub amount: Option<f64>,
    pub is_meter_read_missing_on_date: Option<bool>,
}

impl CostRow {
    pub fn date(&self) -> Option<Date> {
        parse_date(&self.date)
    }
}

/// A 5 minute reading of a register, with the charge type it is billed at.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// Unix time of its start.
    pub start: i64,
    pub end: i64,
    /// E1 for import, B1 for export.
    pub register: String,
    /// The charge type whose periods it is in, e.g. Peak Usage, empty if none is.
    pub band: String,
    pub kwh: f64,
}

/// The readings of the rows of a day, each register's once, each with the charge type whose periods it is in.
pub fn readings(rows: &[UsageRow]) -> Vec<Reading> {
    let mut by_register: BTreeMap<(Date, &str), Vec<&UsageRow>> = BTreeMap::new();
    for row in rows {
        if let (Some(date), Some(register)) = (row.date(), row.suffix.as_deref()) {
            by_register.entry((date, register)).or_default().push(row);
        }
    }
    let mut readings = Vec::new();
    for ((date, register), rows) in by_register {
        let day_start = nem_day_start(date);
        for (slot, kwh) in rows[0].usage_array.iter().enumerate() {
            let Some(kwh) = kwh else { continue };
            let band = rows.iter().find(|row| row.contains_slot(slot)).and_then(|row| row.charge_type.clone()).unwrap_or_default();
            let start = day_start + slot as i64 * 300;
            readings.push(Reading { start, end: start + 300, register: register.to_string(), band, kwh: *kwh });
        }
    }
    readings
}

/// What a cached day holds, for the Days page.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DaySummary {
    pub date: Option<Date>,
    /// kWh imported, by charge type, e.g. Peak Usage.
    pub import: BTreeMap<String, f64>,
    pub export: f64,
    /// How many 5 minute import readings there are, 288 for a whole day.
    pub readings: usize,
    /// Whether any readings are not actual ones, e.g. estimated.
    pub estimated: bool,
    /// AUD, by category, e.g. USAGE.
    pub cost: BTreeMap<String, f64>,
    /// Whether GloBird says a meter read is missing for the day's costs.
    pub cost_read_missing: bool,
}

impl DaySummary {
    pub fn total_cost(&self) -> f64 {
        self.cost.values().sum()
    }
}

/// The summary of each day of the usage and cost rows, newest first.
pub fn summarise(usage: &[UsageRow], cost: &[CostRow]) -> Vec<DaySummary> {
    let mut days: BTreeMap<Date, DaySummary> = BTreeMap::new();
    let mut counted: std::collections::HashSet<(Date, String)> = std::collections::HashSet::new();
    for row in usage {
        let Some(date) = row.date() else { continue };
        let day = days.entry(date).or_insert_with(|| DaySummary { date: Some(date), ..Default::default() });
        let kwh = row.usage.unwrap_or_default();
        if row.is_import() {
            *day.import.entry(row.charge_type.clone().unwrap_or_default()).or_default() += kwh;
            // The same readings are on each of a register's rows.
            if counted.insert((date, row.suffix.clone().unwrap_or_default())) {
                day.readings += row.usage_array.iter().flatten().count();
            }
        }
        else {
            day.export += kwh;
        }
        day.estimated |= row.min_quality_method.as_deref().is_some_and(|method| method != "A");
    }
    for row in cost {
        let Some(date) = row.date() else { continue };
        let day = days.entry(date).or_insert_with(|| DaySummary { date: Some(date), ..Default::default() });
        *day.cost.entry(row.charge_category.clone().unwrap_or_default()).or_default() += row.amount.unwrap_or_default();
        day.cost_read_missing |= row.is_meter_read_missing_on_date == Some(true);
    }
    days.into_values().rev().collect()
}

fn write_json(path: &std::path::Path, json: &serde_json::Value) -> anyhow::Result<()> {
    // Compact, as the other modules' cache files are, which Cache Files shows pretty printed.
    serde_json::to_writer(crate::private_file::create_private_file(path)?, json)?;
    Ok(())
}

/// Every row of the cached files of key, e.g. every day's usage.
fn cached_rows<T: serde::de::DeserializeOwned>(cache_config: &CacheConfig, key: &str) -> Vec<T> {
    let mut rows = Vec::new();
    for path in cache_config.files_with_key(key) {
        match CacheConfig::read_json(&path).map(serde_json::from_value::<Vec<T>>) {
            Some(Ok(day)) => rows.extend(day),
            _ => crate::log_warning!("Unable to read the GloBird cache file {}", path.display()),
        }
    }
    rows
}

/// The readings of every cached day, see readings.
pub fn cached_readings(cache_config: &CacheConfig) -> Vec<Reading> {
    readings(&cached_rows(cache_config, USAGE_KEY))
}

/// A meter, as the cached readmeters replies list it.
#[derive(Debug, Clone, PartialEq)]
pub struct Meter {
    pub serial: String,
    /// e.g. SMART.
    pub read_type: String,
    /// e.g. Energized, Removed.
    pub status: String,
}

/// The meters of every power service, see Meter.
pub fn meters(cache_config: &CacheConfig) -> Vec<Meter> {
    let text = |meter: &serde_json::Value, field: &str| meter.get(field).and_then(|value| value.as_str()).unwrap_or_default().to_string();
    // Saved by hash key alone, see fetch, so not found by files_with_key, which looks for day and month files.
    let Ok(entries) = std::fs::read_dir(cache_config.dir_path()) else { return Vec::new() };
    entries.flatten()
        .filter(|entry| entry.file_name().to_str().is_some_and(|name| name.starts_with(METERS_KEY)))
        .filter_map(|entry| CacheConfig::read_json(&entry.path()))
        .flat_map(|meters| meters.as_array().cloned().unwrap_or_default())
        .map(|meter| Meter { serial: text(&meter, "serialNumber"), read_type: text(&meter, "meterReadType"), status: text(&meter, "serialStatus") })
        .collect()
}

/// A register of the meter, as the cached readings have it.
#[derive(Debug, Clone, PartialEq)]
pub struct Register {
    /// e.g. E1 for import, B1 for export.
    pub name: String,
    /// Its charge types, e.g. Peak Usage.
    pub bands: Vec<String>,
    /// How many days of its readings are cached.
    pub days: usize,
}

impl Register {
    pub fn is_import(&self) -> bool {
        self.name.starts_with('E')
    }
}

/// Each register of the cached readings, see Register.
pub fn registers(cache_config: &CacheConfig) -> Vec<Register> {
    let mut registers: BTreeMap<String, (std::collections::BTreeSet<String>, std::collections::BTreeSet<String>)> = BTreeMap::new();
    for row in cached_rows::<UsageRow>(cache_config, USAGE_KEY) {
        let Some(name) = row.suffix.clone() else { continue };
        let (bands, days) = registers.entry(name).or_default();
        bands.extend(row.charge_type.clone());
        days.insert(row.read_date.clone());
    }
    registers.into_iter()
        .map(|(name, (bands, days))| Register { name, bands: bands.into_iter().collect(), days: days.len() })
        .collect()
}

/// The summary of every cached day, newest first.
pub fn summaries(cache_config: &CacheConfig) -> Vec<DaySummary> {
    summarise(&cached_rows(cache_config, USAGE_KEY), &cached_rows(cache_config, COST_KEY))
}

/// Saves the rows of a reply's data, a list of rows each with a NEM day in date_field, a file for each day, e.g.
/// 2026/November/1#usage-<nmi>, returning how many days.
fn save_days(cache_config: &CacheConfig, key: &str, data: &serde_json::Value, date_field: &str) -> anyhow::Result<usize> {
    let mut days: BTreeMap<Date, Vec<serde_json::Value>> = BTreeMap::new();
    for row in data.as_array().into_iter().flatten() {
        if let Some(date) = row.get(date_field).and_then(|date| date.as_str()).and_then(parse_date) {
            days.entry(date).or_default().push(row.clone());
        }
    }
    for (date, rows) in &days {
        write_json(&cache_config.path_for_day(key, &(*date).into())?, &serde_json::Value::Array(rows.clone()))?;
    }
    Ok(days.len())
}

/// The first day from today - FETCH_DAYS which needs fetching: one which is not cached, or one of the last
/// REFETCH_DAYS days. None if none does.
fn first_day_to_fetch(cache_config: &CacheConfig, key: &str, today: Date) -> Option<Date> {
    (0..=FETCH_DAYS).rev()
        .map(|days_ago| today - Duration::days(days_ago))
        .find(|date| *date > today - Duration::days(REFETCH_DAYS) || !cache_config.existing_path_for_day(key, &(*date).into()).exists())
}

/// Fetches the account, its power services' meters, and the usage and costs of every day which needs it, see
/// first_day_to_fetch, saying what it did with progress.
pub async fn fetch(client: &GlobirdClient, cache_config: &CacheConfig, progress: &mut dyn FnMut(String)) -> anyhow::Result<()> {
    if cache_config.offline {
        return Err(anyhow!("This profile is offline, so nothing can be fetched"));
    }
    progress("Fetching the account".to_string());
    let user = client.current_user().await?;
    write_json(&cache_config.path_for_hash_key(CURRENT_USER_KEY), &user)?;
    let services = power_services(&user);
    if services.is_empty() {
        progress("The account has no open power service".to_string());
    }
    let today = nem_today();
    for service in &services {
        let meters = client.read_meters(service.account_service_id).await?;
        write_json(&cache_config.path_for_hash_key(&format!("{METERS_KEY}{}", service.account_service_id)), &meters)?;
        let Some(serial) = smart_meter_serial(&meters) else {
            progress(format!("Service {} has no energised smart meter, so it has no readings", service.account_service_id));
            continue;
        };
        let usage_key = format!("{USAGE_KEY}{}", service.nmi);
        if let Some(from) = first_day_to_fetch(cache_config, &usage_key, today) {
            progress(format!("Fetching the readings from {from} to {today}"));
            let usage = client.usage(service.account_service_id, &service.nmi, &serial, from, today).await?;
            let days = save_days(cache_config, &usage_key, &usage, "readDate")?;
            progress(format!("Saved the readings of {days} day(s)"));
        }
        let cost_key = format!("{COST_KEY}{}", service.nmi);
        if let Some(from) = first_day_to_fetch(cache_config, &cost_key, today) {
            progress(format!("Fetching the costs from {from} to {today}"));
            let cost = client.cost(service.account_service_id, &service.nmi, from, today).await?;
            let days = save_days(cache_config, &cost_key, &cost, "date")?;
            progress(format!("Saved the costs of {days} day(s)"));
        }
    }
    progress(format!("Fetched {} power service(s)", services.len()));
    Ok(())
}

/// The end of the latest cached reading, to say how up to date they are.
pub fn latest_reading(cache_config: &CacheConfig) -> Option<OffsetDateTime> {
    let end = cached_readings(cache_config).iter().map(|reading| reading.end).max()?;
    OffsetDateTime::from_unix_timestamp(end).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(json: &str) -> serde_json::Value {
        serde_json::from_str::<serde_json::Value>(json).unwrap()["data"].clone()
    }

    fn usage() -> Vec<UsageRow> {
        serde_json::from_value(data(include_str!("../../testdata/globird/usage.json"))).unwrap()
    }

    fn cost() -> Vec<CostRow> {
        serde_json::from_value(data(include_str!("../../testdata/globird/cost.json"))).unwrap()
    }

    #[test]
    fn the_power_service_and_its_smart_meter_are_found() {
        let services = power_services(&data(include_str!("../../testdata/globird/currentuser.json")));
        assert_eq!(services.len(), 1, "only the power service, not the gas one");
        assert!(!services[0].nmi.is_empty());
        let serial = smart_meter_serial(&data(include_str!("../../testdata/globird/readmeters.json")));
        assert!(serial.is_some(), "the energised smart meter, not the removed one");
    }

    #[test]
    fn each_band_of_readings_adds_up_to_its_rows_usage() {
        let rows = usage();
        let readings = readings(&rows);
        assert_eq!(readings.len(), 3 * 2 * 288, "three days of two registers");
        for row in &rows {
            let start = nem_day_start(row.date().unwrap());
            let sum: f64 = readings.iter()
                .filter(|reading| Some(&reading.register) == row.suffix.as_ref() && Some(&reading.band) == row.charge_type.as_ref())
                .filter(|reading| reading.start >= start && reading.start < start + 86400)
                .map(|reading| reading.kwh).sum();
            assert!((sum - row.usage.unwrap()).abs() < 0.002, "{} {:?} {:?}: {sum} != {:?}", row.read_date, row.suffix, row.charge_type, row.usage);
        }
    }

    #[test]
    fn a_nem_day_starts_at_midnight_utc_plus_10() {
        let first = readings(&usage()).iter().map(|reading| reading.start).min().unwrap();
        assert_eq!(OffsetDateTime::from_unix_timestamp(first).unwrap().to_string(), "2026-10-31 14:00:00.0 +00:00:00");
    }

    #[test]
    fn periods_past_midnight_and_to_the_next_midnight_are_understood() {
        let late = TimePeriod { start_time: "21:00:00".to_string(), end_time: "01:00:00".to_string() };
        assert!(late.contains(23 * 60) && late.contains(30) && !late.contains(60) && !late.contains(20 * 60));
        let all = TimePeriod { start_time: "00:00:00".to_string(), end_time: "1.00:00:00".to_string() };
        assert!(all.contains(0) && all.contains(1439));
    }

    #[test]
    fn days_are_summarised_newest_first_with_their_costs() {
        let costs = cost();
        let days = summarise(&usage(), &costs);
        assert_eq!(days.iter().map(|day| day.date.unwrap().to_string()).collect::<Vec<_>>(), ["2026-11-03", "2026-11-02", "2026-11-01"]);
        assert!(days.iter().all(|day| day.readings == 288 && !day.estimated));
        let first = &days[2];
        assert!(first.import.contains_key("Peak Usage") && first.import.contains_key("Offpeak Usage"));
        let expected: f64 = costs.iter().filter(|row| row.date().map(|date| date.to_string()).as_deref() == Some("2026-11-01"))
            .filter_map(|row| row.amount).sum();
        assert!((first.total_cost() - expected).abs() < 1e-9);
    }
}
