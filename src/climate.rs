//! The temperature and humidity which the profile's capture devices measure themselves, e.g. the Waveshare
//! ESP32-S3-RLCD-4.2's sensor, read from them every 5 minutes, see ClimateReader, and kept for the Temp & Humidity
//! page, see views/climate.rs.
//!
//! Unlike the other records the devices give, which are copies of what a source said, these are the readings
//! themselves, so each device's are kept separately. A device gives them as periods of each series, e.g. 23.4 °C from
//! one time to another, see the capture firmware's climate.rs, and gives the one still open again, with its end moved
//! on, each time it is read, so a period is saved by its start, replacing what was saved of it before.
//!
//! They are kept in the profile's <profile>-climate directory in the cache directory, which is counted and deleted with
//! the profile's other data: a directory for each device, by its ID, with its name, as it last said, in device.json,
//! and a directory for each series, of a file for each UTC day, of the periods which start on it, by their starts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::capture_device::{CaptureReader, DeviceProfile};
use crate::{MarcoSparkoContext, Module, PageInfo};

/// The id the reader is polled as, see polling.rs.
pub const MODULE_ID: &str = "climate";

/// How often the devices are read.
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// What a device measures, by the name it gives each series, with how it is shown.
pub const SERIES: [Series; 2] = [
    Series { name: "temperature", label: "Temperature", unit: "°C", decimals: 1 },
    Series { name: "humidity", label: "Humidity", unit: "%", decimals: 0 },
];

/// The board's battery's voltage, which a device with one, e.g. the RLCD, gives with the temperature and humidity, but
/// which is about the device, so is shown on its card on the Devices page, not with them.
pub const BATTERY: Series = Series { name: "battery", label: "Battery", unit: "V", decimals: 2 };

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Series {
    pub name: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub decimals: usize,
}

/// A period as it is saved, by its start, see DayFile.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
struct Saved {
    end: String,
    value: f64,
    /// Whether the device was still in it when it gave it, so that the end may move on.
    open: bool,
}

/// The periods of a series which start on a UTC day, by their starts, as RFC 3339 times, which sort in time order.
type DayFile = BTreeMap<String, Saved>;

/// A period as the page draws it, in unix seconds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Period {
    pub start: i64,
    pub end: i64,
    pub value: f64,
}

/// One device's periods of each of SERIES, in order, see load.
#[derive(Clone, PartialEq, Debug)]
pub struct DeviceClimate {
    pub id: String,
    /// Its name on the network, e.g. rlcd, as it last said, or its ID.
    pub name: String,
    pub series: [Vec<Period>; 2],
}

/// The profile's directory of the readings.
pub fn dir(profile_name: &str) -> anyhow::Result<PathBuf> {
    Ok(MarcoSparkoContext::get_cache_dir_path()?.join(format!("{profile_name}-{MODULE_ID}")))
}

/// A device's directory, by its ID, a MAC address, whose colons are left out of the name.
fn device_dir(dir: &Path, id: &str) -> PathBuf {
    dir.join(id.replace(':', ""))
}

fn day_path(dir: &Path, id: &str, series: &str, day: time::Date) -> PathBuf {
    device_dir(dir, id).join(series).join(format!("{day}.json"))
}

fn read_day(path: &Path) -> DayFile {
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn parse(time: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(time, &Rfc3339).ok()
}

/// Saves the records read from device, see the module documentation, each file once, returning how many were new or
/// moved a period's end on.
fn save(dir: &Path, device: &DeviceProfile, records: &[Value]) -> anyhow::Result<usize> {
    let mut files: BTreeMap<PathBuf, Vec<(String, Saved)>> = BTreeMap::new();
    let mut name = None;
    // Records which are not periods, e.g. another feature's on a damaged SD card, counted, with the first, for one
    // warning, rather than one each, which can be hundreds a read.
    let mut not_periods = (0, None);
    for record in records {
        let field = |key: &str| record.get(key).and_then(Value::as_str);
        let (Some(series), Some(start), Some(end), Some(value)) =
            (field("series"), field("start"), field("end"), record.get("value").and_then(Value::as_f64)) else {
            not_periods.0 += 1;
            not_periods.1.get_or_insert(record);
            continue;
        };
        let Some(start_time) = parse(start) else { continue };
        let open = record.get("open").and_then(Value::as_bool).unwrap_or(false);
        files.entry(day_path(dir, &device.id, series, start_time.date()))
            .or_default()
            .push((start.to_string(), Saved { end: end.to_string(), value, open }));
        name = field("capturedBy").or(name);
    }
    if let (count, Some(first)) = not_periods {
        crate::log_warning!("{count} of the temperature and humidity records from {} are not periods, e.g. {first}", device.id);
    }
    let mut changed = 0;
    for (path, periods) in files {
        let mut day = read_day(&path);
        for (start, saved) in periods {
            // Unless what is saved already ends later, e.g. records read again.
            let later = day.get(&start).is_none_or(|old| parse(&old.end) < parse(&saved.end) || (old.open && !saved.open));
            if later {
                day.insert(start, saved);
                changed += 1;
            }
        }
        // The profile's directory, the device's and the series', each private, as the profile's other data is.
        for level in [dir.to_path_buf(), device_dir(dir, &device.id), path.parent().unwrap_or(dir).to_path_buf()] {
            if !level.exists() {
                crate::private_file::create_private_dir(&level)?;
            }
        }
        std::fs::write(&path, serde_json::to_vec(&day)?)?;
    }
    if let Some(name) = name {
        std::fs::write(device_dir(dir, &device.id).join("device.json"), serde_json::to_vec(&serde_json::json!({ "id": device.id, "name": name }))?)?;
    }
    Ok(changed)
}

/// The end of the latest period saved of device, to read only those after it.
fn latest_end(dir: &Path, id: &str) -> Option<OffsetDateTime> {
    SERIES.iter()
        .filter_map(|series| {
            let series_dir = device_dir(dir, id).join(series.name);
            let latest = std::fs::read_dir(&series_dir).ok()?
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .filter(|name| name.ends_with(".json"))
                .max()?;
            read_day(&series_dir.join(latest)).values().filter_map(|saved| parse(&saved.end)).max()
        })
        .min()
}

/// The periods of every device which has any, of those saved, which overlap from to to, by its name.
pub fn load(dir: &Path, from: OffsetDateTime, to: OffsetDateTime) -> Vec<DeviceClimate> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut devices: Vec<DeviceClimate> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let named: Value = std::fs::read_to_string(path.join("device.json")).ok().and_then(|text| serde_json::from_str(&text).ok())?;
            let id = named.get("id")?.as_str()?.to_string();
            let name = named.get("name").and_then(Value::as_str).filter(|name| !name.is_empty()).unwrap_or(&id).to_string();
            let series = SERIES.map(|series| load_series(dir, &id, &series, from, to));
            Some(DeviceClimate { id, name, series })
        })
        .filter(|device| device.series.iter().any(|periods| !periods.is_empty()))
        .collect();
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    devices
}

/// A device's periods of a series, of those saved, which overlap from to to, in time order.
pub fn load_series(dir: &Path, id: &str, series: &Series, from: OffsetDateTime, to: OffsetDateTime) -> Vec<Period> {
    // A period which starts the day before may reach into the range.
    let mut day = from.date().previous_day().unwrap_or(from.date());
    let mut periods = Vec::new();
    while day <= to.date() {
        for (start, saved) in read_day(&day_path(dir, id, series.name, day)) {
            let (Some(start), Some(end)) = (parse(&start), parse(&saved.end)) else { continue };
            if end > from && start < to {
                periods.push(Period { start: start.unix_timestamp(), end: end.unix_timestamp(), value: saved.value });
            }
        }
        let Some(next) = day.next_day() else { break };
        day = next;
    }
    periods
}

/// Reads the periods from the profile's capture devices which take them, polled as MODULE_ID, see polling.rs.
pub struct ClimateReader {
    profile_name: String,
    capture: CaptureReader,
}

impl ClimateReader {
    pub fn new(profile_name: &str) -> ClimateReader {
        ClimateReader { profile_name: profile_name.to_string(), capture: CaptureReader::new("capture/climate/records") }
    }
}

#[async_trait::async_trait(?Send)]
impl Module for ClimateReader {
    fn get_page_list(&self) -> Vec<PageInfo> {
        Vec::new()
    }

    fn module_id(&self) -> &'static str {
        MODULE_ID
    }

    fn get_component(&self, _page_id: &str, _path: Vec<String>) -> Element {
        rsx! {}
    }

    fn help_topic(&self, _page_id: &str, _path: &[String]) -> String {
        "climate.md".to_string()
    }

    fn cli_debug(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn fetch_latest_data(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn poll_interval(&self) -> Option<Duration> {
        Some(POLL_INTERVAL)
    }

    fn next_poll(&self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        self.capture.next_read(now, POLL_INTERVAL)
    }

    /// Reads each device's periods since the end of the latest saved of it, its own, as each device's are its own, so
    /// that a device with none, e.g. one without a sensor, does not make the others give all they have, saving them a
    /// device at a time, as reading and writing a day's file for each period would be slow when catching up.
    async fn poll(&self, _slot: Option<OffsetDateTime>) -> anyhow::Result<String> {
        let dir = dir(&self.profile_name)?;
        let devices = crate::capture_device::adopted_devices(&self.profile_name);
        let mut read: Vec<(DeviceProfile, Vec<Value>)> = Vec::new();
        let takes = self.capture.read_each(&devices, &|device| latest_end(&dir, &device.id), &mut |device, record| {
            match read.last_mut() {
                Some((last, records)) if last.id == device.id => records.push(record.clone()),
                _ => read.push((device.clone(), vec![record.clone()])),
            }
            Ok(true)
        }, &mut |_| {}).await?;
        if !takes {
            return Ok("No capture device measures the temperature and humidity".to_string());
        }
        let mut changed = 0;
        let count: usize = read.iter().map(|(_, records)| records.len()).sum();
        for (device, records) in &read {
            changed += save(&dir, device, records)?;
        }
        Ok(format!("Read {count} temperature and humidity periods, {changed} new or longer"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_period_read_again_replaces_what_was_saved() {
        let dir = std::env::temp_dir().join(format!("marco-sparko-climate-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let device = DeviceProfile { id: "44:1B:F6:CC:7F:CC".to_string(), address: String::new(), fingerprint: String::new(), key: String::new() };
        let record = |start: &str, end: &str, value: f64, open: bool| serde_json::json!({
            "series": "temperature", "start": start, "end": end, "value": value, "unit": "°C", "open": open, "capturedBy": "rlcd",
        });
        assert_eq!(save(&dir, &device, &[record("2026-11-04T10:00:00Z", "2026-11-04T10:01:00Z", 23.4, true)]).unwrap(), 1);
        // The open period again, with its end moved on, then closed, and the next one.
        assert_eq!(save(&dir, &device, &[
            record("2026-11-04T10:00:00Z", "2026-11-04T10:02:00Z", 23.4, true),
            record("2026-11-04T10:00:00Z", "2026-11-04T10:02:30Z", 23.4, false),
            record("2026-11-04T10:02:30Z", "2026-11-04T10:02:30Z", 23.5, true),
        ]).unwrap(), 3);
        // Read again, nothing changes.
        assert_eq!(save(&dir, &device, &[record("2026-11-04T10:00:00Z", "2026-11-04T10:02:00Z", 23.4, true)]).unwrap(), 0);
        let at = |text: &str| parse(text).unwrap();
        let devices = load(&dir, at("2026-11-04T00:00:00Z"), at("2026-11-05T00:00:00Z"));
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "rlcd");
        assert_eq!(devices[0].series[0], vec![
            Period { start: at("2026-11-04T10:00:00Z").unix_timestamp(), end: at("2026-11-04T10:02:30Z").unix_timestamp(), value: 23.4 },
            Period { start: at("2026-11-04T10:02:30Z").unix_timestamp(), end: at("2026-11-04T10:02:30Z").unix_timestamp(), value: 23.5 },
        ]);
        assert_eq!(latest_end(&dir, &device.id), Some(at("2026-11-04T10:02:30Z")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
