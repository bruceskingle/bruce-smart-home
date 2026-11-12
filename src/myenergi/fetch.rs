//! Fetching myenergi data into the profile's store, see store.rs, and what the days kept add up to.
//!
//! The status, as it is each time, is the kind `myenergi.status`, a DataSet for the site's hub, of each status as it was
//! saved, by its time to the second, written by what took it, the app or a capture device, the app's a microsecond
//! newer, as Tesla's snapshots are. Each device's records are the kind `myenergi.minutes`, a DataSet for each device, by
//! the hash of its letter and serial number, of each record as myenergi gave it, by its minute, with a `#fetched/<date>`
//! record for each UTC day fetched: whether the day had ended when it was, and the name of the device in the response,
//! e.g. U12345678. A day is fetched again until it has ended.

use anyhow::anyhow;
use bruce_db::{Flow, Id, Item, Micros, Part, Put, Store};
use time::{Date, Duration};

use super::client::{Device, MyenergiClient};

/// How many days are fetched, including today.
pub const DAYS: i64 = 3;

const STATUS_KIND: &str = crate::device_kinds::MYENERGI_STATUS;
const MINUTES_KIND: &str = "myenergi.minutes";
const HUB: &str = crate::device_kinds::MYENERGI_HUB;

fn status_key(store: &Store) -> anyhow::Result<String> {
    store.hash_key(STATUS_KIND, &[Part::from(HUB)])
}

/// A device's DataSet of records, by the hash of its letter and serial number, e.g. Z12345678, see the design's IDs.
fn minutes_key(store: &Store, letter: char, serial: &str) -> anyhow::Result<String> {
    store.hash_key(
        MINUTES_KIND,
        &[Part::Id(Id::of(
            &serde_json::json!({"myenergi": format!("{letter}{serial}")}),
        )?)],
    )
}

/// The sort key of a day's #fetched record.
fn fetched_key(date: Date) -> String {
    format!("#fetched/{date}")
}

/// The sort key of a time, to the second, e.g. 2026-11-04T07:17:10Z.
fn second_key(time: time::OffsetDateTime) -> String {
    let format = time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    time.to_offset(time::UtcOffset::UTC).format(format).unwrap_or_default()
}

/// The sort key of a record's minute, e.g. 2026-10-27T13:05:00Z, from its yr, mon, dom, hr and min, which are left
/// out when they are zero.
fn minute_key(record: &serde_json::Value) -> String {
    let part = |name: &str| record.get(name).and_then(|value| value.as_i64()).unwrap_or_default();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:00Z",
        part("yr"),
        part("mon"),
        part("dom"),
        part("hr"),
        part("min")
    )
}

/// Saves a status, {"slot": ..., "requested": ..., "received": ..., "response": status}, taken by the capture device
/// device, or by the app if it is None, see the module documentation, returning whether it was new.
pub fn save_status(store: &Store, saved: &serde_json::Value, device: Option<&str>) -> anyhow::Result<bool> {
    save_status_at(store, saved, None, device)
}

/// Saves a status, see save_status, of time if the status does not say, e.g. one saved before the times were recorded,
/// whose time is in its old file's name, see convert_legacy.
fn save_status_at(
    store: &Store,
    saved: &serde_json::Value,
    time: Option<time::OffsetDateTime>,
    device: Option<&str>,
) -> anyhow::Result<bool> {
    let time = saved_time(saved)
        .or(time)
        .ok_or_else(|| anyhow!("the status has no time"))?;
    let origin = crate::store::origin();
    let writer = device.map_or(
        crate::device_kinds::Writer::App(&origin),
        crate::device_kinds::Writer::Device,
    );
    let item = crate::device_kinds::sampled(&status_key(store)?, Micros::from_datetime(time), saved, writer);
    Ok(store.put(&item)? == Put::Stored)
}

/// Fetches the status into the store, which lists the devices.
async fn fetch_status(
    client: &MyenergiClient,
    store: &Store,
    slot: Option<time::OffsetDateTime>,
    progress: &mut dyn FnMut(String),
) -> anyhow::Result<serde_json::Value> {
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
    save_status(store, &saved, None)?;
    Ok(status)
}

/// The latest saved statuses, up to limit, newest first: of the last day, or of the last week if there are fewer.
fn latest_statuses(store: &Store, limit: usize) -> Vec<serde_json::Value> {
    let Ok(hk) = status_key(store) else { return Vec::new() };
    let mut statuses = Vec::new();
    for days in [1, 7, 3650] {
        let from = second_key(crate::clock::now_utc() - Duration::days(days));
        statuses.clear();
        let _ = store.query(&hk, from.as_str()..bruce_db::ALL.end, &mut |item| {
            statuses.extend(item.attributes);
            Ok(Flow::Continue)
        });
        if statuses.len() >= limit {
            break;
        }
    }
    statuses.into_iter().rev().take(limit).collect()
}

/// The latest saved status, as it was saved.
fn latest_saved(store: &Store) -> Option<serde_json::Value> {
    latest_statuses(store, 1).into_iter().next()
}

/// The time of the latest status saved.
pub fn latest_status_time(store: &Store) -> Option<time::OffsetDateTime> {
    saved_time(&latest_saved(store)?)
}

/// The status in a saved status, which is {"slot": ..., "requested": ..., "received": ..., "response": status}, see
/// fetch_status, or for one saved before those were recorded the status itself.
fn saved_status(saved: &serde_json::Value) -> &serde_json::Value {
    saved.get("response").unwrap_or(saved)
}

/// When a saved status is of: the slot of a scheduled poll, otherwise when it was requested, None for one saved before
/// those were recorded.
fn saved_time(saved: &serde_json::Value) -> Option<time::OffsetDateTime> {
    ["slot", "requested"]
        .iter()
        .find_map(|name| saved.get(*name)?.as_str())
        .and_then(|time| time::OffsetDateTime::parse(time, &time::format_description::well_known::Rfc3339).ok())
}

/// Saves the records of a response of MyenergiClient::day or part_of_day, {"U<serial>": [records]}, each by its minute,
/// and, for a whole day, date's #fetched record, returning how many records there were.
fn save_records(store: &Store, device: &Device, date: Option<Date>, json: &serde_json::Value) -> anyhow::Result<usize> {
    let Some((name, records)) = json.as_object().and_then(|object| object.iter().next()) else {
        return Ok(0);
    };
    let hk = minutes_key(store, device.letter, &device.serial)?;
    let records = records.as_array().cloned().unwrap_or_default();
    // A record of a minute given twice in a response, the last is kept, as a newer version.
    for record in &records {
        store.put(&Item::new(
            &hk,
            &minute_key(record),
            crate::store::version_now(),
            record.clone(),
        ))?;
    }
    if let Some(date) = date {
        let ended = date < crate::clock::now_utc().date();
        store.put(&Item::new(
            &hk,
            &fetched_key(date),
            crate::store::version_now(),
            serde_json::json!({ "ended": ended, "name": name, "records": records.len() }),
        ))?;
    }
    Ok(records.len())
}

/// Fetches the device's records of the UTC day date into the store, unless the day had ended when it was fetched
/// before, or force. Says how many minutes have records, so that a day long ago shows whether myenergi still has it.
async fn fetch_day(
    client: &MyenergiClient,
    store: &Store,
    device: &Device,
    date: Date,
    force: bool,
    progress: &mut dyn FnMut(String),
) -> anyhow::Result<()> {
    let hk = minutes_key(store, device.letter, &device.serial)?;
    let ended = store
        .get(&hk, &fetched_key(date))?
        .and_then(|item| item.attributes?.get("ended")?.as_bool())
        .unwrap_or(false);
    if ended && !force {
        return Ok(());
    }
    progress(format!("Fetching {} {} {date}", device.kind, device.serial));
    match client.day(device, date).await {
        Ok(json) => {
            let minutes = save_records(store, device, Some(date), &json)?;
            progress(format!(
                "{} {} {date} has {minutes} minutes of records",
                device.kind, device.serial
            ));
        }
        // e.g. a harvi, which may not have records of its own.
        Err(error) => {
            crate::log_warning!("Unable to fetch {} {} {date}, {error:#}", device.kind, device.serial);
            progress(format!(
                "Unable to fetch {} {} {date}, {error:#}",
                device.kind, device.serial
            ));
        }
    }
    Ok(())
}

/// Fetches the status and the last DAYS days of every device which has records, saying what it did with progress.
pub async fn fetch(
    client: &MyenergiClient,
    store: &Store,
    offline: bool,
    progress: &mut dyn FnMut(String),
) -> anyhow::Result<()> {
    if offline {
        return Err(anyhow!("This profile is offline, so nothing can be fetched"));
    }
    let today = crate::clock::now_utc().date();
    let devices = Device::from_status(&fetch_status(client, store, None, progress).await?);
    if devices.is_empty() {
        progress("The status lists no devices".to_string());
    }
    for device in &devices {
        for days_ago in 0..DAYS {
            fetch_day(client, store, device, today - Duration::days(days_ago), false, progress).await?;
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

/// Fetches the device's records of the last RECENT_MINUTES minutes before now into the store, two parts if they start
/// yesterday.
async fn fetch_recent(
    client: &MyenergiClient,
    store: &Store,
    device: &Device,
    now: time::OffsetDateTime,
) -> anyhow::Result<()> {
    for (from, count) in recent_parts(now) {
        let json = client
            .part_of_day(device, from.date(), from.hour(), from.minute(), count as u16)
            .await?;
        save_records(store, device, None, &json)?;
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
    } else {
        let midnight = now.replace_time(time::Time::MIDNIGHT);
        vec![
            (start, 24 * 60 - minute_of_day(start)),
            (midnight, minute_of_day(now) + 1),
        ]
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
pub async fn poll(
    client: &MyenergiClient,
    store: &Store,
    offline: bool,
    slot: Option<time::OffsetDateTime>,
    whole_days_fetched: &std::sync::Mutex<Option<Date>>,
    captured: bool,
) -> anyhow::Result<String> {
    if offline {
        return Err(anyhow!("This profile is offline, so nothing can be fetched"));
    }
    let latest = if captured {
        latest_saved(store).map(|saved| saved_status(&saved).clone())
    } else {
        None
    };
    let status = match latest {
        Some(status) => status,
        None => fetch_status(client, store, slot, &mut |_| {}).await?,
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
            fetch_day(client, store, device, *date, true, &mut |_| {}).await?;
        }
        fetch_recent(client, store, device, now).await?;
    }
    *whole_days_fetched.lock().unwrap() = Some(today);
    let zappis: Vec<String> = status
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("zappi")?.as_array())
        .flatten()
        .map(|zappi| {
            let pst = zappi.get("pst").and_then(|value| value.as_str()).unwrap_or_default();
            let che = zappi.get("che").and_then(|value| value.as_f64()).unwrap_or_default();
            let power = zappi.get("div").and_then(|value| value.as_f64()).unwrap_or_default();
            format!(
                "zappi {}: {}, {che} kWh added, {:.2} kW",
                zappi.get("sno").map(|sno| sno.to_string()).unwrap_or_default(),
                plug_state(pst),
                power / 1000.0
            )
        })
        .collect();
    let mut summary = if zappis.is_empty() {
        "No zappi".to_string()
    } else {
        zappis.join("; ")
    };
    if !whole_days.is_empty() {
        summary.push_str(&format!(
            ", and fetched the whole of {}",
            whole_days.iter().map(Date::to_string).collect::<Vec<_>>().join(" and ")
        ));
    }
    Ok(summary)
}

/// Fetches the UTC day date of every device in the latest status, fetching the status first if there is none. The
/// day is fetched again even if it is cached, to see what myenergi has now.
pub async fn fetch_date(
    client: &MyenergiClient,
    store: &Store,
    offline: bool,
    date: Date,
    progress: &mut dyn FnMut(String),
) -> anyhow::Result<()> {
    if offline {
        return Err(anyhow!("This profile is offline, so nothing can be fetched"));
    }
    if date > crate::clock::now_utc().date() {
        return Err(anyhow!("{date} has not happened yet"));
    }
    let status = match latest_saved(store) {
        Some(saved) => saved_status(&saved).clone(),
        None => fetch_status(client, store, None, progress).await?,
    };
    for device in &Device::from_status(&status) {
        fetch_day(client, store, device, date, true, progress).await?;
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
        let part = |record: &serde_json::Value, name: &str| {
            record.get(name).and_then(|value| value.as_i64()).unwrap_or_default()
        };
        let day = |record: &serde_json::Value| (part(record, "yr"), part(record, "mon"), part(record, "dom"));
        let minute = |record: &serde_json::Value| (part(record, "hr") * 60 + part(record, "min")) as usize;
        // A day myenergi has no records of, which says nothing.
        let first = all_records.first()?;
        let (year, month, dom) = day(first);
        let mut seen = std::collections::HashSet::new();
        let records: Vec<&serde_json::Value> = all_records
            .iter()
            .filter(|record| day(record) == day(first) && seen.insert(minute(record)))
            .collect();
        // Started from 0.0, as the sum of no f64s is -0.0.
        let sum = |names: &[&str]| -> f64 {
            records
                .iter()
                .flat_map(|record| {
                    names
                        .iter()
                        .filter_map(|name| record.get(*name).and_then(|value| value.as_f64()))
                })
                .fold(0.0, |total, value| total + value)
                / JOULES_PER_KWH
        };
        let date = format!("{year}-{month:02}-{dom:02}");
        let expected = if date.as_str() < today {
            MINUTES_PER_DAY
        } else {
            records
                .iter()
                .map(|record| minute(record) + 1)
                .max()
                .unwrap_or_default()
        };
        let ct = |n: usize| Ct {
            positive: sum(&[&format!("pect{n}")]),
            negative: sum(&[&format!("nect{n}")]),
        };
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
pub fn ct_names(store: &Store) -> [String; 3] {
    let mut names = [1, 2, 3].map(|n| format!("CT{n}"));
    let status = latest_saved(store).map(|saved| saved_status(&saved).clone());
    // The first zappi's, as only a zappi is known to have them.
    let zappi = status.as_ref().and_then(|status| status.as_array()).and_then(|items| {
        items
            .iter()
            .find_map(|item| item.get("zappi")?.as_array()?.first().cloned())
    });
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
        let joules = |names: &[&str]| {
            names
                .iter()
                .filter_map(|name| record.get(*name).and_then(|value| value.as_f64()))
                .fold(0.0, |total, value| total + value)
        };
        let watts = |names: &[&str]| joules(names) / 60.0;
        let ct = |n: usize| watts(&[&format!("pect{n}")]) - watts(&[&format!("nect{n}")]);
        MinuteSample {
            time,
            charging: watts(&["h1d", "h2d", "h3d", "h1b", "h2b", "h3b"]),
            grid: watts(&["imp"]) - watts(&["exp"]),
            generation: watts(&["gep"]),
            cts: [ct(1), ct(2), ct(3)],
            voltage: record.get("v1").and_then(|value| value.as_f64()).map(|v1| v1 / 10.0),
            frequency: record
                .get("frq")
                .and_then(|value| value.as_f64())
                .map(|frq| frq / 100.0),
        }
    }
}

impl StatusSample {
    /// The samples of the zappis in a cgi-jstatus-* response.
    fn from_status(status: &serde_json::Value) -> Vec<StatusSample> {
        status
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("zappi")?.as_array())
            .flatten()
            .map(|zappi| {
                let number = |name: &str| zappi.get(name).and_then(|value| value.as_f64());
                let text = |name: &str| {
                    zappi
                        .get(name)
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string()
                };
                // The zappi's clock, in UTC, e.g. "28-10-2026" and "15:28:43".
                let format = time::macros::format_description!("[day]-[month]-[year] [hour]:[minute]:[second]");
                let time = time::PrimitiveDateTime::parse(&format!("{} {}", text("dat"), text("tim")), &format)
                    .ok()
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
pub fn status_samples(store: &Store, limit: usize) -> Vec<StatusSample> {
    let mut samples: Vec<StatusSample> = latest_statuses(store, limit)
        .into_iter()
        .flat_map(|saved| {
            let time = saved_time(&saved);
            let captured_by = saved
                .get("capturedBy")
                .and_then(|value| value.as_str())
                .map(String::from);
            StatusSample::from_status(saved_status(&saved))
                .into_iter()
                .map(move |sample| StatusSample {
                    time: time.or(sample.time),
                    captured_by: captured_by.clone(),
                    ..sample
                })
        })
        .collect();
    // By the times they are of, not when they were saved, as a capture device's are saved when it is read.
    samples.sort_by(|a, b| b.time.cmp(&a.time));

    // Each zappi's records of each day, by the minute of the day, read once.
    let mut days: std::collections::HashMap<(String, Date), std::collections::HashMap<i64, serde_json::Value>> =
        Default::default();
    for sample in &mut samples {
        let Some(device_time) = sample.device_time else {
            continue;
        };
        let date = device_time.date();
        let records = days.entry((sample.serial.clone(), date)).or_insert_with(|| {
            zappi_day(store, &sample.serial, date)
                .into_iter()
                .filter(|record| record.get("dom").and_then(|dom| dom.as_i64()) == Some(date.day() as i64))
                .map(|record| {
                    let minute = |name: &str| record.get(name).and_then(|value| value.as_i64()).unwrap_or_default();
                    (minute("hr") * 60 + minute("min"), record)
                })
                .collect()
        });
        let minute_of_day = device_time.hour() as i64 * 60 + device_time.minute() as i64;
        sample.minute = records
            .get(&minute_of_day)
            .map(|record| MinuteSample::from_record(record, device_time.replace_second(0).unwrap_or(device_time)));
    }
    samples
}

/// The zappis in the latest status, by serial number, with whether one of its CT clamps is set to measure the grid,
/// whose readings are its records' imp and exp, true if the status does not say.
pub fn zappis(store: &Store) -> Vec<(String, bool)> {
    let Some(status) = latest_saved(store).map(|saved| saved_status(&saved).clone()) else {
        return Vec::new();
    };
    status
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("zappi")?.as_array())
        .flatten()
        .filter_map(|zappi| {
            let serial = match zappi.get("sno")? {
                serde_json::Value::Number(number) => number.to_string(),
                serde_json::Value::String(text) => text.clone(),
                _ => return None,
            };
            let kinds: Vec<&str> = (2..=4)
                .filter_map(|n| zappi.get(format!("ectt{n}"))?.as_str())
                .collect();
            Some((serial, kinds.is_empty() || kinds.contains(&"Grid")))
        })
        .collect()
}

/// A zappi's records of the UTC day date, see MyenergiClient::day, in order, none if it has not been fetched.
pub fn zappi_day(store: &Store, serial: &str, date: Date) -> Vec<serde_json::Value> {
    let Ok(hk) = minutes_key(store, 'Z', serial) else {
        return Vec::new();
    };
    day_records(store, &hk, date)
}

/// The records of a DataSet of the UTC day date, in order.
fn day_records(store: &Store, hk: &str, date: Date) -> Vec<serde_json::Value> {
    let (from, to) = (
        date.to_string(),
        date.next_day().map(|next| next.to_string()).unwrap_or_default(),
    );
    store
        .query_all(hk, from.as_str()..to.as_str())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| item.attributes)
        .collect()
}

/// The summaries of every day fetched, newest first, from each DataSet's #fetched records.
pub fn summaries(store: &Store) -> Vec<DaySummary> {
    let today = crate::clock::now_utc().date().to_string();
    let prefix = format!("{}/{MINUTES_KIND}/", store.site());
    let mut summaries = Vec::new();
    for hk in store.data_sets(&prefix).unwrap_or_default() {
        for fetched in store.query_all(&hk, "#fetched/".."#fetched0").unwrap_or_default() {
            let Some(date) = fetched
                .sk
                .strip_prefix("#fetched/")
                .and_then(|date| Date::parse(date, &time::format_description::well_known::Iso8601::DATE).ok())
            else {
                continue;
            };
            let name = fetched
                .attributes
                .as_ref()
                .and_then(|fetched| fetched.get("name")?.as_str().map(str::to_string))
                .unwrap_or_default();
            let records = day_records(store, &hk, date);
            summaries.extend(DaySummary::from_day(&serde_json::json!({ name: records }), &today));
        }
    }
    summaries.sort_by(|a, b| b.date.cmp(&a.date).then(a.device.cmp(&b.device)));
    summaries
}

/// Converts the statuses and days kept in the module's directory of the cache before the store, `<day>#status-HHMMSS`
/// and `<day>#jday-<letter><serial>`, see convert.rs.
pub fn convert_legacy(
    store: &std::sync::Arc<Store>,
    dir: &std::path::Path,
    progress: &mut dyn bruce_db::progress::Progress,
) -> anyhow::Result<String> {
    let (mut statuses, mut days) = (0, 0);
    let files = crate::convert::legacy_files(dir);
    for (index, file) in files.iter().enumerate() {
        crate::convert::step(progress, index, files.len())?;
        if let Some(clock) = file.key.strip_prefix("status-") {
            let format = time::macros::format_description!("[hour][minute][second]");
            let time = file
                .date
                .zip(time::Time::parse(clock, format).ok())
                .map(|(date, clock)| date.with_time(clock).assume_utc());
            statuses += save_status_at(store, &file.json()?, time, None)? as usize;
        } else if let Some(device) = file.key.strip_prefix("jday-") {
            let mut characters = device.chars();
            let Some(letter) = characters.next() else { continue };
            let kind = match letter {
                'Z' => "zappi",
                'E' => "eddi",
                'H' => "harvi",
                _ => "device",
            };
            let device = Device {
                kind: kind.to_string(),
                letter,
                serial: characters.collect(),
            };
            save_records(store, &device, file.date, &file.json()?)?;
            days += 1;
        }
    }
    Ok(format!("{statuses} statuses, {days} days of records"))
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
        assert_eq!(
            DaySummary::from_day(&json, "2026-10-28"),
            Some(DaySummary {
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
                    Ct {
                        positive: 0.5,
                        negative: 0.1
                    },
                    Ct::default(),
                    Ct {
                        positive: 1.0,
                        negative: 0.0
                    },
                ],
            })
        );
    }

    #[test]
    fn a_capture_device_status_is_a_sample() {
        // As capture/src/myenergi.rs saves them: every device's serial number, and some of each zappi's fields.
        let saved = serde_json::json!({
            "slot": "2026-10-29T16:15:00.000000Z",
            "requested": "2026-10-29T16:15:00.412000Z",
            "received": "2026-10-29T16:15:02.106000Z",
            "capturedBy": "bruce-smart-home-capture 0.1.0",
            "response": [
                {"eddi": []},
                {"harvi": [{"sno": 12345678}]},
                {"zappi": [{"sno": 20456041, "dat": "29-10-2026", "tim": "16:15:01", "pst": "B2", "sta": 1, "zmo": 3,
                    "che": 0, "div": 0, "ectt2": "Grid", "ectp2": 120}]},
            ],
        });
        assert_eq!(
            saved_time(&saved),
            time::OffsetDateTime::parse("2026-10-29T16:15:00Z", &time::format_description::well_known::Rfc3339).ok()
        );
        let samples = StatusSample::from_status(saved_status(&saved));
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].serial, "20456041");
        assert_eq!(samples[0].plug, "waiting for the car");
        assert_eq!(samples[0].cts[0], Some(120.0));
        let devices: Vec<String> = Device::from_status(saved_status(&saved))
            .iter()
            .map(|device| format!("{}{}", device.letter, device.serial))
            .collect();
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
        assert_eq!(
            recent_parts(datetime!(2026-10-28 10:05:30 UTC)),
            vec![(datetime!(2026-10-28 09:50:30 UTC), 16)]
        );
        assert_eq!(
            recent_parts(datetime!(2026-10-29 00:05:00 UTC)),
            vec![
                (datetime!(2026-10-28 23:50:00 UTC), 10),
                (datetime!(2026-10-29 00:00:00 UTC), 6),
            ]
        );
    }

    #[test]
    fn records_are_kept_by_minute_and_a_day_is_summed_from_them() {
        let dir = std::env::temp_dir().join(format!("bruce-smart-home-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let site = Id::of(&serde_json::json!({"site": "myenergi test"})).unwrap();
        let store = Store::open(&format!("file://{}", dir.display()), &site).unwrap();
        let device = Device {
            kind: "zappi".to_string(),
            letter: 'Z',
            serial: "12345678".to_string(),
        };
        let date = time::macros::date!(2026 - 10 - 28);
        let day = serde_json::json!({"U12345678": [
            {"yr": 2026, "mon": 10, "dom": 28, "imp": 1},
            {"yr": 2026, "mon": 10, "dom": 28, "min": 1, "imp": 2},
            {"yr": 2026, "mon": 10, "dom": 29, "imp": 9},
        ]});
        save_records(&store, &device, Some(date), &day).unwrap();
        let part = serde_json::json!({"U12345678": [
            {"yr": 2026, "mon": 10, "dom": 28, "min": 1, "imp": 3},
            {"yr": 2026, "mon": 10, "dom": 28, "min": 2, "imp": 4},
        ]});
        save_records(&store, &device, None, &part).unwrap();
        let imports = |date| -> Vec<i64> {
            zappi_day(&store, "12345678", date)
                .iter()
                .map(|record| record["imp"].as_i64().unwrap())
                .collect()
        };
        assert_eq!(imports(date), vec![1, 3, 4]);
        assert_eq!(
            imports(time::macros::date!(2026 - 10 - 29)),
            vec![9],
            "the next day's first minute is the next day's"
        );
        let summaries = summaries(&store);
        assert_eq!(summaries.len(), 1);
        assert_eq!(
            (
                summaries[0].date.as_str(),
                summaries[0].device.as_str(),
                summaries[0].minutes
            ),
            ("2026-10-28", "U12345678", 3)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn today_only_expects_the_minutes_until_the_last_record() {
        let json = serde_json::json!({"U12345678": [
            {"yr": 2026, "mon": 10, "dom": 28},
            {"yr": 2026, "mon": 10, "dom": 28, "hr": 1, "min": 59},
        ]});
        assert_eq!(
            DaySummary::from_day(&json, "2026-10-28").map(|summary| summary.missing),
            Some(118)
        );
    }
}
