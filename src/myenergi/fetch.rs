//! Fetching myenergi data into the cache, and what the cached days add up to.
//!
//! For now this is only to see what the API returns: the status as it is each time, and each device's records of a
//! few days, a file for each device and UTC day. A day is fetched again until it has ended.

use std::path::Path;

use anyhow::anyhow;
use time::{Date, Duration};

use crate::CacheConfig;

use super::client::{Device, MyenergiClient};

/// How many days are fetched, including today.
pub const DAYS: i64 = 3;

const STATUS_KEY: &str = "status";
const DAY_KEY: &str = "jday-";

fn write_json(path: &Path, json: &serde_json::Value) -> anyhow::Result<()> {
    // Compact, one record a line, as the Octopus cache files are, which Cache Files shows pretty printed.
    serde_json::to_writer(crate::private_file::create_private_file(path)?, json)?;
    Ok(())
}

/// Fetches the status into the cache, which lists the devices.
async fn fetch_status(client: &MyenergiClient, cache_config: &CacheConfig, slot: Option<time::OffsetDateTime>,
    progress: &mut dyn FnMut(String)) -> anyhow::Result<serde_json::Value> {
    let rfc3339 = &time::format_description::well_known::Rfc3339;
    progress("Fetching the status".to_string());
    let requested = crate::clock::now_utc();
    let status = client.status().await?;
    let received = crate::clock::now_utc();
    let saved = serde_json::json!({
        "slot": slot.map(|slot| slot.format(rfc3339)).transpose()?,
        "requested": requested.format(rfc3339)?,
        "received": received.format(rfc3339)?,
        "response": status,
    });
    write_json(&status_path(cache_config, slot.unwrap_or(requested))?, &saved)?;
    Ok(status)
}

/// The cache file of the status of time, e.g. 2026/October/28#status-134000.
fn status_path(cache_config: &CacheConfig, time_of: time::OffsetDateTime) -> anyhow::Result<std::path::PathBuf> {
    let time = time_of.time();
    let key = format!("{STATUS_KEY}-{:02}{:02}{:02}", time.hour(), time.minute(), time.second());
    cache_config.path_for_day(&key, &time_of.date().into())
}

/// Saves a status taken elsewhere, e.g. by a capture device, in the same form as fetch_status, unless there is one for
/// its time already, returning whether it was saved.
pub fn save_status_if_new(cache_config: &CacheConfig, saved: &serde_json::Value) -> anyhow::Result<bool> {
    let time = saved_time(saved).ok_or_else(|| anyhow!("the status has no time"))?;
    let path = status_path(cache_config, time)?;
    if path.exists() {
        return Ok(false);
    }
    write_json(&path, saved)?;
    Ok(true)
}

/// The time of the latest status in the cache.
pub fn latest_status_time(cache_config: &CacheConfig) -> Option<time::OffsetDateTime> {
    saved_time(&cache_config.latest_json(STATUS_KEY)?)
}

/// The status in a saved status, which is {"slot": ..., "requested": ..., "received": ..., "response": status}, see
/// fetch_status, or for one saved before those were recorded the status itself.
fn saved_status(saved: &serde_json::Value) -> &serde_json::Value {
    saved.get("response").unwrap_or(saved)
}

/// When a saved status is of: the slot of a scheduled poll, otherwise when it was requested, None for one saved before
/// those were recorded.
fn saved_time(saved: &serde_json::Value) -> Option<time::OffsetDateTime> {
    ["slot", "requested"].iter()
        .find_map(|name| saved.get(*name)?.as_str())
        .and_then(|time| time::OffsetDateTime::parse(time, &time::format_description::well_known::Rfc3339).ok())
}

/// Fetches the device's records of the UTC day date into the cache, unless the day has ended and they are cached
/// already, or force. Says how many minutes have records, so that a day long ago shows whether myenergi still has it.
async fn fetch_day(client: &MyenergiClient, cache_config: &CacheConfig, device: &Device, date: Date, force: bool,
    progress: &mut dyn FnMut(String)) -> anyhow::Result<()> {
    let today = crate::clock::now_utc().date();
    let path = cache_config.path_for_day(&format!("{DAY_KEY}{}{}", device.letter, device.serial), &date.into())?;
    if date < today && path.exists() && !force {
        return Ok(());
    }
    progress(format!("Fetching {} {} {date}", device.kind, device.serial));
    match client.day(device, date).await {
        Ok(json) => {
            write_json(&path, &json)?;
            let minutes = json.as_object().and_then(|object| object.values().next()).and_then(|records| records.as_array()).map_or(0, Vec::len);
            progress(format!("{} {} {date} has {minutes} minutes of records", device.kind, device.serial));
        },
        // e.g. a harvi, which may not have records of its own.
        Err(error) => {
            crate::log_warning!("Unable to fetch {} {} {date}, {error:#}", device.kind, device.serial);
            progress(format!("Unable to fetch {} {} {date}, {error:#}", device.kind, device.serial));
        },
    }
    Ok(())
}

/// Fetches the status and the last DAYS days of every device which has records, saying what it did with progress.
pub async fn fetch(client: &MyenergiClient, cache_config: &CacheConfig, progress: &mut dyn FnMut(String)) -> anyhow::Result<()> {
    if cache_config.offline {
        return Err(anyhow!("This profile is offline, so nothing can be fetched"));
    }
    let today = crate::clock::now_utc().date();
    let devices = Device::from_status(&fetch_status(client, cache_config, None, progress).await?);
    if devices.is_empty() {
        progress("The status lists no devices".to_string());
    }
    for device in &devices {
        for days_ago in 0..DAYS {
            fetch_day(client, cache_config, device, today - Duration::days(days_ago), false, progress).await?;
        }
    }
    progress(format!("Fetched {} device(s) at {}", devices.len(), client.host()));
    Ok(())
}

/// How many minutes of records poll fetches: those since the last poll and some more, for records which arrive late.
const RECENT_MINUTES: i64 = 15;

/// What a zappi's plug status, pst, means.
fn plug_state(pst: &str) -> &str {
    match pst {
        "A" => "no car",
        "B1" => "car connected",
        "B2" => "waiting for the car",
        "C1" => "ready to charge",
        "C2" => "charging",
        "F" => "fault",
        other => other,
    }
}

/// Merges the records in json, a response of MyenergiClient::part_of_day, into the device's cache file for the day date,
/// replacing those of the same minutes.
fn merge_into_day(cache_config: &CacheConfig, device: &Device, date: Date, json: &serde_json::Value) -> anyhow::Result<()> {
    let Some((name, new)) = json.as_object().and_then(|object| object.iter().next()) else { return Ok(()) };
    let path = cache_config.path_for_day(&format!("{DAY_KEY}{}{}", device.letter, device.serial), &date.into())?;
    let part = |record: &serde_json::Value, name: &str| record.get(name).and_then(|value| value.as_i64()).unwrap_or_default();
    // By the minute, which with the date also orders the next day's first minute last, as a whole day's response does.
    let minute = |record: &serde_json::Value| (part(record, "yr"), part(record, "mon"), part(record, "dom"), part(record, "hr"), part(record, "min"));
    let mut records: std::collections::BTreeMap<_, serde_json::Value> = CacheConfig::read_json(&path)
        .and_then(|existing| existing.as_object()?.values().next()?.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|record| (minute(&record), record))
        .collect();
    for record in new.as_array().into_iter().flatten() {
        records.insert(minute(record), record.clone());
    }
    let records: Vec<serde_json::Value> = records.into_values().collect();
    write_json(&path, &serde_json::json!({ name: records }))
}

/// Fetches the device's records of the last RECENT_MINUTES minutes before now into the cache, two parts if they start
/// yesterday.
async fn fetch_recent(client: &MyenergiClient, cache_config: &CacheConfig, device: &Device, now: time::OffsetDateTime) -> anyhow::Result<()> {
    for (from, count) in recent_parts(now) {
        let json = client.part_of_day(device, from.date(), from.hour(), from.minute(), count as u16).await?;
        merge_into_day(cache_config, device, from.date(), &json)?;
    }
    Ok(())
}

/// The start and number of minutes of each part of a day in the last RECENT_MINUTES minutes before now, two if they
/// start yesterday.
fn recent_parts(now: time::OffsetDateTime) -> Vec<(time::OffsetDateTime, i64)> {
    let start = now - Duration::minutes(RECENT_MINUTES);
    let minute_of_day = |time: time::OffsetDateTime| time.hour() as i64 * 60 + time.minute() as i64;
    let parts = if start.date() == now.date() {
        vec![(start, minute_of_day(now) - minute_of_day(start) + 1)]
    }
    else {
        let midnight = now.replace_time(time::Time::MIDNIGHT);
        vec![(start, 24 * 60 - minute_of_day(start)), (midnight, minute_of_day(now) + 1)]
    };
    parts
}

/// Fetches the status for the slot, see Module::poll, and the records of the last few minutes. On the first poll, see
/// whole_days_fetched, it fetches the whole of today and yesterday, for the time the app was closed, and after that
/// the whole of yesterday once each new day, for records which arrived late. Returns what each zappi is doing, e.g.
/// "zappi 12345678: car connected, 3.4 kWh added, 0.00 kW". The kWh added since the car was plugged in, che, are kept
/// until it is plugged in again, so polling often enough catches every charging session's total.
///
/// If captured, a capture device has polled the status, see MyenergiModule::poll, so the latest saved is used instead
/// of fetching it.
pub async fn poll(client: &MyenergiClient, cache_config: &CacheConfig, slot: Option<time::OffsetDateTime>,
    whole_days_fetched: &std::sync::Mutex<Option<Date>>, captured: bool) -> anyhow::Result<String> {
    if cache_config.offline {
        return Err(anyhow!("This profile is offline, so nothing can be fetched"));
    }
    let latest = if captured { cache_config.latest_json(STATUS_KEY).map(|saved| saved_status(&saved).clone()) } else { None };
    let status = match latest {
        Some(status) => status,
        None => fetch_status(client, cache_config, slot, &mut |_| {}).await?,
    };
    let now = crate::clock::now_utc();
    let today = now.date();
    let last_fetched = *whole_days_fetched.lock().unwrap();
    let whole_days: Vec<Date> = match last_fetched {
        None => vec![today - Duration::days(1), today],
        Some(date) if date < today => vec![today - Duration::days(1)],
        Some(_) => Vec::new(),
    };
    for device in &Device::from_status(&status) {
        for date in &whole_days {
            fetch_day(client, cache_config, device, *date, true, &mut |_| {}).await?;
        }
        fetch_recent(client, cache_config, device, now).await?;
    }
    *whole_days_fetched.lock().unwrap() = Some(today);
    let zappis: Vec<String> = status.as_array().into_iter().flatten()
        .filter_map(|item| item.get("zappi")?.as_array())
        .flatten()
        .map(|zappi| {
            let pst = zappi.get("pst").and_then(|value| value.as_str()).unwrap_or_default();
            let che = zappi.get("che").and_then(|value| value.as_f64()).unwrap_or_default();
            let power = zappi.get("div").and_then(|value| value.as_f64()).unwrap_or_default();
            format!("zappi {}: {}, {che} kWh added, {:.2} kW", zappi.get("sno").map(|sno| sno.to_string()).unwrap_or_default(), plug_state(pst), power / 1000.0)
        })
        .collect();
    let mut summary = if zappis.is_empty() { "No zappi".to_string() } else { zappis.join("; ") };
    if !whole_days.is_empty() {
        summary.push_str(&format!(", and fetched the whole of {}", whole_days.iter().map(Date::to_string).collect::<Vec<_>>().join(" and ")));
    }
    Ok(summary)
}

/// Fetches the UTC day date of every device in the latest status, fetching the status first if there is none. The
/// day is fetched again even if it is cached, to see what myenergi has now.
pub async fn fetch_date(client: &MyenergiClient, cache_config: &CacheConfig, date: Date, progress: &mut dyn FnMut(String)) -> anyhow::Result<()> {
    if cache_config.offline {
        return Err(anyhow!("This profile is offline, so nothing can be fetched"));
    }
    if date > crate::clock::now_utc().date() {
        return Err(anyhow!("{date} has not happened yet"));
    }
    let status = match cache_config.latest_json(STATUS_KEY) {
        Some(saved) => saved_status(&saved).clone(),
        None => fetch_status(client, cache_config, None, progress).await?,
    };
    for device in &Device::from_status(&status) {
        fetch_day(client, cache_config, device, date, true, progress).await?;
    }
    Ok(())
}

/// The totals of a device's records of a day, in kWh.
#[derive(Debug, Clone, PartialEq)]
pub struct DaySummary {
    pub date: String,
    pub device: String,
    /// How many minutes have a record.
    pub minutes: usize,
    /// How many minutes so far have no record, which myenergi does not have, so the totals are less than they should
    /// be.
    pub missing: usize,
    /// How many records were left out, as they are of another day or a minute which already has one.
    pub extra: usize,
    pub imported: f64,
    pub exported: f64,
    pub generated: f64,
    /// Used by the device itself from surplus generation, e.g. to charge a car, on all phases.
    pub diverted: f64,
    /// Used by the device itself from the grid, when boosting.
    pub boosted: f64,
    /// What each CT clamp measured in each direction, pect1 and nect1 for the first.
    pub cts: [Ct; 3],
}

/// What a CT clamp measured, in kWh, positive and negative.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Ct {
    pub positive: f64,
    pub negative: f64,
}

const MINUTES_PER_DAY: usize = 24 * 60;

const JOULES_PER_KWH: f64 = 3_600_000.0;

impl DaySummary {
    /// The summary of a cgi-jday response, {"U<serial>": [records]}, see MyenergiClient::day. Fields which are zero
    /// are left out of a record, e.g. hr and min at midnight. Some responses also have a record of the first minute of
    /// the next day, which, like a second record of a minute, is left out. today is the UTC date, e.g. 2026-10-28, for
    /// which only the minutes until the last record are expected.
    pub fn from_day(json: &serde_json::Value, today: &str) -> Option<DaySummary> {
        let (device, all_records) = json.as_object()?.iter().next()?;
        let all_records = all_records.as_array()?;
        let part = |record: &serde_json::Value, name: &str| record.get(name).and_then(|value| value.as_i64()).unwrap_or_default();
        let day = |record: &serde_json::Value| (part(record, "yr"), part(record, "mon"), part(record, "dom"));
        let minute = |record: &serde_json::Value| (part(record, "hr") * 60 + part(record, "min")) as usize;
        // A day myenergi has no records of, which says nothing.
        let first = all_records.first()?;
        let (year, month, dom) = day(first);
        let mut seen = std::collections::HashSet::new();
        let records: Vec<&serde_json::Value> = all_records.iter()
            .filter(|record| day(record) == day(first) && seen.insert(minute(record)))
            .collect();
        // Started from 0.0, as the sum of no f64s is -0.0.
        let sum = |names: &[&str]| -> f64 {
            records.iter()
                .flat_map(|record| names.iter().filter_map(|name| record.get(*name).and_then(|value| value.as_f64())))
                .fold(0.0, |total, value| total + value) / JOULES_PER_KWH
        };
        let date = format!("{year}-{month:02}-{dom:02}");
        let expected = if date.as_str() < today {
            MINUTES_PER_DAY
        }
        else {
            records.iter().map(|record| minute(record) + 1).max().unwrap_or_default()
        };
        let ct = |n: usize| Ct { positive: sum(&[&format!("pect{n}")]), negative: sum(&[&format!("nect{n}")]) };
        Some(DaySummary {
            date,
            device: device.clone(),
            minutes: records.len(),
            missing: expected.saturating_sub(records.len()),
            extra: all_records.len() - records.len(),
            imported: sum(&["imp"]),
            exported: sum(&["exp"]),
            generated: sum(&["gep"]),
            diverted: sum(&["h1d", "h2d", "h3d"]),
            boosted: sum(&["h1b", "h2b", "h3b"]),
            cts: [ct(1), ct(2), ct(3)],
        })
    }
}

/// What the CT clamps of the device measure, e.g. Grid, by the types set for them in the latest status, or CT1, CT2
/// and CT3 if there is no status yet. In the status ectt1 is the zappi's internal CT, so the clamps are ectt2 to ectt4.
pub fn ct_names(cache_config: &CacheConfig) -> [String; 3] {
    let mut names = [1, 2, 3].map(|n| format!("CT{n}"));
    let status = cache_config.latest_json(STATUS_KEY).map(|saved| saved_status(&saved).clone());
    // The first zappi's, as only a zappi is known to have them.
    let zappi = status.as_ref()
        .and_then(|status| status.as_array())
        .and_then(|items| items.iter().find_map(|item| item.get("zappi")?.as_array()?.first().cloned()));
    if let Some(zappi) = zappi {
        for (n, name) in names.iter_mut().enumerate() {
            if let Some(kind) = zappi.get(format!("ectt{}", n + 2)).and_then(|kind| kind.as_str()) {
                *name = format!("CT{} {kind}", n + 1);
            }
        }
    }
    names
}

/// What a zappi's status said, for the Samples page, at the time of the poll's slot, or when it was requested, or by
/// the zappi's clock for one saved before those were recorded. Power is in W.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusSample {
    pub time: Option<time::OffsetDateTime>,
    /// The time by the zappi's clock, in UTC, which says which minute's record it is in.
    pub device_time: Option<time::OffsetDateTime>,
    pub serial: String,
    /// What the plug status, pst, means, see plug_state.
    pub plug: String,
    pub status: String,
    pub mode: String,
    /// kWh added since the car was plugged in.
    pub charge_added: Option<f64>,
    /// Power to the car.
    pub charging: Option<f64>,
    pub grid: Option<f64>,
    pub generation: Option<f64>,
    /// The power each CT clamp measures, ectp2 to ectp4, see ct_names.
    pub cts: [Option<f64>; 3],
    pub voltage: Option<f64>,
    pub frequency: Option<f64>,
    /// The zappi's record of the minute the status is from, if it has been fetched, to compare them.
    pub minute: Option<MinuteSample>,
    /// What took the status, e.g. a capture device's name, None if the app did, see capture_device.rs.
    pub captured_by: Option<String>,
}

/// A zappi's record of a minute, see MyenergiClient::day, as the average power in W over the minute, to compare with a
/// status, which is the power at a moment in it.
#[derive(Debug, Clone, PartialEq)]
pub struct MinuteSample {
    /// The time the minute starts.
    pub time: time::OffsetDateTime,
    /// h1d + h1b, on every phase.
    pub charging: f64,
    /// imp - exp
    pub grid: f64,
    /// gep
    pub generation: f64,
    /// pect1 - nect1 to pect3 - nect3, which are the clamps of ectp2 to ectp4 in the status.
    pub cts: [f64; 3],
    pub voltage: Option<f64>,
    pub frequency: Option<f64>,
}

impl MinuteSample {
    fn from_record(record: &serde_json::Value, time: time::OffsetDateTime) -> MinuteSample {
        let joules = |names: &[&str]| names.iter().filter_map(|name| record.get(*name).and_then(|value| value.as_f64())).fold(0.0, |total, value| total + value);
        let watts = |names: &[&str]| joules(names) / 60.0;
        let ct = |n: usize| watts(&[&format!("pect{n}")]) - watts(&[&format!("nect{n}")]);
        MinuteSample {
            time,
            charging: watts(&["h1d", "h2d", "h3d", "h1b", "h2b", "h3b"]),
            grid: watts(&["imp"]) - watts(&["exp"]),
            generation: watts(&["gep"]),
            cts: [ct(1), ct(2), ct(3)],
            voltage: record.get("v1").and_then(|value| value.as_f64()).map(|v1| v1 / 10.0),
            frequency: record.get("frq").and_then(|value| value.as_f64()).map(|frq| frq / 100.0),
        }
    }
}

impl StatusSample {
    /// The samples of the zappis in a cgi-jstatus-* response.
    fn from_status(status: &serde_json::Value) -> Vec<StatusSample> {
        status.as_array().into_iter().flatten()
            .filter_map(|item| item.get("zappi")?.as_array())
            .flatten()
            .map(|zappi| {
                let number = |name: &str| zappi.get(name).and_then(|value| value.as_f64());
                let text = |name: &str| zappi.get(name).and_then(|value| value.as_str()).unwrap_or_default().to_string();
                // The zappi's clock, in UTC, e.g. "28-10-2026" and "15:28:43".
                let format = time::macros::format_description!("[day]-[month]-[year] [hour]:[minute]:[second]");
                let time = time::PrimitiveDateTime::parse(&format!("{} {}", text("dat"), text("tim")), &format).ok()
                    .map(|time| time.assume_utc());
                StatusSample {
                    time,
                    device_time: time,
                    serial: zappi.get("sno").map(|sno| sno.to_string()).unwrap_or_default(),
                    plug: plug_state(&text("pst")).to_string(),
                    status: match number("sta") {
                        Some(1.0) => "paused".to_string(),
                        Some(3.0) => "charging".to_string(),
                        // e.g. when Octopus starts a charge.
                        Some(4.0) => "boosting".to_string(),
                        Some(5.0) => "complete".to_string(),
                        Some(other) => other.to_string(),
                        None => String::new(),
                    },
                    mode: match number("zmo") {
                        Some(1.0) => "Fast".to_string(),
                        Some(2.0) => "Eco".to_string(),
                        Some(3.0) => "Eco+".to_string(),
                        Some(4.0) => "Stop".to_string(),
                        Some(other) => other.to_string(),
                        None => String::new(),
                    },
                    charge_added: number("che"),
                    charging: number("div"),
                    grid: number("grd"),
                    generation: number("gen"),
                    cts: [number("ectp2"), number("ectp3"), number("ectp4")],
                    voltage: number("vol").map(|vol| vol / 10.0),
                    frequency: number("frq"),
                    minute: None,
                    captured_by: None,
                }
            })
            .collect()
    }
}

/// The zappis' samples in the latest limit statuses, newest first, each with its zappi's record of the minute it is
/// from, if that day has been fetched.
pub fn status_samples(cache_config: &CacheConfig, limit: usize) -> Vec<StatusSample> {
    let mut samples: Vec<StatusSample> = cache_config.files_with_key(STATUS_KEY).iter()
        .take(limit)
        .filter_map(|path| CacheConfig::read_json(path))
        .flat_map(|saved| {
            let time = saved_time(&saved);
            let captured_by = saved.get("capturedBy").and_then(|value| value.as_str()).map(String::from);
            StatusSample::from_status(saved_status(&saved)).into_iter()
                .map(move |sample| StatusSample { time: time.or(sample.time), captured_by: captured_by.clone(), ..sample })
        })
        .collect();
    // By the times they are of, not when they were saved, as a capture device's are saved when it is read.
    samples.sort_by(|a, b| b.time.cmp(&a.time));

    // Each zappi's records of each day, by the minute of the day, read once.
    let mut days: std::collections::HashMap<(String, Date), std::collections::HashMap<i64, serde_json::Value>> = Default::default();
    for sample in &mut samples {
        let Some(device_time) = sample.device_time else { continue };
        let date = device_time.date();
        let records = days.entry((sample.serial.clone(), date)).or_insert_with(|| {
            let path = cache_config.existing_path_for_day(&format!("{DAY_KEY}Z{}", sample.serial), &date.into());
            CacheConfig::read_json(&path)
                .and_then(|json| json.as_object()?.values().next()?.as_array().cloned())
                .unwrap_or_default()
                .into_iter()
                .filter(|record| record.get("dom").and_then(|dom| dom.as_i64()) == Some(date.day() as i64))
                .map(|record| {
                    let minute = |name: &str| record.get(name).and_then(|value| value.as_i64()).unwrap_or_default();
                    (minute("hr") * 60 + minute("min"), record)
                })
                .collect()
        });
        let minute_of_day = device_time.hour() as i64 * 60 + device_time.minute() as i64;
        sample.minute = records.get(&minute_of_day).map(|record| {
            MinuteSample::from_record(record, device_time.replace_second(0).unwrap_or(device_time))
        });
    }
    samples
}

/// The zappis in the latest status, by serial number, with whether one of its CT clamps is set to measure the grid,
/// whose readings are its records' imp and exp, true if the status does not say.
pub fn zappis(cache_config: &CacheConfig) -> Vec<(String, bool)> {
    let Some(status) = cache_config.latest_json(STATUS_KEY).map(|saved| saved_status(&saved).clone()) else {
        return Vec::new();
    };
    status.as_array().into_iter().flatten()
        .filter_map(|item| item.get("zappi")?.as_array())
        .flatten()
        .filter_map(|zappi| {
            let serial = match zappi.get("sno")? {
                serde_json::Value::Number(number) => number.to_string(),
                serde_json::Value::String(text) => text.clone(),
                _ => return None,
            };
            let kinds: Vec<&str> = (2..=4).filter_map(|n| zappi.get(format!("ectt{n}"))?.as_str()).collect();
            Some((serial, kinds.is_empty() || kinds.contains(&"Grid")))
        })
        .collect()
}

/// A zappi's cached records of the UTC day date, see MyenergiClient::day, none if it is not cached.
pub fn zappi_day(cache_config: &CacheConfig, serial: &str, date: Date) -> Vec<serde_json::Value> {
    CacheConfig::read_json(&cache_config.existing_path_for_day(&format!("{DAY_KEY}Z{serial}"), &date.into()))
        .and_then(|json| json.as_object()?.values().next()?.as_array().cloned())
        .unwrap_or_default()
}

/// The summaries of every cached day, newest first.
pub fn summaries(cache_config: &CacheConfig) -> Vec<DaySummary> {
    let today = crate::clock::now_utc().date().to_string();
    let mut summaries: Vec<DaySummary> = cache_config.files_with_key(DAY_KEY).iter()
        .filter_map(|path| DaySummary::from_day(&CacheConfig::read_json(path)?, &today))
        .collect();
    summaries.sort_by(|a, b| b.date.cmp(&a.date).then(a.device.cmp(&b.device)));
    summaries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_is_summed_in_kwh() {
        let json = serde_json::json!({"U12345678": [
            {"yr": 2026, "mon": 10, "dom": 27, "imp": 1_800_000, "h1d": 3_600_000, "pect1": 1_800_000},
            {"yr": 2026, "mon": 10, "dom": 27, "min": 1, "imp": 1_800_000, "exp": 360_000, "gep": 720_000, "nect1": 360_000, "pect3": 3_600_000, "h1b": 1_800_000},
            // A second record of a minute, and the first minute of the next day, which are both left out.
            {"yr": 2026, "mon": 10, "dom": 27, "min": 1, "imp": 1_800_000},
            {"yr": 2026, "mon": 10, "dom": 28, "imp": 1_800_000},
        ]});
        assert_eq!(DaySummary::from_day(&json, "2026-10-28"), Some(DaySummary {
            date: "2026-10-27".to_string(),
            device: "U12345678".to_string(),
            minutes: 2,
            missing: 1438,
            extra: 2,
            imported: 1.0,
            exported: 0.1,
            generated: 0.2,
            diverted: 1.0,
            boosted: 0.5,
            cts: [
                Ct { positive: 0.5, negative: 0.1 },
                Ct::default(),
                Ct { positive: 1.0, negative: 0.0 },
            ],
        }));
    }

    #[test]
    fn a_capture_device_status_is_a_sample() {
        // As capture/src/myenergi.rs saves them: every device's serial number, and some of each zappi's fields.
        let saved = serde_json::json!({
            "slot": "2026-10-29T16:15:00.000000Z",
            "requested": "2026-10-29T16:15:00.412000Z",
            "received": "2026-10-29T16:15:02.106000Z",
            "capturedBy": "marco-sparko-capture 0.1.0",
            "response": [
                {"eddi": []},
                {"harvi": [{"sno": 12345678}]},
                {"zappi": [{"sno": 20456041, "dat": "29-10-2026", "tim": "16:15:01", "pst": "B2", "sta": 1, "zmo": 3,
                    "che": 0, "div": 0, "ectt2": "Grid", "ectp2": 120}]},
            ],
        });
        assert_eq!(saved_time(&saved), time::OffsetDateTime::parse("2026-10-29T16:15:00Z", &time::format_description::well_known::Rfc3339).ok());
        let samples = StatusSample::from_status(saved_status(&saved));
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].serial, "20456041");
        assert_eq!(samples[0].plug, "waiting for the car");
        assert_eq!(samples[0].cts[0], Some(120.0));
        let devices: Vec<String> = Device::from_status(saved_status(&saved)).iter().map(|device| format!("{}{}", device.letter, device.serial)).collect();
        assert_eq!(devices, ["H12345678", "Z20456041"]);
    }

    #[test]
    fn nothing_is_zero_not_minus_zero() {
        let json = serde_json::json!({"U12345678": [{"yr": 2023, "mon": 10, "dom": 9, "v1": 2394}]});
        let summary = DaySummary::from_day(&json, "2026-10-28").unwrap();
        assert!(summary.imported.is_sign_positive(), "{}", summary.imported);
    }

    #[test]
    fn the_recent_minutes_are_split_at_midnight() {
        use time::macros::datetime;
        assert_eq!(recent_parts(datetime!(2026-10-28 10:05:30 UTC)), vec![(datetime!(2026-10-28 09:50:30 UTC), 16)]);
        assert_eq!(recent_parts(datetime!(2026-10-29 00:05:00 UTC)), vec![
            (datetime!(2026-10-28 23:50:00 UTC), 10),
            (datetime!(2026-10-29 00:00:00 UTC), 6),
        ]);
    }

    #[test]
    fn records_are_merged_into_the_day_by_minute() {
        let dir = std::env::temp_dir().join(format!("marco-sparko-merge-{}", std::process::id()));
        // As the app makes it, see MarcoSparkoContext::create_cache_config.
        std::fs::create_dir_all(&dir).unwrap();
        let cache_config = CacheConfig::new(dir.clone(), false, false);
        let device = Device { kind: "zappi".to_string(), letter: 'Z', serial: "12345678".to_string() };
        let date = time::macros::date!(2026-10-28);
        let day = serde_json::json!({"U12345678": [
            {"yr": 2026, "mon": 10, "dom": 28, "imp": 1},
            {"yr": 2026, "mon": 10, "dom": 28, "min": 1, "imp": 2},
            {"yr": 2026, "mon": 10, "dom": 29, "imp": 9},
        ]});
        merge_into_day(&cache_config, &device, date, &day).unwrap();
        let part = serde_json::json!({"U12345678": [
            {"yr": 2026, "mon": 10, "dom": 28, "min": 1, "imp": 3},
            {"yr": 2026, "mon": 10, "dom": 28, "min": 2, "imp": 4},
        ]});
        merge_into_day(&cache_config, &device, date, &part).unwrap();
        let merged = CacheConfig::read_json(&cache_config.existing_path_for_day("jday-Z12345678", &date.into())).unwrap();
        let imports: Vec<i64> = merged["U12345678"].as_array().unwrap().iter().map(|record| record["imp"].as_i64().unwrap()).collect();
        assert_eq!(imports, vec![1, 3, 4, 9]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn today_only_expects_the_minutes_until_the_last_record() {
        let json = serde_json::json!({"U12345678": [
            {"yr": 2026, "mon": 10, "dom": 28},
            {"yr": 2026, "mon": 10, "dom": 28, "hr": 1, "min": 59},
        ]});
        assert_eq!(DaySummary::from_day(&json, "2026-10-28").map(|summary| summary.missing), Some(118));
    }
}
